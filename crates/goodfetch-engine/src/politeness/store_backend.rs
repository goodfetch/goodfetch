//! The Politeness store backend (ADRs 0053, 0059 and 0142): serves
//! `politeness.sharing: shared` across machines through Valkey or Redis.
//!
//! A key only this engine uses is served from memory under a Delegation
//! (`E6.transactions = 0`), whose expiry the actor's heartbeat renews on
//! the server's clock; another engine needing the key recalls it through
//! the same pub/sub channel that wakes waiters, moving the key to shared
//! leased Permits (at most 3 round trips per attempt, within
//! `E6.shared_transactions`) until this engine is alone again. With
//! `politeness.delegation: false`, every Permit goes straight to the store.
//! Active shared Permit leases are renewed in the background while their
//! requests remain in flight.
//!
//! Size: one actor module owning the Politeness store's Delegations, shared
//! Permit leases, pub/sub wake/recall loop, and heartbeat renewal together.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::{CountedWork, Row};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::store::{
    DelegationAcquire, PeekShared, PolitenessStore, StoreError, TakeShared, WakeEvent, WakeEvents,
    WakeSubscriptions,
};
use super::{
    DELEGATIONS_PER_TRANSACTION, Ending, Hold, Holder, KeyState, MAILBOX, Permit, PolitenessError,
    PolitenessOptions, SharedRule, Take, Taken, Verdict, Wake, memory,
};
use crate::clock::Clock;
use crate::tasks::TaskOwner;

/// How long shared key state (`:state` hash) lives without updates.
const STATE_TTL: Duration = Duration::from_secs(3600);

/// Default TTL of a Delegation in the store (renewed every TTL / 3).
pub(super) const DEFAULT_DELEGATION_TTL: Duration = Duration::from_secs(15);

/// Default lease duration of a shared Permit in the store (renewed every lease / 3).
pub(super) const DEFAULT_PERMIT_LEASE: Duration = Duration::from_secs(30);

/// Handle to the async store actor, held inside [`super::Politeness`].
pub(super) struct StoreBackend {
    mailbox: mpsc::Sender<StoreCommand>,
}

impl StoreBackend {
    /// Starts the store actor on `owner`, opening one multiplexed pub/sub
    /// connection for wakes and recalls (`E6.opens = 0`).
    pub(super) async fn start(
        store: PolitenessStore,
        options: &PolitenessOptions,
        engine: String,
        clock: Clock,
        wakes: broadcast::Sender<Wake>,
        owner: &TaskOwner,
    ) -> Result<Self, PolitenessError> {
        let (subs, events) = store
            .open_wake_bus()
            .await
            .map_err(|error| store_error(&error))?;
        let (mailbox, commands) = mpsc::channel(MAILBOX);
        let actor = StoreActor {
            store,
            subs,
            engine,
            clock,
            work: options.work.clone(),
            delegation_ttl: options.store_delegation_ttl,
            permit_lease: options.store_permit_lease,
            wakes,
            memory: memory::Store::default(),
            delegations: BTreeMap::new(),
            recalled: HashMap::new(),
            active_leases: BTreeMap::new(),
            next_shared_id: -1,
            shared: HashSet::new(),
            subscribed: HashMap::new(),
            channel_to_key: HashMap::new(),
            waiters: BTreeMap::new(),
            recall_retries: Vec::new(),
        };
        let cancelled = owner.token().clone();
        owner.spawn(actor.run(commands, events, cancelled));
        Ok(Self { mailbox })
    }

    pub(super) async fn take(&self, take: Take<'static>) -> Result<Taken, PolitenessError> {
        let (answer, answered) = oneshot::channel();
        self.mailbox
            .send(StoreCommand::Take { take, answer })
            .await
            .map_err(|_| PolitenessError::Closed)?;
        answered.await.map_err(|_| PolitenessError::Closed)?
    }

    pub(super) async fn give_back(
        &self,
        permit: Permit,
        rule: SharedRule,
        ending: Ending,
        now: SystemTime,
    ) -> Result<Option<SystemTime>, PolitenessError> {
        let (answer, answered) = oneshot::channel();
        self.mailbox
            .send(StoreCommand::GiveBack {
                permit,
                rule,
                ending,
                now,
                answer,
            })
            .await
            .map_err(|_| PolitenessError::Closed)?;
        answered.await.map_err(|_| PolitenessError::Closed)?
    }

