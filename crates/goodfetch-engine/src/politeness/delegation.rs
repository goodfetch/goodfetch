//! The Politeness file's Delegations (ADR 0142, issue 2172): a key whose
//! engine holds its Delegation is served from memory, its Permits granted
//! and given back in [`memory::Store`] with no transaction, whichever of
//! the engine's Runs takes them. The file records each Delegation, its key,
//! Run, run epoch and folder, and the engine's wake endpoint. Another
//! engine's take of that key waits in turn and recalls the Delegation
//! through that endpoint: the holder writes the key's state and a row for
//! each of its Permits in flight to the file, in one transaction, puts its
//! own Runs' waiters ahead of the recaller's turn, gives the Delegation
//! back and wakes the waiter whose turn it is; from then on
//! both engines take the key's Permits in the file, until a give back
//! finds no other engine's Permit or waiter of it. A Delegation whose Run
//! is gone is reclaimed, its delay counted from the reclaim.
//!
//! Delegations are taken by group commit, with no timer: the keys asked
//! for while the actor runs a group of jobs are delegated after it, at most
//! [`DELEGATIONS_PER_TRANSACTION`] in one transaction. A Delegation is
//! given back lazily, once every Run of the engine that took one of its
//! Permits has stopped and none is in flight; a site's ask, such as a
//! backoff or a Crawl-delay, is written to the file as soon as it is
//! learned.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::mem;
use std::path::Path;
use std::sync::PoisonError;
use std::time::SystemTime;

use goodfetch_counted_work::{CountedWork, Row};
use rusqlite::{Connection, OptionalExtension as _};
use tokio::sync::oneshot;

use super::{
    Context, DELEGATE, DELEGATION, DELEGATIONS_PER_TRANSACTION, Ending, Erased, GONE_WAITERS,
    Holder, KeyState, NEXT_GENERATION, Permit, PolitenessError, REHOLD, STOP_WAITING, SharedRule,
    TAKE, Taken, Turn, UNDELEGATE, WAITERS_OF, changed, in_flight, memory, milliseconds, next_turn,
    reclaim, sqlite_error,
};
use crate::sqlite_store;
use crate::sync::{Arc, Mutex};

/// Where a take's answer goes, once its group's Delegations are taken.
pub(super) type Answer = oneshot::Sender<Result<(Taken, Option<Turn>), PolitenessError>>;

/// The row each commit of the file counts on, which the job that runs a
/// transaction names before it: a Permit's, a Delegation's, or another.
#[derive(Clone)]
pub(super) struct Commits(Arc<Mutex<Row>>);

impl Commits {
    /// Has each commit of `connection` count in `work` on the row named
    /// last.
    ///
    /// # Errors
    ///
    /// SQLite's error.
    pub(super) fn count(connection: &Connection, work: &CountedWork) -> rusqlite::Result<Self> {
        let commits = Self(Arc::new(Mutex::new(Row::PolitenessOtherTransactions)));
        let counted = commits.clone();
        let work = work.clone();
        connection.commit_hook(Some(move || {
            work.count(counted.row());
            false
        }))?;
        Ok(commits)
    }

