//! Plugin spend reservation, settlement and over-maximum accounting on
//! [`RunCharging`] (ADR 0033, issue 940), split from [`live`](super::live) to
//! keep each file focused.

use goodfetch_protocol::catalog::PriceValue;
use goodfetch_protocol::usage::PLUGIN_UNITS;

use super::live::{ChargeError, RunCharging};
use super::{Charged, Counted, Reservation, signed};

/// The `usage_parts.part` key for raw units of `unit` reported by `plugin`.
#[must_use]
pub fn plugin_unit_qty_part(plugin: &str, unit: &str) -> String {
    format!("qty:{plugin}:{unit}")
}

/// The `usage_parts.part` key for the price in `currency` of `unit` spent by
/// `plugin`.
#[must_use]
pub fn plugin_unit_price_part(plugin: &str, currency: &str, unit: &str) -> String {
    format!("price:{plugin}:{currency}:{unit}")
}

/// One declared spend unit of a Plugin to reserve ahead of a call (ADR 0033).
#[derive(Clone, Debug)]
pub struct PluginUnitReserve {
    /// The declared unit name.
    pub unit: String,
    /// The declared maximum quantity spent in one call.
    pub max_per_call: u64,
    /// Its resolved price per unit, or none when unpriced.
    pub price: Option<PriceValue>,
}

/// An open currency reservation for one declared spend unit of a Plugin call.
#[derive(Clone, Debug)]
pub struct PluginUnitReservation {
    /// The declared unit name.
    pub unit: String,
    /// The currency reserved in.
    pub currency: String,
    /// The underlying ledger reservation.
    pub reservation: Reservation,
}

/// What a completed Plugin call reported for one declared spend unit.
#[derive(Clone, Debug)]
pub struct PluginUnitOutcome {
    /// The declared unit name.
    pub unit: String,
    /// The raw quantity reported by the Plugin via `usage.report`.
    pub reported: u64,
    /// The declared maximum quantity per call.
    pub max_per_call: u64,
    /// Its resolved price per unit, or none when unpriced.
    pub price: Option<PriceValue>,
    /// The open reservation held for this unit, if one was made.
    pub reservation: Option<Reservation>,
}

impl RunCharging {
    /// Reserves each priced unit of `units` for `plugin` before a call (ADR
    /// 0033): `max_per_call * price` against every currency Limit on
    /// `plugin_units`. If any unit has no price while a currency Limit on
    /// `plugin_units` holds the Run, reserves nothing and answers
    /// `Ok(Err(unit))` so the caller refuses with `plugin_price_unknown`.
    /// Runs in memory with zero SQL when the held Limits are current and the
    /// held Slices cover the reservations (ADR 0140's E3).
    ///
    /// # Errors
    ///
    /// As [`RunCharging::charge`]: nothing is reserved when a Limit refuses.
    pub async fn reserve_plugin_spend(
        &self,
        plugin: &str,
        units: Vec<PluginUnitReserve>,
    ) -> Result<Result<Vec<PluginUnitReservation>, String>, ChargeError> {
        if units.is_empty() {
            return Ok(Ok(Vec::new()));
        }
        let plugin = plugin.to_owned();
        let reserved = self
            .charged(move |change| {
                if let Some(unpriced) = units.iter().find(|entry| entry.price.is_none())
                    && change.has_currency_limit(PLUGIN_UNITS)?
                {
                    return Ok((Err(unpriced.unit.clone()), Charged::default()));
                }
                let mut made: Vec<PluginUnitReservation> = Vec::new();
                let mut total = Charged::default();
                for entry in &units {
                    let Some(price) = &entry.price else {
                        continue;
                    };
                    let worst = entry.max_per_call.saturating_mul(price.millionths);
                    if worst == 0 {
                        continue;
                    }
                    match change.reserve(PLUGIN_UNITS, Some(&price.currency), worst) {
                        Ok((reservation, mut charged)) => {
                            let part =
                                plugin_unit_price_part(&plugin, &price.currency, &entry.unit);
                            for counted in &mut charged.counted {
                                counted.part = Some(part.clone());
                            }
                            total.used.extend(charged.used);
                            total.counted.extend(charged.counted);
                            made.push(PluginUnitReservation {
                                unit: entry.unit.clone(),
                                currency: price.currency.clone(),
                                reservation,
                            });
                        }
                        Err(unmade) => {
                            for prior in made {
                                let _ = change.settle(prior.reservation, 0);
                            }
                            return Err(unmade);
                        }
                    }
                }
                Ok((Ok(made), total))
            })
            .await?;
        Ok(reserved.unwrap_or_else(|| Ok(Vec::new())))
    }

