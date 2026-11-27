//! Run-backed process-Plugin [`SecretProvider`] (ADRs 0015, 0033 and 0092,
//! issue 996): resolves `plugin:<scheme>:<rest>` references for a live Run by
//! starting the installed Secret provider Plugin claiming `<scheme>` through
//! [`RunPlugins::secret_provider`], calling `secret.resolve` through
//! [`SecretProviderDriver`], accounting `plugin_calls` and `plugin_time` and
//! recording only plugin/method/class/time/outcome in `trace/plugin-calls.jsonl`.

use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_protocol::connection::INTERNAL;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::usage::{PLUGIN_CALLS, PLUGIN_TIME};

use super::plugin_spend::{limit_not_understood, limit_reached};
use crate::call_error::CallError;
use crate::ledger::{ChargeError, Refusal, RunCharging};
use crate::plugin_kinds::run_plugins::{RunPlugins, SecretStartError, StartError};
use crate::plugin_kinds::secret::{SecretProviderDriver, secret_deadline};
use crate::plugin_kinds::{Accounts, PluginCall};
use crate::run_identity::RunIdentity;
use crate::run_trace::RunTrace;
use crate::sandbox_probe::SandboxProbe;
use crate::secrets::{ConfigProblem, ProviderFailure, Resolving, RunSecrets, SecretProvider};
use crate::settings::RunSettings;

/// Resolves `plugin:<scheme>:<rest>` Secret references through the live Run's
/// installed Secret provider Plugins.
pub(super) struct PluginSecretResolver {
    plugins: Arc<RunPlugins>,
    identity: RunIdentity,
    charging: RunCharging,
    trace: RunTrace,
    env_secrets: RunSecrets,
    probe: SandboxProbe,
}

impl PluginSecretResolver {
    /// Creates the resolver for one live Run. `env_secrets` must not carry a
    /// plugin Secret provider, so a Secret provider Plugin's own environment
    /// cannot recurse into `plugin:` references.
    #[must_use]
    pub(super) fn new(
        plugins: Arc<RunPlugins>,
        identity: RunIdentity,
        charging: RunCharging,
        trace: RunTrace,
        env_secrets: RunSecrets,
        probe: SandboxProbe,
    ) -> Self {
        Self {
            plugins,
            identity,
            charging,
            trace,
            env_secrets,
            probe,
        }
    }
}

impl SecretProvider for PluginSecretResolver {
    fn resolve<'a>(&'a self, reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async move {
            let Some(scheme) = reference.plugin_scheme() else {
                return Err(ProviderFailure::NotConfigured(ConfigProblem {
                    fix: format!(
                        "The reference `{reference}` does not name a Secret provider Plugin scheme (`plugin:<scheme>:<rest>`)."
                    ),
                }));
            };
            let settings = self.identity.settings().map(RunSettings::resolved);
            let provider = self
                .plugins
                .secret_provider(scheme, settings, &self.env_secrets, &self.probe)
                .await
                .map_err(|error| map_start_error(scheme, error))?;
            let accounts = SecretCallAccounts {
                identity: &self.identity,
                charging: &self.charging,
                trace: &self.trace,
            };
            let clock = self.identity.clock();
            let deadline = secret_deadline(settings);
            SecretProviderDriver::new(&provider, &accounts, &clock, &deadline)
                .resolve(reference)
                .await
        })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

/// Translates a Secret provider Plugin start failure into a
/// [`ProviderFailure`] without exposing any secret values.
fn map_start_error(scheme: &str, error: SecretStartError) -> ProviderFailure {
    match error {
        SecretStartError::NoScheme(_) => ProviderFailure::NotConfigured(ConfigProblem {
            fix: format!(
                "No installed Secret provider Plugin declares the scheme `{scheme}`; install one with `plugin add` or use another Secret reference."
            ),
        }),
        SecretStartError::SchemeConflict { existing, .. } => {
            ProviderFailure::NotConfigured(ConfigProblem {
                fix: format!(
                    "Multiple installed Secret provider Plugins claim the scheme `{scheme}` ({existing}); remove the conflicting Plugin."
                ),
            })
        }
        SecretStartError::Start(StartError::Refused(call_error)) => ProviderFailure::Refused {
            fix: format!(
                "The Secret provider Plugin for scheme `{scheme}` could not start ({}): check the Plugin's approval, settings and confinement.",
                call_error.code(),
            ),
        },
        SecretStartError::Start(StartError::Secret(secret_error)) => ProviderFailure::Refused {
            fix: format!(
                "A Secret needed to start the Secret provider Plugin for scheme `{scheme}` could not be resolved ({secret_error})."
            ),
        },
    }
}

/// Accounts `secret.resolve` calls against the live Run's charging and Run
/// trace (recording only plugin, kind, method, class, time and outcome).
struct SecretCallAccounts<'a> {
    identity: &'a RunIdentity,
    charging: &'a RunCharging,
    trace: &'a RunTrace,
}

impl Accounts for SecretCallAccounts<'_> {
    fn call_starts<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> BoxFuture<'call, Result<(), CallError>> {
        Box::pin(async move { charge_part(self.charging, PLUGIN_CALLS, call.plugin, 1).await })
    }

    fn call_ended<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> BoxFuture<'call, Result<(), CallError>> {
        Box::pin(async move {
            let charged = charge_part(self.charging, PLUGIN_TIME, call.plugin, call.time).await;
            if let Err(error) = self.trace.plugin_call(call).await {
                tracing::warn!(
                    run_id = %self.identity.id(),
                    plugin = call.plugin,
                    method = call.method,
                    %error,
                    "a Secret provider Plugin call's line of the Run trace cannot be written"
                );
            }
            charged
        })
    }
}

async fn charge_part(
    charging: &RunCharging,
    usage_kind: &str,
    plugin: &str,
    amount: u64,
) -> Result<(), CallError> {
    charging
        .charge_part(usage_kind, plugin, amount, None)
        .await
        .map_err(|error| match error {
            ChargeError::Refused(Refusal::Reached(reached)) => limit_reached(&reached),
            ChargeError::Refused(Refusal::NotUnderstood(limit)) => limit_not_understood(*limit),
            other => CallError::new(
                &INTERNAL,
                format!("Secret provider Plugin usage accounting failed: {other}"),
                &(),
            ),
        })
}
