//! A live Run's charging (ADR 0145): the one value through which a Run
//! charges, reserves and settles Usage against the Limits that hold it,
//! reads the Limits that apply to it and gives back its Slices as it ends.
//! The Runs' registry builds it as a Run opens, with the value of its that
//! asks the Run to stop and the command that reports a ledger that cannot
//! be stored; the Run's handle hands it out, and its charging methods only
//! forward to it. What a charge took and counted is journaled in the Run's
//! run state under its run epoch, by the Runs' module's [`Journal`], whose
//! tables they are.

use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;

use goodfetch_protocol::run::{
    AppliedLimit, LimitSetter, NotUnderstoodLimit, ReachedLimit, StopReason,
};
use jiff::Timestamp;
use rusqlite::Connection;

use super::{Change, Charged, Charging, Counted, Refusal, Reservation, Unmade};
use crate::ledger::{LedgerError, SliceId};
use crate::run_identity::{RunIdentity, StopAsker};
use crate::run_state::{ReservedWrite, RunState, RunStateError, Syncing};
use crate::tasks::TaskStarter;

/// Journals in a Run's run state, under its run epoch, what a charge took
/// from each Slice and what it counted, and that the Run is *uncapped*
/// once it is.
pub(crate) type Journal = fn(&Connection, u64, &Charged, bool) -> rusqlite::Result<()>;

/// Reports to the Runs' registry that the Usage ledger in a folder cannot
/// be stored: a command to its loop, which stops every Run of the engine
/// (ADR 0122).
pub(crate) type StorageFailed =
    Box<dyn Fn(PathBuf) -> Pin<Box<dyn Future<Output = ()> + Send>> + Send + Sync>;

/// What the Runs' registry builds a live Run's charging from.
pub(crate) struct ChargingParts {
    /// Its Slices of the Limits that hold it, read from nothing yet.
    pub(crate) charging: Charging,
    /// Its identity: its clock places each charge in its period, and its
    /// task owner runs the ledger's blocking calls.
    pub(crate) identity: RunIdentity,
    /// Its run state, which journals what it charged.
    pub(crate) state: RunState,
    /// What asks it to stop when a Limit refuses a charge.
    pub(crate) stop: StopAsker,
    /// What journals a charge in its run state.
    pub(crate) journal: Journal,
    /// What a ledger that cannot be stored does.
    pub(crate) storage_failed: StorageFailed,
}

/// Why a Run could not charge Usage.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ChargeError {
    /// The Usage ledger refused it: a Limit has too little left, which asks
    /// the Run to stop at its next safe point, or cannot be evaluated, or
    /// the ledger could not be read or written.
    #[error(transparent)]
    Refused(#[from] Refusal),
    /// What it charged could not be journaled in its run state.
    #[error(transparent)]
    State(#[from] RunStateError),
}

/// What a Run charged with [`RunCharging::charge_unjournaled`] that no
/// journal holds yet: the Slices its charges took from, and what they
/// counted in the Run's Usage, which [`RunCharging::queue_charged`]
/// journals.
#[derive(Debug, Default)]
pub struct UnjournaledCharges {
    slices: Vec<SliceId>,
    counted: Vec<Counted>,
}

impl UnjournaledCharges {
    /// Whether no charge is held.
    pub fn is_empty(&self) -> bool {
        self.slices.is_empty() && self.counted.is_empty()
    }

    /// Adds `charged`: each Slice it touched once, and its counts summed
    /// with those of the same kind, currency and part.
    fn add(&mut self, charged: Charged) {
        for (slice, _) in charged.used {
            if !self.slices.contains(&slice) {
                self.slices.push(slice);
            }
        }
        for counted in charged.counted {
            match self.counted.iter_mut().find(|held| {
                held.usage_kind == counted.usage_kind
                    && held.currency == counted.currency
                    && held.part == counted.part
            }) {
                Some(held) => held.delta += counted.delta,
                None => self.counted.push(counted),
            }
        }
    }
}

/// What a Run reserved ahead of a part of a Usage kind it is about to use,
/// such as a Session's proxy bytes under its proxy kind (ADRs 0021 and
/// 0041): the amount it grants, and that amount's cost when priced, each a
/// reservation settled at what was used ([`RunCharging::settle_part`]).
#[derive(Debug)]
#[must_use = "an allowance is settled once its use is known"]
pub struct Allowance {
    granted: u64,
    amount: Reservation,
    price: Option<Reservation>,
    part: String,
}

impl Allowance {
    /// The amount it grants.
    pub fn granted(&self) -> u64 {
        self.granted
    }
}

/// A live Run's charging: what charges, reserves and settles its Usage,
/// and what a refusal does to it. Cloning it is one reference count.
#[derive(Clone)]
pub struct RunCharging(Arc<Inner>);

struct Inner {
    /// Its Slices of the Limits that hold it, until it ends: taken in the
    /// ledger's blocking calls and as the Run is made, and tried, never
    /// waited for, by a change that is a step in memory (a charge or a
    /// reservation its Slices cover, a settlement), which calls nothing
    /// that blocks; never held across an await.
    #[expect(
        clippy::disallowed_types,
        reason = "the ledger's calls block: one Mutex guards a Run's charging, taken inside them, or tried without waiting, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
    )]
    charging: std::sync::Mutex<Option<Charging>>,
    identity: RunIdentity,
    state: RunState,
    stop: StopAsker,
    journal: Journal,
    storage_failed: StorageFailed,
}

