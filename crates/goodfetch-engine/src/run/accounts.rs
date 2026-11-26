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
use crate::ledger::{ChargeError, Refusal};
use crate::plugin_kinds::{Accounts, PluginCall};
use crate::runs::{self, Run};

impl Accounts for Run {
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
            if let Err(error) = runs::run_trace(self).plugin_call(call).await {
                tracing::warn!(
                    run_id = %self.id(),
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

/// Charges `amount` of `usage_kind` to `run` under the part `plugin`, the
/// implementation's name, read into the error a refused charge answers.
async fn charged(run: &Run, usage_kind: &str, plugin: &str, amount: u64) -> Result<(), CallError> {
    runs::charging(run)
        .charge_part(usage_kind, plugin, amount, None)
        .await
        .map_err(|error| match error {
            ChargeError::Refused(Refusal::Reached(reached)) => limit_reached(&reached),
            ChargeError::Refused(Refusal::NotUnderstood(limit)) => limit_not_understood(*limit),
            error => state_unavailable(run.id().clone(), run.folder(), &error.to_string()),
        })
}