    pub(super) async fn stop_waiting(
        &self,
        run_id: String,
        now: SystemTime,
    ) -> Result<(), PolitenessError> {
        let (answer, answered) = oneshot::channel();
        self.mailbox
            .send(StoreCommand::StopWaiting {
                run_id,
                now,
                answer,
            })
            .await
            .map_err(|_| PolitenessError::Closed)?;
        answered.await.map_err(|_| PolitenessError::Closed)?
    }

    pub(super) async fn change<S, K, T, F>(
        &self,
        key: K,
        reads: S::Reads,
        change: F,
    ) -> Result<T, PolitenessError>
    where
        S: KeyState,
        K: std::borrow::Borrow<S::Key> + Send + 'static,
        T: Send + 'static,
        F: FnOnce(&mut S) -> (T, S::Writes) + Send + 'static,
    {
        let (answer, answered) = oneshot::channel();
        let job = Box::new(TypedChange::<S, K, T, F> {
            key,
            reads,
            change: Some(change),
            answer: Some(answer),
        });
        self.mailbox
            .send(StoreCommand::Change(job))
            .await
            .map_err(|_| PolitenessError::Closed)?;
        answered.await.map_err(|_| PolitenessError::Closed)?
    }
}

enum StoreCommand {
    Take {
        take: Take<'static>,
        answer: oneshot::Sender<Result<Taken, PolitenessError>>,
    },
    GiveBack {
        permit: Permit,
        rule: SharedRule,
        ending: Ending,
        now: SystemTime,
        answer: oneshot::Sender<Result<Option<SystemTime>, PolitenessError>>,
    },
    StopWaiting {
        run_id: String,
        now: SystemTime,
        answer: oneshot::Sender<Result<(), PolitenessError>>,
    },
    Change(Box<dyn ErasedStoreChange>),
}

