//! One run's charging against the Limits that hold it (ADR 0041): before a
//! run starts, every Limit covering it must have something left in its
//! period; while it runs, each Usage it charges comes out of its Slices, a
//! new Slice taken when they run out; a cost about to be paid reserves its
//! worst case first, settled at what it cost; when it ends, what is left of
//! them goes back. A run that cannot take a Slice it needs is refused the charge
//! and stops at its next safe point.
//!
//! A run holds the Limits that hold it in memory (ADR 0140's E3, #1888):
//! it reads them from the ledger as it starts, and again only once an
//! engine of its process changed them, before it takes a Slice, or after
//! its read interval, `usage.ledger_read_interval` of its engine's clock
//! (issue 2602). A charge its Slices cover is then a step in memory: no
//! SQL, no file, no Limit evaluated again.

use goodfetch_protocol::run::LimitSetter;
use goodfetch_protocol::run::{AppliedLimit, ReachedLimit};
use goodfetch_protocol::usage::{LEDGER_READ_INTERVAL_DEFAULT, Limit};
use jiff::{SignedDuration, Timestamp};
use rusqlite::Connection;

use super::{
    Holder, Ledger, LedgerError, LimitId, PeriodBounds, Placement, SliceId, SliceSizing,
    StoredLimit, TARGET, point_in_time,
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
    /// What the run's Usage of its kind changed by, which its report
    /// counts (ADR 0101).
    pub counted: Vec<Counted>,
}

/// What a run's Usage of one kind, in one currency or as a count, changed
/// by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Counted {
    /// The Usage kind.
    pub usage_kind: String,
    /// The currency, or none for a count.
    pub currency: Option<String>,
    /// The change: a charge or a reservation adds, a settlement below its
    /// worst case takes the rest off.
    pub delta: i64,
    /// The part of the kind it is counted under too, such as a proxy kind
    /// of `proxy_bytes`, if it has one.
    pub part: Option<String>,
}

impl Counted {
    fn of(usage_kind: &str, currency: Option<&str>, delta: i64) -> Self {
        Self {
            usage_kind: usage_kind.to_owned(),
            currency: currency.map(str::to_owned),
            delta,
            part: None,
        }
    }
}

/// A worst case a run reserved and has not settled yet: what it took of
/// each Slice.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[must_use = "a reservation is settled once the cost is known"]
pub struct Reservation {
    worst: u64,
    taken: Vec<(SliceId, u64)>,
    usage_kind: String,
    currency: Option<String>,
}

/// Why a [`Change`] of a run's charging was not made.
#[derive(Debug)]
pub enum Unmade {
    /// It is no step in memory: it needs the ledger, for a Slice, the
    /// run's Limits or a refusal. Only a change tried in memory
    /// ([`Charging::change_held`]) answers it.
    NeedsLedger,
    /// A Limit refused it, or the ledger could not be read or written.
    Refused(Refusal),
}

impl Unmade {
    /// The refusal, of a change the ledger refused.
    fn refusal(self) -> Option<Refusal> {
        match self {
            Self::NeedsLedger => None,
            Self::Refused(refusal) => Some(refusal),
        }
    }
}

impl From<Refusal> for Unmade {
    fn from(refusal: Refusal) -> Self {
        Self::Refused(refusal)
    }
}

/// What a [`Change`] may reach.
#[derive(Clone, Copy, Debug)]
enum Reach {
    /// What the run holds in memory, and nothing else.
    Held,
    /// The ledger too.
    Ledger,
}

/// One change of a run's charging at one instant, made of charges,
/// reservations and settlements: as a step in memory, which never reaches
/// the ledger ([`Charging::change_held`]), or reaching it
/// ([`Charging::change`]). One description of a change serves both, so
/// whether it is a step in memory is asked of the charging as each part of
/// it is made, never stated beside it.
#[derive(Debug)]
pub struct Change<'a> {
    charging: &'a mut Charging,
    now: Timestamp,
    reach: Reach,
}

