//! The Extractor (ADRs 0007, 0033 and 0129): its one primitive,
//! `extract.content`, which reads a page's main content, and the driver
//! that calls it under `timeout.plugin` and checks its answer's shape.
//!
//! Everything an Extractor answers is page content (ADR 0014): the driver
//! hands it on as such and never reads an instruction in it.

use futures_util::future::BoxFuture;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::page::CONTENT_EXTRACTOR;
use goodfetch_protocol::plugin::extractor::{
    EXTRACT_CONTENT, EXTRACT_HANDLER_FAILED, EXTRACTOR_UNAVAILABLE, ExtractContentParams,
    ExtractContentResult, ExtractHandlerFailedDetails, ExtractorUnavailableDetails,
};
use url::Url;

use super::Failure;
use crate::call_error::{CallError, named};
use crate::clock::Clock;
use crate::deadline::Deadline;
use crate::plugin_host::instance::{CallFailure, PluginInstance};
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
}

/// The Extractor's driver: it calls one implementation under a deadline
/// and checks what it answers.
pub struct ExtractorDriver<'a> {
    implementation: &'a dyn ExtractPrimitives,
    clock: &'a Clock,
    deadline: &'a Deadline,
}

impl<'a> ExtractorDriver<'a> {
    /// The driver of `implementation`, whose calls wait on `clock` until
    /// `deadline`, which `timeout.plugin` sets.
    #[must_use]
    pub fn new(
        implementation: &'a dyn ExtractPrimitives,
        clock: &'a Clock,
        deadline: &'a Deadline,
    ) -> Self {
        Self {
            implementation,
            clock,
            deadline,
        }
    }

    /// The main content of the page at `url`, read from its document
    /// `html`.
    ///
    /// # Errors
    ///
    /// The call's [`Failure`]: `deadline_exceeded` once `timeout.plugin`
    /// passed first, the Error code the implementation answered when the
    /// catalog knows it, and `extract_handler_failed` for any answer the
    /// driver cannot take.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the call, whose answer, when it comes,
    /// goes to nobody.
    pub async fn content(&self, url: &Url, html: String) -> Result<ExtractContentResult, Failure> {
        let params = ExtractContentParams {
            url: url.as_str().to_owned(),
            html,
        };
        let handler_failed = || self.handler_failed(url);
        let received = self
            .deadline
            .within(
                self.clock,
                WaitedFor::PLUGIN_ANSWER,
                self.implementation.content(&params),
            )
            .await
            .map_err(Failure::DeadlineExceeded)?
            .map_err(|failure| Failure::of(failure, handler_failed))?;
        let result = received.read::<ExtractContentResult>().map_err(|error| {
            tracing::warn!(
                extractor = self.implementation.name(),
                %error,
                "an Extractor's result is not `{{ title, text, markdown, metadata }}`"
            );
            Failure::Malformed(handler_failed())
        })?;
        if let Some(key) = result.metadata.keys().find(|key| !is_snake_case(key)) {
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

/// Whether `key` is a `snake_case` name: a lowercase ASCII letter, then
/// lowercase ASCII letters, digits and underscores.
fn is_snake_case(key: &str) -> bool {
    let mut characters = key.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase())
        && characters.all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
        })
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::is_snake_case;

    #[rstest]
    #[case::one_word("author", true)]
    #[case::words("published_at", true)]
    #[case::digit("h1_title", true)]
    #[case::empty("", false)]
    #[case::upper("Author", false)]
    #[case::camel("publishedAt", false)]
    #[case::leading_digit("1st", false)]
    #[case::leading_underscore("_author", false)]
    #[case::dash("published-at", false)]
    fn a_metadata_key_is_snake_case(#[case] key: &str, #[case] expected: bool) {
        assert_eq!(is_snake_case(key), expected);
    }
}