/// Type-erased [`KeyState`] change run on the store actor.
trait ErasedStoreChange: Send {
    fn store_target(&self) -> Option<(String, &'static str)>;
    fn delegated_key(&self) -> Option<String>;
    fn apply_in_memory(&mut self, store: &mut memory::Store) -> Option<Vec<u8>>;
    fn apply_with_store_bytes(
        &mut self,
        bytes: Option<&[u8]>,
        store: &mut memory::Store,
    ) -> Option<Vec<u8>>;
    fn fail(&mut self, error: PolitenessError);
}

struct TypedChange<S: KeyState, K, T, F> {
    key: K,
    reads: S::Reads,
    change: Option<F>,
    answer: Option<oneshot::Sender<Result<T, PolitenessError>>>,
}

impl<S, K, T, F> ErasedStoreChange for TypedChange<S, K, T, F>
where
    S: KeyState,
    K: std::borrow::Borrow<S::Key> + Send + 'static,
    T: Send + 'static,
    F: FnOnce(&mut S) -> (T, S::Writes) + Send + 'static,
{
    fn store_target(&self) -> Option<(String, &'static str)> {
        let part = S::STORE_PART?;
        let key = S::store_key(self.key.borrow())?;
        Some((key.to_owned(), part))
    }

    fn delegated_key(&self) -> Option<String> {
        S::delegated_under(self.key.borrow()).map(str::to_owned)
    }

    fn apply_in_memory(&mut self, store: &mut memory::Store) -> Option<Vec<u8>> {
        let change = self.change.take()?;
        let key = self.key.borrow();
        let mut state = S::load_from(&store.tables, key, self.reads);
        let (result, writes) = change(&mut state);
        state.save_to(&mut store.tables, key, &writes);
        let persist = if S::asked(&writes) {
            S::encode_store_from(&store.tables, key).or_else(|| state.encode_store())
        } else {
            None
        };
        if let Some(answer) = self.answer.take() {
            let _ = answer.send(Ok(result));
        }
        persist
    }

    fn apply_with_store_bytes(
        &mut self,
        bytes: Option<&[u8]>,
        store: &mut memory::Store,
    ) -> Option<Vec<u8>> {
        let change = self.change.take()?;
        let key = self.key.borrow();
        let mut state = S::load_store(bytes, self.reads)
            .unwrap_or_else(|| S::load_from(&store.tables, key, self.reads));
        let (result, writes) = change(&mut state);
        state.save_to(&mut store.tables, key, &writes);
        let encoded = S::encode_store_from(&store.tables, key).or_else(|| state.encode_store());
        if let Some(answer) = self.answer.take() {
            let _ = answer.send(Ok(result));
        }
        encoded
    }

    fn fail(&mut self, error: PolitenessError) {
        if let Some(answer) = self.answer.take() {
            let _ = answer.send(Err(error));
        }
    }
}

struct StoreDelegation {
    runs: Vec<Holder>,
    rule: SharedRule,
    expires_at: Instant,
    last_renewed: Instant,
}

struct ActiveLease {
    key: String,
    token: String,
    last_renewed: Instant,
    last_in_flight: u64,
}

struct AskedStoreTake {
    take: Take<'static>,
    answer: oneshot::Sender<Result<Taken, PolitenessError>>,
}

struct StoreActor {
    store: PolitenessStore,
    subs: WakeSubscriptions,
    engine: String,
    clock: Clock,
    work: CountedWork,
    delegation_ttl: Duration,
    permit_lease: Duration,
    wakes: broadcast::Sender<Wake>,
    memory: memory::Store,
    delegations: BTreeMap<String, StoreDelegation>,
    /// Local delegated permit ID -> `(key, store_token)` for permits promoted
    /// to store leases when their Delegation was recalled.
    recalled: HashMap<i64, (String, String)>,
    /// Active non-delegated (or recalled) store Permit leases, by local permit ID.
    active_leases: BTreeMap<i64, ActiveLease>,
    next_shared_id: i64,
    /// Keys shared across engines (do not ask for a Delegation until alone).
    shared: HashSet<String>,
    /// Subscribed key -> channel name.
    subscribed: HashMap<String, String>,
    /// Channel name -> Politeness key.
    channel_to_key: HashMap<String, String>,
    /// Waiting local Run IDs per key (woken on pub/sub wake or recall).
    waiters: BTreeMap<String, Vec<String>>,
    /// Fallback wake deadlines `(when, key)` when a recall went unanswered
    /// (e.g., the previous Delegation holder crashed and its TTL will expire).
    recall_retries: Vec<(Instant, String)>,
}

impl StoreActor {
    async fn run(
        mut self,
        mut commands: mpsc::Receiver<StoreCommand>,
        mut events: WakeEvents,
        cancelled: CancellationToken,
    ) {
        let tick_period = (self.delegation_ttl / 3)
            .min(self.permit_lease / 3)
            .max(Duration::from_millis(25));
        let mut heartbeat = tokio::time::interval(tick_period);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Consume the immediate first tick so it fires after one interval.
        heartbeat.tick().await;

        loop {
            tokio::select! {
                () = cancelled.cancelled() => {
                    self.release_all_delegations().await;
                    break;
                }
                first = commands.recv() => {
                    let Some(first) = first else {
                        self.release_all_delegations().await;
                        break;
                    };
                    let mut batch = vec![first];
                    while let Ok(next) = commands.try_recv() {
                        batch.push(next);
                    }
                    self.handle_batch(batch).await;
                }
                event = events.next() => {
                    match event {
                        Some((channel, event)) => self.handle_wake_event(&channel, event).await,
                        None => {
                            tracing::debug!("the Politeness store pub/sub connection ended");
                        }
                    }
                }
                _ = heartbeat.tick() => {
                    self.heartbeat_tick().await;
                }
            }
        }
    }