    /// Cancels open Plugin spend `reservations` for a call that was never sent
    /// to the Plugin, refunding each reservation in full.
    ///
    /// # Errors
    ///
    /// `State` when the run state could not journal the refund.
    pub async fn cancel_plugin_spend(
        &self,
        plugin: &str,
        reservations: Vec<PluginUnitReservation>,
    ) -> Result<(), ChargeError> {
        if reservations.is_empty() {
            return Ok(());
        }
        let plugin = plugin.to_owned();
        self.settled(move |charging| {
            let mut total = Charged::default();
            for open in reservations {
                let mut settled = charging.settle(open.reservation, 0);
                let part = plugin_unit_price_part(&plugin, &open.currency, &open.unit);
                for counted in &mut settled.counted {
                    counted.part = Some(part.clone());
                }
                total.used.extend(settled.used);
                total.counted.extend(settled.counted);
            }
            total
        })
        .await
    }

    /// Settles a completed Plugin call's spend `outcomes`: refunds unused
    /// reservation capacity, journals every reported raw unit in full, and for
    /// any unit reported above its declared `max_per_call` charges the excess
    /// against the currency Limit — stopping the Run if the excess crosses a
    /// Limit while still journaling the full overrun so the Run report never
    /// caps reported spend (ADR 0033).
    ///
    /// # Errors
    ///
    /// `Refused` when an over-maximum excess crossed a currency Limit (the Run
    /// has already been asked to stop and the full overrun has been journaled);
    /// `State` when the run state could not be written.
    pub async fn settle_plugin_spend(
        &self,
        plugin: &str,
        outcomes: Vec<PluginUnitOutcome>,
    ) -> Result<(), ChargeError> {
        if outcomes.is_empty() {
            return Ok(());
        }
        let mut excesses: Vec<(String, String, u64)> = Vec::new();
        for outcome in &outcomes {
            if let Some(price) = &outcome.price {
                let reserved = if outcome.reservation.is_some() {
                    outcome.max_per_call.saturating_mul(price.millionths)
                } else {
                    0
                };
                let actual = outcome.reported.saturating_mul(price.millionths);
                let excess = actual.saturating_sub(reserved);
                if excess > 0 {
                    excesses.push((outcome.unit.clone(), price.currency.clone(), excess));
                }
            }
        }
        let plugin_name = plugin.to_owned();
        self.settled(move |charging| {
            let mut total = Charged::default();
            for outcome in outcomes {
                if outcome.reported > 0 {
                    total.counted.push(Counted {
                        usage_kind: PLUGIN_UNITS.to_owned(),
                        currency: None,
                        delta: signed(outcome.reported),
                        part: Some(plugin_unit_qty_part(&plugin_name, &outcome.unit)),
                    });
                }
                if let (Some(reservation), Some(price)) = (outcome.reservation, outcome.price) {
                    let worst = outcome.max_per_call.saturating_mul(price.millionths);
                    let actual = outcome.reported.saturating_mul(price.millionths);
                    let settle_at = actual.min(worst);
                    let mut settled = charging.settle(reservation, settle_at);
                    let part = plugin_unit_price_part(&plugin_name, &price.currency, &outcome.unit);
                    for counted in &mut settled.counted {
                        counted.part = Some(part.clone());
                    }
                    total.used.extend(settled.used);
                    total.counted.extend(settled.counted);
                }
            }
            total
        })
        .await?;

        let mut first_refusal: Option<ChargeError> = None;
        for (unit, currency, excess) in excesses {
            let part = plugin_unit_price_part(plugin, &currency, &unit);
            let part_for_charge = part.clone();
            let currency_for_charge = currency.clone();
            let charged = self
                .charged(move |change| {
                    let (reservation, mut charged) =
                        change.reserve(PLUGIN_UNITS, Some(&currency_for_charge), excess)?;
                    // Excess spend has already happened: it stays charged as reserved.
                    drop(reservation);
                    for counted in &mut charged.counted {
                        counted.part = Some(part_for_charge.clone());
                    }
                    Ok(((), charged))
                })
                .await;
            if let Err(error) = charged {
                // Even when the excess crossed the currency Limit, ADR 0033
                // requires recording the reported overrun in full, never capped.
                let _ = self
                    .record_plugin_spend_counts(vec![Counted {
                        usage_kind: PLUGIN_UNITS.to_owned(),
                        currency: Some(currency),
                        delta: signed(excess),
                        part: Some(part),
                    }])
                    .await;
                if first_refusal.is_none() {
                    first_refusal = Some(error);
                }
            }
        }
        first_refusal.map_or(Ok(()), Err)
    }

    /// Journals `counted` directly into the Run's run state without touching
    /// ledger Slices (used to record full over-maximum spend when the excess
    /// crossed a Limit, or raw units reported before an abandoned call ended).
    ///
    /// # Errors
    ///
    /// `State` when the run state could not be written.
    pub async fn record_plugin_spend_counts(
        &self,
        counted: Vec<Counted>,
    ) -> Result<(), ChargeError> {
        if counted.is_empty() {
            return Ok(());
        }
        self.record_counted_direct(counted).await
    }
}