impl Change<'_> {
    /// Charges as [`Charging::charge`] does.
    ///
    /// # Errors
    ///
    /// `NeedsLedger` when the change is tried in memory and this charge is
    /// no step in memory; `Refused` as [`Charging::charge`]'s.
    pub fn charge(&mut self, usage_kind: &str, amount: u64) -> Result<Charged, Unmade> {
        match self.reach {
            Reach::Held => self
                .charging
                .charge_held(usage_kind, amount, self.now)
                .ok_or(Unmade::NeedsLedger),
            Reach::Ledger => Ok(self.charging.charge(usage_kind, amount, self.now)?),
        }
    }

    /// Reserves as [`Charging::reserve`] does.
    ///
    /// # Errors
    ///
    /// As [`Change::charge`]'s.
    pub fn reserve(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        worst: u64,
    ) -> Result<(Reservation, Charged), Unmade> {
        match self.reach {
            Reach::Held => self
                .charging
                .reserve_held(usage_kind, currency, worst, self.now)
                .ok_or(Unmade::NeedsLedger),
            Reach::Ledger => Ok(self
                .charging
                .reserve(usage_kind, currency, worst, self.now)?),
        }
    }

    /// Settles as [`Charging::settle`] does: always a step in memory.
    pub fn settle(&mut self, reservation: Reservation, cost: u64) -> Charged {
        self.charging.settle(reservation, cost)
    }

    /// Who allows the run to pay, as [`Charging::allowed_by`] answers,
    /// which reads the ledger.
    ///
    /// # Errors
    ///
    /// `NeedsLedger` when the change is tried in memory; `Refused` as
    /// [`Charging::allowed_by`]'s.
    pub fn allowed_by(
        &mut self,
        usage_kind: &str,
        currency: &str,
    ) -> Result<Option<LimitSetter>, Unmade> {
        match self.reach {
            Reach::Held => Err(Unmade::NeedsLedger),
            Reach::Ledger => Ok(self.charging.allowed_by(usage_kind, currency)?),
        }
    }
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

/// A Limit that holds the run, as the run read it last: what its engine
/// makes of it is settled by then, and the period its Slices are of is
/// worked out again only once a charge falls outside it (E3-1 of the audit
/// #1882).
#[derive(Debug)]
struct Holding {
    stored: StoredLimit,
    /// The key of the period that holds the charges placed in it.
    period: String,
    /// That period's bounds; none for a Limit without a period.
    bounds: Option<PeriodBounds>,
}

impl Holding {
    /// `stored`, in the period that holds `now`.
    fn at(stored: StoredLimit, now: Timestamp) -> Self {
        let (period, bounds) = stored.period_key(now);
        Self {
            stored,
            period,
            bounds,
        }
    }

    /// Whether it is a Limit on `usage_kind` in `currency`.
    fn caps(&self, usage_kind: &str, currency: Option<&str>) -> bool {
        self.stored.usage_kind == usage_kind && self.stored.currency.as_deref() == currency
    }

    /// It, moved to the period that holds `now` once `now` left its own.
    fn place(&mut self, now: Timestamp) -> &mut Self {
        if self
            .bounds
            .is_some_and(|bounds| now < bounds.start || now >= bounds.end)
        {
            (self.period, self.bounds) = self.stored.period_key(now);
        }
        self
    }

    /// Whether `held` is a Slice of it and of its period.
    fn holds(&self, held: &Held) -> bool {
        held.limit == self.stored.id && held.period == self.period
    }

    /// What is left of the Slices of `held` that are of it and its period.
    fn room(&self, held: &[Held]) -> u64 {
        held.iter()
            .filter(|held| self.holds(held))
            .map(|held| held.size - held.used)
            .sum()
    }
}

/// The Limits a run holds in memory, from one read of the ledger to the
/// next.
#[derive(Debug)]
struct Limits {
    /// The read they come from: none before the run's first read and
    /// after a read that failed, so the next charge reads.
    read: Option<Read>,
    /// The Limits that hold the run, or the one flagged *must understand*
    /// that its engine cannot evaluate and cannot place it outside of.
    holding: Result<Vec<Holding>, Box<Limit>>,
}