    async fn handle_batch(&mut self, batch: Vec<StoreCommand>) {
        let mut asked = Vec::new();
        for command in batch {
            match command {
                StoreCommand::Take { take, answer } => {
                    self.handle_take(take, answer, &mut asked).await;
                }
                StoreCommand::GiveBack {
                    permit,
                    rule,
                    ending,
                    now,
                    answer,
                } => {
                    // Flush pending Delegation requests before a give-back so
                    // ordering stays faithful to the caller's sequence.
                    if !asked.is_empty() {
                        self.take_asked_delegations(std::mem::take(&mut asked))
                            .await;
                    }
                    let result = self.handle_give_back(&permit, &rule, ending, now).await;
                    let _ = answer.send(result);
                }
                StoreCommand::StopWaiting {
                    run_id,
                    now,
                    answer,
                } => {
                    if !asked.is_empty() {
                        self.take_asked_delegations(std::mem::take(&mut asked))
                            .await;
                    }
                    let result = self.handle_stop_waiting(&run_id, now).await;
                    let _ = answer.send(result);
                }
                StoreCommand::Change(change) => {
                    if !asked.is_empty() {
                        self.take_asked_delegations(std::mem::take(&mut asked))
                            .await;
                    }
                    self.handle_change(change).await;
                }
            }
        }
        if !asked.is_empty() {
            self.take_asked_delegations(asked).await;
        }
    }

    async fn handle_take(
        &mut self,
        take: Take<'static>,
        answer: oneshot::Sender<Result<Taken, PolitenessError>>,
        asked: &mut Vec<AskedStoreTake>,
    ) {
        let delegates = take.rule.0.delegates();
        if delegates && self.has_live_delegation(&take.key) {
            let taken = self.taken_delegated(&take);
            let _ = answer.send(Ok(taken));
        } else if delegates && take.holder.delegates && !self.shared.contains(&*take.key) {
            // Drop any expired local delegation before attempting a fresh one.
            self.expire_delegation_if_needed(&take.key);
            asked.push(AskedStoreTake { take, answer });
        } else {
            self.expire_delegation_if_needed(&take.key);
            let result = self.taken_shared(&take).await;
            let _ = answer.send(result);
        }
    }

    fn has_live_delegation(&self, key: &str) -> bool {
        let now = self.clock.now();
        self.delegations
            .get(key)
            .is_some_and(|delegation| now < delegation.expires_at)
    }

    fn expire_delegation_if_needed(&mut self, key: &str) {
        let now = self.clock.now();
        let expired = self
            .delegations
            .get(key)
            .is_some_and(|delegation| now >= delegation.expires_at);
        if expired {
            tracing::debug!(key, "a store Delegation expired locally");
            if let Some(delegation) = self.delegations.remove(key) {
                let (permits, waiting) = self.memory.recall_view(key);
                for permit_id in permits {
                    let token = self.permit_token(permit_id);
                    self.recalled
                        .insert(permit_id, (key.to_owned(), token.clone()));
                    self.active_leases.insert(
                        permit_id,
                        ActiveLease {
                            key: key.to_owned(),
                            token,
                            last_renewed: now,
                            last_in_flight: 1,
                        },
                    );
                }
                for run_id in waiting {
                    self.record_waiter(key, &run_id);
                }
                delegation.rule.0.forget(&mut self.memory.tables, key);
                self.memory.recall(key);
                self.wake_local_waiters(key);
            }
        }
    }

    fn taken_delegated(&mut self, take: &Take<'_>) -> Taken {
        let holder = &*take.holder;
        if let Some(delegation) = self.delegations.get_mut(&*take.key)
            && !delegation
                .runs
                .iter()
                .any(|run| run.run_id == holder.run_id)
        {
            delegation.runs.push(holder.clone());
        }
        let (mut taken, turn) = take.in_memory(&mut self.memory);
        if let Taken::Granted(permit) = &mut taken {
            permit.delegated = true;
        }
        if let Some(turn) = turn {
            let _ = self.wakes.send(turn.wake);
        }
        taken
    }

