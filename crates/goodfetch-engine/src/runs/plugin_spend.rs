//! Run-backed implementation of [`SpendAccountant`] (ADR 0033, issue 940):
//! connects process Plugin spend reservations and `usage.report` settlements
//! to a live Run's [`RunCharging`] and [`ReportWarnings`], and owns the
//! canonical `limit_reached` / `limit_not_understood` error builders.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_protocol::connection::INTERNAL;
use goodfetch_protocol::run::warning::{PLUGIN_FIGURES, PLUGIN_QUANTITY_ABOVE_MAXIMUM};
use goodfetch_protocol::run::{ReachedLimit, RunWarning};
use goodfetch_protocol::usage::{
    LIMIT_NOT_UNDERSTOOD, LIMIT_REACHED, Limit, LimitNotUnderstoodDetails, PLUGIN_STREAM_BYTES,
    PLUGIN_STREAM_TIME, PLUGIN_UNITS,
};

use super::report::ReportWarnings;
use crate::call_error::CallError;
use crate::ledger::{
    ChargeError, Counted, PluginUnitOutcome, PluginUnitReservation, PluginUnitReserve, Refusal,
    RunCharging, plugin_unit_qty_part,
};
use crate::plugin_kinds::{
    DeclaredUnit, ReportedSpend, SpendAccountant, SpendTicket, StreamAccountant, StreamRecord,
    plugin_price_unknown,
};
use crate::run_trace::RunTrace;

/// `limit_reached`, naming the Limit's scope, Usage kind and period.
pub fn limit_reached(reached: &ReachedLimit) -> CallError {
    let period = reached
        .period
        .as_ref()
        .map(|period| format!(" this {} ({})", period.every.as_str(), period.time_zone))
        .unwrap_or_default();
    CallError::new(
        &LIMIT_REACHED,
        format!(
            "The {} Limit on {} has nothing left{period}; wait for its next period, or ask a person to raise it.",
            reached.scope.kind, reached.usage_kind
        ),
        reached,
    )
}

/// `limit_not_understood`, naming the Limit.
pub fn limit_not_understood(limit: Limit) -> CallError {
    CallError::new(
        &LIMIT_NOT_UNDERSTOOD,
        format!(
            "A newer engine set a {} Limit on {} that this engine cannot evaluate; upgrade it.",
            limit.scope.kind, limit.usage_kind
        ),
        &LimitNotUnderstoodDetails { limit },
    )
}

/// Live-Run spend accountant backed by [`RunCharging`] and [`ReportWarnings`].
#[derive(Clone)]
pub struct RunSpendAccountant {
    charging: RunCharging,
    warnings: ReportWarnings,
}

impl fmt::Debug for RunSpendAccountant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunSpendAccountant")
            .finish_non_exhaustive()
    }
}

impl RunSpendAccountant {
    /// Creates an accountant backed by `charging` and `warnings`.
    #[must_use]
    pub fn new(charging: RunCharging, warnings: ReportWarnings) -> Arc<Self> {
        Arc::new(Self { charging, warnings })
    }
}

impl SpendAccountant for RunSpendAccountant {
    fn reserve<'call>(
        &'call self,
        plugin: &'call str,
        units: Vec<DeclaredUnit>,
    ) -> BoxFuture<'call, Result<Box<dyn SpendTicket>, CallError>> {
        Box::pin(async move {
            let reserve_units: Vec<PluginUnitReserve> = units
                .iter()
                .map(|entry| PluginUnitReserve {
                    unit: entry.unit.clone(),
                    max_per_call: entry.max_per_call,
                    price: entry.price.clone(),
                })
                .collect();
            let reserved = self
                .charging
                .reserve_plugin_spend(plugin, reserve_units)
                .await
                .map_err(charge_error)?;
            match reserved {
                Ok(reservations) => Ok(Box::new(RunSpendTicket {
                    plugin: plugin.to_owned(),
                    units,
                    reservations,
                    charging: self.charging.clone(),
                    warnings: self.warnings.clone(),
                }) as Box<dyn SpendTicket>),
                Err(unit) => Err(plugin_price_unknown(plugin, &unit)),
            }
        })
    }
}

struct RunSpendTicket {
    plugin: String,
    units: Vec<DeclaredUnit>,
    reservations: Vec<PluginUnitReservation>,
    charging: RunCharging,
    warnings: ReportWarnings,
}