/// One read of the Limits from the ledger.
#[derive(Clone, Copy, Debug)]
struct Read {
    /// The ledger's count of changes as the read began.
    changes: u64,
    /// The time of the charge, or of the start, that read them.
    at: Timestamp,
}

/// Why a run reads its Limits from the ledger.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reading {
    /// It starts or resumes.
    Start,
    /// A charge needs them: its statement is the charge's counted work
    /// (ADR 0140).
    Charge,
}

impl Limits {
    /// No Limit read yet.
    fn unread() -> Self {
        Self {
            read: None,
            holding: Ok(Vec::new()),
        }
    }

    /// The Limits that hold the run.
    ///
    /// # Errors
    ///
    /// `NotUnderstood` naming the Limit it cannot be placed outside of.
    fn holding(&mut self) -> Result<&mut Vec<Holding>, Refusal> {
        self.holding
            .as_mut()
            .map_err(|limit| Refusal::NotUnderstood(limit.clone()))
    }

    /// Those of them on `usage_kind` in `currency`, each placed in its
    /// period at `now`.
    ///
    /// # Errors
    ///
    /// As [`Limits::holding`]'s.
    fn capping<'a>(
        &'a mut self,
        usage_kind: &'a str,
        currency: Option<&'a str>,
        now: Timestamp,
    ) -> Result<impl Iterator<Item = &'a mut Holding>, Refusal> {
        Ok(self
            .holding()?
            .iter_mut()
            .filter(move |holding| holding.caps(usage_kind, currency))
            .map(move |holding| holding.place(now)))
    }
}

/// One run's Slices and the Limits it charges.
#[derive(Debug)]
pub struct Charging {
    ledger: Ledger,
    holder: Holder,
    placement: Placement,
    sizing: SliceSizing,
    /// How long it charges on the Limits it holds in memory before it
    /// reads them again though no engine of its process changed them
    /// (ADR 0041): an engine of another process may have, which this
    /// process's ledgers cannot see. One read each interval, whatever it
    /// charges meanwhile, in place of one at each charge.
    read_interval: SignedDuration,
    held: Vec<Held>,
    uncapped: bool,
    /// The Limits that hold it, as it read them last.
    limits: Limits,
    /// The connection its reads of the Limits, the Slices it takes and its
    /// give-back run on, opened at the first and kept, each statement
    /// prepared once on it, so a run opens the ledger once (#1886), as it
    /// starts; let go when a call fails, so the next one opens afresh.
    connection: Option<Connection>,
}

impl Charging {
    /// The charging of the run `holder` names, placed at `placement`. It
    /// reads nothing yet: [`Charging::may_start`] reads its Limits as the
    /// run starts, and its first charge does when nothing has. It reads
    /// them again for another process's changes each
    /// `usage.ledger_read_interval`'s default, unless
    /// [`Charging::reading_every`] sets another interval.
    pub fn new(ledger: Ledger, holder: Holder, placement: Placement, sizing: SliceSizing) -> Self {
        Self {
            ledger,
            holder,
            placement,
            sizing,
            read_interval: read_interval(LEDGER_READ_INTERVAL_DEFAULT),
            held: Vec::new(),
            uncapped: false,
            limits: Limits::unread(),
            connection: None,
        }
    }

    /// The same charging, reading its Limits again for another process's
    /// changes each `interval` of its engine's clock, the Run's
    /// `usage.ledger_read_interval`: at every charge when it is zero.
    #[must_use]
    pub fn reading_every(self, interval: std::time::Duration) -> Self {
        Self {
            read_interval: read_interval(interval),
            ..self
        }
    }

    /// Whether the run may start at `now`: every Limit holding it has
    /// something left in its period. The Limits it reads for this are the
    /// ones it then holds in memory, on the connection it keeps, so its
    /// charges open nothing and read none.
    ///
    /// # Errors
    ///
    /// `Reached` naming the first Limit with nothing left, `NotUnderstood`
    /// naming a Limit it cannot be placed outside of, `Ledger` when the
    /// ledger cannot be read.
    pub fn may_start(&mut self, now: Timestamp) -> Result<(), Refusal> {
        self.read(now, Reading::Start)?;
        let Self {
            ledger,
            connection,
            limits,
            ..
        } = self;
        for holding in limits.holding()?.iter() {
            if ledger.left_on(connection, &holding.stored, now)? == 0 {
                return Err(reached(&holding.stored, now));
            }
        }
        Ok(())
    }