    async fn take_asked_delegations(&mut self, asked: Vec<AskedStoreTake>) {
        let mut seen = HashSet::new();
        let unique_keys: Vec<(String, SharedRule)> = asked
            .iter()
            .filter_map(|item| {
                let key = item.take.key.as_ref();
                if !self.has_live_delegation(key) && seen.insert(key.to_owned()) {
                    Some((key.to_owned(), item.take.rule.as_ref().clone()))
                } else {
                    None
                }
            })
            .collect();

        for chunk in unique_keys.chunks(DELEGATIONS_PER_TRANSACTION) {
            for (key, rule) in chunk {
                if let Err(error) = self.ensure_subscribed(key).await {
                    tracing::warn!(%error, key, "could not subscribe to Politeness store wakes before delegating");
                    continue;
                }
                let before = self.clock.now();
                self.work.count(Row::PolitenessDelegationTransactions);
                match self
                    .store
                    .delegate(key, &self.engine, self.delegation_ttl)
                    .await
                {
                    Ok(DelegationAcquire::Granted { state }) => {
                        self.work.count(Row::PolitenessDelegations);
                        if let Some(part) = rule.0.store_part()
                            && let Some(bytes) = state.get(part)
                        {
                            rule.0
                                .decode_store_into(&mut self.memory.tables, key, bytes);
                        }
                        self.delegations.insert(
                            key.clone(),
                            StoreDelegation {
                                runs: Vec::new(),
                                rule: rule.clone(),
                                expires_at: before + self.delegation_ttl,
                                last_renewed: self.clock.now(),
                            },
                        );
                    }
                    Ok(DelegationAcquire::HeldByOther { .. } | DelegationAcquire::Shared) => {
                        self.shared.insert(key.clone());
                    }
                    Err(error) => {
                        tracing::warn!(%error, key, "could not acquire Politeness store Delegation; falling back to shared take");
                    }
                }
            }
        }

        for item in asked {
            let answer = if self.has_live_delegation(&item.take.key) {
                Ok(self.taken_delegated(&item.take))
            } else {
                self.taken_shared(&item.take).await
            };
            let _ = item.answer.send(answer);
        }
    }

    async fn taken_shared(&mut self, take: &Take<'_>) -> Result<Taken, PolitenessError> {
        let key = &*take.key;
        self.ensure_subscribed(key).await?;

        let store_part = take.rule.0.store_part();
        let (cap, state_update) = if let Some(part) = store_part {
            self.work.count(Row::PolitenessTransactions);
            let peek = self
                .store
                .peek_shared(key, &self.engine)
                .await
                .map_err(|error| store_error(&error))?;
            let (in_flight, state_map) = match peek {
                PeekShared::Recalled => {
                    if take.in_turn.waits() {
                        self.record_waiter(key, &take.holder.run_id);
                        self.schedule_recall_retry(key);
                    }
                    return Ok(Taken::Wait(None));
                }
                PeekShared::Ready { in_flight, state } => (in_flight, state),
            };
            let stored = state_map.get(part).map(Vec::as_slice);
            let (verdict, effective_cap, updated) = take.rule.0.admit_and_start_store(
                &mut self.memory.tables,
                key,
                stored,
                in_flight,
                take.now,
            );
            match verdict {
                Verdict::Go => (effective_cap, updated.map(|bytes| (part, bytes))),
                Verdict::Until(until, hold) => return Ok(Taken::Wait(Some((until, hold)))),
                Verdict::Refused(until) => return Ok(Taken::Refused(until)),
                Verdict::Busy => {
                    if take.in_turn.waits() {
                        self.record_waiter(key, &take.holder.run_id);
                    }
                    return Ok(Taken::Wait(None));
                }
            }
        } else {
            (take.rule.0.concurrency(), None)
        };

        let id = self.next_shared_id;
        self.next_shared_id -= 1;
        let token = self.permit_token(id);
        let pace = take.rule.0.store_pace();
        let state_arg = state_update
            .as_ref()
            .map(|(part, bytes)| (*part, bytes.as_slice()));

        self.work.count(Row::PolitenessTransactions);
        let result = self
            .store
            .take_shared(
                key,
                &self.engine,
                &token,
                cap,
                self.permit_lease,
                pace,
                STATE_TTL,
                state_arg,
            )
            .await
            .map_err(|error| store_error(&error))?;

        match result {
            TakeShared::Granted { in_flight } => {
                self.remove_waiter(key, &take.holder.run_id);
                self.active_leases.insert(
                    id,
                    ActiveLease {
                        key: key.to_owned(),
                        token,
                        last_renewed: self.clock.now(),
                        last_in_flight: in_flight,
                    },
                );
                Ok(Taken::Granted(Permit {
                    id,
                    key: key.to_owned(),
                    send_at: None,
                    delegated: false,
                }))
            }
            TakeShared::Full { .. } => {
                if take.in_turn.waits() {
                    self.record_waiter(key, &take.holder.run_id);
                }
                Ok(Taken::Wait(None))
            }
            TakeShared::Recalled => {
                if take.in_turn.waits() {
                    self.record_waiter(key, &take.holder.run_id);
                    self.schedule_recall_retry(key);
                }
                Ok(Taken::Wait(None))
            }
            TakeShared::PaceWait(wait) => Ok(Taken::Wait(Some((take.now + wait, Hold::Delay)))),
        }
    }