    fn row(&self) -> Row {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn count_on(&self, row: Row) {
        *self.0.lock().unwrap_or_else(PoisonError::into_inner) = row;
    }
}

/// A key's Delegation this engine holds.
struct Delegation {
    /// The Run the file's row names, live while the engine holds it.
    holder: Holder,
    /// The engine's Runs that took a Permit of it and have not stopped.
    runs: Vec<Holder>,
    /// The rule whose state it holds in memory, which writes it back.
    rule: SharedRule,
}

/// A take waiting for its group's Delegations.
struct Asked {
    key: String,
    holder: Holder,
    rule: SharedRule,
    now: SystemTime,
    in_turn: bool,
    answer: Answer,
}

/// The Politeness file's connection, and the keys its Delegations hold in
/// memory.
pub(super) struct FileBackend {
    connection: Connection,
    commits: Commits,
    work: CountedWork,
    store: memory::Store,
    delegations: BTreeMap<String, Delegation>,
    asked: Vec<Asked>,
    /// The file's row of each Permit a recalled Delegation granted and
    /// wrote to the file, by the ID its grant gave it.
    recalled: HashMap<i64, i64>,
    /// The keys another engine also uses, whose Permits are taken in the
    /// file without asking for their Delegation.
    shared: HashSet<String>,
}

/// Runs `work` in one `IMMEDIATE` transaction of `connection`, its commit
/// counted on `row`.
fn transaction<T>(
    connection: &Connection,
    commits: &Commits,
    row: Row,
    work: impl FnOnce(&Connection) -> rusqlite::Result<T>,
) -> rusqlite::Result<T> {
    commits.count_on(row);
    sqlite_store::immediate(connection, work)
}

impl FileBackend {
    pub(super) fn new(connection: Connection, commits: Commits, work: CountedWork) -> Self {
        Self {
            connection,
            commits,
            work,
            store: memory::Store::default(),
            delegations: BTreeMap::new(),
            asked: Vec::new(),
            recalled: HashMap::new(),
            shared: HashSet::new(),
        }
    }

