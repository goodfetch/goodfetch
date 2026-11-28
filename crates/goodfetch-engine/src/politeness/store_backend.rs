//! The Politeness store backend (ADRs 0053, 0059, 0060 and 0142): serves
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
//! Every engine heartbeats each active key with its longest request timeout
//! and receives its store-computed degraded share (`heartbeat.lua`). When the
//! store becomes unreachable, a Delegation holder keeps serving its delegated
//! key until the Delegation expires, then follows ADR 0053's degraded shares:
//! keeping held Permits until their leases expire, waiting the longest
//! reported request timeout, then admitting within its last granted share
//! scaled by each Run's cap for at most `politeness.degraded_max` (counted on
//! this engine's clock from the moment the last acknowledged heartbeat was
//! sent). Past `politeness.degraded_max`, admission to shared keys stops while
//! Runs stay live and resumable. Reconnection is automatic with exponential
//! backoff and atomically registers all in-flight Permits BEFORE freeing the
//! engine's stale reservation on the store.
//!
//! Transitions are emitted under the `goodfetch::politeness` tracing target
//! for TLA+ trace validation (ADR 0060).
//!
//! Size: one actor module owning the Politeness store's Delegations, shared
//! Permit leases, heartbeats, degraded-share admission, automatic
//! reconnection, and pub/sub wake/recall loop together.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::{CountedWork, Row};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tokio::time::Instant;
use tokio_util::sync::CancellationToken;

use super::store::{
    DelegationAcquire, DelegationHeartbeat, HeartbeatParams, PeekShared, PolitenessStore,
    StoreError, TakeShared, WakeEvent, WakeEvents, WakeSubscriptions,
};
use super::{
    DELEGATIONS_PER_TRANSACTION, Ending, Hold, Holder, KeyState, MAILBOX, Permit, PolitenessError,
    PolitenessOptions, SharedRule, StoreDegradedStatus, Take, Taken, Verdict, Wake, memory,
};
use crate::clock::Clock;
use crate::metrics::Metrics;
use crate::tasks::TaskOwner;

/// How long shared key state (`:state` hash) lives without updates.
const STATE_TTL: Duration = Duration::from_secs(3600);

/// Default TTL of a Delegation in the store (renewed every TTL / 3).
pub(super) const DEFAULT_DELEGATION_TTL: Duration = Duration::from_secs(15);

/// Default lease duration of a shared Permit in the store (renewed every lease / 3).
pub(super) const DEFAULT_PERMIT_LEASE: Duration = Duration::from_secs(30);

/// Default heartbeat TTL (never longer than `DEFAULT_PERMIT_LEASE`, ADR 0053).
pub(super) const DEFAULT_HEARTBEAT_TTL: Duration = Duration::from_secs(15);

/// Default `politeness.degraded_max` (10 minutes, ADR 0053).
pub(super) const DEFAULT_DEGRADED_MAX: Duration = Duration::from_secs(600);

/// Default longest request timeout reported in heartbeats.
pub(super) const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// Initial reconnection backoff when the Politeness store becomes unreachable.
pub(super) const DEFAULT_RECONNECT_INITIAL: Duration = Duration::from_millis(100);

/// Maximum reconnection backoff duration.
const MAX_RECONNECT_BACKOFF: Duration = Duration::from_secs(5);