    async fn handle_give_back(
        &mut self,
        permit: &Permit,
        rule: &SharedRule,
        ending: Ending,
        now: SystemTime,
    ) -> Result<Option<SystemTime>, PolitenessError> {
        let promoted = if permit.delegated {
            self.recalled.remove(&permit.id)
        } else {
            None
        };

        if !permit.delegated || promoted.is_some() {
            let lease = self.active_leases.remove(&permit.id);
            let token = promoted
                .map(|(_, token)| token)
                .or_else(|| lease.as_ref().map(|active| active.token.clone()))
                .unwrap_or_else(|| self.permit_token(permit.id));
            let others = lease
                .as_ref()
                .map_or(0, |active| active.last_in_flight.saturating_sub(1));
            let (ready_at, updated_bytes) =
                rule.0
                    .ended_store(&mut self.memory.tables, &permit.key, others, ending, now);
            let state_part = rule.0.store_part();
            let state_arg = match (state_part, updated_bytes.as_deref()) {
                (Some(part), Some(bytes)) => Some((part, bytes)),
                _ => None,
            };

            self.work.count(Row::PolitenessTransactions);
            let given = self
                .store
                .give_back_shared(&permit.key, &self.engine, &token, STATE_TTL, state_arg)
                .await
                .map_err(|error| store_error(&error))?;
            if given.alone {
                self.shared.remove(&permit.key);
            }
            return Ok(ready_at);
        }

        // Delegated Permit: served completely in memory (`E6.transactions = 0`).
        let (ready_at, turn) = rule
            .0
            .given_back_in_memory(&mut self.memory, permit, ending, now);
        if ending != Ending::Done
            && let Some(part) = rule.0.store_part()
            && let Some(bytes) = rule.0.encode_store_from(&self.memory.tables, &permit.key)
        {
            self.work.count(Row::PolitenessOriginTransactions);
            self.store
                .save_state(&permit.key, STATE_TTL, part, &bytes)
                .await
                .map_err(|error| store_error(&error))?;
        }
        if let Some(turn) = turn {
            let _ = self.wakes.send(turn.wake);
            return Ok(ready_at);
        }
        let idle = self.delegations.get(&permit.key).is_some_and(|delegation| {
            delegation.runs.is_empty() && self.memory.in_flight(&permit.key) == 0
        });
        if idle {
            self.release_delegation_key(&permit.key, false).await?;
        }
        Ok(ready_at)
    }

    async fn handle_stop_waiting(
        &mut self,
        run_id: &str,
        _now: SystemTime,
    ) -> Result<(), PolitenessError> {
        for waiters in self.waiters.values_mut() {
            waiters.retain(|waiting| waiting != run_id);
        }
        self.memory.stop_waiting(run_id);
        let mut idle_keys = Vec::new();
        for (key, delegation) in &mut self.delegations {
            delegation.runs.retain(|run| run.run_id != run_id);
            if delegation.runs.is_empty() && self.memory.in_flight(key) == 0 {
                idle_keys.push(key.clone());
            }
        }
        for key in idle_keys {
            self.release_delegation_key(&key, false).await?;
        }
        Ok(())
    }

