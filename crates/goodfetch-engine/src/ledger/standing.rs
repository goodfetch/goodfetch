//! What an engine makes of each Limit row (ADRs 0041, 0066, 0087 and 0098):
//! a row it can evaluate holds its runs; a row on a value only Foxhound
//! carries is kept untouched by a Collie engine, whose runs never use it;
//! and a row whose scope kind, Usage kind, currency or period it cannot
//! evaluate, which a newer engine wrote, refuses the runs it cannot place
//! outside it when the row is flagged *must understand*, and is ignored
//! otherwise.

use goodfetch_protocol::catalog::{Product, SharedType};
use goodfetch_protocol::entries::product_of_value;
use goodfetch_protocol::run::LimitScope;
use goodfetch_protocol::usage::LimitStanding;
use jiff::tz::TimeZone;

use super::StoredLimit;

/// What `product`'s engine makes of `stored`.
pub fn standing(stored: &StoredLimit, product: Product) -> LimitStanding {
    let values = [
        product_of_value(SharedType::LimitScope, &stored.scope.kind),
        product_of_value(SharedType::UsageKind, &stored.usage_kind),
    ];
    if values.contains(&None) || !currency_known(stored) || !period_known(stored) {
        return LimitStanding::NotUnderstood;
    }
    if values
        .into_iter()
        .flatten()
        .all(|needs| product.contains(needs))
    {
        LimitStanding::Held
    } else {
        LimitStanding::Foxhound
    }
}

fn currency_known(stored: &StoredLimit) -> bool {
    stored.currency.as_deref().is_none_or(|currency| {
        currency.len() == 3 && currency.bytes().all(|byte| byte.is_ascii_uppercase())
    })
}

fn period_known(stored: &StoredLimit) -> bool {
    match (&stored.period, &stored.time_zone) {
        (None, None) => true,
        (Some(_), Some(zone)) => stored.known_period().is_some() && TimeZone::get(zone).is_ok(),
        _ => false,
    }
}

/// Where a run stands: its engine's Product and the scopes that cover it
/// (the install, its person, its Client connection, its own `run` scope
/// and, on a Daemon, the Daemon).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Placement {
    /// The Product of the engine that runs it.
    pub product: Product,
    /// The scopes that cover it.
    pub scopes: Vec<LimitScope>,
}

impl Placement {
    /// The Limits of `limits` that hold the run, or the first one flagged
    /// *must understand* that this engine cannot evaluate and cannot place
    /// the run outside of: one whose scope kind it does not know, or whose
    /// scope covers the run.
    ///
    /// # Errors
    ///
    /// That Limit.
    pub fn holding<'a>(
        &self,
        limits: &'a [StoredLimit],
    ) -> Result<Vec<&'a StoredLimit>, &'a StoredLimit> {
        let mut holding = Vec::new();
        for stored in limits {
            match standing(stored, self.product) {
                LimitStanding::Held if self.scopes.contains(&stored.scope) => holding.push(stored),
                LimitStanding::NotUnderstood if stored.must_understand => {
                    let scope_known =
                        product_of_value(SharedType::LimitScope, &stored.scope.kind).is_some();
                    let outside = scope_known && !self.scopes.contains(&stored.scope);
                    if !outside {
                        return Err(stored);
                    }
                }
                LimitStanding::Held | LimitStanding::Foxhound | LimitStanding::NotUnderstood => {}
            }
        }
        Ok(holding)
    }
}