    /// Takes a Permit under `key` for `holder` at `now` and sends the
    /// answer to `answer`: from memory when the engine holds the key's
    /// Delegation, whatever `holder`'s `politeness.delegation`, so the
    /// key's limits hold across its Runs; after the group's Delegations are
    /// taken ([`take_asked_delegations`](Self::take_asked_delegations)) when
    /// `holder` delegates and `rule` may; otherwise in one transaction of
    /// the file.
    #[expect(
        clippy::too_many_arguments,
        reason = "a take's own arguments, beside where its answer goes"
    )]
    pub(super) fn take(
        &mut self,
        context: &Context,
        key: String,
        holder: Holder,
        rule: SharedRule,
        now: SystemTime,
        in_turn: bool,
        answer: Answer,
    ) {
        let delegates = rule.0.delegates();
        if delegates && self.delegations.contains_key(&key) {
            let _ = answer.send(Ok(
                self.taken_delegated(context, &key, &holder, &rule, now, in_turn)
            ));
        } else if delegates && holder.delegates && !self.shared.contains(&key) {
            self.asked.push(Asked {
                key,
                holder,
                rule,
                now,
                in_turn,
                answer,
            });
        } else {
            let _ = answer.send(self.taken_in_file(context, &key, &holder, &rule, now, in_turn));
        }
    }

    /// A take in one transaction of the file, counted on `E6.transactions`.
    fn taken_in_file(
        &self,
        context: &Context,
        key: &str,
        holder: &Holder,
        rule: &SharedRule,
        now: SystemTime,
        in_turn: bool,
    ) -> Result<(Taken, Option<Turn>), PolitenessError> {
        transaction(
            &self.connection,
            &self.commits,
            Row::PolitenessTransactions,
            |transaction| {
                rule.0
                    .taken(transaction, context, key, holder, now, in_turn)
            },
        )
        .map_err(|error| sqlite_error(&context.path, &error))
    }

    /// A take under the key's Delegation, in memory: its waiter is woken
    /// within the engine.
    fn taken_delegated(
        &mut self,
        context: &Context,
        key: &str,
        holder: &Holder,
        rule: &SharedRule,
        now: SystemTime,
        in_turn: bool,
    ) -> (Taken, Option<Turn>) {
        if let Some(delegation) = self.delegations.get_mut(key)
            && !delegation
                .runs
                .iter()
                .any(|run| run.run_id == holder.run_id)
        {
            delegation.runs.push(holder.clone());
        }
        let (mut taken, turn) = rule
            .0
            .taken_in_memory(&mut self.store, key, holder, now, in_turn);
        if let Taken::Granted(permit) = &mut taken {
            permit.delegated = true;
        }
        (taken, turn.map(|turn| context.local(turn)))
    }

    /// Takes the Delegations of the keys asked for since the last group, at
    /// most [`DELEGATIONS_PER_TRANSACTION`] in one transaction counted on
    /// `E6.delegation_transactions`, then answers each take: from memory
    /// under its key's Delegation, otherwise in the file.
    pub(super) fn take_asked_delegations(&mut self, context: &Context) {
        if self.asked.is_empty() {
            return;
        }
        let asked = mem::take(&mut self.asked);
        let mut seen = HashSet::new();
        let keys: Vec<&Asked> = asked
            .iter()
            .filter(|asked| {
                !self.delegations.contains_key(&asked.key) && seen.insert(asked.key.as_str())
            })
            .collect();
        for group in keys.chunks(DELEGATIONS_PER_TRANSACTION) {
            self.delegate(context, group);
        }
        for asked in asked {
            let answer = if self.delegations.contains_key(&asked.key) {
                Ok(self.taken_delegated(
                    context,
                    &asked.key,
                    &asked.holder,
                    &asked.rule,
                    asked.now,
                    asked.in_turn,
                ))
            } else {
                self.taken_in_file(
                    context,
                    &asked.key,
                    &asked.holder,
                    &asked.rule,
                    asked.now,
                    asked.in_turn,
                )
            };
            let _ = asked.answer.send(answer);
        }
    }

    /// Takes in one transaction the Delegation of each key `group` asks
    /// for that no other engine's live Run holds, and that has no Permit
    /// in the file or waiter; a failed transaction takes none, and their
    /// takes go to the file. A key it could not take is shared: its
    /// takes go to the file until a give back finds it no longer is.
    fn delegate(&mut self, context: &Context, group: &[&Asked]) {
        let Some(endpoint) = &context.endpoint else {
            return;
        };
        let tables = &mut self.store.tables;
        let mut delegated = Vec::new();
        let taken = transaction(
            &self.connection,
            &self.commits,
            Row::PolitenessDelegationTransactions,
            |transaction| {
                for asked in group {
                    if delegated_in(transaction, context, endpoint, asked, tables)? {
                        delegated.push(*asked);
                    }
                }
                Ok(())
            },
        );
        match taken {
            Ok(()) => {
                for asked in group {
                    if !delegated.iter().any(|taken| taken.key == asked.key) {
                        self.shared.insert(asked.key.clone());
                    }
                }
                for asked in delegated {
                    self.work.count(Row::PolitenessDelegations);
                    self.delegations.insert(
                        asked.key.clone(),
                        Delegation {
                            holder: asked.holder.clone(),
                            runs: Vec::new(),
                            rule: asked.rule.clone(),
                        },
                    );
                }
            }
            Err(error) => {
                for asked in delegated {
                    asked.rule.0.forget(&mut self.store.tables, &asked.key);
                }
                tracing::warn!(%error, "the Politeness file's Delegations were not taken; their Permits are taken in the file");
            }
        }
    }

    /// Gives `permit` back at `now` after its request ended as `ending`:
    /// in memory when its key's Delegation granted it, a site's ask then
    /// written to the file in one transaction, otherwise in one
    /// transaction of the file.
    pub(super) fn give_back(
        &mut self,
        context: &Context,
        permit: &Permit,
        rule: &SharedRule,
        ending: Ending,
        now: SystemTime,
    ) -> Result<(Option<SystemTime>, Option<Turn>), PolitenessError> {
        let filed = if permit.delegated {
            self.recalled.remove(&permit.id).map(|id| Permit {
                id,
                key: permit.key.clone(),
                send_at: permit.send_at,
                delegated: false,
            })
        } else {
            None
        };
        if let Some(permit) = filed.as_ref().or((!permit.delegated).then_some(permit)) {
            let shared = self.shared.contains(&permit.key);
            let (given, alone) = transaction(
                &self.connection,
                &self.commits,
                Row::PolitenessTransactions,
                |transaction| {
                    let given = rule.0.given_back(transaction, permit, ending, now)?;
                    let alone =
                        shared && used_by_this_engine_alone(transaction, context, &permit.key)?;
                    Ok((given, alone))
                },
            )
            .map_err(|error| sqlite_error(&context.path, &error))?;
            if alone {
                self.shared.remove(&permit.key);
            }
            return Ok(given);
        }
        let (ready_at, turn) = rule
            .0
            .given_back_in_memory(&mut self.store, permit, ending, now);
        if ending != Ending::Done {
            let tables = &self.store.tables;
            transaction(
                &self.connection,
                &self.commits,
                Row::PolitenessOtherTransactions,
                |transaction| rule.0.write_back(tables, &permit.key, transaction),
            )
            .map_err(|error| sqlite_error(&context.path, &error))?;
        }
        if let Some(turn) = turn {
            return Ok((ready_at, Some(context.local(turn))));
        }
        // The last Permit in flight of a Delegation whose Runs all stopped
        // gives it back.
        let idle = self
            .delegations
            .get(&permit.key)
            .is_some_and(|delegation| delegation.runs.is_empty());
        if !idle {
            return Ok((ready_at, None));
        }
        let mut released = self
            .release(now, vec![permit.key.clone()])
            .map_err(|error| sqlite_error(&context.path, &error))?;
        Ok((ready_at, released.pop()))
    }

    /// Changes the state `S` keeps under `key` as
    /// [`Politeness::change`](super::Politeness::change) does: in memory
    /// while the engine holds the Delegation it is kept under, a site's
    /// ask then written to the file; otherwise in one transaction of the
    /// file, counted on [`KeyState::TRANSACTIONS`].
    pub(super) fn change<S: KeyState, T>(
        &mut self,
        context: &Context,
        key: &S::Key,
        reads: S::Reads,
        change: impl FnOnce(&mut S) -> (T, S::Writes),
    ) -> Result<T, PolitenessError> {
        let failed = |error: rusqlite::Error| sqlite_error(&context.path, &error);
        let delegated =
            S::delegated_under(key).is_some_and(|under| self.delegations.contains_key(under));
        if !delegated {
            return transaction(
                &self.connection,
                &self.commits,
                S::TRANSACTIONS,
                |transaction| changed(transaction, key, reads, change),
            )
            .map_err(failed);
        }
        let mut state =
            S::load_delegated(&self.connection, &self.store.tables, key, reads).map_err(failed)?;
        let (answer, writes) = change(&mut state);
        state.save_to(&mut self.store.tables, key, &writes);
        if S::asked(&writes) {
            transaction(
                &self.connection,
                &self.commits,
                Row::PolitenessOtherTransactions,
                |transaction| state.save(transaction, key, &writes),
            )
            .map_err(failed)?;
        }
        Ok(answer)
    }

    /// Gives back the Delegation of `key`, which another engine recalled
    /// at `now`, in one transaction: the key's state written back and a
    /// row in the file for each of its Permits in flight, which their
    /// give backs then delete. Answers the waiters to wake: the file's
    /// whose turn came, and each of this engine's Runs that waited in
    /// memory, which take the key in the file from then on.
    pub(super) fn recall(
        &mut self,
        context: &Context,
        key: &str,
        now: SystemTime,
    ) -> Result<Vec<Turn>, PolitenessError> {
        let Some(delegation) = self.delegations.get(key) else {
            return Ok(Vec::new());
        };
        let (permits, waiting) = self.store.recall_view(key);
        let (holder, tables) = (&delegation.holder, &self.store.tables);
        let endpoint = context.endpoint.as_deref().unwrap_or_default();
        let (rows, turn) = transaction(
            &self.connection,
            &self.commits,
            Row::PolitenessOtherTransactions,
            |transaction| {
                delegation.rule.0.write_back(tables, key, transaction)?;
                let mut rows = Vec::with_capacity(permits.len());
                for _ in &permits {
                    transaction.prepare_cached(TAKE)?.execute((
                        key,
                        &holder.run_id,
                        i64::try_from(holder.epoch).unwrap_or(i64::MAX),
                        holder.run_folder.to_string_lossy(),
                        &context.engine,
                        milliseconds(now),
                    ))?;
                    rows.push(transaction.last_insert_rowid());
                }
                transaction.prepare_cached(UNDELEGATE)?.execute([key])?;
                let generation: i64 = transaction
                    .prepare_cached(NEXT_GENERATION)?
                    .query_row([key], |row| row.get(0))?;
                // The engine's own waiters waited before the recall: they
                // keep their turns ahead of the recaller's, and wait for the
                // next give back as they did in memory.
                for run_id in waiting.iter().rev() {
                    transaction
                        .prepare_cached(JOIN_AHEAD)?
                        .execute((key, run_id, endpoint, generation))?;
                }
                Ok((rows, next_turn(transaction, key, generation, now)?))
            },
        )
        .map_err(|error| sqlite_error(&context.path, &error))?;
        tracing::debug!(
            key,
            in_flight = rows.len(),
            "a recalled Delegation is given back"
        );
        self.recalled.extend(permits.into_iter().zip(rows));
        if let Some(delegation) = self.delegations.remove(key) {
            delegation.rule.0.forget(&mut self.store.tables, key);
        }
        self.store.recall(key);
        self.shared.insert(key.to_owned());
        Ok(turn.into_iter().collect())
    }

    /// Withdraws `run_id` as a waiter of every key, as its Run stops, and
    /// gives back the Delegations no running Run of the engine holds a
    /// Permit of; answers the waiters whose turn came.
    pub(super) fn stop_waiting(
        &mut self,
        context: &Context,
        run_id: &str,
        now: SystemTime,
    ) -> Result<Vec<Turn>, PolitenessError> {
        let failed = |error: rusqlite::Error| sqlite_error(&context.path, &error);
        let endpoint = context.endpoint.as_deref().unwrap_or_default();
        transaction(
            &self.connection,
            &self.commits,
            Row::PolitenessOtherTransactions,
            |transaction| {
                transaction
                    .prepare_cached(STOP_WAITING)?
                    .execute((run_id, endpoint))
                    .map(drop)
            },
        )
        .map_err(failed)?;
        self.store.stop_waiting(run_id);
        let mut stopped = Vec::new();
        for (key, delegation) in &mut self.delegations {
            let before = delegation.runs.len();
            delegation.runs.retain(|run| run.run_id != run_id);
            if delegation.runs.len() < before || delegation.holder.run_id == run_id {
                stopped.push(key.clone());
            }
        }
        self.release(now, stopped).map_err(failed)
    }

    /// Gives back, in one transaction, each Delegation of `keys` no
    /// running Run of the engine took a Permit of with none in flight, its
    /// state written back to the file, and names another running Run in
    /// the row of one whose Run stopped; answers the waiters of the keys
    /// given back whose turn came.
    fn release(&mut self, now: SystemTime, keys: Vec<String>) -> rusqlite::Result<Vec<Turn>> {
        let mut idle = Vec::new();
        let mut rehold = Vec::new();
        for key in keys {
            let Some(delegation) = self.delegations.get(&key) else {
                continue;
            };
            if delegation.runs.is_empty() {
                if self.store.in_flight(&key) == 0 {
                    idle.push(key);
                }
            } else if !delegation
                .runs
                .iter()
                .any(|run| run.run_id == delegation.holder.run_id)
                && let Some(run) = delegation.runs.first()
            {
                rehold.push((key, run.clone()));
            }
        }
        if idle.is_empty() && rehold.is_empty() {
            return Ok(Vec::new());
        }
        let (delegations, tables) = (&self.delegations, &self.store.tables);
        let turns = transaction(
            &self.connection,
            &self.commits,
            Row::PolitenessOtherTransactions,
            |transaction| {
                let mut turns = Vec::new();
                for key in &idle {
                    if let Some(delegation) = delegations.get(key) {
                        delegation.rule.0.write_back(tables, key, transaction)?;
                    }
                    transaction.prepare_cached(UNDELEGATE)?.execute([key])?;
                    let generation: i64 = transaction
                        .prepare_cached(NEXT_GENERATION)?
                        .query_row([key], |row| row.get(0))?;
                    turns.extend(next_turn(transaction, key, generation, now)?);
                }
                for (key, holder) in &rehold {
                    transaction.prepare_cached(REHOLD)?.execute((
                        key,
                        &holder.run_id,
                        i64::try_from(holder.epoch).unwrap_or(i64::MAX),
                        holder.run_folder.to_string_lossy(),
                    ))?;
                }
                Ok(turns)
            },
        )?;
        for key in idle {
            if let Some(delegation) = self.delegations.remove(&key) {
                delegation.rule.0.forget(&mut self.store.tables, &key);
                self.store.forget(&key);
            }
        }
        for (key, holder) in rehold {
            if let Some(delegation) = self.delegations.get_mut(&key) {
                delegation.holder = holder;
            }
        }
        Ok(turns)
    }

    /// Removes the waiters whose endpoint `gone` is gone, and answers the
    /// next waiter of `key` in turn for `generation`, in one transaction.
    pub(super) fn gone_waiters(
        &self,
        context: &Context,
        gone: &str,
        key: &str,
        generation: i64,
        now: SystemTime,
    ) -> Result<Option<Turn>, PolitenessError> {
        transaction(
            &self.connection,
            &self.commits,
            Row::PolitenessOtherTransactions,
            |transaction| {
                transaction.prepare_cached(GONE_WAITERS)?.execute([gone])?;
                next_turn(transaction, key, generation, now)
            },
        )
        .map_err(|error| sqlite_error(&context.path, &error))
    }

    /// Whether the Delegation of `key` is no longer held by another
    /// engine's live Run, so a recall that found its holder's endpoint
    /// gone lets the recaller retake the key at once. A read outside a
    /// transaction: it changes nothing, and the retake reclaims.
    pub(super) fn holder_gone(
        &self,
        context: &Context,
        key: &str,
    ) -> Result<bool, PolitenessError> {
        delegation_holder(&self.connection, context, key)
            .map(|holder| !matches!(holder, DelegationHolder::Live(_)))
            .map_err(|error| sqlite_error(&context.path, &error))
    }
}