    async fn handle_change(&mut self, mut change: Box<dyn ErasedStoreChange>) {
        if let Some(delegated_key) = change.delegated_key()
            && self.has_live_delegation(&delegated_key)
        {
            let persist = change.apply_in_memory(&mut self.memory);
            if let (Some(bytes), Some((store_key, part))) = (persist, change.store_target()) {
                self.work.count(Row::PolitenessOriginTransactions);
                if let Err(error) = self
                    .store
                    .save_state(&store_key, STATE_TTL, part, &bytes)
                    .await
                {
                    tracing::warn!(%error, key = %store_key, "could not persist site ask to Politeness store");
                }
            }
            return;
        }
        let Some((store_key, part)) = change.store_target() else {
            change.apply_in_memory(&mut self.memory);
            return;
        };
        let loaded = match self.store.load_state(&store_key, part).await {
            Ok(bytes) => bytes,
            Err(error) => {
                change.fail(store_error(&error));
                return;
            }
        };
        if let Some(updated) = change.apply_with_store_bytes(loaded.as_deref(), &mut self.memory)
            && let Err(error) = self
                .store
                .save_state(&store_key, STATE_TTL, part, &updated)
                .await
        {
            tracing::warn!(%error, key = %store_key, "could not save shared Politeness state");
        }
    }

    async fn handle_wake_event(&mut self, channel: &str, event: WakeEvent) {
        let Some(key) = self.channel_to_key.get(channel).cloned() else {
            return;
        };
        match event {
            WakeEvent::Recall { holder } if holder == self.engine => {
                if let Err(error) = self.release_delegation_key(&key, true).await {
                    tracing::warn!(%error, key, "could not release recalled store Delegation");
                }
            }
            WakeEvent::Recall { .. } => {}
            WakeEvent::Recalled | WakeEvent::Wake => {
                self.recall_retries.retain(|(_, k)| k != &key);
                self.wake_local_waiters(&key);
            }
        }
    }

    async fn release_delegation_key(
        &mut self,
        key: &str,
        recalled: bool,
    ) -> Result<(), PolitenessError> {
        let Some(delegation) = self.delegations.remove(key) else {
            return Ok(());
        };
        let (permits, waiting) = self.memory.recall_view(key);
        let mut tokens = Vec::with_capacity(permits.len());
        for permit_id in &permits {
            tokens.push(self.permit_token(*permit_id));
        }
        let state_part = delegation.rule.0.store_part();
        let state_bytes = state_part.and_then(|_| {
            delegation
                .rule
                .0
                .encode_store_from(&self.memory.tables, key)
        });
        let mut state_args: Vec<(&str, &[u8])> = Vec::new();
        if let (Some(part), Some(bytes)) = (state_part, state_bytes.as_deref()) {
            state_args.push((part, bytes));
        }

        self.work.count(Row::PolitenessRunTransactions);
        self.store
            .release_delegation(
                key,
                &self.engine,
                self.permit_lease,
                STATE_TTL,
                &tokens,
                &state_args,
            )
            .await
            .map_err(|error| store_error(&error))?;

        let now_instant = self.clock.now();
        let in_flight = u64::try_from(tokens.len()).unwrap_or(u64::MAX);
        for (permit_id, token) in permits.into_iter().zip(tokens) {
            self.recalled
                .insert(permit_id, (key.to_owned(), token.clone()));
            self.active_leases.insert(
                permit_id,
                ActiveLease {
                    key: key.to_owned(),
                    token,
                    last_renewed: now_instant,
                    last_in_flight: in_flight,
                },
            );
        }
        for run_id in waiting {
            self.record_waiter(key, &run_id);
        }
        delegation.rule.0.forget(&mut self.memory.tables, key);
        if recalled {
            self.memory.recall(key);
            self.shared.insert(key.to_owned());
            self.wake_local_waiters(key);
        } else {
            self.memory.forget(key);
        }
        Ok(())
    }

    async fn release_all_delegations(&mut self) {
        let keys: Vec<String> = self.delegations.keys().cloned().collect();
        for key in keys {
            let _ = self.release_delegation_key(&key, false).await;
        }
    }