impl Inner {
    /// Its charging, locked: called inside the ledger's blocking calls,
    /// and as the Run is made, before anything else can hold it.
    fn charging(&self) -> std::sync::MutexGuard<'_, Option<Charging>> {
        self.charging
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Its charging, when nothing else holds it: for a step in memory,
    /// outside the blocking calls, which never waits for the lock. A
    /// ledger call holds it, a claim's journal as it is queued, and
    /// another task's step in memory on a runtime of several threads.
    fn charging_if_free(&self) -> Option<std::sync::MutexGuard<'_, Option<Charging>>> {
        match self.charging.try_lock() {
            Ok(held) => Some(held),
            Err(std::sync::TryLockError::Poisoned(poisoned)) => Some(poisoned.into_inner()),
            Err(std::sync::TryLockError::WouldBlock) => None,
        }
    }
}

impl RunCharging {
    /// A live Run's charging, of `parts`.
    pub(crate) fn new(parts: ChargingParts) -> Self {
        let ChargingParts {
            charging,
            identity,
            state,
            stop,
            journal,
            storage_failed,
        } = parts;
        Self(Arc::new(Inner {
            #[expect(
                clippy::disallowed_types,
                reason = "the Run's charging, the Mutex its field declares (ADR 0060)"
            )]
            charging: std::sync::Mutex::new(Some(charging)),
            identity,
            state,
            stop,
            journal,
            storage_failed,
        }))
    }

    /// Holds `charging` in place of the one it was made with: a new Run's
    /// start read its Limits with it.
    pub(crate) fn hold(&self, charging: Charging) {
        *self.0.charging() = Some(charging);
    }

    /// The wall time now, which places each charge in its period.
    fn now(&self) -> Timestamp {
        self.0.identity.clock().timestamp()
    }

    /// The Limits that apply to the Run, with what is left of each (ADR
    /// 0101), as its charging holds them: none once it ended.
    ///
    /// # Errors
    ///
    /// `Refused` when the Usage ledger cannot be read.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future loses only the answer.
    pub(crate) async fn applied(&self) -> Result<Vec<AppliedLimit>, ChargeError> {
        let inner = Arc::clone(&self.0);
        let now = self.now();
        Ok(self
            .0
            .identity
            .owner()
            .blocking(move || {
                inner
                    .charging()
                    .as_mut()
                    .map(|charging| charging.applied(now))
                    .transpose()
            })
            .await?
            .unwrap_or_default())
    }

    /// Reads the Limits that hold the Run as it resumes, on the connection
    /// its charging keeps, under `owner`'s blocking work, as a new Run's
    /// start does (ADR 0140's E3). A read that fails is left to its first
    /// charge, which reads again and refuses as a charge does.
    pub(crate) async fn read_limits(&self, owner: &TaskStarter) {
        let inner = Arc::clone(&self.0);
        let now = self.now();
        owner
            .blocking(move || {
                if let Some(charging) = inner.charging().as_mut()
                    && let Err(refusal) = charging.resume(now)
                {
                    tracing::debug!(%refusal, "the resumed Run's Limits could not be read; its first charge reads them again");
                }
            })
            .await;
    }

    /// Gives back what is left of the Slices the Run holds, under
    /// `writer`'s blocking work, and answers the Limits that then apply to
    /// it, with what is left of each (ADR 0101). Its charging ends: it
    /// charges and settles nothing after.
    pub(crate) async fn give_back(&self, writer: &TaskStarter) -> Vec<AppliedLimit> {
        let inner = Arc::clone(&self.0);
        let now = self.now();
        writer
            .blocking(move || {
                let Some(mut charging) = inner.charging().take() else {
                    return Ok(Vec::new());
                };
                if let Err(error) = charging.give_back() {
                    tracing::warn!(%error, "the Run's Slices were not given back; the next engine to open the Usage ledger reclaims them");
                }
                charging.applied(now)
            })
            .await
            .unwrap_or_else(|error| {
                tracing::warn!(%error, "the Limits that apply to the Run could not be read; its report goes without them");
                Vec::new()
            })
    }

    /// Charges `amount` of `usage_kind` against every Limit that holds the
    /// Run, before the Run uses it (ADR 0041): out of its Slices, taking
    /// another Slice in one ledger transaction when they run out, then
    /// journaled in its run state under its run epoch, from which an engine
    /// reclaiming its Slices after a crash charges them. A Run with nothing
    /// left of a Limit, or covered by a Limit flagged *must understand*
    /// that this engine cannot evaluate, is asked to stop at its next safe
    /// point for that Limit, and must not use what it was refused. A charge
    /// its Slices cover is a step in memory, made on the task that asks
    /// with no blocking call (ADR 0140's E3); one that needs the ledger
    /// runs under the Run's blocking policy, as does one that finds the
    /// Run's charging held.
    ///
    /// # Errors
    ///
    /// `Refused` when the ledger refused the charge or could not be read or
    /// written, `State` when the run state could not journal it: nothing
    /// may be used then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a Slice taken or the charge journaled,
    /// never more charged than the Run used.
    pub async fn charge(&self, usage_kind: &str, amount: u64) -> Result<(), ChargeError> {
        let kind = usage_kind.to_owned();
        self.charged(move |change| change.charge(&kind, amount).map(|charged| ((), charged)))
            .await
            .map(drop)
    }

    /// Charges `amount` of `usage_kind` as [`RunCharging::charge`] does,
    /// and counts it under `part` of the kind in the Run's Usage, such as
    /// the proxy kind of `proxy_bytes` (ADR 0021), in the same run-state
    /// write; with `price`, a currency and a cost already paid, its cost
    /// too, reserved as [`RunCharging::reserve`] does at that cost: both in
    /// one ledger transaction, so a Limit that refuses one charges neither.
    ///
    /// # Errors
    ///
    /// As [`RunCharging::charge`].
    ///
    /// # Cancel safety
    ///
    /// As [`RunCharging::charge`]'s.
    pub async fn charge_part(
        &self,
        usage_kind: &str,
        part: &str,
        amount: u64,
        price: Option<(&str, u64)>,
    ) -> Result<(), ChargeError> {
        let (kind, part) = (usage_kind.to_owned(), part.to_owned());
        let price = price.map(|(currency, cost)| (currency.to_owned(), cost));
        self.charged(move |change| {
            let priced = match &price {
                Some((currency, cost)) => Some(change.reserve(&kind, Some(currency), *cost)?),
                None => None,
            };
            let mut charged = match change.charge(&kind, amount) {
                Ok(charged) => charged,
                Err(unmade) => {
                    if let Some((reservation, _)) = priced {
                        // The price is given back: nothing was used.
                        let _ = change.settle(reservation, 0);
                    }
                    return Err(unmade);
                }
            };
            for counted in &mut charged.counted {
                counted.part = Some(part.clone());
            }
            if let Some((reservation, priced)) = priced {
                // A cost already paid is its own worst case: it stays
                // charged as reserved.
                drop(reservation);
                charged.used.extend(priced.used);
                charged.counted.extend(priced.counted);
            }
            Ok(((), charged))
        })
        .await
        .map(drop)
    }

    /// Charges `amount` of `usage_kind` as [`RunCharging::charge`] does,
    /// without journaling it: what it charged joins `charges`, which
    /// [`RunCharging::queue_charged`] journals in the write that uses it,
    /// so the journal and that write commit as one job of the run state's
    /// writer.
    ///
    /// # Errors
    ///
    /// `Refused` as [`RunCharging::charge`]'s, which asks the Run to stop
    /// as its does; nothing joins `charges` then.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a Slice taken, or the charge made and
    /// not in `charges`: charged and never journaled, before anything used
    /// it, so never more charged than the Run used.
    pub async fn charge_unjournaled(
        &self,
        charges: &mut UnjournaledCharges,
        usage_kind: &str,
        amount: u64,
    ) -> Result<(), ChargeError> {
        let now = self.now();
        let kind = usage_kind.to_owned();
        // A charge the Run's Slices cover is a step in memory, made here
        // with no blocking task (ADR 0140's E3; E3-3 of the audit #1882).
        let made = self
            .on_charging(
                (),
                move |charging, ()| {
                    charging
                        .charge_held(usage_kind, amount, now)
                        .map(Ok)
                        .ok_or(())
                },
                move |charging, ()| charging.charge(&kind, amount, now),
            )
            .await;
        match made {
            None => Ok(()),
            Some(Ok(journaled)) => {
                charges.add(journaled);
                Ok(())
            }
            Some(Err(refusal)) => Err(self.refused(refusal).await),
        }
    }

    /// Queues `work` as one write of the Run's state that first journals
    /// `charges` under its run epoch, as [`RunCharging::charge`] journals
    /// each: with what the Run has now used of each Slice they took from,
    /// read under its charging's lock as the write takes its place in the
    /// queue, so journals reach the run state in the order of the changes.
    /// A Run whose charging ended journals nothing. Answers once the write
    /// is queued, with the write, which answers what `work` returned once
    /// its commit is acknowledged, before a sync of its own: it is an
    /// attempt's claim, which a power cut may take, and the writer bounds
    /// what such writes charge by [`DEFERRED_CHARGES`] (ADR 0138's note of
    /// #1926). At a level that batches attempts it rides the open batch
    /// once the file declares its charges unsynced: answered once its
    /// savepoint is released, and seen by a reader only once the batch
    /// commits (ADR 0142). The caller need not wait for it: dropping it
    /// does not withdraw the write, and every write queued after it runs
    /// after it.
    ///
    /// [`DEFERRED_CHARGES`]: crate::run_state::DEFERRED_CHARGES
    ///
    /// # Errors
    ///
    /// `State` when the run state's writer stopped; the write answers
    /// `State` when `work` or its commit failed, or the writer stopped.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future once the write is queued does not withdraw it.
    pub async fn queue_charged<T, F>(
        &self,
        charges: UnjournaledCharges,
        work: F,
    ) -> Result<impl Future<Output = Result<T, ChargeError>> + Send + 'static, ChargeError>
    where
        T: Send + 'static,
        F: FnOnce(&rusqlite::Connection) -> rusqlite::Result<T> + Send + 'static,
    {
        let place = self.0.state.reserve().await?;
        let inner = Arc::clone(&self.0);
        let epoch = self.0.state.epoch();
        let journal = self.0.journal;
        let written = self
            .0
            .identity
            .owner()
            .blocking(move || {
                let held = inner.charging();
                let at_most = charges
                    .counted
                    .iter()
                    .map(|counted| u64::try_from(counted.delta).unwrap_or(0))
                    .fold(0_u64, u64::saturating_add);
                let journaled = held.as_ref().map(|charging| {
                    let UnjournaledCharges { slices, counted } = charges;
                    (
                        Charged {
                            used: charging.used_of(&slices),
                            counted,
                        },
                        charging.uncapped(),
                    )
                });
                place.write_with(Syncing::Deferred { charged: at_most }, move |connection| {
                    if let Some((journaled, uncapped)) = journaled
                        && (!journaled.used.is_empty() || !journaled.counted.is_empty() || uncapped)
                    {
                        journal(connection, epoch, &journaled, uncapped)?;
                    }
                    work(connection)
                })
            })
            .await;
        Ok(async move { Ok(written.await?) })
    }

    /// Reserves ahead an amount of `usage_kind` the Run is about to use, as
    /// [`RunCharging::reserve`] reserves a worst case, counted under `part`
    /// of the kind, and with `currency` its `cost` too, in one ledger
    /// transaction: `wanted` when every Limit holding the Run allows it,
    /// else the largest of its halves down to `least` that they allow, so a
    /// large allowance that a nearly spent Limit refuses gives way to what
    /// is left of it, and only `least`'s refusal refuses it and asks the
    /// Run to stop as [`RunCharging::charge`]'s does. What the Run then
    /// uses of it is settled with [`RunCharging::settle_part`], so its
    /// Usage counts what it used: an amount charged before it is used, with
    /// one ledger and journal write per allowance instead of one per use
    /// (the deep review's 599-12). None once the Run ended.
    ///
    /// # Errors
    ///
    /// As [`RunCharging::charge`]: none of it may be used then.
    ///
    /// # Cancel safety
    ///
    /// As [`RunCharging::charge`]'s: an allowance dropped unsettled stays
    /// charged in full.
    pub async fn reserve_part(
        &self,
        usage_kind: &str,
        part: &str,
        currency: Option<&str>,
        cost: impl Fn(u64) -> u64 + Send + 'static,
        wanted: u64,
        least: u64,
    ) -> Result<Option<Allowance>, ChargeError> {
        let (kind, part) = (usage_kind.to_owned(), part.to_owned());
        let currency = currency.map(str::to_owned);
        self.charged(move |change| {
            let reserve = |change: &mut Change<'_>, amount: u64| {
                let cost = cost(amount);
                let (reserved, mut charged) = change.reserve(&kind, None, amount)?;
                for counted in &mut charged.counted {
                    counted.part = Some(part.clone());
                }
                let price = match &currency {
                    Some(currency) => match change.reserve(&kind, Some(currency), cost) {
                        Ok((price, priced)) => {
                            charged.used.extend(priced.used);
                            charged.counted.extend(priced.counted);
                            Some(price)
                        }
                        Err(unmade) => {
                            // Nothing was used: the amount is given back.
                            let _ = change.settle(reserved, 0);
                            return Err(unmade);
                        }
                    },
                    None => None,
                };
                let allowance = Allowance {
                    granted: amount,
                    amount: reserved,
                    price,
                    part: part.clone(),
                };
                Ok((allowance, charged))
            };
            let least = least.min(wanted);
            let mut amount = wanted;
            loop {
                match reserve(change, amount) {
                    // Only the ledger says a Limit refuses the amount: a
                    // smaller allowance is no step in memory.
                    Err(Unmade::Refused(Refusal::Reached(_))) if amount > least => {
                        amount = (amount / 2).max(least);
                    }
                    reserved => break reserved,
                }
            }
        })
        .await
    }

    /// Settles `allowance` at `used` of its amount and `cost` of its
    /// price, which its grant bounds: the rest is the Run's to use again,
    /// and its Usage then counts what it used.
    ///
    /// # Errors
    ///
    /// `State` when the run state could not journal it: the allowance then
    /// stays charged in full.
    ///
    /// # Cancel safety
    ///
    /// As [`RunCharging::charge`]'s.
    pub async fn settle_part(
        &self,
        allowance: Allowance,
        used: u64,
        cost: u64,
    ) -> Result<(), ChargeError> {
        self.settled(move |charging| {
            let Allowance {
                amount,
                price,
                part,
                ..
            } = allowance;
            let mut settled = charging.settle(amount, used);
            for counted in &mut settled.counted {
                counted.part = Some(part.clone());
            }
            if let Some(price) = price {
                let priced = charging.settle(price, cost);
                settled.used.extend(priced.used);
                settled.counted.extend(priced.counted);
            }
            settled
        })
        .await
    }

    /// Reserves `worst`, the worst case of a cost the Run is about to pay
    /// in `currency`, such as a model call or a paid solve, against every
    /// Limit on `usage_kind` in that currency that holds it (ADR 0041):
    /// taken in one ledger transaction and journaled in its run state
    /// before the Run pays, so that an engine reclaiming its Slices after a
    /// crash charges it at its worst case. A currency no Limit holding the
    /// Run caps marks it *uncapped* in its Run report. A refusal asks the
    /// Run to stop as [`RunCharging::charge`]'s does.
    ///
    /// # Errors
    ///
    /// As [`RunCharging::charge`]: the cost must not be paid then.
    ///
    /// # Cancel safety
    ///
    /// As [`RunCharging::charge`]'s.
    pub async fn reserve(
        &self,
        usage_kind: &str,
        currency: Option<&str>,
        worst: u64,
    ) -> Result<Reservation, ChargeError> {
        let kind = usage_kind.to_owned();
        let currency = currency.map(str::to_owned);
        self.charged(move |change| change.reserve(&kind, currency.as_deref(), worst))
            .await
            .map(Option::unwrap_or_default)
    }

    /// Who allows the Run to pay for `usage_kind` in `currency`, as
    /// [`Charging::allowed_by`] answers (ADR 0045): a person or a managed
    /// Operator policy, or none, when the Run must not pay. A refusal asks
    /// the Run to stop as [`RunCharging::charge`]'s does.
    ///
    /// # Errors
    ///
    /// As [`RunCharging::charge`].
    pub async fn allowed_by(
        &self,
        usage_kind: &str,
        currency: &str,
    ) -> Result<Option<LimitSetter>, ChargeError> {
        let (kind, currency) = (usage_kind.to_owned(), currency.to_owned());
        self.charged(move |change| {
            change
                .allowed_by(&kind, &currency)
                .map(|setter| (setter, Charged::default()))
        })
        .await
        .map(Option::flatten)
    }

    /// Settles `reservation` at `cost`, what the Run paid, which its worst
    /// case bounds: the rest is the Run's to use again, journaled as such.
    ///
    /// # Errors
    ///
    /// `State` when the run state could not journal it: the reservation
    /// then stays charged at its worst case.
    ///
    /// # Cancel safety
    ///
    /// As [`RunCharging::charge`]'s.
    pub async fn settle(&self, reservation: Reservation, cost: u64) -> Result<(), ChargeError> {
        self.settled(move |charging| charging.settle(reservation, cost))
            .await
    }

    /// Makes `change` on the Run's charging, journals what it charged, and
    /// asks the Run to stop at its next safe point when a Limit refused it;
    /// answers none once the Run ended. A change that is a step in memory
    /// ([`Charging::change_held`]) is made where the Run asks for it, with
    /// no blocking call (ADR 0140's E3); one that needs the ledger is made
    /// again, whole, in a blocking call ([`Charging::change`]), so
    /// `change` may run twice. Either way the journal is queued under the
    /// charging's lock ([`RunCharging::on_charging`]), so journals reach
    /// the run state in the order of the changes.
    async fn charged<T, F>(&self, change: F) -> Result<Option<T>, ChargeError>
    where
        T: Send + 'static,
        F: Fn(&mut Change<'_>) -> Result<(T, Charged), Unmade> + Send + 'static,
    {
        let place = self.0.state.reserve().await?;
        let now = self.now();
        let epoch = self.0.state.epoch();
        let journal = self.0.journal;
        let changed = self
            .on_charging(
                (change, place),
                move |charging, (change, place)| match charging.change_held(now, &change) {
                    Some((value, charged)) => Ok(Ok((
                        value,
                        journaled(journal, charging, place, epoch, charged),
                    ))),
                    None => Err((change, place)),
                },
                move |charging, (change, place)| {
                    charging.change(now, &change).map(|(value, charged)| {
                        (value, journaled(journal, charging, place, epoch, charged))
                    })
                },
            )
            .await;
        match changed {
            None => Ok(None),
            Some(Ok((value, journaling))) => {
                if let Some(journaling) = journaling {
                    journaling.await?;
                }
                Ok(Some(value))
            }
            Some(Err(refusal)) => Err(self.refused(refusal).await),
        }
    }

    /// Makes `settle`, a settlement, on the Run's charging and journals
    /// what it gave back: always a step in memory, made where the Run asks
    /// for it unless its charging is held ([`RunCharging::on_charging`]).
    /// A Run that ended settles nothing.
    async fn settled<F>(&self, settle: F) -> Result<(), ChargeError>
    where
        F: FnOnce(&mut Charging) -> Charged + Send + 'static,
    {
        let place = self.0.state.reserve().await?;
        let epoch = self.0.state.epoch();
        let journal = self.0.journal;
        let settled = move |charging: &mut Charging, (settle, place): (F, ReservedWrite)| {
            let charged = settle(charging);
            journaled(journal, charging, place, epoch, charged)
        };
        let journaling = self
            .on_charging(
                (settle, place),
                move |charging, settlement| Ok(settled(charging, settlement)),
                settled,
            )
            .await;
        if let Some(journaling) = journaling.flatten() {
            journaling.await?;
        }
        Ok(())
    }

    /// Makes one change of the Run's charging, under the charging's lock:
    /// `held` where the Run asks for it, with no blocking call, when
    /// nothing else holds the charging; `ledger` in a blocking call, under
    /// the Run's blocking policy, when `held` hands `change` back because
    /// it is no step in memory, and when the charging is held: by a ledger
    /// call, by a claim's journal, or by another task's step in memory.
    /// The lock is tried and never waited for where the Run asks, and
    /// never held across an await. None once the Run ended.
    async fn on_charging<C, T>(
        &self,
        change: C,
        held: impl FnOnce(&mut Charging, C) -> Result<T, C> + Send,
        ledger: impl FnOnce(&mut Charging, C) -> T + Send + 'static,
    ) -> Option<T>
    where
        C: Send + 'static,
        T: Send + 'static,
    {
        let change = match self.0.charging_if_free() {
            Some(mut free) => match free.as_mut().map(|charging| held(charging, change)) {
                None => return None,
                Some(Ok(made)) => return Some(made),
                Some(Err(change)) => change,
            },
            None => change,
        };
        let inner = Arc::clone(&self.0);
        self.0
            .identity
            .owner()
            .blocking(move || {
                inner
                    .charging()
                    .as_mut()
                    .map(|charging| ledger(charging, change))
            })
            .await
    }

    /// What `refusal`, a Limit's or the ledger's, does to the Run: a Limit
    /// with nothing left, or one this engine cannot evaluate, asks it to
    /// stop at its next safe point; a ledger that cannot be stored stops
    /// every Run of the engine (ADR 0122).
    async fn refused(&self, refusal: Refusal) -> ChargeError {
        match refusal {
            Refusal::Reached(reached) => {
                self.0
                    .stop
                    .at_safe_point(StopReason::Limit(Box::new(ReachedLimit::clone(&reached))));
                Refusal::Reached(reached).into()
            }
            Refusal::NotUnderstood(limit) => {
                self.0
                    .stop
                    .at_safe_point(StopReason::LimitNotUnderstood(Box::new(
                        NotUnderstoodLimit {
                            scope: limit.scope.clone(),
                            usage_kind: limit.usage_kind.clone(),
                            currency: limit.currency.clone(),
                            period: limit.period.clone(),
                        },
                    )));
                Refusal::NotUnderstood(limit).into()
            }
            Refusal::Ledger(LedgerError::Storage { path, message }) => {
                // The ledger holds every Run's Limits: each Run of the
                // engine stops as at `critical` (ADR 0122).
                let folder = path.parent().map_or_else(|| path.clone(), Path::to_owned);
                (self.0.storage_failed)(folder).await;
                Refusal::Ledger(LedgerError::Storage { path, message }).into()
            }
            refusal => refusal.into(),
        }
    }
}

/// Queues in `place`, a place in a Run's run state's queue, the journal
/// under `epoch` of what a change of its `charging` charged, by `journal`,
/// when there is something to journal; answers the future of its commit.
/// Called under the charging's lock, so journals are queued in the order
/// of the changes.
fn journaled(
    journal: Journal,
    charging: &Charging,
    place: ReservedWrite,
    epoch: u64,
    charged: Charged,
) -> Option<impl Future<Output = Result<(), RunStateError>> + Send + 'static + use<>> {
    let uncapped = charging.uncapped();
    (!charged.used.is_empty() || !charged.counted.is_empty() || uncapped)
        .then(|| place.write(move |connection| journal(connection, epoch, &charged, uncapped)))
}
