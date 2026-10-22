//! One run's charging against the Limits that hold it (ADR 0041): before a
//! run starts, every Limit covering it must have something left in its
//! period; while it runs, each Usage it charges comes out of its Slices, a
//! new Slice taken when they run out; a cost about to be paid reserves its
//! worst case first, settled at what it cost; when it ends, what is left of
//! them goes back. A run that cannot take a Slice it needs is refused the charge
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

/// A worst case a run reserved and has not settled yet: what it took of
/// each Slice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[must_use = "a reservation is settled once the cost is known"]
pub struct Reservation {
    worst: u64,
    taken: Vec<(SliceId, u64)>,
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
    uncapped: bool,
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
            uncapped: false,
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
    /// taking another Slice, large enough, when they run short.
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
        let (_, charged) = self.take(usage_kind, None, amount, now)?;
        if !charged.used.is_empty() {
            tracing::trace!(target: TARGET, transition = "charge", usage_kind, amount, run_id = %self.holder.run_id, epoch = self.holder.epoch, "a run charged Usage against its Slices");
        }
        Ok(charged)
    }

    /// Reserves `worst`, the worst case of a cost about to be paid, such as
    /// a model call or a paid solve, against every Limit on `usage_kind` in
    /// `currency` that holds the run (ADR 0041): out of its Slices, or a
    /// Slice taken at least as large in one transaction. The run journals
    /// what it charged before it pays, so a reclaimer charges an open
    /// reservation at its worst case. A cost in a currency no Limit holding
    /// the run caps marks the run *uncapped*.
    ///
    /// # Errors
    ///
    /// As [`Charging::charge`]: the cost is then not to be paid.
    pub fn reserve(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        worst: u64,
        now: Timestamp,
    ) -> Result<(Reservation, Charged), Refusal> {
        let (taken, charged) = self.take(usage_kind, currency, worst, now)?;
        if taken.is_empty() && currency.is_some() {
            self.uncapped = true;
        }
        tracing::trace!(target: TARGET, transition = "reserve", usage_kind, worst, run_id = %self.holder.run_id, epoch = self.holder.epoch, "a run reserved a worst case against its Slices");
        Ok((Reservation { worst, taken }, charged))
    }

    /// Settles `reservation` at `cost`, what was paid, which its worst case
    /// bounds: what it reserved beyond `cost` is the run's to use again.
    /// Answers each Slice touched with the run's Usage of it now, for the
    /// run to journal.
    pub fn settle(&mut self, reservation: Reservation, cost: u64) -> Charged {
        let Reservation { worst, taken } = reservation;
        let refund = worst.saturating_sub(cost);
        let mut owed: Vec<(LimitId, u64)> = Vec::new();
        let mut charged = Charged::default();
        for (slice, taken) in taken.into_iter().rev() {
            let Some(held) = self.held.iter_mut().find(|held| held.id == slice) else {
                continue;
            };
            let index = owed
                .iter()
                .position(|(limit, _)| *limit == held.limit)
                .unwrap_or_else(|| {
                    owed.push((held.limit, refund));
                    owed.len() - 1
                });
            let back = owed[index].1.min(taken).min(held.used);
            held.used -= back;
            owed[index].1 -= back;
            charged.used.push((held.id, held.used));
        }
        tracing::trace!(target: TARGET, transition = "settle", cost, refund, run_id = %self.holder.run_id, epoch = self.holder.epoch, "a run settled a reservation at what it cost");
        charged
    }

    /// Whether a reservation of this run found a currency no Limit caps.
    pub fn uncapped(&self) -> bool {
        self.uncapped
    }

    /// Takes `amount` from the run's Slices of every Limit on `usage_kind`
    /// in `currency` that holds it, taking a Slice of each Limit that runs
    /// short; answers what it took of each Slice, and the run's Usage of
    /// each Slice touched.
    fn take(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        amount: u64,
        now: Timestamp,
    ) -> Result<(Vec<(SliceId, u64)>, Charged), Refusal> {
        let limits = self.ledger.limits()?;
        let holding: Vec<StoredLimit> = self
            .placement
            .holding(&limits)
            .map_err(|stored| Refusal::NotUnderstood(Box::new(stored.limit())))?
            .into_iter()
            .filter(|stored| {
                stored.usage_kind == usage_kind && stored.currency.as_deref() == currency
            })
            .cloned()
            .collect();
        let mut periods = Vec::new();
        for stored in &holding {
            let (period, _) = stored.period_key(now);
            let room: u64 = self
                .held
                .iter()
                .filter(|held| held.limit == stored.id && held.period == period)
                .map(|held| held.size - held.used)
                .sum();
            if room < amount {
                let Some(slice) = self.ledger.take_slice(
                    stored,
                    &self.holder,
                    now,
                    self.sizing,
                    amount - room,
                )?
                else {
                    return Err(reached(stored, now));
                };
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
        let mut taken = Vec::new();
        let mut charged = Charged::default();
        for (limit, period) in periods {
            let mut owed = amount;
            for held in self
                .held
                .iter_mut()
                .filter(|held| held.limit == limit && held.period == period)
            {
                let took = owed.min(held.size - held.used);
                if took == 0 {
                    continue;
                }
                held.used += took;
                owed -= took;
                taken.push((held.id, took));
                charged.used.push((held.id, held.used));
                if owed == 0 {
                    break;
                }
            }
        }
        Ok((taken, charged))
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
