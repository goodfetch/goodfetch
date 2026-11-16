//! A Politeness of one Run's own (`politeness.sharing = run`), kept in
//! memory without SQLite (issue 2171): its Permits, waiters and wakes
//! under the same rules as the file's, and each contribution's values in
//! [`MemoryTables`], so a take, a give back or a change is plain work on
//! values, with no transaction and no open (ADR 0140's E6).

use std::any::{Any, TypeId};
use std::collections::HashMap;
use std::time::SystemTime;

use super::{
    Ending, Holder, IN_MEMORY_ENDPOINT, KeyState, Permit, PermitRule, Step, Taken, Turn, Verdict,
    WAKE_GRACE, Wake, milliseconds,
};

/// The values a Politeness of one Run's own keeps for its contributions,
/// one table of each type, such as Foxhound's domain keys' limits by key:
/// what a [`KeyState`] loads from and saves to when no file holds it.
#[derive(Default)]
pub struct MemoryTables(HashMap<TypeId, Box<dyn Any + Send>>);

impl MemoryTables {
    /// The table of type `T`; `None` while nothing was saved to it.
    pub fn get<T: Any + Send>(&self) -> Option<&T> {
        self.0
            .get(&TypeId::of::<T>())
            .and_then(|table| table.downcast_ref())
    }

    /// The table of type `T`, made empty the first time.
    pub fn get_mut<T: Any + Send + Default>(&mut self) -> &mut T {
        self.0
            .entry(TypeId::of::<T>())
            .or_insert_with(|| Box::new(T::default()))
            .downcast_mut()
            .expect("invariant: a table is kept under its own type")
    }
}

/// A waiter of a key, as the file's `waiters` row keeps it; its endpoint
/// is always this engine's.
#[derive(Debug)]
struct Waiter {
    run_id: String,
    turn: i64,
    woken: i64,
    woken_at: i64,
}

/// The Permits, waiters and contributions' values of one Run's own
/// Politeness: the file's tables as plain values.
#[derive(Default)]
pub(super) struct Store {
    /// The key of each Permit held, by its ID.
    permits: HashMap<i64, String>,
    /// The ID the next Permit takes.
    next_permit: i64,
    /// How many Permits are held under each key.
    in_flight: HashMap<String, u64>,
    /// The waiters of each key.
    waiters: HashMap<String, Vec<Waiter>>,
    /// How many Permits under each key were given back.
    generations: HashMap<String, i64>,
    tables: MemoryTables,
}

impl Store {
    fn in_flight(&self, key: &str) -> u64 {
        self.in_flight.get(key).copied().unwrap_or_default()
    }

    fn generation(&self, key: &str) -> i64 {
        self.generations.get(key).copied().unwrap_or_default()
    }

    /// As the file's `taken_room`: the Permits held under `key`, and the
    /// room kept for another run's waiter woken within [`WAKE_GRACE`] for
    /// the latest give back.
    fn taken_room(&self, key: &str, run_id: &str, now: SystemTime) -> u64 {
        let generation = self.generation(key);
        let since =
            milliseconds(now).saturating_sub(i64::try_from(WAKE_GRACE.as_millis()).unwrap_or(0));
        let kept = self.waiters.get(key).map_or(0, |waiters| {
            waiters
                .iter()
                .filter(|waiter| {
                    waiter.run_id != run_id
                        && waiter.woken > 0
                        && waiter.woken == generation
                        && waiter.woken_at > since
                })
                .count()
        });
        self.in_flight(key) + u64::try_from(kept).unwrap_or_default()
    }

    /// The turn after the last of `key`'s waiters.
    fn back_of_turns(&self, key: &str) -> i64 {
        self.waiters
            .get(key)
            .and_then(|waiters| waiters.iter().map(|waiter| waiter.turn).max())
            .unwrap_or_default()
            + 1
    }