    async fn heartbeat_tick(&mut self) {
        let now = self.clock.now();

        // Fire any fallback wake timers for unanswered recalls (e.g. crashed holder).
        let mut due_keys = Vec::new();
        self.recall_retries.retain(|(when, key)| {
            if *when <= now {
                due_keys.push(key.clone());
                false
            } else {
                true
            }
        });
        for key in due_keys {
            self.wake_local_waiters(&key);
        }

        // Renew held Delegations whose heartbeat interval elapsed.
        let delegation_interval = self.delegation_ttl / 3;
        let due_delegations: Vec<String> = self
            .delegations
            .iter()
            .filter(|(_, delegation)| {
                now.saturating_duration_since(delegation.last_renewed) >= delegation_interval
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in due_delegations {
            let before = self.clock.now();
            match self
                .store
                .renew_delegation(&key, &self.engine, self.delegation_ttl)
                .await
            {
                Ok(true) => {
                    if let Some(delegation) = self.delegations.get_mut(&key) {
                        delegation.expires_at = before + self.delegation_ttl;
                        delegation.last_renewed = self.clock.now();
                    }
                }
                Ok(false) => {
                    tracing::debug!(key, "store Delegation was lost on renewal");
                    if let Some(delegation) = self.delegations.get_mut(&key) {
                        delegation.expires_at = before;
                    }
                    self.expire_delegation_if_needed(&key);
                }
                Err(error) => {
                    tracing::debug!(%error, key, "store Delegation renewal failed; expiry unchanged");
                    self.expire_delegation_if_needed(&key);
                }
            }
        }

        // Renew active shared Permit leases while their requests remain in flight.
        let lease_interval = self.permit_lease / 3;
        let now = self.clock.now();
        let due_leases: Vec<(i64, String, String)> = self
            .active_leases
            .iter()
            .filter(|(_, lease)| {
                now.saturating_duration_since(lease.last_renewed) >= lease_interval
            })
            .map(|(id, lease)| (*id, lease.key.clone(), lease.token.clone()))
            .collect();
        for (id, key, token) in due_leases {
            match self.store.renew(&key, &token, self.permit_lease).await {
                Ok(true) => {
                    if let Some(lease) = self.active_leases.get_mut(&id) {
                        lease.last_renewed = self.clock.now();
                    }
                }
                Ok(false) => {
                    tracing::debug!(key, "shared Permit lease expired before renewal");
                }
                Err(error) => {
                    tracing::debug!(%error, key, "shared Permit lease renewal failed");
                }
            }
        }
    }

    async fn ensure_subscribed(&mut self, key: &str) -> Result<(), PolitenessError> {
        if self.subscribed.contains_key(key) {
            return Ok(());
        }
        let channel = self
            .subs
            .subscribe(key)
            .await
            .map_err(|error| store_error(&error))?;
        self.subscribed.insert(key.to_owned(), channel.clone());
        self.channel_to_key.insert(channel, key.to_owned());
        Ok(())
    }

    fn record_waiter(&mut self, key: &str, run_id: &str) {
        let waiters = self.waiters.entry(key.to_owned()).or_default();
        if !waiters.iter().any(|existing| existing == run_id) {
            waiters.push(run_id.to_owned());
        }
    }

    fn remove_waiter(&mut self, key: &str, run_id: &str) {
        if let Some(waiters) = self.waiters.get_mut(key) {
            waiters.retain(|existing| existing != run_id);
        }
    }

    fn wake_local_waiters(&mut self, key: &str) {
        if let Some(waiters) = self.waiters.get(key) {
            for run_id in waiters {
                let _ = self.wakes.send(Wake {
                    run_id: run_id.clone(),
                    key: key.to_owned(),
                });
            }
        }
    }

    fn schedule_recall_retry(&mut self, key: &str) {
        let deadline = self.clock.now() + self.delegation_ttl;
        if !self.recall_retries.iter().any(|(_, k)| k == key) {
            self.recall_retries.push((deadline, key.to_owned()));
        }
    }

    fn permit_token(&self, id: i64) -> String {
        format!("{}:{id}", self.engine)
    }
}

fn store_error(error: &StoreError) -> PolitenessError {
    PolitenessError::Store(error.to_string())
}