impl SpendTicket for RunSpendTicket {
    fn cancel(self: Box<Self>) -> BoxFuture<'static, ()> {
        Box::pin(async move {
            if let Err(error) = self
                .charging
                .cancel_plugin_spend(&self.plugin, self.reservations)
                .await
            {
                tracing::warn!(
                    plugin = %self.plugin,
                    %error,
                    "a cancelled Plugin call's spend reservation could not be refunded"
                );
            }
        })
    }

    fn finish(
        self: Box<Self>,
        answered: bool,
        reported: ReportedSpend,
    ) -> BoxFuture<'static, Result<(), CallError>> {
        Box::pin(async move {
            if reported.any
                && let Err(error) = self
                    .warnings
                    .warn_once(RunWarning {
                        code: PLUGIN_FIGURES.to_owned(),
                        message: "Usage figures come from a Plugin.".to_owned(),
                        sessions: Vec::new(),
                        folder: None,
                    })
                    .await
            {
                tracing::warn!(
                    plugin = %self.plugin,
                    %error,
                    "could not record `plugin_figures` warning"
                );
            }

            for unit in &self.units {
                let quantity = reported.units.get(&unit.unit).copied().unwrap_or(0);
                if quantity > unit.max_per_call {
                    let message = format!(
                        "Plugin `{}` reported {} `{}` in one call, above its declared maximum of {}.",
                        self.plugin, quantity, unit.unit, unit.max_per_call
                    );
                    if let Err(error) = self
                        .warnings
                        .warn(RunWarning {
                            code: PLUGIN_QUANTITY_ABOVE_MAXIMUM.to_owned(),
                            message,
                            sessions: Vec::new(),
                            folder: None,
                        })
                        .await
                    {
                        tracing::warn!(
                            plugin = %self.plugin,
                            unit = %unit.unit,
                            %error,
                            "could not record `plugin_quantity_above_maximum` warning"
                        );
                    }
                }
            }

            if !answered {
                let mut has_overrun = false;
                let mut raw_only = Vec::new();
                for unit in &self.units {
                    let quantity = reported.units.get(&unit.unit).copied().unwrap_or(0);
                    if quantity > unit.max_per_call {
                        has_overrun = true;
                    }
                    if quantity > 0 {
                        raw_only.push(Counted {
                            usage_kind: PLUGIN_UNITS.to_owned(),
                            currency: None,
                            delta: i64::try_from(quantity).unwrap_or(i64::MAX),
                            part: Some(plugin_unit_qty_part(&self.plugin, &unit.unit)),
                        });
                    }
                }
                if !has_overrun {
                    if !raw_only.is_empty() {
                        let _ = self.charging.record_plugin_spend_counts(raw_only).await;
                    }
                    drop(self.reservations);
                    return Ok(());
                }
            }

            let mut reservations: BTreeMap<String, PluginUnitReservation> = self
                .reservations
                .into_iter()
                .map(|entry| (entry.unit.clone(), entry))
                .collect();
            let outcomes: Vec<PluginUnitOutcome> = self
                .units
                .into_iter()
                .map(|unit| {
                    let quantity = reported.units.get(&unit.unit).copied().unwrap_or(0);
                    let effective_reported = if answered {
                        quantity
                    } else {
                        quantity.max(unit.max_per_call)
                    };
                    let reservation = reservations.remove(&unit.unit).map(|open| open.reservation);
                    PluginUnitOutcome {
                        unit: unit.unit,
                        reported: effective_reported,
                        max_per_call: unit.max_per_call,
                        price: unit.price,
                        reservation,
                    }
                })
                .collect();
            self.charging
                .settle_plugin_spend(&self.plugin, outcomes)
                .await
                .map_err(charge_error)
        })
    }
}

fn charge_error(error: ChargeError) -> CallError {
    match error {
        ChargeError::Refused(Refusal::Reached(reached)) => limit_reached(&reached),
        ChargeError::Refused(Refusal::NotUnderstood(limit)) => limit_not_understood(*limit),
        other => CallError::new(
            &INTERNAL,
            format!("Plugin spend accounting failed: {other}"),
            &(),
        ),
    }
}

/// Live-Run stream accountant backed by [`RunCharging`] and [`RunTrace`]
/// (ADR 0033, issue 833): charges `plugin_stream_bytes` and
/// `plugin_stream_time` per destination through held Slices (`charge_part`)
/// and records each stream's destination through `RunTrace`'s writer.
#[derive(Clone)]
pub struct RunStreamAccountant {
    charging: RunCharging,
    trace: RunTrace,
}

impl fmt::Debug for RunStreamAccountant {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RunStreamAccountant")
            .finish_non_exhaustive()
    }
}

impl RunStreamAccountant {
    /// Creates a stream accountant backed by `charging` and `trace`.
    #[must_use]
    pub fn new(charging: RunCharging, trace: RunTrace) -> Arc<Self> {
        Arc::new(Self { charging, trace })
    }
}

impl StreamAccountant for RunStreamAccountant {
    fn record(&self, record: StreamRecord) -> BoxFuture<'static, ()> {
        let charging = self.charging.clone();
        let trace = self.trace.clone();
        Box::pin(async move {
            if record.bytes > 0
                && let Err(error) = charging
                    .charge_part(PLUGIN_STREAM_BYTES, &record.destination, record.bytes, None)
                    .await
            {
                tracing::warn!(
                    plugin = %record.plugin,
                    destination = %record.destination,
                    %error,
                    "failed to charge `plugin_stream_bytes`"
                );
            }
            if record.duration_ms > 0
                && let Err(error) = charging
                    .charge_part(
                        PLUGIN_STREAM_TIME,
                        &record.destination,
                        record.duration_ms,
                        None,
                    )
                    .await
            {
                tracing::warn!(
                    plugin = %record.plugin,
                    destination = %record.destination,
                    %error,
                    "failed to charge `plugin_stream_time`"
                );
            }
            if let Err(error) = trace.plugin_stream(&record).await {
                tracing::warn!(
                    plugin = %record.plugin,
                    destination = %record.destination,
                    %error,
                    "failed to write Plugin stream entry to Run trace"
                );
            }
        })
    }
}