/// Who holds the Delegation of `key`, as one engine sees it.
enum DelegationHolder {
    /// No other engine's Run holds it.
    Free,
    /// Another engine's live Run holds it, woken at this endpoint.
    Live(String),
    /// Another engine's Run that is gone holds it, until it is reclaimed.
    Gone { run_id: String, epoch: i64 },
}

/// Who holds the Delegation of `key`, read on `connection`.
fn delegation_holder(
    connection: &Connection,
    context: &Context,
    key: &str,
) -> rusqlite::Result<DelegationHolder> {
    let held: Option<(String, i64, String, String, String)> = connection
        .prepare_cached(DELEGATION)?
        .query_row([key], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                row.get(2)?,
                row.get(3)?,
                row.get(4)?,
            ))
        })
        .optional()?;
    let Some((run_id, epoch, folder, engine, endpoint)) = held else {
        return Ok(DelegationHolder::Free);
    };
    if engine == context.engine {
        return Ok(DelegationHolder::Free);
    }
    if (context.holder_gone)(Path::new(&folder), u64::try_from(epoch).unwrap_or_default()) {
        Ok(DelegationHolder::Gone { run_id, epoch })
    } else {
        Ok(DelegationHolder::Live(endpoint))
    }
}

/// The wake endpoint of another engine whose live Run holds the
/// Delegation of `key`, if one does; one whose Run is gone is reclaimed,
/// in `transaction`, and the key's next request counts its delay from
/// `now`, as its holder may have sent one then.
///
/// # Errors
///
/// SQLite's error.
pub(super) fn held_elsewhere(
    transaction: &Connection,
    context: &Context,
    key: &str,
    rule: &dyn Erased,
    now: SystemTime,
) -> rusqlite::Result<Option<String>> {
    let (run_id, epoch) = match delegation_holder(transaction, context, key)? {
        DelegationHolder::Free => return Ok(None),
        DelegationHolder::Live(endpoint) => return Ok(Some(endpoint)),
        DelegationHolder::Gone { run_id, epoch } => (run_id, epoch),
    };
    tracing::debug!(%run_id, epoch, key, "the Delegation of a gone Run is reclaimed");
    transaction.prepare_cached(UNDELEGATE)?.execute([key])?;
    rule.restarted(transaction, key, now)?;
    Ok(None)
}

