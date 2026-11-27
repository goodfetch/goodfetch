//! The Secret provider (ADRs 0015, 0033, 0092 and 0129): its one primitive,
//! `secret.resolve`, which resolves a Secret reference under the scheme the
//! Plugin's manifest declares, and the driver that calls it under
//! `timeout.secret`, wraps its answer immediately in a zeroizing
//! `SecretString`, accounts for the call to the Run, and maps any failure to
//! `ProviderFailure` without ever logging or returning the Plugin's raw text.

use futures_util::future::BoxFuture;
use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::plugin::secret::{
    SECRET_RESOLVE, SecretResolveParams, SecretResolveResult,
};
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::{SECRET_UNAVAILABLE, TIMEOUT_SECRET};
use secrecy::SecretString;

use super::{Accounts, PluginCall, Primitive};
use crate::clock::Clock;
use crate::deadline::Deadline;
use crate::plugin_host::instance::{CallFailure, CallOptions, PluginInstance};
use crate::secrets::ProviderFailure;
use crate::settings::Resolved;
use crate::wire::Received;

/// The deadline for a `secret.resolve` call under `settings` (or
/// `timeout.secret`'s catalog default when `settings` is `None`).
#[must_use]
pub fn secret_deadline(settings: Option<&Resolved>) -> Deadline {
    Deadline::of_resolved(settings, &TIMEOUT_SECRET)
}

/// What every Secret provider implementation fills: the one primitive of its
/// kind, answered without any of the driver's checks.
pub trait SecretPrimitives: Send + Sync {
    /// Its name, as its manifest names it.
    fn name(&self) -> &str;

    /// Answers `secret.resolve` for `params`.
    fn resolve<'call>(
        &'call self,
        params: &'call SecretResolveParams,
    ) -> BoxFuture<'call, Result<Received, CallFailure>>;

    /// Answers `secret.resolve` for `params` under `deadline` directly when
    /// the implementation enforces deadlines itself (such as a process Plugin
    /// sending `$/cancel`). Defaults to delegating to [`Self::resolve`] while
    /// the driver enforces `deadline`.
    fn resolve_under<'call>(
        &'call self,
        params: &'call SecretResolveParams,
        deadline: &'call Deadline,
    ) -> Option<BoxFuture<'call, Result<Received, CallFailure>>> {
        let _ = (params, deadline);
        None
    }
}

/// A Secret provider a process Plugin implements, through its instance.
#[derive(Clone, Debug)]
pub struct ProcessSecretProvider {
    name: String,
    instance: PluginInstance,
}

impl ProcessSecretProvider {
    /// The Secret provider `name`, which `instance` answers for.
    #[must_use]
    pub fn new(name: String, instance: PluginInstance) -> Self {
        Self { name, instance }
    }

    /// Its Plugin name.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl SecretPrimitives for ProcessSecretProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn resolve<'call>(
        &'call self,
        params: &'call SecretResolveParams,
    ) -> BoxFuture<'call, Result<Received, CallFailure>> {
        Box::pin(self.instance.call(SECRET_RESOLVE, params))
    }

    fn resolve_under<'call>(
        &'call self,
        params: &'call SecretResolveParams,
        deadline: &'call Deadline,
    ) -> Option<BoxFuture<'call, Result<Received, CallFailure>>> {
        Some(Box::pin(self.instance.call_under(
            SECRET_RESOLVE,
            params,
            CallOptions::read(deadline.clone()),
        )))
    }
}

/// The Secret provider's driver: it calls one implementation under
/// `timeout.secret`, wraps its answer immediately in a `SecretString`,
/// accounts for the call (recording only the plugin, method, class, time and
/// outcome, never the answer), and translates any failure into
/// [`ProviderFailure`] without logging or returning raw Plugin text.
pub struct SecretProviderDriver<'a> {
    implementation: &'a dyn SecretPrimitives,
    accounts: &'a dyn Accounts,
    clock: &'a Clock,
    deadline: &'a Deadline,
}