    /// As the file's `take_in`, for a rule `R`: the key's state is loaded
    /// once, for the admission and the start, and only what the start
    /// wrote is saved. No Run of another engine holds a Permit here, so
    /// nothing is reclaimed.
    pub(super) fn taken<R: PermitRule>(
        &mut self,
        key: &str,
        holder: &Holder,
        rule: &R,
        now: SystemTime,
        in_turn: bool,
    ) -> (Taken, Option<Turn>) {
        let in_flight = self.taken_room(key, &holder.run_id, now);
        let mut state = R::State::load_from(&self.tables, key, rule.reads(Step::Take));
        let mut turn = None;
        let taken = match rule.admit(&state, in_flight, now) {
            Verdict::Go => {
                let id = self.next_permit;
                self.next_permit += 1;
                self.permits.insert(id, key.to_owned());
                *self.in_flight.entry(key.to_owned()).or_default() += 1;
                let writes = rule.started(&mut state, now);
                state.save_to(&mut self.tables, key, &writes);
                if in_turn && let Some(waiters) = self.waiters.get_mut(key) {
                    waiters.retain(|waiter| waiter.run_id != holder.run_id);
                }
                Taken::Granted(Permit {
                    id,
                    key: key.to_owned(),
                })
            }
            Verdict::Until(until, hold) => Taken::Wait(Some((until, hold))),
            Verdict::Refused(until) => Taken::Refused(until),
            Verdict::Busy => {
                if in_turn {
                    turn = self.wait_in_turn(key, &holder.run_id, now);
                }
                Taken::Wait(None)
            }
        };
        (taken, turn)
    }

    /// As the file's `wait_in_turn`.
    fn wait_in_turn(&mut self, key: &str, run_id: &str, now: SystemTime) -> Option<Turn> {
        let generation = self.generation(key);
        let back = self.back_of_turns(key);
        let waiters = self.waiters.entry(key.to_owned()).or_default();
        match waiters.iter_mut().find(|waiter| waiter.run_id == run_id) {
            None => {
                waiters.push(Waiter {
                    run_id: run_id.to_owned(),
                    turn: back,
                    woken: generation,
                    woken_at: 0,
                });
                None
            }
            Some(waiter) if waiter.woken == generation && generation > 0 => {
                waiter.turn = back;
                waiter.woken_at = 0;
                self.next_turn(key, generation, now)
            }
            Some(_) => None,
        }
    }

    /// As the file's `next_turn`: the first waiter of `key` in turn not
    /// yet woken for `generation`, marked woken for it at `now`.
    fn next_turn(&mut self, key: &str, generation: i64, now: SystemTime) -> Option<Turn> {
        let waiter = self
            .waiters
            .get_mut(key)?
            .iter_mut()
            .filter(|waiter| waiter.woken < generation)
            .min_by_key(|waiter| waiter.turn)?;
        waiter.woken = generation;
        waiter.woken_at = milliseconds(now);
        Some(Turn {
            endpoint: IN_MEMORY_ENDPOINT.to_owned(),
            wake: Wake {
                run_id: waiter.run_id.clone(),
                key: key.to_owned(),
            },
            generation,
            now,
        })
    }

    /// As the file's `given_back_in`, for a rule `R`.
    pub(super) fn given_back<R: PermitRule>(
        &mut self,
        permit: &Permit,
        rule: &R,
        ending: Ending,
        now: SystemTime,
    ) -> (Option<SystemTime>, Option<Turn>) {
        if let Some(key) = self.permits.remove(&permit.id)
            && let Some(count) = self.in_flight.get_mut(&key)
        {
            *count = count.saturating_sub(1);
        }
        let others = self.in_flight(&permit.key);
        let mut state = R::State::load_from(&self.tables, &permit.key, rule.reads(Step::GiveBack));
        let (ready_at, writes) = rule.ended(&mut state, others, ending, now);
        state.save_to(&mut self.tables, &permit.key, &writes);
        let generation = {
            let generation = self.generations.entry(permit.key.clone()).or_default();
            *generation += 1;
            *generation
        };
        let turn = self.next_turn(&permit.key, generation, now);
        (ready_at, turn)
    }

    /// Withdraws `run_id` as a waiter of every key.
    pub(super) fn stop_waiting(&mut self, run_id: &str) {
        for waiters in self.waiters.values_mut() {
            waiters.retain(|waiter| waiter.run_id != run_id);
        }
    }

    /// As [`Politeness::change`](super::Politeness::change): loads what
    /// `reads` names, runs `change` and saves what it wrote.
    pub(super) fn change<S: KeyState, T>(
        &mut self,
        key: &S::Key,
        reads: S::Reads,
        change: impl FnOnce(&mut S) -> (T, S::Writes),
    ) -> T {
        let mut state = S::load_from(&self.tables, key, reads);
        let (answer, writes) = change(&mut state);
        state.save_to(&mut self.tables, key, &writes);
        answer
    }
}