    /// Reads the Limits that hold the run as it resumes, as
    /// [`Charging::may_start`] reads them for a new one, whatever is left
    /// of them: a Limit with nothing left refuses its next charge.
    ///
    /// # Errors
    ///
    /// `NotUnderstood` and `Ledger` as [`Charging::may_start`]'s; its next
    /// charge then reads them again.
    pub fn resume(&mut self, now: Timestamp) -> Result<(), Refusal> {
        self.read(now, Reading::Start)?;
        self.limits.holding().map(drop)
    }

    /// Reads the Limits that hold the run from the ledger, on the
    /// connection it keeps, and holds them in memory: what its engine
    /// makes of each and the period each is in at `now`, settled here and
    /// not at each charge. A run that a Limit holds prepares the
    /// statements of a Slice's transaction with them, so no Slice it takes
    /// prepares one. A ledger no engine created holds no Limit and stays
    /// uncreated.
    ///
    /// # Errors
    ///
    /// `Ledger` when the ledger cannot be read: it then holds none, and
    /// its next charge reads again.
    fn read(&mut self, now: Timestamp, reading: Reading) -> Result<(), Refusal> {
        self.limits = Limits::unread();
        // Counted before the read: a change that lands while it reads
        // moves the count past this one, and the next charge reads again.
        let changes = self.ledger.changes();
        let stored = match reading {
            Reading::Start => self.ledger.limits_on(&mut self.connection)?,
            Reading::Charge => self.ledger.charge_limits(&mut self.connection)?,
        };
        let holding = match self.placement.holding(&stored) {
            Ok(holding) => Ok(holding
                .into_iter()
                .map(|stored| Holding::at(stored.clone(), now))
                .collect::<Vec<_>>()),
            Err(stored) => Err(Box::new(stored.limit())),
        };
        if holding.as_ref().is_ok_and(|holding| !holding.is_empty()) {
            self.ledger.prepare_slices(&mut self.connection)?;
        }
        self.limits = Limits {
            read: Some(Read { changes, at: now }),
            holding,
        };
        Ok(())
    }

    /// Whether the Limits it holds in memory are still the ledger's, as
    /// far as its process can tell at `now`: no engine of the process
    /// changed them since it read them, and it read them less than its
    /// read interval ago.
    fn current(&self, now: Timestamp) -> bool {
        self.limits.read.is_some_and(|read| {
            read.changes == self.ledger.changes()
                && now >= read.at
                && now.duration_since(read.at) < self.read_interval
        })
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
        Ok(self.charged(usage_kind, amount, charged))
    }

    /// Whether a charge or a reservation of `amount` of `usage_kind` in
    /// `currency` is a step in memory at `now`: the Slices the run holds
    /// cover it, and the Limits it holds are still the ledger's as far as
    /// its process can tell. False when it needs the ledger, for a Slice,
    /// its Limits or a refusal.
    fn covers(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        amount: u64,
        now: Timestamp,
    ) -> bool {
        self.current(now)
            && !self
                .short(usage_kind, currency, amount, now)
                .unwrap_or(true)
    }

    /// Charges `amount` of `usage_kind` as [`Charging::charge`] does, when
    /// that is a step in memory: the Slices the run holds cover it, and
    /// the Limits it holds are still the ledger's as far as its process
    /// can tell. None, and nothing charged, when the charge needs the
    /// ledger, for a Slice, its Limits or a refusal: [`Charging::charge`]
    /// then makes it.
    pub fn charge_held(
        &mut self,
        usage_kind: &str,
        amount: u64,
        now: Timestamp,
    ) -> Option<Charged> {
        if !self.covers(usage_kind, None, amount, now) {
            return None;
        }
        let (_, charged) = self.draw(usage_kind, None, amount).ok()?;
        Some(self.charged(usage_kind, amount, charged))
    }

