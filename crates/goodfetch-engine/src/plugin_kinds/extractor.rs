//! The Extractor (ADRs 0007, 0033 and 0129): its one primitive,
//! `extract.content`, which reads a page's main content, and the driver
//! that calls it under `timeout.plugin`, checks its answer's shape and
//! accounts for it to the Run.
//!
//! Everything an Extractor answers is page content (ADR 0014): the driver
//! hands it on as such and never reads an instruction in it.

use futures_util::future::BoxFuture;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::page::CONTENT_EXTRACTOR;
use goodfetch_protocol::plugin::extractor::{
    EXTRACT_CONTENT, EXTRACT_HANDLER_FAILED, EXTRACTOR_UNAVAILABLE, ExtractContentParams,
    ExtractContentResult, ExtractHandlerFailedDetails, ExtractorUnavailableDetails,
    is_metadata_key,
};
use url::Url;

use super::{Accounts, Failure, PluginCall, Primitive};
use crate::call_error::{CallError, named};
use crate::client::{Client, ClientRawError};
use crate::clock::Clock;
use crate::deadline::Deadline;
use crate::plugin_host::instance::{CallFailure, CallOptions, PluginInstance};
use crate::wire::Received;

/// What every Extractor implementation fills: the one primitive of its
/// kind, answered without any of the driver's checks.
pub trait ExtractPrimitives: Send + Sync {
    /// Its name, as `content.extractor` names it.
    fn name(&self) -> &str;

    /// Answers `extract.content` for `params`.
    fn content<'call>(
        &'call self,
        params: &'call ExtractContentParams,
    ) -> BoxFuture<'call, Result<Received, CallFailure>>;

    /// Answers `extract.content` for `params` under `deadline` directly
    /// when the implementation enforces deadlines itself (such as a process
    /// Plugin sending `$/cancel`). Defaults to delegating to
    /// [`Self::content`] while the driver enforces `deadline`.
    fn content_under<'call>(
        &'call self,
        params: &'call ExtractContentParams,
        deadline: &'call Deadline,
    ) -> Option<BoxFuture<'call, Result<Received, CallFailure>>> {
        let _ = (params, deadline);
        None
    }
}

/// An Extractor a process Plugin implements, through its instance.
#[derive(Clone, Debug)]
pub struct ProcessExtractor {
    name: String,
    instance: PluginInstance,
}

impl ProcessExtractor {
    /// The Extractor `name`, which `instance` answers for.
    #[must_use]
    pub fn new(name: String, instance: PluginInstance) -> Self {
        Self { name, instance }
    }
}

impl ExtractPrimitives for ProcessExtractor {
    fn name(&self) -> &str {
        &self.name
    }

    fn content<'call>(
        &'call self,
        params: &'call ExtractContentParams,
    ) -> BoxFuture<'call, Result<Received, CallFailure>> {
        Box::pin(self.instance.call(EXTRACT_CONTENT, params))
    }

    fn content_under<'call>(
        &'call self,
        params: &'call ExtractContentParams,
        deadline: &'call Deadline,
    ) -> Option<BoxFuture<'call, Result<Received, CallFailure>>> {
        Some(Box::pin(self.instance.call_under(
            EXTRACT_CONTENT,
            params,
            CallOptions::read(deadline.clone()),
        )))
    }
}

/// An Extractor implemented by an SDK handler on the Run's owning Connection
/// (ADR 0104): sends `extract.content` with `handler` naming it, cancels with
/// `$/cancel` if the deadline passes, and returns the raw reply without
/// copying (ADR 0140's E1).
#[derive(Clone, Debug)]
pub struct SdkExtractor {
    name: String,
    client: Client,
    clock: Clock,
}

impl SdkExtractor {
    /// The SDK-handler Extractor `name` served by `client`, timed on `clock`.
    #[must_use]
    pub fn new(name: String, client: Client, clock: Clock) -> Self {
        Self {
            name,
            client,
            clock,
        }
    }