/// Whether the file holds no Permit of `key` another engine took and no
/// waiter of it at another endpoint, so this engine may ask for its
/// Delegation again.
fn used_by_this_engine_alone(
    transaction: &Connection,
    context: &Context,
    key: &str,
) -> rusqlite::Result<bool> {
    let endpoint = context.endpoint.as_deref().unwrap_or_default();
    let others: i64 = transaction
        .prepare_cached(USED_ELSEWHERE)?
        .query_row((key, &context.engine, endpoint), |row| row.get(0))?;
    Ok(others == 0)
}

/// The Permits of a key another engine holds, and its waiters at another
/// endpoint.
pub(super) const USED_ELSEWHERE: &str =
    "SELECT (SELECT count(*) FROM permits WHERE key = ?1 AND engine != ?2)
    + (SELECT count(*) FROM waiters WHERE key = ?1 AND endpoint != ?3)";

/// A waiter of a recalled Delegation, put ahead of every waiter the file
/// has for its key, the recaller's included, as woken for the recall's
/// generation: the next give back wakes it.
pub(super) const JOIN_AHEAD: &str =
    "INSERT OR IGNORE INTO waiters (key, run_id, endpoint, turn, woken, woken_at)
    VALUES (?1, ?2, ?3, (SELECT coalesce(min(turn), 1) FROM waiters WHERE key = ?1) - 1, ?4, 0)";