    /// What a charge of `amount` of `usage_kind` that took `charged` from
    /// the run's Slices answers: with the change of the run's Usage.
    fn charged(&self, usage_kind: &str, amount: u64, mut charged: Charged) -> Charged {
        charged.counted = vec![Counted::of(usage_kind, None, signed(amount))];
        if !charged.used.is_empty() {
            tracing::trace!(target: TARGET, transition = "charge", usage_kind, amount, run_id = %self.holder.run_id, epoch = self.holder.epoch, "a run charged Usage against its Slices");
        }
        charged
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
        Ok(self.reserved(usage_kind, currency, worst, taken, charged))
    }

    /// Reserves `worst` as [`Charging::reserve`] does, when that is a step
    /// in memory ([`Charging::charge_held`]'s condition). None, and
    /// nothing reserved, when the reservation needs the ledger:
    /// [`Charging::reserve`] then makes it.
    pub fn reserve_held(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        worst: u64,
        now: Timestamp,
    ) -> Option<(Reservation, Charged)> {
        if !self.covers(usage_kind, currency, worst, now) {
            return None;
        }
        let (taken, charged) = self.draw(usage_kind, currency, worst).ok()?;
        Some(self.reserved(usage_kind, currency, worst, taken, charged))
    }

    /// What a reservation of `worst` of `usage_kind` in `currency` that
    /// took `taken` from the run's Slices answers: the reservation, and
    /// the change of the run's Usage. A currency no Limit caps marks the
    /// run *uncapped*.
    fn reserved(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        worst: u64,
        taken: Vec<(SliceId, u64)>,
        mut charged: Charged,
    ) -> (Reservation, Charged) {
        if taken.is_empty() && currency.is_some() {
            self.uncapped = true;
        }
        charged.counted = vec![Counted::of(usage_kind, currency, signed(worst))];
        tracing::trace!(target: TARGET, transition = "reserve", usage_kind, worst, run_id = %self.holder.run_id, epoch = self.holder.epoch, "a run reserved a worst case against its Slices");
        (
            Reservation {
                worst,
                taken,
                usage_kind: usage_kind.to_owned(),
                currency: currency.map(str::to_owned),
            },
            charged,
        )
    }