    async fn call_client(
        &self,
        params: &ExtractContentParams,
        deadline: Option<&Deadline>,
    ) -> Result<Received, CallFailure> {
        let wire_params = ExtractContentParams {
            url: params.url.clone(),
            html: params.html.clone(),
            handler: Some(self.name.clone()),
        };
        let mut sent = self
            .client
            .send_raw(EXTRACT_CONTENT, &wire_params)
            .await
            .map_err(|_| CallFailure::Unavailable(unavailable(&self.name)))?;
        let outcome = if let Some(deadline) = deadline {
            match deadline
                .within(&self.clock, WaitedFor::PLUGIN_ANSWER, sent.answer())
                .await
            {
                Ok(outcome) => outcome,
                Err(exceeded) => {
                    sent.cancel().await;
                    return Err(CallFailure::DeadlineExceeded(exceeded));
                }
            }
        } else {
            sent.answer().await
        };
        match outcome {
            Ok(received) => Ok(received),
            Err(ClientRawError::Gone) => Err(CallFailure::Unavailable(unavailable(&self.name))),
            Err(ClientRawError::Answered(mut error)) => {
                if error.data.code == EXTRACT_HANDLER_FAILED.name {
                    error
                        .data
                        .details
                        .entry("extractor".to_owned())
                        .or_insert_with(|| serde_json::Value::String(self.name.clone()));
                    error
                        .data
                        .details
                        .entry("handler".to_owned())
                        .or_insert_with(|| serde_json::Value::String(self.name.clone()));
                    error
                        .data
                        .details
                        .entry("url".to_owned())
                        .or_insert_with(|| serde_json::Value::String(params.url.clone()));
                }
                Err(CallFailure::Answered(error))
            }
        }
    }
}

impl ExtractPrimitives for SdkExtractor {
    fn name(&self) -> &str {
        &self.name
    }

    fn content<'call>(
        &'call self,
        params: &'call ExtractContentParams,
    ) -> BoxFuture<'call, Result<Received, CallFailure>> {
        Box::pin(self.call_client(params, None))
    }

    fn content_under<'call>(
        &'call self,
        params: &'call ExtractContentParams,
        deadline: &'call Deadline,
    ) -> Option<BoxFuture<'call, Result<Received, CallFailure>>> {
        Some(Box::pin(self.call_client(params, Some(deadline))))
    }
}

/// An Extractor resolved for a Run: either a confined process Plugin or an
/// unconfined SDK handler on the Run's owning Connection.
#[derive(Clone, Debug)]
pub enum RunExtractor {
    /// A confined process Plugin instance.
    Process(ProcessExtractor),
    /// An unconfined SDK handler served by the Run's owning Client.
    Sdk(SdkExtractor),
}

impl RunExtractor {
    /// Whether this Extractor runs as an unconfined SDK handler.
    #[must_use]
    pub const fn is_sdk_handler(&self) -> bool {
        matches!(self, Self::Sdk(_))
    }
}

impl ExtractPrimitives for RunExtractor {
    fn name(&self) -> &str {
        match self {
            Self::Process(extractor) => extractor.name(),
            Self::Sdk(extractor) => extractor.name(),
        }
    }

    fn content<'call>(
        &'call self,
        params: &'call ExtractContentParams,
    ) -> BoxFuture<'call, Result<Received, CallFailure>> {
        match self {
            Self::Process(extractor) => extractor.content(params),
            Self::Sdk(extractor) => extractor.content(params),
        }
    }

    fn content_under<'call>(
        &'call self,
        params: &'call ExtractContentParams,
        deadline: &'call Deadline,
    ) -> Option<BoxFuture<'call, Result<Received, CallFailure>>> {
        match self {
            Self::Process(extractor) => extractor.content_under(params, deadline),
            Self::Sdk(extractor) => extractor.content_under(params, deadline),
        }
    }
}

/// The Extractor's driver: it calls one implementation under a deadline,
/// checks what it answers and accounts for the call.
pub struct ExtractorDriver<'a> {
    implementation: &'a dyn ExtractPrimitives,
    accounts: &'a dyn Accounts,
    clock: &'a Clock,
    deadline: &'a Deadline,
}

impl<'a> ExtractorDriver<'a> {
    /// The driver of `implementation`, whose calls go to `accounts`, the
    /// Run's, and wait on `clock` until `deadline`, which `timeout.plugin`
    /// sets.
    #[must_use]
    pub fn new(
        implementation: &'a dyn ExtractPrimitives,
        accounts: &'a dyn Accounts,
        clock: &'a Clock,
        deadline: &'a Deadline,
    ) -> Self {
        Self {
            implementation,
            accounts,
            clock,
            deadline,
        }
    }