/// Takes the Delegation of `asked`'s key in `transaction`, its state loaded
/// into `tables`, when no other engine's live Run holds it and the file
/// holds no Permit or waiter of it, the Permits of gone Runs reclaimed
/// first; answers whether it did.
fn delegated_in(
    transaction: &Connection,
    context: &Context,
    endpoint: &str,
    asked: &Asked,
    tables: &mut memory::MemoryTables,
) -> rusqlite::Result<bool> {
    let key = asked.key.as_str();
    if held_elsewhere(transaction, context, key, &*asked.rule.0, asked.now)?.is_some() {
        return Ok(false);
    }
    reclaim(transaction, context, key)?;
    let waiters: i64 = transaction
        .prepare_cached(WAITERS_OF)?
        .query_row([key], |row| row.get(0))?;
    if in_flight(transaction, key)? > 0 || waiters > 0 {
        return Ok(false);
    }
    let holder = &asked.holder;
    transaction.prepare_cached(DELEGATE)?.execute((
        key,
        &holder.run_id,
        i64::try_from(holder.epoch).unwrap_or(i64::MAX),
        holder.run_folder.to_string_lossy(),
        &context.engine,
        endpoint,
    ))?;
    asked.rule.0.delegate(transaction, key, tables)?;
    Ok(true)
}