    /// Settles `reservation` at `cost`, what was paid, which its worst case
    /// bounds: what it reserved beyond `cost` is the run's to use again.
    /// Answers each Slice touched with the run's Usage of it now, for the
    /// run to journal.
    pub fn settle(&mut self, reservation: Reservation, cost: u64) -> Charged {
        let Reservation {
            worst,
            taken,
            usage_kind,
            currency,
        } = reservation;
        let refund = worst.saturating_sub(cost);
        let mut owed: Vec<(LimitId, u64)> = Vec::new();
        let mut charged = Charged {
            counted: vec![Counted::of(
                &usage_kind,
                currency.as_deref(),
                -signed(refund),
            )],
            ..Charged::default()
        };
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

    /// Who allows the run to pay for `usage_kind` in `currency` (ADRs 0041
    /// and 0045): the setter of the tightest Limit on it holding the run
    /// that a person or a managed Operator policy set, the only ones that
    /// allow paying; none when no such Limit holds it, whatever Limits a
    /// Client set for its own work, which cap payments but never allow
    /// them. Run and Schedule Limits are run parameters, not rows of the
    /// ledger, so they never allow paying either.
    ///
    /// # Errors
    ///
    /// `NotUnderstood` when a Limit the run cannot be placed outside of
    /// appeared, `Ledger` when the ledger cannot be read.
    pub fn allowed_by(
        &mut self,
        usage_kind: &str,
        currency: &str,
    ) -> Result<Option<LimitSetter>, Refusal> {
        let limits = self.ledger.limits_on(&mut self.connection)?;
        let holding = self
            .placement
            .holding(&limits)
            .map_err(|stored| Refusal::NotUnderstood(Box::new(stored.limit())))?;
        Ok(holding
            .into_iter()
            .filter(|stored| {
                stored.usage_kind == usage_kind && stored.currency.as_deref() == Some(currency)
            })
            .map(|stored| (stored.amount, stored.limit().set_by))
            .filter(|(_, set_by)| {
                matches!(set_by, LimitSetter::Person | LimitSetter::ManagedPolicy)
            })
            .min_by_key(|(amount, _)| *amount)
            .map(|(_, set_by)| set_by))
    }

    /// Each Limit that holds the run, with what is left of it in the period
    /// that holds `now` (ADRs 0041 and 0101).
    ///
    /// # Errors
    ///
    /// `NotUnderstood` when a Limit the run cannot be placed outside of
    /// appeared, `Ledger` when the ledger cannot be read.
    pub fn applied(&mut self, now: Timestamp) -> Result<Vec<AppliedLimit>, Refusal> {
        let limits = self.ledger.limits_on(&mut self.connection)?;
        let holding = self
            .placement
            .holding(&limits)
            .map_err(|stored| Refusal::NotUnderstood(Box::new(stored.limit())))?;
        holding
            .into_iter()
            .map(|stored| {
                let (_, bounds) = stored.period_key(now);
                Ok(AppliedLimit {
                    scope: stored.scope.clone(),
                    usage_kind: stored.usage_kind.clone(),
                    currency: stored.currency.clone(),
                    period: stored.known_period(),
                    resets_at: bounds.map(|bounds| point_in_time(bounds.end)),
                    amount: stored.amount,
                    left: self.ledger.left_on(&mut self.connection, stored, now)?,
                    set_by: stored.limit().set_by,
                })
            })
            .collect()
    }

    /// Whether a reservation of this run found a currency no Limit caps.
    pub fn uncapped(&self) -> bool {
        self.uncapped
    }

    /// What the run has now used of each of `slices` it holds, for it to
    /// journal: a charge's journal written later than the charge, with the
    /// Usage of the Slices it took from as it is when the journal is queued.
    pub fn used_of(&self, slices: &[SliceId]) -> Vec<(SliceId, u64)> {
        self.held
            .iter()
            .filter(|held| slices.contains(&held.id))
            .map(|held| (held.id, held.used))
            .collect()
    }

    /// Makes `change` at `now` as a step in memory, when it is one: every
    /// charge and reservation in it is one the held Slices cover while
    /// the Limits held are current, and it reaches neither SQL nor a
    /// file, so a Run makes it where it asks for it, with no blocking
    /// call (E3-3 of the audit #1882, issue 2603). None when it needs the
    /// ledger, with nothing changed once `change` gave back what it had
    /// taken on the way, the run no more *uncapped* than it was:
    /// [`Charging::change`] then makes it.
    pub fn change_held<T>(
        &mut self,
        now: Timestamp,
        change: impl FnOnce(&mut Change<'_>) -> Result<T, Unmade>,
    ) -> Option<T> {
        let uncapped = self.uncapped;
        let made = change(&mut Change {
            charging: self,
            now,
            reach: Reach::Held,
        })
        .ok();
        if made.is_none() {
            // A price reserved in a currency no Limit caps, then given
            // back, was not paid: the ledger says whether one caps it by
            // the time the change is made.
            self.uncapped = uncapped;
        }
        made
    }

    /// Makes `change` at `now`, reaching the ledger for what it needs: a
    /// Slice, the Limits, a refusal. It blocks then: run it under a
    /// blocking policy.
    ///
    /// # Errors
    ///
    /// As [`Charging::charge`]'s: `change` then gives back what it had
    /// taken on the way.
    pub fn change<T>(
        &mut self,
        now: Timestamp,
        change: impl FnOnce(&mut Change<'_>) -> Result<T, Unmade>,
    ) -> Result<T, Refusal> {
        change(&mut Change {
            charging: self,
            now,
            reach: Reach::Ledger,
        })
        .map_err(|unmade| {
            unmade
                .refusal()
                .expect("invariant: a change that reaches the ledger is made or refused")
        })
    }

    /// Takes `amount` from the run's Slices of every Limit on `usage_kind`
    /// in `currency` that holds it, taking a Slice of each Limit that runs
    /// short; answers what it took of each Slice, and the run's Usage of
    /// each Slice touched. It reads the Limits from the ledger only when
    /// those it holds may no longer be the ledger's, and before it takes a
    /// Slice, which is taken of the Limit as the ledger holds it then (ADR
    /// 0041): a charge its Slices cover reads nothing.
    fn take(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        amount: u64,
        now: Timestamp,
    ) -> Result<(Vec<(SliceId, u64)>, Charged), Refusal> {
        let read = !self.current(now);
        if read {
            self.read(now, Reading::Charge)?;
        }
        if self.short(usage_kind, currency, amount, now)? {
            if !read {
                self.read(now, Reading::Charge)?;
            }
            self.take_slices(usage_kind, currency, amount, now)?;
        }
        self.draw(usage_kind, currency, amount)
    }

    /// Whether the Slices the run holds lack room for `amount` of a Limit
    /// on `usage_kind` in `currency` that holds it, each such Limit placed
    /// in its period at `now`.
    ///
    /// # Errors
    ///
    /// `NotUnderstood` when a Limit the run cannot be placed outside of
    /// appeared.
    fn short(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        amount: u64,
        now: Timestamp,
    ) -> Result<bool, Refusal> {
        let Self { limits, held, .. } = self;
        // Every one of them is placed, the first short one's followers
        // too: the charge draws on each in the period it is placed in.
        Ok(limits
            .capping(usage_kind, currency, now)?
            .fold(false, |short, holding| {
                short | (holding.room(held) < amount)
            }))
    }

    /// Takes a Slice, large enough, of each Limit on `usage_kind` in
    /// `currency` that holds the run and whose Slices lack room for
    /// `amount` at `now`.
    ///
    /// # Errors
    ///
    /// `Reached` naming the first Limit with too little left, though the
    /// Slices taken before it stay the run's; `NotUnderstood` and `Ledger`
    /// as [`Charging::charge`]'s.
    fn take_slices(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        amount: u64,
        now: Timestamp,
    ) -> Result<(), Refusal> {
        let Self {
            ledger,
            holder,
            sizing,
            held,
            limits,
            connection,
            ..
        } = self;
        for holding in limits.capping(usage_kind, currency, now)? {
            let room = holding.room(held);
            if room >= amount {
                continue;
            }
            let Some(slice) = ledger.take_slice(
                connection,
                &holding.stored,
                holder,
                now,
                *sizing,
                amount - room,
            )?
            else {
                return Err(reached(&holding.stored, now));
            };
            held.push(Held {
                limit: holding.stored.id,
                period: holding.period.clone(),
                id: slice.id,
                size: slice.size,
                used: 0,
            });
        }
        Ok(())
    }

    /// Takes `amount` from the run's Slices of each Limit on `usage_kind`
    /// in `currency` that holds it, which have room for it; answers what
    /// it took of each Slice, and the run's Usage of each Slice touched.
    ///
    /// # Errors
    ///
    /// `NotUnderstood` when a Limit the run cannot be placed outside of
    /// appeared: nothing is taken then.
    fn draw(
        &mut self,
        usage_kind: &str,
        currency: Option<&str>,
        amount: u64,
    ) -> Result<(Vec<(SliceId, u64)>, Charged), Refusal> {
        let Self { limits, held, .. } = self;
        let mut taken = Vec::new();
        let mut charged = Charged::default();
        for holding in limits
            .holding()?
            .iter()
            .filter(|holding| holding.caps(usage_kind, currency))
        {
            let mut owed = amount;
            for held in held.iter_mut().filter(|held| holding.holds(held)) {
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
        self.ledger.give_back(&mut self.connection, &used)?;
        self.held.clear();
        Ok(())
    }
}

/// `amount` as a change of a run's Usage.
fn signed(amount: u64) -> i64 {
    i64::try_from(amount).unwrap_or(i64::MAX)
}

/// `interval` on the engine's clock, the longest it holds past its range.
fn read_interval(interval: std::time::Duration) -> SignedDuration {
    SignedDuration::try_from(interval).unwrap_or(SignedDuration::MAX)
}

fn reached(stored: &StoredLimit, now: Timestamp) -> Refusal {
    let (_, bounds) = stored.period_key(now);
    Refusal::Reached(Box::new(stored.reached(bounds.map(|bounds| bounds.end))))
}