    /// The main content of the page at `url`, read from its document
    /// `html`. The call is charged before it goes and recorded once it
    /// ended, however it ended ([`Accounts`]).
    ///
    /// # Errors
    ///
    /// The call's [`Failure`]: `limit_reached` when the Run's Usage refused
    /// it, `deadline_exceeded` once `timeout.plugin` passed first, the Error
    /// code the implementation answered when the catalog knows it, and
    /// `extract_handler_failed` for any answer the driver cannot take.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the call, whose answer, when it comes,
    /// goes to nobody; a call dropped once it went stays charged, with no
    /// time and no line in the Run trace.
    pub async fn content(&self, url: &Url, html: String) -> Result<ExtractContentResult, Failure> {
        let call = PluginCall::of(self.implementation.name(), Primitive::ExtractContent);
        self.accounts
            .call_starts(&call)
            .await
            .map_err(Failure::Refused)?;
        let started = self.clock.now();
        let read = self.read(url, html).await;
        let failed = read.as_ref().err().map(|failure| failure.error().code());
        let ended = call.ended(self.clock.now().duration_since(started), failed);
        match (read, self.accounts.call_ended(&ended).await) {
            (Ok(_), Err(refused)) => Err(Failure::Refused(refused)),
            (read, _) => read,
        }
    }

    /// What the implementation answers for the page at `url`, as the
    /// driver reads and checks it.
    async fn read(&self, url: &Url, html: String) -> Result<ExtractContentResult, Failure> {
        let params = ExtractContentParams {
            url: url.as_str().to_owned(),
            html,
            handler: None,
        };
        let handler_failed = || self.handler_failed(url);
        let received =
            if let Some(under) = self.implementation.content_under(&params, self.deadline) {
                under
                    .await
                    .map_err(|failure| Failure::of(failure, handler_failed))?
            } else {
                self.deadline
                    .within(
                        self.clock,
                        WaitedFor::PLUGIN_ANSWER,
                        self.implementation.content(&params),
                    )
                    .await
                    .map_err(Failure::DeadlineExceeded)?
                    .map_err(|failure| Failure::of(failure, handler_failed))?
            };
        let result = received.read::<ExtractContentResult>().map_err(|error| {
            tracing::warn!(
                extractor = self.implementation.name(),
                %error,
                "an Extractor's result is not `{{ title, text, markdown, metadata }}`"
            );
            Failure::Malformed(handler_failed())
        })?;
        if let Some(key) = result.metadata.keys().find(|key| !is_metadata_key(key)) {
            tracing::warn!(
                extractor = self.implementation.name(),
                key,
                "an Extractor's metadata holds a key that is not snake_case"
            );
            return Err(Failure::Malformed(handler_failed()));
        }
        Ok(result)
    }

    /// The `extract_handler_failed` of a call for the page at `url`, which
    /// names the Extractor and never holds what it answered.
    fn handler_failed(&self, url: &Url) -> CallError {
        let extractor = self.implementation.name();
        CallError::new(
            &EXTRACT_HANDLER_FAILED,
            format!(
                "The Extractor `{extractor}` that {} names answered what the engine cannot take; its answer is in the log at `warn`.",
                named::setting(CONTENT_EXTRACTOR.name),
            ),
            &ExtractHandlerFailedDetails {
                extractor: extractor.to_owned(),
                url: url.as_str().to_owned(),
                handler: None,
                exception_type: None,
                exception_message: None,
                exception_stack: None,
            },
        )
    }
}

/// The `extractor_unavailable` of the Extractor `extractor`, which no
/// built-in and no installed Plugin of the Extractor kind answers for, or
/// whose Plugin cannot start. It names the Extractor, never a path.
pub fn unavailable(extractor: &str) -> CallError {
    CallError::new(
        &EXTRACTOR_UNAVAILABLE,
        format!(
            "No one answers for the Extractor `{extractor}` that {} names: no built-in has its name, and no installed Plugin of the Extractor kind does or its Plugin cannot start; why is in the log at `warn`.",
            named::setting(CONTENT_EXTRACTOR.name),
        ),
        &ExtractorUnavailableDetails {
            extractor: extractor.to_owned(),
        },
    )
}
