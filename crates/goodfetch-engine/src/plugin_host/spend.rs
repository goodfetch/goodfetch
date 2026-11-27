//! Spend accounting interface for a process Plugin's calls (ADR 0033):
//! reserves each declared spend unit's `max_per_call * price` before a call
//! goes and settles it once the call ends against what the Plugin reported via
//! `usage.report`. Concrete ledger and Run-report wiring lives in
//! `runs::plugin_spend`, keeping `plugin_host` decoupled from `ledger` and
//! `runs`.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::Arc;

use futures_util::future::BoxFuture;
use goodfetch_protocol::catalog::PriceValue;
use goodfetch_protocol::plugin::Spend;
use goodfetch_protocol::plugin::spend::{PLUGIN_PRICE_UNKNOWN, PluginPriceUnknownDetails};

use crate::call_error::CallError;
use crate::plugin_host::instance::CallFailure;

/// One parsed spend unit declared in a Plugin manifest (after applying any
/// operator price override).
#[derive(Clone, Debug)]
pub struct DeclaredUnit {
    /// The declared unit name.
    pub unit: String,
    /// The declared maximum quantity per call.
    pub max_per_call: u64,
    /// Its resolved price per unit, or none when unpriced.
    pub price: Option<PriceValue>,
}

/// Raw units reported by the Plugin via `usage.report` during one call.
#[derive(Clone, Debug, Default)]
pub struct ReportedSpend {
    /// Whether the Plugin sent at least one `usage.report` for this call.
    pub any: bool,
    /// Total raw quantity reported per declared unit name.
    pub units: BTreeMap<String, u64>,
}

impl ReportedSpend {
    /// Adds `quantity` for `unit`.
    pub(super) fn add(&mut self, unit: String, quantity: u64) {
        self.any = true;
        let entry = self.units.entry(unit).or_insert(0);
        *entry = entry.saturating_add(quantity);
    }
}

/// What a live Run hands a [`PluginInstance`](super::instance::PluginInstance)
/// so each call reserves and settles declared spend against the Run's Usage
/// ledger and report (ADR 0033).
pub trait SpendAccountant: fmt::Debug + Send + Sync {
    /// Reserves declared spend for one call to `plugin` before it goes.
    fn reserve<'call>(
        &'call self,
        plugin: &'call str,
        units: Vec<DeclaredUnit>,
    ) -> BoxFuture<'call, Result<Box<dyn SpendTicket>, CallError>>;
}

/// Open spend reservations held for one in-flight Plugin call.
pub trait SpendTicket: Send {
    /// Refunds every reservation for a call that was never sent to the Plugin.
    fn cancel(self: Box<Self>) -> BoxFuture<'static, ()>;

    /// Settles a sent call once it ends (answered normally or abandoned),
    /// recording reported raw units, emitting `plugin_figures` and
    /// `plugin_quantity_above_maximum`, and charging any over-maximum excess.
    fn finish(
        self: Box<Self>,
        answered: bool,
        reported: ReportedSpend,
    ) -> BoxFuture<'static, Result<(), CallError>>;
}

/// Parsed declared spend entries held by a
/// [`PluginInstance`](super::instance::PluginInstance).
#[derive(Clone, Debug)]
pub(super) struct DeclaredSpend {
    plugin: String,
    units: Vec<DeclaredUnit>,
    accountant: Arc<dyn SpendAccountant>,
}

impl DeclaredSpend {
    /// Builds spend tracking for `plugin` from its manifest's `spend` table
    /// when both `spend` and `accountant` are present.
    pub(super) fn new(
        plugin: String,
        spend: &BTreeMap<String, Spend>,
        accountant: Option<Arc<dyn SpendAccountant>>,
    ) -> Option<Self> {
        if spend.is_empty() {
            return None;
        }
        let accountant = accountant?;
        let units = spend
            .iter()
            .map(|(unit, declared)| {
                let price = declared.price.as_deref().and_then(|text| {
                    let parsed = PriceValue::parse(text);
                    if parsed.is_none() {
                        tracing::warn!(
                            plugin = %plugin,
                            unit = %unit,
                            price = text,
                            "a Plugin manifest spend price cannot be parsed; treating the unit as unpriced"
                        );
                    }
                    parsed
                });
                DeclaredUnit {
                    unit: unit.clone(),
                    max_per_call: declared.max_per_call,
                    price,
                }
            })
            .collect();
        Some(Self {
            plugin,
            units,
            accountant,
        })
    }

    /// Reserves each priced unit before a call goes, refusing an unpriced unit
    /// under a currency Limit with `plugin_price_unknown` and a crossed Limit
    /// with `limit_reached`.
    pub(super) async fn reserve(&self) -> Result<CallSpend, CallFailure> {
        let ticket = self
            .accountant
            .reserve(&self.plugin, self.units.clone())
            .await
            .map_err(CallFailure::Refused)?;
        Ok(CallSpend { ticket })
    }
}

/// Open spend ticket for one in-flight Plugin call.
pub(super) struct CallSpend {
    ticket: Box<dyn SpendTicket>,
}

impl CallSpend {
    /// Finishes spend accounting for the call once it ends.
    pub(super) async fn finish(
        self,
        sent: bool,
        answered: bool,
        reported: ReportedSpend,
    ) -> Result<(), CallFailure> {
        if !sent {
            self.ticket.cancel().await;
            return Ok(());
        }
        self.ticket
            .finish(answered, reported)
            .await
            .map_err(CallFailure::Refused)
    }
}

/// Builds a `plugin_price_unknown` [`CallError`] for `plugin`'s unpriced `unit`.
#[must_use]
pub fn plugin_price_unknown(plugin: &str, unit: &str) -> CallError {
    CallError::new(
        &PLUGIN_PRICE_UNKNOWN,
        format!(
            "The Plugin `{plugin}` declares spend unit `{unit}` with no price, which a currency Limit treats as unbounded; set `plugins.{plugin}.spend.{unit}.price` in Operator policy."
        ),
        &PluginPriceUnknownDetails {
            plugin: plugin.to_owned(),
            unit: unit.to_owned(),
        },
    )
}
