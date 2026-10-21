//! One run's charging against the Limits that hold it (ADR 0041): before a
//! run starts, every Limit covering it must have something left in its
//! period; while it runs, each Usage it charges comes out of its Slices, a
//! new Slice taken when they run out; when it ends, what is left of them
//! goes back. A run that cannot take a Slice it needs is refused the charge
//! and stops at its next safe point.

use goodfetch_protocol::run::ReachedLimit;
use goodfetch_protocol::usage::Limit;
use jiff::Timestamp;

use super::{
    Holder, Ledger, LedgerError, LimitId, Placement, SliceId, SliceSizing, StoredLimit, TARGET,
};

/// Why a run may not start, or may not charge more.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Refusal {
    /// A Limit covering it has nothing left in its period: `limit_reached`.
    #[error("a Limit on {} {} has nothing left", .0.scope.kind, .0.usage_kind)]
    Reached(Box<ReachedLimit>),
    /// A Limit flagged *must understand* that this engine cannot evaluate
    /// may cover it: `limit_not_understood`.
    #[error("a Limit on {} cannot be evaluated by this engine", .0.scope.kind)]
    NotUnderstood(Box<Limit>),
    /// The ledger could not be read or written.
    #[error(transparent)]
    Ledger(#[from] LedgerError),
}

/// What a charge took from each Slice: every Slice it touched, with what
/// the run has now used of it in all, which the run journals under its
/// epoch before it uses what it charged.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Charged {
    /// Each Slice touched, with its run's Usage of it so far.
    pub used: Vec<(SliceId, u64)>,
}

/// A Slice the run holds, and what it used of it.
#[derive(Clone, Debug)]
struct Held {
    limit: LimitId,
    period: String,
    id: SliceId,
    size: u64,
    used: u64,
}

/// One run's Slices and the Limits it charges.
#[derive(Clone, Debug)]
pub struct Charging {
    ledger: Ledger,
    holder: Holder,
    placement: Placement,
    sizing: SliceSizing,
    held: Vec<Held>,
}

impl Charging {
    /// The charging of the run `holder` names, placed at `placement`.
    pub fn new(ledger: Ledger, holder: Holder, placement: Placement, sizing: SliceSizing) -> Self {
        Self {
            ledger,
            holder,
            placement,
            sizing,
            held: Vec::new(),
        }
    }

    /// Whether a run placed at `placement` may start at `now`: every Limit
    /// holding it has something left in its period.
    ///
    /// # Errors
    ///
    /// `Reached` naming the first Limit with nothing left, `NotUnderstood`
    /// naming a Limit it cannot be placed outside of, `Ledger` when the
    /// ledger cannot be read.
    pub fn may_start(
        ledger: &Ledger,
        placement: &Placement,
        now: Timestamp,
    ) -> Result<(), Refusal> {
        let limits = ledger.limits()?;
        let holding = placement
            .holding(&limits)
            .map_err(|stored| Refusal::NotUnderstood(Box::new(stored.limit())))?;
        for stored in holding {
            if ledger.left(stored, now)? == 0 {
                return Err(reached(stored, now));
            }
        }
        Ok(())
    }

    /// Charges `amount` of `usage_kind` against every Limit on it that
    /// holds the run, out of its Slices of each Limit's period at `now`,
    /// taking more Slices as needed.
    ///
    /// # Errors
    ///
    /// `Reached` when a Limit has too little left for it, `NotUnderstood`
    /// when a Limit the run cannot be placed outside of appeared, `Ledger`
    /// when the ledger cannot be read or written. Nothing is charged then,
    /// though Slices taken on the way stay the run's until it ends.
    pub fn charge(
        &mut self,
        usage_kind: &str,
        amount: u64,
        now: Timestamp,
    ) -> Result<Charged, Refusal> {
        let limits = self.ledger.limits()?;
        let holding: Vec<StoredLimit> = self
            .placement
            .holding(&limits)
            .map_err(|stored| Refusal::NotUnderstood(Box::new(stored.limit())))?
            .into_iter()
            .filter(|stored| stored.usage_kind == usage_kind && stored.currency.is_none())
            .cloned()
            .collect();
        let mut periods = Vec::new();
        for stored in &holding {
            let (period, _) = stored.period_key(now);
            let mut room: u64 = self
                .held
                .iter()
                .filter(|held| held.limit == stored.id && held.period == period)
                .map(|held| held.size - held.used)
                .sum();
            while room < amount {
                let Some(slice) = self
                    .ledger
                    .take_slice(stored, &self.holder, now, self.sizing)?
                else {
                    return Err(reached(stored, now));
                };
                room += slice.size;
                self.held.push(Held {
                    limit: stored.id,
                    period: period.clone(),
                    id: slice.id,
                    size: slice.size,
                    used: 0,
                });
            }
            periods.push((stored.id, period));
        }
        let mut charged = Charged::default();
        for (limit, period) in periods {
            let mut owed = amount;
            for held in self
                .held
                .iter_mut()
                .filter(|held| held.limit == limit && held.period == period)
            {
                let taken = owed.min(held.size - held.used);
                if taken == 0 {
                    continue;
                }
                held.used += taken;
                owed -= taken;
                charged.used.push((held.id, held.used));
                if owed == 0 {
                    break;
                }
            }
        }
        if !charged.used.is_empty() {
            tracing::trace!(target: TARGET, transition = "charge", usage_kind, amount, run_id = %self.holder.run_id, epoch = self.holder.epoch, "a run charged Usage against its Slices");
        }
        Ok(charged)
    }

    /// Gives back what is left of every Slice the run holds, charging what
    /// it used; the run holds none afterwards.
    ///
    /// # Errors
    ///
    /// When the ledger cannot be written: the Slices then stay held until
    /// an engine that opens the ledger reclaims them.
    pub fn give_back(&mut self) -> Result<(), LedgerError> {
        let used: Vec<(SliceId, u64)> = self.held.iter().map(|held| (held.id, held.used)).collect();
        self.ledger.give_back(&used)?;
        self.held.clear();
        Ok(())
    }
}

fn reached(stored: &StoredLimit, now: Timestamp) -> Refusal {
    let (_, bounds) = stored.period_key(now);
    Refusal::Reached(Box::new(stored.reached(bounds.map(|bounds| bounds.end))))
}