/// Tracing target for TLA+ degraded-shares model transitions (ADR 0060).
const TRACE_TARGET: &str = "goodfetch::politeness";

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
        degraded: watch::Sender<StoreDegradedStatus>,
        owner: &TaskOwner,
    ) -> Result<Self, PolitenessError> {
        let (subs, events) = store
            .open_wake_bus()
            .await
            .map_err(|error| store_error(&error))?;
        let (mailbox, commands) = mpsc::channel(MAILBOX);
        // ADR 0053: a heartbeat goes stale no later than a permit's lease expires.
        let heartbeat_ttl = options.store_heartbeat_ttl.min(options.store_permit_lease);
        let actor = StoreActor {
            store,
            subs,
            engine,
            clock,
            work: options.work.clone(),
            delegation_ttl: options.store_delegation_ttl,
            permit_lease: options.store_permit_lease,
            heartbeat_ttl,
            degraded_max: options.store_degraded_max,
            request_timeout: options.store_request_timeout,
            reconnect_initial: options.store_reconnect_initial,
            wakes,
            degraded,
            metrics: options.metrics.clone(),
            connection: ConnectionState::Connected,
            memory: memory::Store::default(),
            delegations: BTreeMap::new(),
            recalled: HashMap::new(),
            active_leases: BTreeMap::new(),
            degraded_permits: BTreeMap::new(),
            heartbeats: BTreeMap::new(),
            next_shared_id: -1,
            shared: HashSet::new(),
            subscribed: HashMap::new(),
            channel_to_key: HashMap::new(),
            waiters: BTreeMap::new(),
            recall_retries: Vec::new(),
        };
        let cancelled = owner.token().clone();
        owner.spawn(actor.run(commands, Some(events), cancelled));
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

enum ConnectionState {
    Connected,
    Degraded {
        reason: String,
        next_reconnect_at: Instant,
        backoff: Duration,
    },
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

struct DegradedPermit {
    key: String,
    token: String,
    last_in_flight: u64,
}

struct KeyHeartbeat {
    reported_cap: Option<u64>,
    reported_timeout: Duration,
    last_hb_sent_at: Option<Instant>,
    granted_share: u64,
    acked_share: u64,
    max_cap: u64,
    max_timeout: Duration,
    degraded_admit_at: Option<Instant>,
    degraded_entered: bool,
    degraded_stopped: bool,
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
    heartbeat_ttl: Duration,
    degraded_max: Duration,
    request_timeout: Duration,
    reconnect_initial: Duration,
    wakes: broadcast::Sender<Wake>,
    degraded: watch::Sender<StoreDegradedStatus>,
    metrics: Option<Metrics>,
    connection: ConnectionState,
    memory: memory::Store,
    delegations: BTreeMap<String, StoreDelegation>,
    /// Local delegated permit ID -> `(key, store_token)` for permits promoted
    /// to store leases (or degraded permits on outage expiry).
    recalled: HashMap<i64, (String, String)>,
    /// Active non-delegated (or recalled) store Permit leases, by local permit ID.
    active_leases: BTreeMap<i64, ActiveLease>,
    /// Permits admitted locally during a degraded period (or promoted from an
    /// expired Delegation while cut off), registered atomically on reconnect.
    degraded_permits: BTreeMap<i64, DegradedPermit>,
    /// Heartbeat and store-granted degraded share state per active key.
    heartbeats: BTreeMap<String, KeyHeartbeat>,
    next_shared_id: i64,
    /// Keys shared across engines (do not ask for a Delegation until alone).
    shared: HashSet<String>,
    /// Subscribed key -> channel name.
    subscribed: HashMap<String, String>,
    /// Channel name -> Politeness key.
    channel_to_key: HashMap<String, String>,
    /// Waiting local Run IDs per key (woken on pub/sub wake, recall, or reconnect).
    waiters: BTreeMap<String, Vec<String>>,
    /// Fallback wake deadlines `(when, key)` when a recall went unanswered
    /// (e.g., the previous Delegation holder crashed and its TTL will expire).
    recall_retries: Vec<(Instant, String)>,
}

async fn next_wake_event(events: &mut Option<WakeEvents>) -> Option<(String, WakeEvent)> {
    match events {
        Some(stream) => stream.next().await,
        None => std::future::pending().await,
    }
}

impl StoreActor {
    async fn run(
        mut self,
        mut commands: mpsc::Receiver<StoreCommand>,
        mut events: Option<WakeEvents>,
        cancelled: CancellationToken,
    ) {
        let tick_period = (self.delegation_ttl / 3)
            .min(self.permit_lease / 3)
            .min(self.heartbeat_ttl / 3)
            .min(Duration::from_millis(50))
            .max(Duration::from_millis(15));
        let mut heartbeat = tokio::time::interval(tick_period);
        heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // Consume the immediate first tick so it fires after one interval.
        heartbeat.tick().await;

        loop {
            tokio::select! {
                () = cancelled.cancelled() => {
                    self.clean_shutdown().await;
                    break;
                }
                first = commands.recv() => {
                    let Some(first) = first else {
                        self.clean_shutdown().await;
                        break;
                    };
                    let mut batch = vec![first];
                    while let Ok(next) = commands.try_recv() {
                        batch.push(next);
                    }
                    self.handle_batch(batch).await;
                }
                event = next_wake_event(&mut events), if events.is_some() => {
                    if let Some((channel, event)) = event {
                        self.handle_wake_event(&channel, event).await;
                    } else {
                        events = None;
                        tracing::debug!("the Politeness store pub/sub connection ended");
                        self.enter_degraded("the Politeness store connection dropped".to_owned());
                    }
                }
                _ = heartbeat.tick() => {
                    if let Some(reconnected_events) = self.heartbeat_tick().await {
                        events = Some(reconnected_events);
                    }
                }
            }
        }
    }

    fn is_degraded(&self) -> bool {
        matches!(self.connection, ConnectionState::Degraded { .. })
    }

    fn enter_degraded(&mut self, reason: String) {
        if self.is_degraded() {
            return;
        }
        let now = self.clock.now();
        tracing::warn!(%reason, engine = %self.engine, "the Politeness store is unreachable; entering degraded mode");
        self.connection = ConnectionState::Degraded {
            reason: reason.clone(),
            next_reconnect_at: now + self.reconnect_initial,
            backoff: self.reconnect_initial,
        };
        let keys: Vec<String> = self.heartbeats.keys().cloned().collect();
        for key in keys {
            let has_live_delegation = self.has_live_delegation(&key);
            let max_lease_end = self.max_lease_expiry(&key);
            if let Some(hb) = self.heartbeats.get_mut(&key) {
                if has_live_delegation {
                    // While Delegation is still live, it keeps serving until
                    // `delegation.expires_at`; degraded_admit_at is set on expiry.
                    hb.degraded_admit_at = None;
                } else {
                    let hb_stale = hb
                        .last_hb_sent_at
                        .map_or(now, |sent| sent + self.heartbeat_ttl);
                    let leases_end = max_lease_end.map_or(hb_stale, |lease| lease.max(hb_stale));
                    hb.degraded_admit_at = Some(leases_end + hb.max_timeout);
                }
                hb.degraded_entered = false;
                hb.degraded_stopped = false;
            }
            tracing::trace!(
                target: TRACE_TARGET,
                transition = "cut_off",
                engine = %self.engine,
                key = %key,
            );
        }
        self.publish_degraded_status(true, false, Some(reason));
    }

    fn all_degraded_stopped(&self) -> bool {
        self.heartbeats
            .values()
            .filter(|hb| hb.last_hb_sent_at.is_some())
            .all(|hb| hb.degraded_stopped)
    }

    fn publish_degraded_status(
        &self,
        degraded: bool,
        admission_stopped: bool,
        reason: Option<String>,
    ) {
        if let Some(metrics) = &self.metrics {
            metrics.politeness_store_degraded(degraded);
        }
        let next = StoreDegradedStatus {
            degraded,
            admission_stopped,
            degraded_max: self.degraded_max,
            reason,
        };
        self.degraded.send_if_modified(|current| {
            if *current == next {
                false
            } else {
                *current = next;
                true
            }
        });
    }

    fn max_lease_expiry(&self, key: &str) -> Option<Instant> {
        self.active_leases
            .values()
            .filter(|lease| lease.key == key)
            .map(|lease| lease.last_renewed + self.permit_lease)
            .max()
    }

    fn local_in_flight_count(&self, key: &str) -> u64 {
        let active = self
            .active_leases
            .values()
            .filter(|lease| lease.key == key)
            .count();
        let degraded = self
            .degraded_permits
            .values()
            .filter(|permit| permit.key == key)
            .count();
        let mem = self.memory.in_flight(key);
        mem.saturating_add(u64::try_from(active.saturating_add(degraded)).unwrap_or(u64::MAX))
    }

    fn ensure_heartbeat_entry(&mut self, key: &str, cap: Option<u64>) {
        let entry = self
            .heartbeats
            .entry(key.to_owned())
            .or_insert_with(|| KeyHeartbeat {
                reported_cap: cap,
                reported_timeout: self.request_timeout,
                last_hb_sent_at: None,
                granted_share: 0,
                acked_share: 0,
                max_cap: cap.unwrap_or(0),
                max_timeout: self.request_timeout,
                degraded_admit_at: None,
                degraded_entered: false,
                degraded_stopped: false,
            });
        if let Some(new_cap) = cap {
            entry.reported_cap = Some(entry.reported_cap.map_or(new_cap, |old| old.max(new_cap)));
        }
    }

    async fn send_key_heartbeat(&mut self, key: &str, tokens: &[String]) -> Result<(), StoreError> {
        let Some(hb) = self.heartbeats.get(key) else {
            return Ok(());
        };
        let params = HeartbeatParams {
            key,
            engine_id: &self.engine,
            heartbeat_ttl: self.heartbeat_ttl,
            degraded_max: self.degraded_max,
            timeout: hb.reported_timeout,
            cap: hb.reported_cap,
            acked_share: hb.acked_share,
            permit_lease: self.permit_lease,
            in_flight_tokens: tokens,
        };
        let before = self.clock.now();
        let ack = self.store.heartbeat(&params).await?;
        if ack.live_engines == 0 {
            // Another engine holds a live Delegation; wait for recall before
            // registering this engine's heartbeat and share.
            return Ok(());
        }
        if let Some(hb) = self.heartbeats.get_mut(key) {
            hb.last_hb_sent_at = Some(before);
            hb.granted_share = ack.granted_share;
            hb.acked_share = ack.granted_share;
            hb.max_cap = ack.max_cap;
            hb.max_timeout = Duration::from_millis(ack.max_timeout_ms);
        }
        if tokens.is_empty() {
            tracing::trace!(
                target: TRACE_TARGET,
                transition = "heartbeat",
                engine = %self.engine,
                key = %key,
                share = ack.granted_share,
            );
        }
        Ok(())
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
        self.ensure_heartbeat_entry(&take.key, take.rule.0.concurrency());
        let delegates = take.rule.0.delegates();
        if delegates && self.has_live_delegation(&take.key) {
            let taken = self.taken_delegated(&take);
            let _ = answer.send(Ok(taken));
        } else if self.is_degraded() {
            self.expire_delegation_if_needed(&take.key);
            let taken = self.taken_degraded(&take);
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
            tracing::trace!(
                target: TRACE_TARGET,
                transition = "expire_delegation",
                engine = %self.engine,
                key = %key,
            );
            if let Some(delegation) = self.delegations.remove(key) {
                let (permits, waiting) = self.memory.recall_view(key);
                let in_flight = u64::try_from(permits.len()).unwrap_or(u64::MAX);
                if self.is_degraded() {
                    for permit_id in permits {
                        let token = self.permit_token(permit_id);
                        self.recalled
                            .insert(permit_id, (key.to_owned(), token.clone()));
                        self.degraded_permits.insert(
                            permit_id,
                            DegradedPermit {
                                key: key.to_owned(),
                                token,
                                last_in_flight: in_flight,
                            },
                        );
                    }
                    if let Some(hb) = self.heartbeats.get_mut(key) {
                        hb.degraded_admit_at = Some(delegation.expires_at + hb.max_timeout);
                    }
                    // Keep local rule state in memory.tables so Retry-After,
                    // backoff and halving continue locally during the outage.
                } else {
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
                                last_in_flight: in_flight,
                            },
                        );
                    }
                    delegation.rule.0.forget(&mut self.memory.tables, key);
                }
                for run_id in waiting {
                    self.record_waiter(key, &run_id);
                }
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

    fn check_degraded_window(
        &mut self,
        take: &Take<'_>,
        now: Instant,
    ) -> Result<(u64, u64), Taken> {
        let key = &*take.key;
        let wait_none = |this: &mut Self| {
            if take.in_turn.waits() {
                this.record_waiter(key, &take.holder.run_id);
            }
            Taken::Wait(None)
        };
        let Some((last_hb, admit_at, granted_share, max_cap)) =
            self.heartbeats.get(key).and_then(|hb| {
                let last_hb = hb.last_hb_sent_at?;
                let admit_at = hb
                    .degraded_admit_at
                    .unwrap_or(last_hb + self.heartbeat_ttl + hb.max_timeout);
                Some((last_hb, admit_at, hb.granted_share, hb.max_cap))
            })
        else {
            return Err(wait_none(self));
        };
        if now >= last_hb + self.degraded_max {
            if let Some(hb_mut) = self.heartbeats.get_mut(key)
                && !hb_mut.degraded_stopped
            {
                hb_mut.degraded_stopped = true;
                tracing::trace!(
                    target: TRACE_TARGET,
                    transition = "stop_degraded",
                    engine = %self.engine,
                    key = %key,
                );
                if self.all_degraded_stopped() {
                    let reason = match &self.connection {
                        ConnectionState::Degraded { reason, .. } => Some(reason.clone()),
                        ConnectionState::Connected => None,
                    };
                    self.publish_degraded_status(true, true, reason);
                }
            }
            return Err(wait_none(self));
        }
        if now < admit_at {
            let wait = admit_at.saturating_duration_since(now);
            if take.in_turn.waits() {
                self.record_waiter(key, &take.holder.run_id);
            }
            return Err(Taken::Wait(Some((take.now + wait, Hold::Delay))));
        }
        if let Some(hb_mut) = self.heartbeats.get_mut(key)
            && !hb_mut.degraded_entered
        {
            hb_mut.degraded_entered = true;
            tracing::trace!(
                target: TRACE_TARGET,
                transition = "enter_degraded",
                engine = %self.engine,
                key = %key,
                share = granted_share,
            );
        }
        Ok((granted_share, max_cap))
    }

    fn taken_degraded(&mut self, take: &Take<'_>) -> Taken {
        let key = &*take.key;
        let now = self.clock.now();
        let (granted_share, max_cap) = match self.check_degraded_window(take, now) {
            Ok(window) => window,
            Err(wait) => return wait,
        };
        let local_in_flight = self.local_in_flight_count(key);
        let verdict = take.rule.0.admit_degraded_store(
            &mut self.memory.tables,
            key,
            local_in_flight,
            granted_share,
            max_cap,
            take.now,
        );
        match verdict {
            Verdict::Go => {
                self.remove_waiter(key, &take.holder.run_id);
                let id = self.next_shared_id;
                self.next_shared_id -= 1;
                let token = self.permit_token(id);
                let in_flight = local_in_flight + 1;
                self.degraded_permits.insert(
                    id,
                    DegradedPermit {
                        key: key.to_owned(),
                        token,
                        last_in_flight: in_flight,
                    },
                );
                tracing::trace!(
                    target: TRACE_TARGET,
                    transition = "take_degraded",
                    engine = %self.engine,
                    key = %key,
                    in_flight,
                    share = granted_share,
                );
                Taken::Granted(Permit {
                    id,
                    key: key.to_owned(),
                    send_at: None,
                    delegated: false,
                })
            }
            Verdict::Until(until, hold) => Taken::Wait(Some((until, hold))),
            Verdict::Refused(until) => Taken::Refused(until),
            Verdict::Busy => {
                if take.in_turn.waits() {
                    self.record_waiter(key, &take.holder.run_id);
                }
                Taken::Wait(None)
            }
        }
    }

    async fn try_acquire_delegation(&mut self, key: &str, rule: &SharedRule) {
        if let Err(error) = self.ensure_subscribed(key).await {
            if matches!(error, StoreError::Unreachable(_)) {
                self.enter_degraded(error.to_string());
            } else {
                tracing::warn!(%error, key, "could not subscribe to Politeness store wakes before delegating");
            }
            return;
        }
        let before = self.clock.now();
        let reported_cap = self
            .heartbeats
            .get(key)
            .and_then(|hb| hb.reported_cap)
            .or_else(|| rule.0.concurrency());
        self.work.count(Row::PolitenessDelegationTransactions);
        let hb = DelegationHeartbeat {
            heartbeat_ttl: self.heartbeat_ttl,
            degraded_max: self.degraded_max,
            timeout: self.request_timeout,
            cap: reported_cap,
        };
        match self
            .store
            .delegate_with_heartbeat(key, &self.engine, self.delegation_ttl, hb)
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
                    key.to_owned(),
                    StoreDelegation {
                        runs: Vec::new(),
                        rule: rule.clone(),
                        expires_at: before + self.delegation_ttl,
                        last_renewed: self.clock.now(),
                    },
                );
                let cap_val = reported_cap.unwrap_or(0);
                if let Some(entry) = self.heartbeats.get_mut(key) {
                    entry.last_hb_sent_at = Some(before);
                    entry.granted_share = cap_val;
                    entry.acked_share = cap_val;
                    entry.max_cap = cap_val;
                    entry.max_timeout = self.request_timeout;
                }
                tracing::trace!(
                    target: TRACE_TARGET,
                    transition = "heartbeat",
                    engine = %self.engine,
                    key = %key,
                    share = cap_val,
                );
                tracing::trace!(
                    target: TRACE_TARGET,
                    transition = "acquire_delegation",
                    engine = %self.engine,
                    key = %key,
                );
            }
            Ok(DelegationAcquire::HeldByOther { .. } | DelegationAcquire::Shared) => {
                self.shared.insert(key.to_owned());
            }
            Err(error) => {
                if matches!(error, StoreError::Unreachable(_)) {
                    self.enter_degraded(error.to_string());
                } else {
                    tracing::warn!(%error, key, "could not acquire Politeness store Delegation; falling back to shared take");
                }
            }
        }
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
                if self.is_degraded() {
                    break;
                }
                self.try_acquire_delegation(key, rule).await;
            }
        }

        for item in asked {
            let answer = if self.has_live_delegation(&item.take.key) {
                Ok(self.taken_delegated(&item.take))
            } else if self.is_degraded() {
                Ok(self.taken_degraded(&item.take))
            } else {
                self.taken_shared(&item.take).await
            };
            let _ = item.answer.send(answer);
        }
    }

    async fn prepare_shared_take(
        &mut self,
        take: &Take<'_>,
    ) -> Result<Result<(Option<u64>, Option<(&'static str, Vec<u8>)>), Taken>, PolitenessError>
    {
        let key = &*take.key;
        let Some(part) = take.rule.0.store_part() else {
            return Ok(Ok((take.rule.0.concurrency(), None)));
        };
        self.work.count(Row::PolitenessTransactions);
        let peek = match self.store.peek_shared(key, &self.engine).await {
            Ok(peek) => peek,
            Err(error) if matches!(error, StoreError::Unreachable(_)) => {
                self.enter_degraded(error.to_string());
                return Ok(Err(self.taken_degraded(take)));
            }
            Err(error) => return Err(store_error(&error)),
        };
        let (in_flight, state_map) = match peek {
            PeekShared::Recalled => {
                if take.in_turn.waits() {
                    self.record_waiter(key, &take.holder.run_id);
                    self.schedule_recall_retry(key);
                }
                return Ok(Err(Taken::Wait(None)));
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
            Verdict::Go => Ok(Ok((effective_cap, updated.map(|bytes| (part, bytes))))),
            Verdict::Until(until, hold) => Ok(Err(Taken::Wait(Some((until, hold))))),
            Verdict::Refused(until) => Ok(Err(Taken::Refused(until))),
            Verdict::Busy => {
                if take.in_turn.waits() {
                    self.record_waiter(key, &take.holder.run_id);
                }
                Ok(Err(Taken::Wait(None)))
            }
        }
    }

    async fn taken_shared(&mut self, take: &Take<'_>) -> Result<Taken, PolitenessError> {
        let key = &*take.key;
        if let Err(error) = self.ensure_subscribed(key).await {
            if matches!(error, StoreError::Unreachable(_)) {
                self.enter_degraded(error.to_string());
                return Ok(self.taken_degraded(take));
            }
            return Err(store_error(&error));
        }

        let needs_initial_hb = self
            .heartbeats
            .get(key)
            .is_none_or(|hb| hb.last_hb_sent_at.is_none());
        if needs_initial_hb && let Err(error) = self.send_key_heartbeat(key, &[]).await {
            if matches!(error, StoreError::Unreachable(_)) {
                self.enter_degraded(error.to_string());
                return Ok(self.taken_degraded(take));
            }
            return Err(store_error(&error));
        }

        let (cap, state_update) = match self.prepare_shared_take(take).await? {
            Ok(prepared) => prepared,
            Err(early) => return Ok(early),
        };

        let id = self.next_shared_id;
        self.next_shared_id -= 1;
        let token = self.permit_token(id);
        let pace = take.rule.0.store_pace();
        let state_arg = state_update
            .as_ref()
            .map(|(part, bytes)| (*part, bytes.as_slice()));

        self.work.count(Row::PolitenessTransactions);
        let result = match self
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
        {
            Ok(result) => result,
            Err(error) if matches!(error, StoreError::Unreachable(_)) => {
                self.enter_degraded(error.to_string());
                return Ok(self.taken_degraded(take));
            }
            Err(error) => return Err(store_error(&error)),
        };

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
                tracing::trace!(
                    target: TRACE_TARGET,
                    transition = "take_shared",
                    engine = %self.engine,
                    key = %key,
                    in_flight,
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

    async fn give_back_shared_permit(
        &mut self,
        permit: &Permit,
        promoted: Option<(String, String)>,
        rule: &SharedRule,
        ending: Ending,
        now: SystemTime,
    ) -> Result<Option<SystemTime>, PolitenessError> {
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

        tracing::trace!(
            target: TRACE_TARGET,
            transition = "give_back",
            engine = %self.engine,
            key = %permit.key,
        );

        if self.is_degraded() {
            self.wake_local_waiters(&permit.key);
            return Ok(ready_at);
        }

        let state_part = rule.0.store_part();
        let state_arg = match (state_part, updated_bytes.as_deref()) {
            (Some(part), Some(bytes)) => Some((part, bytes)),
            _ => None,
        };

        self.work.count(Row::PolitenessTransactions);
        match self
            .store
            .give_back_shared(&permit.key, &self.engine, &token, STATE_TTL, state_arg)
            .await
        {
            Ok(given) => {
                if given.alone {
                    self.shared.remove(&permit.key);
                }
                Ok(ready_at)
            }
            Err(error) if matches!(error, StoreError::Unreachable(_)) => {
                self.enter_degraded(error.to_string());
                self.wake_local_waiters(&permit.key);
                Ok(ready_at)
            }
            Err(error) => Err(store_error(&error)),
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

        if let Some(degraded_permit) = self.degraded_permits.remove(&permit.id) {
            let others = degraded_permit.last_in_flight.saturating_sub(1);
            let (ready_at, _) =
                rule.0
                    .ended_store(&mut self.memory.tables, &permit.key, others, ending, now);
            tracing::trace!(
                target: TRACE_TARGET,
                transition = "give_back",
                engine = %self.engine,
                key = %permit.key,
            );
            self.wake_local_waiters(&permit.key);
            return Ok(ready_at);
        }

        if !permit.delegated || promoted.is_some() {
            return self
                .give_back_shared_permit(permit, promoted, rule, ending, now)
                .await;
        }

        // Delegated Permit: served completely in memory (`E6.transactions = 0`).
        let (ready_at, turn) = rule
            .0
            .given_back_in_memory(&mut self.memory, permit, ending, now);
        tracing::trace!(
            target: TRACE_TARGET,
            transition = "give_back",
            engine = %self.engine,
            key = %permit.key,
        );
        if ending != Ending::Done
            && !self.is_degraded()
            && let Some(part) = rule.0.store_part()
            && let Some(bytes) = rule.0.encode_store_from(&self.memory.tables, &permit.key)
        {
            self.work.count(Row::PolitenessOriginTransactions);
            if let Err(error) = self
                .store
                .save_state(&permit.key, STATE_TTL, part, &bytes)
                .await
            {
                if matches!(error, StoreError::Unreachable(_)) {
                    self.enter_degraded(error.to_string());
                } else {
                    return Err(store_error(&error));
                }
            }
        }
        if let Some(turn) = turn {
            let _ = self.wakes.send(turn.wake);
            return Ok(ready_at);
        }
        let idle = self.delegations.get(&permit.key).is_some_and(|delegation| {
            delegation.runs.is_empty() && self.memory.in_flight(&permit.key) == 0
        });
        if idle
            && !self.is_degraded()
            && let Err(error) = self.release_delegation_key(&permit.key, false).await
        {
            if self.is_degraded() {
                return Ok(ready_at);
            }
            return Err(error);
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
        if !self.is_degraded() {
            for key in idle_keys {
                let _ = self.release_delegation_key(&key, false).await;
            }
        }
        Ok(())
    }

    async fn handle_change(&mut self, mut change: Box<dyn ErasedStoreChange>) {
        if let Some(delegated_key) = change.delegated_key()
            && self.has_live_delegation(&delegated_key)
        {
            let persist = change.apply_in_memory(&mut self.memory);
            if !self.is_degraded()
                && let (Some(bytes), Some((store_key, part))) = (persist, change.store_target())
            {
                self.work.count(Row::PolitenessOriginTransactions);
                if let Err(error) = self
                    .store
                    .save_state(&store_key, STATE_TTL, part, &bytes)
                    .await
                {
                    if matches!(error, StoreError::Unreachable(_)) {
                        self.enter_degraded(error.to_string());
                    } else {
                        tracing::warn!(%error, key = %store_key, "could not persist site ask to Politeness store");
                    }
                }
            }
            return;
        }
        let Some((store_key, part)) = change.store_target() else {
            change.apply_in_memory(&mut self.memory);
            return;
        };
        if self.is_degraded() {
            change.apply_in_memory(&mut self.memory);
            return;
        }
        let loaded = match self.store.load_state(&store_key, part).await {
            Ok(bytes) => bytes,
            Err(error) if matches!(error, StoreError::Unreachable(_)) => {
                self.enter_degraded(error.to_string());
                change.apply_in_memory(&mut self.memory);
                return;
            }
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
            if matches!(error, StoreError::Unreachable(_)) {
                self.enter_degraded(error.to_string());
            } else {
                tracing::warn!(%error, key = %store_key, "could not save shared Politeness state");
            }
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
            WakeEvent::Delegated { holder } if holder != self.engine => {
                // Another engine took the Delegation while we were idle; drop
                // our old heartbeat record so we don't keep a stale share.
                self.heartbeats.remove(&key);
            }
            WakeEvent::Recall { .. } | WakeEvent::Delegated { .. } => {}
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
        if let Err(error) = self
            .store
            .release_delegation(
                key,
                &self.engine,
                self.permit_lease,
                STATE_TTL,
                &tokens,
                &state_args,
            )
            .await
        {
            if matches!(error, StoreError::Unreachable(_)) {
                // Keep the Delegation locally if it hasn't expired yet.
                self.delegations.insert(key.to_owned(), delegation);
                self.enter_degraded(error.to_string());
                return Ok(());
            }
            return Err(store_error(&error));
        }

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

    async fn clean_shutdown(&mut self) {
        if self.is_degraded() {
            return;
        }
        let delegation_keys: Vec<String> = self.delegations.keys().cloned().collect();
        for key in delegation_keys {
            let _ = self.release_delegation_key(&key, false).await;
        }
        // ADR 0053: an engine that exits cleanly deletes its own heartbeat record.
        let hb_keys: Vec<String> = self.heartbeats.keys().cloned().collect();
        for key in hb_keys {
            let _ = self.store.leave_engine(&key, &self.engine).await;
        }
    }

    async fn renew_due_delegations(&mut self, now: Instant) -> bool {
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
                    tracing::trace!(
                        target: TRACE_TARGET,
                        transition = "renew_delegation",
                        engine = %self.engine,
                        key = %key,
                    );
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
                    if matches!(error, StoreError::Unreachable(_)) {
                        self.enter_degraded(error.to_string());
                        self.expire_delegation_if_needed(&key);
                        return false;
                    }
                    self.expire_delegation_if_needed(&key);
                }
            }
        }
        true
    }

    async fn heartbeat_tick(&mut self) -> Option<WakeEvents> {
        let now = self.clock.now();

        if self.is_degraded() {
            return self.degraded_tick(now).await;
        }

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

        if !self.renew_due_delegations(now).await {
            return None;
        }

        // Send periodic engine heartbeats for every active key to refresh
        // liveness and store-computed degraded shares (ADR 0053).
        let hb_interval = self.heartbeat_ttl / 3;
        let now = self.clock.now();
        let due_hbs: Vec<String> = self
            .heartbeats
            .iter()
            .filter(|(_, hb)| {
                hb.last_hb_sent_at
                    .is_none_or(|sent| now.saturating_duration_since(sent) >= hb_interval)
            })
            .map(|(key, _)| key.clone())
            .collect();
        for key in due_hbs {
            if let Err(error) = self.send_key_heartbeat(&key, &[]).await {
                if matches!(error, StoreError::Unreachable(_)) {
                    self.enter_degraded(error.to_string());
                    return None;
                }
                tracing::debug!(%error, key, "Politeness store heartbeat failed");
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
                    if matches!(error, StoreError::Unreachable(_)) {
                        self.enter_degraded(error.to_string());
                        return None;
                    }
                    tracing::debug!(%error, key, "shared Permit lease renewal failed");
                }
            }
        }
        None
    }

    async fn degraded_tick(&mut self, now: Instant) -> Option<WakeEvents> {
        // Expire any held Delegations whose TTL elapsed during the outage so
        // they transition to ADR 0053's degraded-share wait and admission.
        let delegation_keys: Vec<String> = self.delegations.keys().cloned().collect();
        for key in delegation_keys {
            self.expire_delegation_if_needed(&key);
        }

        // Check `politeness.degraded_max` expiry and degraded-admission readiness.
        let mut wake_keys = Vec::new();
        let mut newly_stopped = false;
        for (key, hb) in &mut self.heartbeats {
            let Some(last_hb) = hb.last_hb_sent_at else {
                continue;
            };
            let stop_at = last_hb + self.degraded_max;
            if now >= stop_at {
                if !hb.degraded_stopped {
                    hb.degraded_stopped = true;
                    newly_stopped = true;
                    tracing::trace!(
                        target: TRACE_TARGET,
                        transition = "stop_degraded",
                        engine = %self.engine,
                        key = %key,
                    );
                }
                continue;
            }
            if let Some(admit_at) = hb.degraded_admit_at
                && now >= admit_at
                && !hb.degraded_entered
            {
                wake_keys.push(key.clone());
            }
        }
        if newly_stopped && self.all_degraded_stopped() {
            let reason = match &self.connection {
                ConnectionState::Degraded { reason, .. } => Some(reason.clone()),
                ConnectionState::Connected => None,
            };
            self.publish_degraded_status(true, true, reason);
        }
        for key in wake_keys {
            self.wake_local_waiters(&key);
        }

        // Attempt automatic reconnection with exponential backoff.
        let should_retry = match &self.connection {
            ConnectionState::Degraded {
                next_reconnect_at, ..
            } => now >= *next_reconnect_at,
            ConnectionState::Connected => false,
        };
        if !should_retry {
            return None;
        }
        match self.try_reconnect().await {
            Ok(new_events) => {
                self.connection = ConnectionState::Connected;
                self.publish_degraded_status(false, false, None);
                let all_keys: Vec<String> = self.waiters.keys().cloned().collect();
                for key in all_keys {
                    self.wake_local_waiters(&key);
                }
                tracing::info!(engine = %self.engine, "reconnected to the Politeness store");
                Some(new_events)
            }
            Err(error) => {
                let next_now = self.clock.now();
                if let ConnectionState::Degraded {
                    next_reconnect_at,
                    backoff,
                    ..
                } = &mut self.connection
                {
                    *backoff = (*backoff * 2).min(MAX_RECONNECT_BACKOFF);
                    *next_reconnect_at = next_now + *backoff;
                }
                tracing::debug!(%error, engine = %self.engine, "Politeness store reconnect attempt failed");
                None
            }
        }
    }

    async fn try_reconnect(&mut self) -> Result<WakeEvents, StoreError> {
        let new_store = self.store.reconnect().await?;
        let (mut new_subs, new_events) = new_store.open_wake_bus().await?;

        let keys_to_subscribe: Vec<String> = self
            .subscribed
            .keys()
            .chain(self.heartbeats.keys())
            .cloned()
            .collect::<HashSet<_>>()
            .into_iter()
            .collect();
        let mut subscribed = HashMap::with_capacity(keys_to_subscribe.len());
        let mut channel_to_key = HashMap::with_capacity(keys_to_subscribe.len());
        for key in keys_to_subscribe {
            let channel = new_subs.subscribe(&key).await?;
            subscribed.insert(key.clone(), channel.clone());
            channel_to_key.insert(channel, key);
        }

        self.store = new_store;
        self.subs = new_subs;
        self.subscribed = subscribed;
        self.channel_to_key = channel_to_key;

        // Reconcile any Delegations held during the outage: if a Delegation
        // survived on the server, renew it; otherwise expire it locally so
        // its in-flight Permits are registered atomically below.
        let delegation_keys: Vec<String> = self.delegations.keys().cloned().collect();
        for key in delegation_keys {
            if self.has_live_delegation(&key) {
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
                        if let Some(delegation) = self.delegations.get_mut(&key) {
                            delegation.expires_at = before;
                        }
                        self.expire_delegation_if_needed(&key);
                    }
                    Err(error) => return Err(error),
                }
            } else {
                self.expire_delegation_if_needed(&key);
            }
        }

        // ADR 0053: on reconnecting, register all requests in flight as
        // Permits BEFORE the store frees our stale reservation.
        let keys_to_sync: Vec<String> = self.heartbeats.keys().cloned().collect();
        let now = self.clock.now();
        for key in keys_to_sync {
            let mut tokens: Vec<String> = self
                .active_leases
                .values()
                .filter(|lease| lease.key == key)
                .map(|lease| lease.token.clone())
                .collect();
            let degraded_ids: Vec<(i64, String, u64)> = self
                .degraded_permits
                .iter()
                .filter(|(_, permit)| permit.key == key)
                .map(|(id, permit)| (*id, permit.token.clone(), permit.last_in_flight))
                .collect();
            for (_, token, _) in &degraded_ids {
                tokens.push(token.clone());
            }

            self.send_key_heartbeat(&key, &tokens).await?;

            for lease in self.active_leases.values_mut() {
                if lease.key == key {
                    lease.last_renewed = now;
                }
            }
            for (id, token, last_in_flight) in degraded_ids {
                self.degraded_permits.remove(&id);
                self.active_leases.insert(
                    id,
                    ActiveLease {
                        key: key.clone(),
                        token,
                        last_renewed: now,
                        last_in_flight,
                    },
                );
            }
            if let Some(hb) = self.heartbeats.get_mut(&key) {
                hb.degraded_admit_at = None;
                hb.degraded_entered = false;
                hb.degraded_stopped = false;
            }
            tracing::trace!(
                target: TRACE_TARGET,
                transition = "reconnect",
                engine = %self.engine,
                key = %key,
                registered = tokens.len(),
            );
        }

        Ok(new_events)
    }

    async fn ensure_subscribed(&mut self, key: &str) -> Result<(), StoreError> {
        if self.subscribed.contains_key(key) {
            return Ok(());
        }
        let channel = self.subs.subscribe(key).await?;
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