impl<'a> SecretProviderDriver<'a> {
    /// The driver of `implementation`, whose calls go to `accounts`, the
    /// Run's, and wait on `clock` until `deadline`, which `timeout.secret`
    /// sets.
    #[must_use]
    pub fn new(
        implementation: &'a dyn SecretPrimitives,
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

    /// Resolves `reference` through the Secret provider. The call is charged
    /// before it goes and recorded once it ended, however it ended
    /// ([`Accounts`]). Neither the answer nor any raw Plugin text is ever
    /// logged or returned in an error.
    ///
    /// # Errors
    ///
    /// [`ProviderFailure::Refused`] when the call is refused by Usage, fails
    /// in the provider, times out, returns an unexpected shape, or when the
    /// Plugin cannot be reached.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future abandons the call; a call dropped once it went
    /// stays charged, with no time and no line in the Run trace.
    pub async fn resolve(
        &self,
        reference: &SecretReference,
    ) -> Result<SecretString, ProviderFailure> {
        let call = PluginCall::of(self.implementation.name(), Primitive::SecretResolve);
        if let Err(refused) = self.accounts.call_starts(&call).await {
            return Err(ProviderFailure::Refused {
                fix: format!(
                    "The Run's Usage refused calling Secret provider `{}` ({}); raise the Limit or wait for its next period.",
                    self.implementation.name(),
                    refused.code(),
                ),
            });
        }
        let started = self.clock.now();
        let outcome = self.ask(reference).await;
        let failed = outcome.as_ref().err().map(|_| SECRET_UNAVAILABLE.name);
        let ended = call.ended(self.clock.now().duration_since(started), failed);
        match (outcome, self.accounts.call_ended(&ended).await) {
            (Ok(_), Err(refused)) => Err(ProviderFailure::Refused {
                fix: format!(
                    "The Run's Usage refused the time taken by Secret provider `{}` ({}); raise the Limit or wait for its next period.",
                    self.implementation.name(),
                    refused.code(),
                ),
            }),
            (outcome, _) => outcome,
        }
    }

    /// Sends `secret.resolve` and parses `{ value }` into a `SecretString`,
    /// never logging or returning any raw Plugin text.
    async fn ask(&self, reference: &SecretReference) -> Result<SecretString, ProviderFailure> {
        let plugin = self.implementation.name();
        let params = SecretResolveParams {
            reference: reference.as_str().to_owned(),
        };
        let answer = if let Some(under) = self.implementation.resolve_under(&params, self.deadline)
        {
            under.await
        } else {
            let Ok(answer) = self
                .deadline
                .within(
                    self.clock,
                    WaitedFor::PLUGIN_ANSWER,
                    self.implementation.resolve(&params),
                )
                .await
            else {
                tracing::warn!(
                    plugin,
                    reference = reference.as_str(),
                    "a Secret provider Plugin timed out"
                );
                return Err(ProviderFailure::Refused {
                    fix: format!(
                        "The Secret provider Plugin `{plugin}` gave no answer in time for `{reference}`; check the Plugin or raise `timeout.secret`."
                    ),
                });
            };
            answer
        };
        let received = match answer {
            Ok(received) => received,
            Err(failure) => {
                let kind = match failure {
                    CallFailure::Answered(_) => "answered_error",
                    CallFailure::Malformed(_) => "malformed",
                    CallFailure::DeadlineExceeded(_) => "deadline_exceeded",
                    CallFailure::Refused(_) => "refused",
                    CallFailure::Unavailable(_) => "unavailable",
                };
                tracing::warn!(
                    plugin,
                    reference = reference.as_str(),
                    failure = kind,
                    "a Secret provider Plugin failed to resolve a reference"
                );
                return Err(ProviderFailure::Refused {
                    fix: format!(
                        "The Secret provider Plugin `{plugin}` failed to resolve `{reference}` ({kind}); check the Plugin and its configuration."
                    ),
                });
            }
        };
        let result = received.read::<SecretResolveResult>().map_err(|_| {
            tracing::warn!(
                plugin,
                reference = reference.as_str(),
                "a Secret provider Plugin's answer is not `{{ value }}`"
            );
            ProviderFailure::Refused {
                fix: format!(
                    "The Secret provider Plugin `{plugin}` returned an answer for `{reference}` that is not `{{ value }}`; fix or update the Plugin."
                ),
            }
        })?;
        Ok(SecretString::from(result.value))
    }
}
