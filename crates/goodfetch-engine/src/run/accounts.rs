//! How a Run accounts for the calls Plugin kinds' drivers make for it
//! (ADR 0033): each call charges one `plugin_calls` before it goes and its
//! `plugin_time` once it ended, both under the implementation's name as
//! their part of the kind, through the Run's charging (ADR 0145), and
//! writes one line of the Run trace's Plugin call log through the Run's one
//! writer. A charge the Run's Usage refuses fails the call with the Error
//! code a refused charge answers everywhere; a line the trace cannot write
//! is logged at `warn` and fails nothing.

use futures_util::future::BoxFuture;
use goodfetch_protocol::usage::{PLUGIN_CALLS, PLUGIN_TIME};

use super::{limit_not_understood, limit_reached, state_unavailable};
use crate::call_error::CallError;
use crate::ledger::{ChargeError, Refusal, RunCharging};
use crate::plugin_kinds::{Accounts, PluginCall};
use crate::run_identity::RunIdentity;
use crate::run_trace::RunTrace;
use crate::runs::{self, Run};

/// What a Run's Plugin calls are accounted to: its identity, its charging
/// and its Run trace, each the Run's own (ADR 0145).
#[derive(Clone, Copy)]
pub struct RunAccounts<'run> {
    /// Which Run it is.
    pub identity: &'run RunIdentity,
    /// Its charging, which each call is charged to.
    pub charging: &'run RunCharging,
    /// Its Run trace, whose Plugin call log records each call.
    pub trace: &'run RunTrace,
}

impl Accounts for RunAccounts<'_> {
    fn call_starts<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> BoxFuture<'call, Result<(), CallError>> {
        Box::pin(async move { charged(self, PLUGIN_CALLS, call.plugin, 1).await })
    }

    fn call_ended<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> BoxFuture<'call, Result<(), CallError>> {
        Box::pin(async move {
            let charged = charged(self, PLUGIN_TIME, call.plugin, call.time).await;
            if let Err(error) = self.trace.plugin_call(call).await {
                tracing::warn!(
                    run_id = %self.identity.id(),
                    plugin = call.plugin,
                    method = call.method,
                    %error,
                    "a Plugin call's line of the Run trace cannot be written"
                );
            }
            charged
        })
    }
}

impl Accounts for Run {
    fn call_starts<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> BoxFuture<'call, Result<(), CallError>> {
        Box::pin(async move { accounts_of(self).call_starts(call).await })
    }

    fn call_ended<'call>(
        &'call self,
        call: &'call PluginCall<'call>,
    ) -> BoxFuture<'call, Result<(), CallError>> {
        Box::pin(async move { accounts_of(self).call_ended(call).await })
    }
}

/// What `run`'s Plugin calls are accounted to.
fn accounts_of(run: &Run) -> RunAccounts<'_> {
    RunAccounts {
        identity: runs::identity(run),
        charging: runs::charging(run),
        trace: runs::run_trace(run),
    }
}

/// Charges `amount` of `usage_kind` to the Run `accounts` are of, under the
/// part `plugin`, the implementation's name, read into the error a refused
/// charge answers.
async fn charged(
    accounts: &RunAccounts<'_>,
    usage_kind: &str,
    plugin: &str,
    amount: u64,
) -> Result<(), CallError> {
    accounts
        .charging
        .charge_part(usage_kind, plugin, amount, None)
        .await
        .map_err(|error| match error {
            ChargeError::Refused(Refusal::Reached(reached)) => limit_reached(&reached),
            ChargeError::Refused(Refusal::NotUnderstood(limit)) => limit_not_understood(*limit),
            error => state_unavailable(
                accounts.identity.id().clone(),
                accounts.identity.folder(),
                &error.to_string(),
            ),
        })
}
