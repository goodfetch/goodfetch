//! Partitioned crawl coordinator, operation guards, and mTLS peer-link
//! extension handlers (ADRs 0054, 0056, 0087, 0095 and 0140).
//!
//! Size: one concern, the Partitioned crawl coordinator contribution and its
//! peer-link RPC/extension handlers (`crawl.start`, `run.*`, `dataset.*`,
//! `partition.*`) kept together so every Partition fan-out, forwarder lifecycle,
//! and local/SQLite state transition shares one state registry.
#![expect(
    clippy::disallowed_types,
    reason = "short in-memory coordinator metadata locks held across no await, outside loom"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::{Arc, Mutex, PoisonError, RwLock, Weak};
use std::time::{Duration, SystemTime};

use goodfetch_crawl::CrawlRuntime;
use goodfetch_crawl::partition::{ForwardedItem, PartitionRouter, PartitionStartOptions};
use goodfetch_dataset::Query;
use goodfetch_engine::run::{run_id, run_layer, settings_error};
use goodfetch_engine::settings::Opening;
use goodfetch_engine::tasks::BlockingThreads;
use goodfetch_engine::{
    Answered, CallContext, CallError, Contribution, GroupMembershipContext, GroupMembershipHook,
    GroupPeer, Guard, PeerExtensionAnswer, PeerExtensionHandler, RunIndex, give_scoped_limits,
};
use goodfetch_frontier::{DomainKeys, host_of};
use goodfetch_protocol::catalog::{Operation, Product};
use goodfetch_protocol::checkpoint::{
    RUN_CHECKPOINT, RUN_PAUSE, RunCheckpointParams, RunResumeParams,
};
use goodfetch_protocol::foxhound::crawl::{CRAWL_START, CrawlStartParams, Seed};
use goodfetch_protocol::foxhound::dataset::{
    DatasetRecord, RECORD, RUN_RECORDS, RecordRead, RecordsRead, RunRecordsParams,
};
use goodfetch_protocol::foxhound::dataset_view::{
    DATASET_LIST, DATASET_PAGE, DATASET_PAGE_DEFAULT, DatasetList, DatasetListParams, DatasetPage,
    DatasetPageParams,
};
use goodfetch_protocol::foxhound::usage::PARTITIONED_CRAWL;
use goodfetch_protocol::group::{LeadEpoch, PlacementRequest};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::proxy::{PROXY_PROVIDER, PROXY_SERVER};
use goodfetch_protocol::run::{
    Checkpoint, Lifecycle, LimitScope, RUN_ABORT, RUN_PROGRESS_READ, RUN_REPORT, RUN_STOP,
    RUN_USAGE, RunAbortParams, RunId, RunParams, RunProgress, RunProgressParams, RunReport,
    RunStarted, RunStopParams, RunUsage, StartedAs, StartedPartition,
};
use goodfetch_protocol::settings::session_source::SessionSource;
use goodfetch_protocol::settings::{RunOrigin, SESSION_SOURCE};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

use crate::aggregation::{aggregate_lifecycle, aggregate_reports, running_placeholder, sum_usage};
use crate::forwarding::{
    CreditWindow, ForwardLinksRequest, ForwardLinksResponse, ForwardedLink, PEER_FORWARD_LINKS,
    classify_incoming,
};
use crate::handoff::{
    ActivateDomainRequest, DrainDomainsRequest, DrainDomainsResponse, HandoffDomainRequest,
    HandoffDomainResponse, PEER_ACTIVATE_DOMAIN, PEER_DRAIN_DOMAINS, PEER_HANDOFF_DOMAIN,
    PEER_PUBLISH_MAP, PEER_REMOVE_MEMBER_MAP, PartitionAdoptHook, PublishMapRequest,
    RemoveMemberMapRequest, RemoveMemberMapResponse, TransferredRefetchStop, compute_moves,
    distinct_domains_in_run_file, map_with_added_member, map_without_member, trace_ack_handoff,
    trace_admit_domain, trace_assign_initial, trace_drain_inflight, trace_fence_domain,
    trace_publish_map, trace_reroute_forward, trace_send_handoff,
};
use crate::rendezvous::{PartitionMap, PartitionSlot};
use crate::store::{
    self, PartitionedCrawlRecord, RUN_STATE_TABLES, STORE, read_partition_limit_scopes,
    write_partition_membership,
};
use crate::validation::{
    PartitionedStartCheck, ValidatedPartitions, check_session_source_for_partitions,
    validate_and_select,
};

/// Peer extension method that persists & caches a [`PartitionedCrawlRecord`].
const PEER_REGISTER: &str = "partition.register";
/// Peer extension method that prepares a target Group member to start a
/// Partition Run with its own `PartitionMap` and initial seeds.
const PEER_PREPARE_MEMBER: &str = "partition.prepare_member";
/// Peer extension method that reads committed Dataset Records from a local
/// Partition's run file so `run.records` returns the union across Partitions.
const PEER_READ_RECORDS: &str = "partition.read_records";

/// Operations intercepted when their `run_id` names a Partitioned crawl.
const PARTITIONED_RUN_OPS: &[&str] = &[
    RUN_REPORT.name,
    RUN_USAGE.name,
    RUN_PROGRESS_READ.name,
    RUN_STOP.name,
    RUN_PAUSE.name,
    RUN_ABORT.name,
    RUN_CHECKPOINT.name,
    "run.resume",
    RUN_RECORDS.name,
    DATASET_PAGE.name,
    DATASET_LIST.name,
];

/// Retry backoff when forwarding links to a peer fails transiently.
const FORWARD_RETRY_DELAY: Duration = Duration::from_millis(50);

/// Pending Partition preparation delivered to a member right before its
/// `crawl.start` RPC arrives.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct PreparedPartition {
    crawl_id: RunId,
    partition: u32,
    run_id: RunId,
    map: PartitionMap,
    initial_seeds: Vec<Seed>,
    #[serde(default)]
    initial_held_domains: Vec<String>,
}

/// Request payload for `partition.read_records`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReadRecordsRequest {
    run_id: RunId,
    dataset: String,
    after: u64,
    #[serde(default)]
    through: Option<u64>,
    #[serde(default)]
    wall: Option<bool>,
}

/// Response payload for `partition.read_records`.
#[derive(Clone, Debug, Serialize, Deserialize)]
struct ReadRecordsResponse {
    records: Vec<DatasetRecord>,
}

/// Shared state of the Partitioned crawl contribution on one engine instance.
///
/// Holds NO strong `EngineContext` or `Runs` handles so shutdown never forms an
/// `Arc` cycle.
struct State {
    runtime: CrawlRuntime,
    /// Known Partitioned crawls keyed by Partitioned crawl ID.
    known: RwLock<BTreeMap<RunId, PartitionedCrawlRecord>>,
    /// Reverse lookup from a Partition's Run ID to `(crawl_id, partition_index)`.
    by_partition_run: RwLock<BTreeMap<RunId, (RunId, u32)>>,
    /// Pending Partition start preparations keyed by Partition Run ID.
    prepared: Mutex<BTreeMap<RunId, PreparedPartition>>,
    /// Local Partition run folders keyed by Partition Run ID (for read-only
    /// Dataset reads after a Partition finishes).
    run_folders: Mutex<BTreeMap<RunId, PathBuf>>,
    /// Domain-key settings cached per Partition Run ID so resume preserves them.
    domain_keys: Mutex<BTreeMap<RunId, Arc<DomainKeys>>>,
    /// Tracked blocking threads handle from the engine's task owner (no strong
    /// `EngineContext` or `Runs` reference, so no shutdown `Arc` cycle).
    blocking: Mutex<Option<BlockingThreads>>,
    /// Path to this engine's per-major `partitioned-crawls.sqlite` (populated
    /// lazily from `EngineContext::home()` on first call).
    store_path: Mutex<Option<PathBuf>>,
    /// Whether `known` has been hydrated from `partitioned-crawls.sqlite` once.
    hydrated: Mutex<bool>,
    /// Shared live `PartitionMap` handles per Partitioned crawl ID so
    /// membership changes atomically update every live router and forwarder.
    live_maps: RwLock<BTreeMap<RunId, Arc<RwLock<Arc<PartitionMap>>>>>,
    /// Start parameters cached on the Lead Daemon so a newly added Group
    /// member can start a new Partition for any running Partitioned crawl.
    start_params: RwLock<BTreeMap<RunId, CrawlStartParams>>,
    /// Known domain keys seen/seeded for each Partitioned crawl (used when a
    /// Group member joins to hold incoming moving domains on the new Partition).
    known_domains: RwLock<BTreeMap<RunId, BTreeSet<String>>>,
    /// Weak handle to the engine's `GroupPeer` (never strong, avoiding any
    /// `EngineContext <-> State` shutdown `Arc` cycle).
    peer_weak: Mutex<Option<Weak<dyn GroupPeer>>>,
    /// Local Run index handle for moving Wall allowance refetch stops
    /// (ADR 0052 and 0054; holds no `EngineContext`).
    run_index: Mutex<Option<RunIndex>>,
}

impl State {
    fn new(runtime: CrawlRuntime) -> Self {
        Self {
            runtime,
            known: RwLock::new(BTreeMap::new()),
            by_partition_run: RwLock::new(BTreeMap::new()),
            prepared: Mutex::new(BTreeMap::new()),
            run_folders: Mutex::new(BTreeMap::new()),
            domain_keys: Mutex::new(BTreeMap::new()),
            blocking: Mutex::new(None),
            store_path: Mutex::new(None),
            hydrated: Mutex::new(false),
            live_maps: RwLock::new(BTreeMap::new()),
            start_params: RwLock::new(BTreeMap::new()),
            known_domains: RwLock::new(BTreeMap::new()),
            peer_weak: Mutex::new(None),
            run_index: Mutex::new(None),
        }
    }

    fn live_map_handle(
        &self,
        crawl_id: &RunId,
        initial: PartitionMap,
    ) -> Arc<RwLock<Arc<PartitionMap>>> {
        let mut guard = self
            .live_maps
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        Arc::clone(
            guard
                .entry(crawl_id.clone())
                .or_insert_with(|| Arc::new(RwLock::new(Arc::new(initial)))),
        )
    }

    fn update_live_map(&self, crawl_id: &RunId, new_map: PartitionMap) {
        if let Some(handle) = self
            .live_maps
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(crawl_id)
            .cloned()
        {
            let mut slot = handle.write().unwrap_or_else(PoisonError::into_inner);
            if new_map.map_epoch >= slot.map_epoch {
                *slot = Arc::new(new_map);
            }
        }
    }

    fn record_domains(&self, crawl_id: &RunId, domains: impl IntoIterator<Item = String>) {
        let mut guard = self
            .known_domains
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        guard.entry(crawl_id.clone()).or_default().extend(domains);
    }

    fn known_domains_of(&self, crawl_id: &RunId) -> Vec<String> {
        let mut set: BTreeSet<String> = self
            .known_domains
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(crawl_id)
            .cloned()
            .unwrap_or_default();
        if let Some(record) = self.get_known(crawl_id) {
            let folders = self
                .run_folders
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            for slot in &record.map.slots {
                if let Some(folder) = folders.get(&slot.run_id) {
                    set.extend(distinct_domains_in_run_file(&folder.join("run.sqlite")));
                }
            }
        }
        set.into_iter().collect()
    }

    fn peer(&self) -> Option<Arc<dyn GroupPeer>> {
        self.peer_weak
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .and_then(Weak::upgrade)
    }

    fn run_index(&self) -> Option<RunIndex> {
        self.run_index
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn cache_record(&self, record: PartitionedCrawlRecord) {
        self.update_live_map(&record.crawl_id, record.map.clone());
        let mut by_part = self
            .by_partition_run
            .write()
            .unwrap_or_else(PoisonError::into_inner);
        for slot in &record.map.slots {
            by_part.insert(
                slot.run_id.clone(),
                (record.crawl_id.clone(), slot.partition),
            );
        }
        drop(by_part);
        self.known
            .write()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(record.crawl_id.clone(), record);
    }

    fn remember_store_path(&self, context: &CallContext) {
        let mut slot = self
            .store_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            let path = context
                .engine()
                .home()
                .per_major(&context.identity().product_version)
                .join(STORE.file());
            *slot = Some(path);
        }
        drop(slot);
        let mut blocking = self.blocking.lock().unwrap_or_else(PoisonError::into_inner);
        if blocking.is_none() {
            *blocking = Some(context.engine().task_starter().blocking_threads());
        }
        drop(blocking);
        if let Some(peer) = context.engine().group_peer() {
            *self
                .peer_weak
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Arc::downgrade(&peer));
        }
        let mut ri = self
            .run_index
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if ri.is_none() {
            *ri = Some(context.engine().run_index().clone());
        }
    }

    /// Ensures `known` is hydrated from the local `partitioned-crawls.sqlite`
    /// after a Daemon restart so guards see persisted Partitioned crawl IDs
    /// synchronously.
    fn hydrate_if_needed(&self, context: &CallContext) {
        self.remember_store_path(context);
        let mut hydrated = self.hydrated.lock().unwrap_or_else(PoisonError::into_inner);
        if *hydrated {
            return;
        }
        *hydrated = true;
        let path = self
            .store_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(path) = path else {
            return;
        };
        if !path.exists() {
            return;
        }
        if let Ok(conn) = Connection::open_with_flags(&path, OpenFlags::SQLITE_OPEN_READ_ONLY)
            && let Ok(mut stmt) =
                conn.prepare("SELECT crawl_id, map_json, created_at FROM partitioned_crawls")
            && let Ok(rows) = stmt.query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })
        {
            for row in rows.flatten() {
                if let (Ok(crawl_id), Ok(map)) = (
                    RunId::parse(&row.0),
                    serde_json::from_str::<PartitionMap>(&row.1),
                ) {
                    self.cache_record(PartitionedCrawlRecord {
                        crawl_id,
                        map,
                        created_at: row.2,
                    });
                }
            }
        }
    }

    fn get_known(&self, crawl_id: &RunId) -> Option<PartitionedCrawlRecord> {
        self.known
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(crawl_id)
            .cloned()
    }

    fn partition_of(&self, partition_run_id: &RunId) -> Option<(PartitionedCrawlRecord, u32)> {
        let (crawl_id, idx) = self
            .by_partition_run
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .get(partition_run_id)
            .cloned()?;
        self.get_known(&crawl_id).map(|record| (record, idx))
    }

    fn persist_local_sqlite(&self, record: &PartitionedCrawlRecord) {
        let path = self
            .store_path
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let Some(path) = path else {
            return;
        };
        if let Ok(conn) = Connection::open(&path) {
            for stmt in store::STORE_STATEMENTS {
                let _ = conn.execute_batch(stmt);
            }
            let map_json =
                serde_json::to_string(&record.map).expect("invariant: PartitionMap encodes");
            if let Ok(mut stmt) = conn.prepare_cached(
                "INSERT INTO partitioned_crawls (crawl_id, map_json, created_at)
                 VALUES (?1, ?2, ?3)
                 ON CONFLICT (crawl_id) DO UPDATE SET map_json = excluded.map_json",
            ) {
                let _ = stmt.execute((record.crawl_id.as_str(), map_json, record.created_at));
            }
        }
    }
}

/// Live [`PartitionRouter`] sharing an atomically updatable [`PartitionMap`]
/// across `partition_map_epoch` changes (ADR 0054 and ADR 0140, F3).
#[derive(Debug)]
struct LivePartitionRouter {
    local_partition: u32,
    map: Arc<RwLock<Arc<PartitionMap>>>,
    domain_keys: Arc<DomainKeys>,
    outbound: mpsc::UnboundedSender<Vec<ForwardedItem>>,
}

impl LivePartitionRouter {
    fn current_map(&self) -> Arc<PartitionMap> {
        Arc::clone(&self.map.read().unwrap_or_else(PoisonError::into_inner))
    }
}

impl PartitionRouter for LivePartitionRouter {
    fn map_epoch(&self) -> u64 {
        self.current_map().map_epoch
    }

    fn route_url(&self, url: &str) -> Option<(String, u32)> {
        let host = host_of(url);
        if host.is_empty() {
            return None;
        }
        let domain_key = self.domain_keys.key(host);
        let map = self.current_map();
        let owner = map.owner_of(&domain_key);
        if owner.partition == self.local_partition {
            None
        } else {
            Some((domain_key, owner.partition))
        }
    }

    fn forward(&self, items: Vec<ForwardedItem>) {
        if !items.is_empty() {
            let _ = self.outbound.send(items);
        }
    }
}

struct ForwarderTarget {
    crawl_id: RunId,
    local_partition: u32,
    local_run_id: RunId,
    map: Arc<RwLock<Arc<PartitionMap>>>,
    domain_keys: Arc<DomainKeys>,
}

/// Spawns the per-Partition credit-based at-least-once link forwarder task on
/// `context.engine().task_starter()`.
fn spawn_forwarder(
    target: ForwarderTarget,
    runtime: CrawlRuntime,
    peer: Arc<dyn GroupPeer>,
    context: &CallContext,
) -> Arc<LivePartitionRouter> {
    let (tx, mut rx) = mpsc::unbounded_channel::<Vec<ForwardedItem>>();
    let router = Arc::new(LivePartitionRouter {
        local_partition: target.local_partition,
        map: Arc::clone(&target.map),
        domain_keys: target.domain_keys,
        outbound: tx,
    });
    let starter = context.engine().task_starter().clone();
    let clock = context.engine().clock();
    let task_starter = starter.clone();
    starter.spawn(async move {
        let mut windows: BTreeMap<u32, CreditWindow> = BTreeMap::new();
        loop {
            tokio::select! {
                () = task_starter.cancelled() => return,
                batch = rx.recv() => {
                    let Some(items) = batch else {
                        return;
                    };
                    let map = Arc::clone(&target.map.read().unwrap_or_else(PoisonError::into_inner));
                    enqueue_outbound_items(&mut windows, items, &map);
                    if !drain_outbound_windows(
                        &mut windows,
                        (&target.crawl_id, target.local_partition, &target.local_run_id),
                        &target.map,
                        &runtime,
                        &peer,
                        (&clock, &task_starter),
                    )
                    .await
                    {
                        return;
                    }
                }
            }
        }
    });
    router
}

fn enqueue_outbound_items(
    windows: &mut BTreeMap<u32, CreditWindow>,
    items: Vec<ForwardedItem>,
    map: &PartitionMap,
) {
    for item in items {
        let target = map.owner_of(&item.domain_key).partition;
        windows.entry(target).or_default().push(ForwardedLink {
            url: item.url,
            label: item.label,
            payload: item.payload,
            method: item.method,
            body: item.body,
            key: item.key,
            priority: item.priority,
            forefront: item.forefront,
            domain_key: item.domain_key,
        });
    }
}

async fn drain_outbound_windows(
    windows: &mut BTreeMap<u32, CreditWindow>,
    (crawl_id, local_partition, local_run_id): (&RunId, u32, &RunId),
    map_handle: &Arc<RwLock<Arc<PartitionMap>>>,
    runtime: &CrawlRuntime,
    peer: &Arc<dyn GroupPeer>,
    (clock, starter): (
        &goodfetch_engine::clock::Clock,
        &goodfetch_engine::tasks::TaskStarter,
    ),
) -> bool {
    loop {
        let map = Arc::clone(&map_handle.read().unwrap_or_else(PoisonError::into_inner));
        let mut any_sent = false;
        let targets: Vec<u32> = windows.keys().copied().collect();
        for target in targets {
            let Some(slot) = map.slot(target) else {
                // Target Partition was removed from the map: re-classify its
                // queued links under the current map.
                if let Some(mut stale) = windows.remove(&target) {
                    while let Some(batch) = stale.take_batch() {
                        for link in batch {
                            let next_owner = map.owner_of(&link.domain_key).partition;
                            if next_owner != local_partition {
                                windows.entry(next_owner).or_default().push(link);
                            }
                        }
                        stale.acknowledge(64);
                    }
                }
                continue;
            };
            let Some(window) = windows.get_mut(&target) else {
                continue;
            };
            let Some(links) = window.take_batch() else {
                continue;
            };
            any_sent = true;
            let run_epoch = runtime.live_run_epoch(local_run_id).await.unwrap_or(1);
            let req = ForwardLinksRequest {
                partitioned_crawl_id: crawl_id.clone(),
                target_partition: slot.partition,
                target_run_id: slot.run_id.clone(),
                sender_partition: local_partition,
                sender_run_epoch: run_epoch,
                partition_map_epoch: map.map_epoch,
                credits_requested: u32::try_from(links.len()).unwrap_or(16),
                links,
            };
            let payload =
                serde_json::to_value(&req).expect("invariant: ForwardLinksRequest encodes");
            match peer
                .call_member_extension(
                    slot.member.clone(),
                    slot.peer_address.clone(),
                    PEER_FORWARD_LINKS.to_owned(),
                    payload,
                )
                .await
            {
                Ok(raw) => {
                    let granted = serde_json::from_value::<ForwardLinksResponse>(raw)
                        .map_or(16, |resp| resp.credits_granted);
                    window.acknowledge(granted);
                }
                Err(error) => {
                    tracing::debug!(
                        %crawl_id,
                        target,
                        %error,
                        "forwarded link batch failed; requeuing for at-least-once delivery"
                    );
                    window.fail_in_flight();
                    tokio::select! {
                        () = starter.cancelled() => return false,
                        () = clock.sleep(FORWARD_RETRY_DELAY) => {}
                    }
                }
            }
        }
        if !any_sent || !windows.values().any(CreditWindow::has_unacked) {
            return true;
        }
    }
}

/// What Foxhound's Partitioned crawl crate adds to the engine (ADRs 0054,
/// 0087 and 0088).
pub fn contribution(runtime: CrawlRuntime) -> Contribution {
    let state = Arc::new(State::new(runtime));
    let crawl_start_guard = build_crawl_start_guard(Arc::clone(&state));
    let run_ops_guard = build_run_ops_guard(Arc::clone(&state));

    Contribution::new("goodfetch-partitioned-crawl")
        .with_user_store(&STORE)
        .with_run_state_tables(RUN_STATE_TABLES)
        .with_limit_scope_reader(Arc::new(read_partition_limit_scopes))
        .with_peer_extension(PEER_REGISTER, Arc::new(RegisterHandler(Arc::clone(&state))))
        .with_peer_extension(
            PEER_PREPARE_MEMBER,
            Arc::new(PrepareMemberHandler(Arc::clone(&state))),
        )
        .with_peer_extension(
            PEER_FORWARD_LINKS,
            Arc::new(ForwardLinksHandler(Arc::clone(&state))),
        )
        .with_peer_extension(
            PEER_READ_RECORDS,
            Arc::new(ReadRecordsHandler(Arc::clone(&state))),
        )
        .with_peer_extension(
            PEER_PUBLISH_MAP,
            Arc::new(PublishMapHandler(Arc::clone(&state))),
        )
        .with_peer_extension(
            PEER_DRAIN_DOMAINS,
            Arc::new(DrainDomainsHandler(Arc::clone(&state))),
        )
        .with_peer_extension(
            PEER_HANDOFF_DOMAIN,
            Arc::new(HandoffDomainHandler(Arc::clone(&state))),
        )
        .with_peer_extension(
            PEER_ACTIVATE_DOMAIN,
            Arc::new(ActivateDomainHandler(Arc::clone(&state))),
        )
        .with_peer_extension(
            PEER_REMOVE_MEMBER_MAP,
            Arc::new(RemoveMemberMapHandler(Arc::clone(&state))),
        )
        .with_group_membership_hook(Arc::new(PartitionMembershipHook(Arc::clone(&state))))
        .with_run_adopt_hook(Arc::new(PartitionAdoptHook))
        .with_operation_guard(&[CRAWL_START.name], crawl_start_guard)
        .with_operation_guard(PARTITIONED_RUN_OPS, run_ops_guard)
}

fn build_crawl_start_guard(crawl_start_state: Arc<State>) -> Guard {
    Arc::new(move |_op, _params, typed, context| {
        crawl_start_state.hydrate_if_needed(context);
        let params = typed.get::<CrawlStartParams>()?.clone();

        // Case A: Prepared Partition member start triggered by coordinator.
        if let Some(id_str) = params.run_id.as_deref()
            && let Ok(parsed_id) = RunId::parse(id_str)
            && let Some(prepared) = crawl_start_state
                .prepared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&parsed_id)
        {
            let state = Arc::clone(&crawl_start_state);
            let context = context.clone();
            return Some(Box::pin(async move {
                let started =
                    start_prepared_partition(state, prepared, params, context.clone()).await?;
                Answered::encode(&context, &started)
            }));
        }

        // Case B: User requested `partitions > 1` or `"auto"`.
        if params
            .partitions
            .is_some_and(goodfetch_protocol::run::Partitions::is_multi)
        {
            let state = Arc::clone(&crawl_start_state);
            let context = context.clone();
            return Some(Box::pin(async move {
                let started = start_partitioned_crawl(state, params, context.clone()).await?;
                Answered::encode(&context, &started)
            }));
        }

        None
    })
}

fn build_run_ops_guard(run_ops_state: Arc<State>) -> Guard {
    Arc::new(move |op, _params, typed, context| {
        run_ops_state.hydrate_if_needed(context);

        // On `run.resume` for an individual Partition Run, re-attach its
        // PartitionRouter synchronously before letting the normal `run.resume`
        // handler run.
        if op.name == "run.resume"
            && let Some(params) = typed.get::<RunResumeParams>()
            && let Some((record, partition_idx)) = run_ops_state.partition_of(&params.run_id)
            && let Some(peer) = context.engine().group_peer()
        {
            reattach_partition_router(&run_ops_state, params, record, partition_idx, peer, context);
            return None;
        }

        // Check if this call targets a Partitioned crawl ID.
        let crawl_id = extract_run_id_from_typed(op, typed)?;
        let record = run_ops_state.get_known(&crawl_id)?;
        dispatch_partitioned_run_op(
            op,
            typed,
            Arc::clone(&run_ops_state),
            record,
            context.clone(),
        )
    })
}

fn reattach_partition_router(
    state: &State,
    params: &RunResumeParams,
    record: PartitionedCrawlRecord,
    partition_idx: u32,
    peer: Arc<dyn GroupPeer>,
    context: &CallContext,
) {
    let run_id = params.run_id.clone();
    let domain_keys = state
        .domain_keys
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&run_id)
        .cloned()
        .unwrap_or_else(|| Arc::new(domain_keys_from_map(&params.settings)));
    let map_handle = state.live_map_handle(&record.crawl_id, record.map);
    let router = spawn_forwarder(
        ForwarderTarget {
            crawl_id: record.crawl_id,
            local_partition: partition_idx,
            local_run_id: run_id.clone(),
            map: map_handle,
            domain_keys,
        },
        state.runtime.clone(),
        peer,
        context,
    );
    state.runtime.register_router(run_id.clone(), router);
    let fallback_folder = context
        .engine()
        .home()
        .per_major(&context.identity().product_version)
        .join("runs")
        .join(run_id.as_str());
    state
        .run_folders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .entry(run_id)
        .or_insert(fallback_folder);
}

fn dispatch_partitioned_run_op(
    op: &'static Operation,
    typed: goodfetch_engine::ReadParams<'_>,
    state: Arc<State>,
    record: PartitionedCrawlRecord,
    context: CallContext,
) -> Option<goodfetch_engine::GuardAnswer> {
    match op.name {
        name if name == RUN_REPORT.name => Some(Box::pin(async move {
            let report = handle_report(&state, &record, &context).await?;
            Answered::encode(&context, &report)
        })),
        name if name == RUN_USAGE.name => Some(Box::pin(async move {
            let usage = handle_usage(&record, &context).await?;
            Answered::encode(&context, &usage)
        })),
        name if name == RUN_PROGRESS_READ.name => Some(Box::pin(async move {
            let progress = handle_progress(&record, &context).await?;
            Answered::encode(&context, &progress)
        })),
        name if name == RUN_STOP.name => {
            let params = typed.get::<RunStopParams>()?.clone();
            Some(Box::pin(async move {
                let report = handle_stop(&record, params, &context).await?;
                Answered::encode(&context, &report)
            }))
        }
        name if name == RUN_PAUSE.name => Some(Box::pin(async move {
            let report = handle_fanout_report(&record, RUN_PAUSE.name, &context).await?;
            Answered::encode(&context, &report)
        })),
        name if name == RUN_ABORT.name => {
            let params = typed.get::<RunAbortParams>()?.clone();
            Some(Box::pin(async move {
                let report = handle_abort(&record, params, &context).await?;
                Answered::encode(&context, &report)
            }))
        }
        name if name == RUN_CHECKPOINT.name => {
            let params = typed.get::<RunCheckpointParams>()?.clone();
            Some(Box::pin(async move {
                let checkpoint = handle_checkpoint(&record, params, &context).await?;
                Answered::encode(&context, &checkpoint)
            }))
        }
        "run.resume" => {
            let params = typed.get::<RunResumeParams>()?.clone();
            Some(Box::pin(async move {
                let started = handle_resume(&record, params, &context).await?;
                Answered::encode(&context, &started)
            }))
        }
        name if name == RUN_RECORDS.name => {
            let params = typed.get::<RunRecordsParams>()?.clone();
            Some(Box::pin(async move {
                let read = handle_records(&record, params, &context).await?;
                Answered::encode(&context, &read)
            }))
        }
        name if name == DATASET_PAGE.name => {
            let params = typed.get::<DatasetPageParams>()?.clone();
            Some(Box::pin(async move {
                let page = handle_dataset_page(&record, params, &context).await?;
                Answered::encode(&context, &page)
            }))
        }
        name if name == DATASET_LIST.name => Some(Box::pin(async move {
            let list = handle_dataset_list(&record, &context).await?;
            Answered::encode(&context, &list)
        })),
        _ => None,
    }
}

fn extract_run_id_from_typed(
    op: &'static Operation,
    typed: goodfetch_engine::ReadParams<'_>,
) -> Option<RunId> {
    match op.name {
        name if name == RUN_REPORT.name || name == RUN_USAGE.name || name == RUN_PAUSE.name => {
            typed.get::<RunParams>().map(|p| p.run_id.clone())
        }
        name if name == RUN_PROGRESS_READ.name => {
            typed.get::<RunProgressParams>().map(|p| p.run_id.clone())
        }
        name if name == RUN_STOP.name => typed.get::<RunStopParams>().map(|p| p.run_id.clone()),
        name if name == RUN_ABORT.name => typed.get::<RunAbortParams>().map(|p| p.run_id.clone()),
        name if name == RUN_CHECKPOINT.name => {
            typed.get::<RunCheckpointParams>().map(|p| p.run_id.clone())
        }
        "run.resume" => typed.get::<RunResumeParams>().map(|p| p.run_id.clone()),
        name if name == RUN_RECORDS.name => {
            typed.get::<RunRecordsParams>().map(|p| p.run_id.clone())
        }
        name if name == DATASET_PAGE.name => {
            typed.get::<DatasetPageParams>().map(|p| p.run_id.clone())
        }
        name if name == DATASET_LIST.name => {
            typed.get::<DatasetListParams>().map(|p| p.run_id.clone())
        }
        _ => None,
    }
}

struct RegisterHandler(Arc<State>);

impl PeerExtensionHandler for RegisterHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let record: PartitionedCrawlRecord =
                serde_json::from_value(payload).map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InvalidParams,
                        format!("invalid `partition.register` payload: {error}"),
                    )
                })?;
            state.persist_local_sqlite(&record);
            state.cache_record(record);
            Ok(json!({ "ok": true }))
        })
    }
}

struct PrepareMemberHandler(Arc<State>);

impl PeerExtensionHandler for PrepareMemberHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let prepared: PreparedPartition = serde_json::from_value(payload).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `partition.prepare_member` payload: {error}"),
                )
            })?;
            state
                .prepared
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(prepared.run_id.clone(), prepared);
            Ok(json!({ "ok": true }))
        })
    }
}

struct ForwardLinksHandler(Arc<State>);

impl PeerExtensionHandler for ForwardLinksHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let req: ForwardLinksRequest = serde_json::from_value(payload).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `partition.forward_links` payload: {error}"),
                )
            })?;
            if req.sender_run_epoch == 0 {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    "forwarded link batch must carry a positive `sender_run_epoch`",
                ));
            }
            let current_record = state.get_known(&req.partitioned_crawl_id);
            let current_map_epoch = current_record
                .as_ref()
                .map_or(req.partition_map_epoch, |r| r.map.map_epoch);

            let (local_links, reroute) = if let Some(record) = &current_record {
                classify_incoming(&record.map, req.target_partition, req.links)
            } else {
                (req.links, BTreeMap::new())
            };

            let seeds: Vec<Seed> = local_links
                .into_iter()
                .map(ForwardedLink::into_seed)
                .collect();
            let accepted = if seeds.is_empty() {
                0
            } else {
                state
                    .runtime
                    .admit_forwarded(&req.target_run_id, seeds)
                    .await?
            };
            // Re-route links whose domain key this Partition no longer owns
            // under the current `PartitionMap` (ADR 0054).
            let mut rerouted = 0_usize;
            if let Some(record) = &current_record
                && let Some(peer) = state.peer()
            {
                let from_member = peer.member_name().to_owned();
                for (next_partition, links) in reroute {
                    let Some(target_slot) = record.map.slot(next_partition) else {
                        continue;
                    };
                    for link in &links {
                        trace_reroute_forward(
                            &link.domain_key,
                            &from_member,
                            &target_slot.member,
                            req.partition_map_epoch,
                            record.map.map_epoch,
                        );
                    }
                    let count = links.len();
                    let reroute_req = ForwardLinksRequest {
                        partitioned_crawl_id: req.partitioned_crawl_id.clone(),
                        target_partition: target_slot.partition,
                        target_run_id: target_slot.run_id.clone(),
                        sender_partition: req.target_partition,
                        sender_run_epoch: req.sender_run_epoch,
                        partition_map_epoch: record.map.map_epoch,
                        credits_requested: u32::try_from(count).unwrap_or(16),
                        links,
                    };
                    let payload = serde_json::to_value(&reroute_req)
                        .expect("invariant: ForwardLinksRequest encodes");
                    peer.call_member_extension(
                        target_slot.member.clone(),
                        target_slot.peer_address.clone(),
                        PEER_FORWARD_LINKS.to_owned(),
                        payload,
                    )
                    .await?;
                    rerouted += count;
                }
            } else {
                rerouted = reroute.values().map(Vec::len).sum();
            }
            let resp = ForwardLinksResponse {
                accepted,
                rerouted,
                credits_granted: req.credits_requested.max(16),
                current_map_epoch,
            };
            Ok(serde_json::to_value(resp).expect("invariant: ForwardLinksResponse encodes"))
        })
    }
}

struct PublishMapHandler(Arc<State>);

impl PeerExtensionHandler for PublishMapHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let req: PublishMapRequest = serde_json::from_value(payload).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `partition.publish_map` payload: {error}"),
                )
            })?;
            let created_at = state
                .get_known(&req.crawl_id)
                .map_or(0, |existing| existing.created_at);
            let record = PartitionedCrawlRecord {
                crawl_id: req.crawl_id.clone(),
                map: req.map.clone(),
                created_at,
            };
            state.persist_local_sqlite(&record);
            state.cache_record(record);
            if !req.hold_incoming_domains.is_empty()
                && let Some(target_run_id) = req.target_run_id
                && state.runtime.is_live(&target_run_id).await
            {
                let _ = state
                    .runtime
                    .hold_incoming_domains(&target_run_id, req.hold_incoming_domains)
                    .await;
            }
            Ok(json!({ "ok": true, "map_epoch": req.map.map_epoch }))
        })
    }
}

struct DrainDomainsHandler(Arc<State>);

impl PeerExtensionHandler for DrainDomainsHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let req: DrainDomainsRequest = serde_json::from_value(payload).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `partition.drain_domains` payload: {error}"),
                )
            })?;
            let Some(peer) = state.peer() else {
                return Err(CallError::protocol(
                    ProtocolFailure::InternalError,
                    "drain_domains invoked without an active GroupPeer",
                ));
            };
            let handed_off = execute_domain_handoffs(
                &state,
                &peer,
                state.run_index().as_ref(),
                &req.crawl_id,
                &req.source_run_id,
                req.source_partition,
                req.map_epoch,
                req.moves,
            )
            .await?;
            Ok(serde_json::to_value(DrainDomainsResponse { handed_off })
                .expect("invariant: DrainDomainsResponse encodes"))
        })
    }
}

struct HandoffDomainHandler(Arc<State>);

impl PeerExtensionHandler for HandoffDomainHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let req: HandoffDomainRequest = serde_json::from_value(payload).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `partition.handoff_domain` payload: {error}"),
                )
            })?;
            // Install Wall allowance refetch stops in the receiving member's
            // Run index (ADR 0052 and 0054).
            if let Some(run_index) = state.run_index() {
                for stop in req.refetch_stops {
                    let _ = run_index
                        .record_transferred_refetch_stop(stop.into_index(req.to_run_id.clone()))
                        .await;
                }
            }
            let domain_key = req.snapshot.domain_key.clone();
            state.record_domains(&req.crawl_id, [domain_key.clone()]);
            let installed = if state.runtime.is_live(&req.to_run_id).await {
                state
                    .runtime
                    .install_domain_handoff(&req.to_run_id, req.snapshot)
                    .await?
            } else {
                0
            };
            if !req.adopted {
                trace_ack_handoff(&domain_key, &req.from_member, &req.to_member, req.map_epoch);
            }
            Ok(serde_json::to_value(HandoffDomainResponse {
                acked: true,
                map_epoch: req.map_epoch,
                installed,
            })
            .expect("invariant: HandoffDomainResponse encodes"))
        })
    }
}

struct ActivateDomainHandler(Arc<State>);

impl PeerExtensionHandler for ActivateDomainHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let req: ActivateDomainRequest = serde_json::from_value(payload).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `partition.activate_domain` payload: {error}"),
                )
            })?;
            if state.runtime.is_live(&req.target_run_id).await {
                state
                    .runtime
                    .unfence_domains(&req.target_run_id, vec![req.domain_key.clone()])
                    .await?;
            }
            trace_admit_domain(&req.domain_key, &req.member, req.map_epoch);
            Ok(json!({ "ok": true }))
        })
    }
}

struct RemoveMemberMapHandler(Arc<State>);

impl PeerExtensionHandler for RemoveMemberMapHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let req: RemoveMemberMapRequest = serde_json::from_value(payload).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `partition.remove_member_map` payload: {error}"),
                )
            })?;
            let Some(mut record) = state.get_known(&req.crawl_id) else {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("unknown Partitioned crawl `{}`", req.crawl_id),
                ));
            };
            if let Some(new_map) = map_without_member(&record.map, &req.member) {
                trace_publish_map(new_map.map_epoch);
                record.map = new_map.clone();
                state.persist_local_sqlite(&record);
                state.cache_record(record.clone());
                if let Some(peer) = state.peer() {
                    broadcast_publish_map(&peer, &req.crawl_id, &new_map, &BTreeMap::new()).await;
                }
            }
            Ok(
                serde_json::to_value(RemoveMemberMapResponse { map: record.map })
                    .expect("invariant: RemoveMemberMapResponse encodes"),
            )
        })
    }
}

/// Executes fenced domain handoffs from `source_run_id` to their new owners:
/// 1. Fences moving domains on `source_run_id`, waits for in-flight requests,
///    seals the current Run-state batch, and exports pending URLs, seen
///    fingerprints, and attached per-domain state.
/// 2. Reads Wall allowance refetch stops for each moving domain from `run_index`.
/// 3. Sends `partition.handoff_domain` to each new owner and waits for ack.
/// 4. Removes transferred refetch stops from old owner's `run_index` and sends
///    `partition.activate_domain` so the new owner starts admitting ONLY after
///    acknowledgement (ADR 0054).
#[expect(
    clippy::too_many_arguments,
    reason = "handoff coordinator helper passing peer, run_index, crawl/run ids, epoch and domain moves"
)]
async fn execute_domain_handoffs(
    state: &State,
    peer: &Arc<dyn GroupPeer>,
    run_index: Option<&RunIndex>,
    crawl_id: &RunId,
    source_run_id: &RunId,
    source_partition: u32,
    map_epoch: u64,
    moves: Vec<crate::handoff::DomainMove>,
) -> Result<Vec<String>, CallError> {
    if moves.is_empty() {
        return Ok(Vec::new());
    }
    if !state.runtime.is_live(source_run_id).await {
        return Ok(activate_ended_source_moves(peer, crawl_id, map_epoch, moves).await);
    }
    let from_member = peer.member_name().to_owned();
    let domains: Vec<String> = moves.iter().map(|m| m.domain_key.clone()).collect();
    for mv in &moves {
        trace_fence_domain(&mv.domain_key, &from_member, &mv.to_member, map_epoch);
    }
    let run_epoch = state
        .runtime
        .live_run_epoch(source_run_id)
        .await
        .unwrap_or(1);
    let snapshots = state
        .runtime
        .fence_and_drain_domains(source_run_id, domains)
        .await?;
    let mut by_domain: BTreeMap<String, _> = snapshots
        .into_iter()
        .map(|s| (s.domain_key.clone(), s))
        .collect();
    let mut handed_off = Vec::new();
    for mv in moves {
        trace_drain_inflight(&mv.domain_key, &from_member, map_epoch);
        let Some(snapshot) = by_domain.remove(&mv.domain_key) else {
            continue;
        };
        let refetch_stops = if let Some(ri) = run_index {
            ri.refetch_stops_for_run_domain(source_run_id, &mv.domain_key)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(TransferredRefetchStop::from_index)
                .collect()
        } else {
            Vec::new()
        };
        trace_send_handoff(&mv.domain_key, &from_member, &mv.to_member, map_epoch);
        let req = HandoffDomainRequest {
            crawl_id: crawl_id.clone(),
            from_partition: source_partition,
            from_member: from_member.clone(),
            to_partition: mv.to_partition,
            to_run_id: mv.to_run_id.clone(),
            to_member: mv.to_member.clone(),
            map_epoch,
            sender_run_epoch: run_epoch,
            snapshot,
            refetch_stops,
            adopted: false,
        };
        let raw = peer
            .call_member_extension(
                mv.to_member.clone(),
                mv.to_peer_address.clone(),
                PEER_HANDOFF_DOMAIN.to_owned(),
                serde_json::to_value(&req).expect("invariant: HandoffDomainRequest encodes"),
            )
            .await?;
        let ack: HandoffDomainResponse = serde_json::from_value(raw).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding handoff ack failed: {error}"),
            )
        })?;
        if ack.acked {
            if let Some(ri) = run_index {
                let _ = ri.remove_refetch_stop(source_run_id, &mv.domain_key).await;
            }
            let activate = ActivateDomainRequest {
                crawl_id: crawl_id.clone(),
                target_run_id: mv.to_run_id.clone(),
                member: mv.to_member.clone(),
                domain_key: mv.domain_key.clone(),
                map_epoch,
            };
            let _ = peer
                .call_member_extension(
                    mv.to_member.clone(),
                    mv.to_peer_address.clone(),
                    PEER_ACTIVATE_DOMAIN.to_owned(),
                    serde_json::to_value(&activate)
                        .expect("invariant: ActivateDomainRequest encodes"),
                )
                .await?;
            handed_off.push(mv.domain_key);
        }
    }
    Ok(handed_off)
}

async fn activate_ended_source_moves(
    peer: &Arc<dyn GroupPeer>,
    crawl_id: &RunId,
    map_epoch: u64,
    moves: Vec<crate::handoff::DomainMove>,
) -> Vec<String> {
    let mut activated = Vec::with_capacity(moves.len());
    for mv in moves {
        let activate = ActivateDomainRequest {
            crawl_id: crawl_id.clone(),
            target_run_id: mv.to_run_id.clone(),
            member: mv.to_member.clone(),
            domain_key: mv.domain_key.clone(),
            map_epoch,
        };
        let _ = peer
            .call_member_extension(
                mv.to_member.clone(),
                mv.to_peer_address.clone(),
                PEER_ACTIVATE_DOMAIN.to_owned(),
                serde_json::to_value(&activate).expect("invariant: ActivateDomainRequest encodes"),
            )
            .await;
        activated.push(mv.domain_key);
    }
    activated
}

async fn broadcast_publish_map(
    peer: &Arc<dyn GroupPeer>,
    crawl_id: &RunId,
    new_map: &PartitionMap,
    hold_by_member: &BTreeMap<String, Vec<String>>,
) {
    for slot in &new_map.slots {
        let req = PublishMapRequest {
            crawl_id: crawl_id.clone(),
            map: new_map.clone(),
            hold_incoming_domains: hold_by_member
                .get(&slot.member)
                .cloned()
                .unwrap_or_default(),
            target_run_id: Some(slot.run_id.clone()),
        };
        let _ = peer
            .call_member_extension(
                slot.member.clone(),
                slot.peer_address.clone(),
                PEER_PUBLISH_MAP.to_owned(),
                serde_json::to_value(&req).expect("invariant: PublishMapRequest encodes"),
            )
            .await;
    }
}

/// [`GroupMembershipHook`] handling Group member additions, removals, and
/// removal drains for live Partitioned crawls (ADRs 0054 and 0055).
struct PartitionMembershipHook(Arc<State>);

async fn rebalance_crawl_for_added_member(
    state: &State,
    mut record: PartitionedCrawlRecord,
    member: &str,
    peer_address: &str,
    ctx: &GroupMembershipContext,
) {
    let Some(params) = state
        .start_params
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&record.crawl_id)
        .cloned()
    else {
        return;
    };
    if !any_partition_running(&ctx.peer, &record.map).await {
        return;
    }
    let old_map = record.map.clone();
    let Some((new_map, new_slot)) = map_with_added_member(
        &old_map,
        &record.crawl_id,
        member.to_owned(),
        peer_address.to_owned(),
    ) else {
        return;
    };
    let domains = state.known_domains_of(&record.crawl_id);
    let moves = compute_moves(&old_map, &new_map, &domains);
    let held_for_new: Vec<String> = moves.iter().map(|m| m.domain_key.clone()).collect();

    if !prepare_and_start_added_partition(
        &record.crawl_id,
        &new_map,
        &new_slot,
        &held_for_new,
        &params,
        ctx,
    )
    .await
    {
        return;
    }

    trace_publish_map(new_map.map_epoch);
    record.map = new_map.clone();
    state.persist_local_sqlite(&record);
    state.cache_record(record.clone());

    let mut hold_by_member = BTreeMap::new();
    hold_by_member.insert(new_slot.member.clone(), held_for_new.clone());
    broadcast_publish_map(&ctx.peer, &record.crawl_id, &new_map, &hold_by_member).await;
    drain_and_activate_added_domains(
        &ctx.peer,
        &record.crawl_id,
        &old_map,
        &new_map,
        &new_slot,
        moves,
        held_for_new,
    )
    .await;
}

async fn any_partition_running(peer: &Arc<dyn GroupPeer>, map: &PartitionMap) -> bool {
    for slot in &map.slots {
        if let Ok(val) = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                RUN_PROGRESS_READ.name.to_owned(),
                json!({ "run_id": slot.run_id }),
            )
            .await
            && let Ok(progress) = serde_json::from_value::<RunProgress>(val)
            && progress.lifecycle == Lifecycle::Running
        {
            return true;
        }
    }
    false
}

async fn prepare_and_start_added_partition(
    crawl_id: &RunId,
    new_map: &PartitionMap,
    new_slot: &PartitionSlot,
    held_for_new: &[String],
    params: &CrawlStartParams,
    ctx: &GroupMembershipContext,
) -> bool {
    let prepared = PreparedPartition {
        crawl_id: crawl_id.clone(),
        partition: new_slot.partition,
        run_id: new_slot.run_id.clone(),
        map: new_map.clone(),
        initial_seeds: Vec::new(),
        initial_held_domains: held_for_new.to_vec(),
    };
    let prepared_payload =
        serde_json::to_value(&prepared).expect("invariant: PreparedPartition encodes");
    let mut prepared_ok = false;
    for attempt in 0..30_u32 {
        if ctx
            .peer
            .call_member_extension(
                new_slot.member.clone(),
                new_slot.peer_address.clone(),
                PEER_PREPARE_MEMBER.to_owned(),
                prepared_payload.clone(),
            )
            .await
            .is_ok()
        {
            prepared_ok = true;
            break;
        }
        if attempt + 1 < 30 {
            ctx.clock.sleep(Duration::from_millis(100)).await;
        }
    }
    if !prepared_ok {
        return false;
    }
    let mut member_params = params.clone();
    member_params.run_id = Some(new_slot.run_id.as_str().to_owned());
    member_params.partitions = None;
    member_params.group_member = None;
    member_params.limits.clear();
    if params.extraction.is_some() || params.declaration.is_some() {
        member_params.detached = true;
    }
    ctx.peer
        .call_member_rpc(
            new_slot.member.clone(),
            new_slot.peer_address.clone(),
            CRAWL_START.name.to_owned(),
            serde_json::to_value(&member_params).expect("invariant: CrawlStartParams encodes"),
        )
        .await
        .is_ok()
}

async fn drain_and_activate_added_domains(
    peer: &Arc<dyn GroupPeer>,
    crawl_id: &RunId,
    old_map: &PartitionMap,
    new_map: &PartitionMap,
    new_slot: &PartitionSlot,
    moves: Vec<crate::handoff::DomainMove>,
    held_for_new: Vec<String>,
) {
    let mut moves_by_old: BTreeMap<u32, Vec<crate::handoff::DomainMove>> = BTreeMap::new();
    for mv in moves {
        moves_by_old.entry(mv.from_partition).or_default().push(mv);
    }
    let mut activated = BTreeSet::new();
    for (old_part, part_moves) in moves_by_old {
        let Some(old_slot) = old_map.slot(old_part) else {
            continue;
        };
        let drain_req = DrainDomainsRequest {
            crawl_id: crawl_id.clone(),
            source_run_id: old_slot.run_id.clone(),
            source_partition: old_slot.partition,
            map_epoch: new_map.map_epoch,
            moves: part_moves,
        };
        if let Ok(raw) = peer
            .call_member_extension(
                old_slot.member.clone(),
                old_slot.peer_address.clone(),
                PEER_DRAIN_DOMAINS.to_owned(),
                serde_json::to_value(&drain_req).expect("invariant: DrainDomainsRequest encodes"),
            )
            .await
            && let Ok(resp) = serde_json::from_value::<DrainDomainsResponse>(raw)
        {
            activated.extend(resp.handed_off);
        }
    }
    for domain_key in held_for_new {
        if !activated.contains(&domain_key) {
            let activate = ActivateDomainRequest {
                crawl_id: crawl_id.clone(),
                target_run_id: new_slot.run_id.clone(),
                member: new_slot.member.clone(),
                domain_key,
                map_epoch: new_map.map_epoch,
            };
            let _ = peer
                .call_member_extension(
                    new_slot.member.clone(),
                    new_slot.peer_address.clone(),
                    PEER_ACTIVATE_DOMAIN.to_owned(),
                    serde_json::to_value(&activate)
                        .expect("invariant: ActivateDomainRequest encodes"),
                )
                .await;
        }
    }
}

impl GroupMembershipHook for PartitionMembershipHook {
    fn on_member_added(
        &self,
        member: String,
        peer_address: String,
        ctx: GroupMembershipContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let crawls: Vec<PartitionedCrawlRecord> = state
                .known
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .values()
                .cloned()
                .collect();
            for record in crawls {
                rebalance_crawl_for_added_member(&state, record, &member, &peer_address, &ctx)
                    .await;
            }
        })
    }

    fn on_member_removed(
        &self,
        member: String,
        ctx: GroupMembershipContext,
    ) -> Pin<Box<dyn Future<Output = ()> + Send>> {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let crawls: Vec<PartitionedCrawlRecord> = state
                .known
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .values()
                .cloned()
                .collect();
            for mut record in crawls {
                if let Some(new_map) = map_without_member(&record.map, &member) {
                    trace_publish_map(new_map.map_epoch);
                    record.map = new_map.clone();
                    state.persist_local_sqlite(&record);
                    state.cache_record(record.clone());
                    broadcast_publish_map(&ctx.peer, &record.crawl_id, &new_map, &BTreeMap::new())
                        .await;
                }
            }
        })
    }

    fn on_removal_drain(
        &self,
        member: String,
        deadline: tokio::time::Instant,
        ctx: GroupMembershipContext,
    ) -> Pin<Box<dyn Future<Output = Vec<RunId>> + Send>> {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let local_partitions: Vec<(RunId, RunId, u32)> = state
                .by_partition_run
                .read()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .filter_map(|(part_run_id, (crawl_id, part_idx))| {
                    let record = state.get_known(crawl_id)?;
                    let slot = record.map.slot(*part_idx)?;
                    (slot.member == member)
                        .then(|| (crawl_id.clone(), part_run_id.clone(), *part_idx))
                })
                .collect();

            let mut handled_runs = Vec::with_capacity(local_partitions.len());
            for (crawl_id, part_run_id, part_idx) in local_partitions {
                handled_runs.push(part_run_id.clone());
                let Some(record) = state.get_known(&crawl_id) else {
                    continue;
                };
                let old_map = record.map.clone();
                // Ask Lead Daemon to publish a new map without `member`.
                let Ok(raw) = ctx
                    .peer
                    .call_lead_extension(
                        PEER_REMOVE_MEMBER_MAP.to_owned(),
                        serde_json::to_value(RemoveMemberMapRequest {
                            crawl_id: crawl_id.clone(),
                            member: member.clone(),
                        })
                        .expect("invariant: RemoveMemberMapRequest encodes"),
                    )
                    .await
                else {
                    let _ = state.runtime.flush_run_state(&part_run_id).await;
                    continue;
                };
                let Ok(resp) = serde_json::from_value::<RemoveMemberMapResponse>(raw) else {
                    let _ = state.runtime.flush_run_state(&part_run_id).await;
                    continue;
                };
                let new_map = resp.map;
                let mut domains: BTreeSet<String> =
                    state.known_domains_of(&crawl_id).into_iter().collect();
                if let Some(folder) = state.runtime.live_folder(&part_run_id).await {
                    domains.extend(distinct_domains_in_run_file(&folder.join("run.sqlite")));
                }
                let domain_list: Vec<String> = domains.into_iter().collect();
                let moves: Vec<_> = compute_moves(&old_map, &new_map, &domain_list)
                    .into_iter()
                    .filter(|mv| mv.from_member == member)
                    .collect();

                let now = ctx.clock.now();
                if now >= deadline {
                    let _ = state.runtime.flush_run_state(&part_run_id).await;
                    continue;
                }
                let remaining = deadline.saturating_duration_since(now);
                let handoff_fut = execute_domain_handoffs(
                    &state,
                    &ctx.peer,
                    Some(&ctx.run_index),
                    &crawl_id,
                    &part_run_id,
                    part_idx,
                    new_map.map_epoch,
                    moves,
                );
                let timed = tokio::select! {
                    res = handoff_fut => Some(res),
                    () = ctx.clock.sleep(remaining) => None,
                };
                if !matches!(timed, Some(Ok(_))) {
                    // Grace period expired before every domain finished:
                    // seal Run state so `runs adopt` completes the rest from
                    // the kept volume (ADR 0054).
                    let _ = state.runtime.flush_run_state(&part_run_id).await;
                }
            }
            handled_runs
        })
    }
}

struct ReadRecordsHandler(Arc<State>);

impl PeerExtensionHandler for ReadRecordsHandler {
    fn handle(
        &self,
        _sender_member: String,
        _lead_epoch: LeadEpoch,
        payload: Value,
    ) -> PeerExtensionAnswer {
        let state = Arc::clone(&self.0);
        Box::pin(async move {
            let req: ReadRecordsRequest = serde_json::from_value(payload).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!("invalid `partition.read_records` payload: {error}"),
                )
            })?;
            let folder = state
                .run_folders
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(&req.run_id)
                .cloned();
            let Some(folder) = folder else {
                return Ok(serde_json::to_value(ReadRecordsResponse {
                    records: Vec::new(),
                })
                .expect("invariant: ReadRecordsResponse encodes"));
            };
            let db_path = folder.join("run.sqlite");
            let read_work = move || -> Result<Vec<DatasetRecord>, String> {
                let conn = Connection::open_with_flags(&db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
                    .map_err(|e| e.to_string())?;
                let mut all = Vec::new();
                let mut cursor = req.after;
                loop {
                    let page = goodfetch_dataset::read(
                        &conn,
                        Query {
                            dataset: &req.dataset,
                            after: cursor,
                            through: req.through,
                            wall: req.wall,
                        },
                        512,
                    )
                    .map_err(|e| e.to_string())?;
                    let page_len = page.len();
                    if let Some(last) = page.last() {
                        cursor = last.number;
                    }
                    all.extend(page);
                    if page_len < 512 {
                        break;
                    }
                }
                Ok(all)
            };
            let blocking = state
                .blocking
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            let result = match blocking {
                Some(threads) => threads.run(read_work).await,
                None => read_work(),
            };
            let records = result.map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InternalError,
                    format!("reading Partition records failed: {error}"),
                )
            })?;
            Ok(serde_json::to_value(ReadRecordsResponse { records })
                .expect("invariant: ReadRecordsResponse encodes"))
        })
    }
}

/// Starts one prepared Partition Run on the local Group member.
async fn start_prepared_partition(
    state: Arc<State>,
    prepared: PreparedPartition,
    params: CrawlStartParams,
    context: CallContext,
) -> Result<RunStarted, CallError> {
    state.remember_store_path(&context);
    let resolved_settings = open_settings_for_check(&params, &context).await?;
    let domain_keys = Arc::new(DomainKeys::of(resolved_settings.resolved()));
    state
        .domain_keys
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(prepared.run_id.clone(), Arc::clone(&domain_keys));

    // Remember a fallback folder immediately, then overwrite with the exact
    // run folder from `RunIdentity::folder()` once started.
    let fallback_folder = context
        .engine()
        .home()
        .per_major(&context.identity().product_version)
        .join("runs")
        .join(prepared.run_id.as_str());
    state
        .run_folders
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(prepared.run_id.clone(), fallback_folder);

    let peer = context.engine().group_peer().ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            "Partition member start invoked on an engine outside a Daemon group",
        )
    })?;

    let map_handle = state.live_map_handle(&prepared.crawl_id, prepared.map.clone());
    let router = spawn_forwarder(
        ForwarderTarget {
            crawl_id: prepared.crawl_id.clone(),
            local_partition: prepared.partition,
            local_run_id: prepared.run_id.clone(),
            map: map_handle,
            domain_keys,
        },
        state.runtime.clone(),
        peer,
        &context,
    );

    let crawl_id_for_row = prepared.crawl_id.clone();
    let partition_for_row = prepared.partition;
    let map_for_row = prepared.map.clone();
    let part_run_id = prepared.run_id.clone();

    let options = PartitionStartOptions {
        extra_limit_scopes: vec![LimitScope {
            kind: PARTITIONED_CRAWL.to_owned(),
            id: prepared.crawl_id.as_str().to_owned(),
        }],
        router,
        membership_writer: Some(Box::new(move |connection| {
            write_partition_membership(
                connection,
                &crawl_id_for_row,
                partition_for_row,
                &map_for_row,
            )
        })),
        allow_empty_seeds: true,
        initial_held_domains: prepared.initial_held_domains,
    };

    let started = state
        .runtime
        .start_partition(params, prepared.initial_seeds, options, context)
        .await?;
    if let Some(actual_folder) = state.runtime.live_folder(&part_run_id).await {
        state
            .run_folders
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(part_run_id, actual_folder);
    }
    Ok(started)
}

/// Orchestrates a multi-Partition `crawl.start` across distinct Group members
/// (ADRs 0054, 0056 and 0095).
async fn start_partitioned_crawl(
    state: Arc<State>,
    params: CrawlStartParams,
    context: CallContext,
) -> Result<RunStarted, CallError> {
    state.remember_store_path(&context);
    let requested_run_id = run_id(params.run_id.as_deref())?;
    let peer = context.engine().group_peer();

    let resolved = open_settings_for_check(&params, &context).await?;
    let session_source: Option<SessionSource> = resolved
        .resolved()
        .effective(&SESSION_SOURCE)
        .and_then(|entry| serde_json::from_value::<SessionSource>(entry.value.clone()).ok());
    let has_proxy = resolved
        .resolved()
        .effective(&PROXY_SERVER)
        .is_some_and(|entry| !entry.value.is_null())
        || resolved
            .resolved()
            .effective(&PROXY_PROVIDER)
            .is_some_and(|entry| !entry.value.is_null());

    let check = PartitionedStartCheck {
        partitions: params.partitions,
        session_source: session_source.clone(),
        client_browser: false,
        machine_bound_import: false,
        has_proxy,
        browser_host_served: false,
    };

    // Fast refusal for `partitions_need_group` before touching placement.
    let Some(peer) = peer else {
        let _ = validate_and_select(&check, false, Vec::new(), requested_run_id.as_ref())?;
        unreachable!("invariant: multi-partition outside a group is refused");
    };

    // Fast ADR 0056 refusal (`session_bound_to_machine` / `browser_borrowed`).
    check_session_source_for_partitions(&check, requested_run_id.as_ref())?;

    let placement_req = PlacementRequest {
        receiving_member: peer.member_name().to_owned(),
        session_source,
        ..PlacementRequest::default()
    };
    let eligible = peer.eligible_members(placement_req).await?;
    let ValidatedPartitions::Partitioned(chosen_members) =
        validate_and_select(&check, true, eligible, requested_run_id.as_ref())?
    else {
        unreachable!("invariant: multi-partition check only called when is_multi()");
    };

    let now = context.engine().clock().wall();
    let crawl_id = requested_run_id.unwrap_or_else(|| generate_crawl_id(now));

    // Register crawl-level Limits in the Usage ledger under the
    // `partitioned_crawl` Limit scope (ADR 0041, 0054, 0095).
    if !params.limits.is_empty() {
        give_scoped_limits(
            LimitScope {
                kind: PARTITIONED_CRAWL.to_owned(),
                id: crawl_id.as_str().to_owned(),
            },
            params.limits.clone(),
            &context,
        )
        .await?;
    }

    let mut slots = Vec::with_capacity(chosen_members.len());
    for (idx, member) in chosen_members.into_iter().enumerate() {
        let part_idx = u32::try_from(idx).expect("partition index fits u32");
        let part_run_id = RunId::parse(&format!("{}-p{part_idx}", crawl_id.as_str()))
            .unwrap_or_else(|_| generate_crawl_id(now));
        slots.push(PartitionSlot {
            partition: part_idx,
            member: member.member,
            run_id: part_run_id,
            peer_address: member.peer_address,
        });
    }

    let map = PartitionMap::new(1, slots.clone());
    let record = PartitionedCrawlRecord {
        crawl_id: crawl_id.clone(),
        map: map.clone(),
        created_at: epoch_millis(now),
    };
    register_partitioned_record(&state, &peer, &record, &context).await?;
    state
        .start_params
        .write()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(crawl_id.clone(), params.clone());
    let domain_keys = DomainKeys::of(resolved.resolved());
    record_initial_seed_domains(&state, &crawl_id, &params, &map, &domain_keys);
    let seeds_by_partition = distribute_initial_seeds(&params, &map, &domain_keys);
    let started_partitions =
        launch_partitions(&peer, &crawl_id, &map, &params, seeds_by_partition).await?;

    Ok(RunStarted {
        run_id: crawl_id,
        started_as: StartedAs::New,
        lifecycle: Lifecycle::Running,
        needs: Product::Foxhound,
        partitions: started_partitions,
    })
}

fn record_initial_seed_domains(
    state: &State,
    crawl_id: &RunId,
    params: &CrawlStartParams,
    map: &PartitionMap,
    domain_keys: &DomainKeys,
) {
    let mut initial_domains = BTreeSet::new();
    for seed in params.seeds.iter() {
        let url = match seed {
            Seed::Request(req) => &req.url,
            Seed::Sitemap(sm) => &sm.sitemap,
        };
        let host = host_of(url);
        if !host.is_empty() {
            let dk = domain_keys.key(host);
            if initial_domains.insert(dk.clone()) {
                let owner = map.owner_of(&dk);
                trace_assign_initial(&dk, &owner.member);
            }
        }
    }
    state.record_domains(crawl_id, initial_domains);
}

async fn register_partitioned_record(
    state: &State,
    peer: &Arc<dyn GroupPeer>,
    record: &PartitionedCrawlRecord,
    context: &CallContext,
) -> Result<(), CallError> {
    if let Some(user_store) = context.engine().user_store(&STORE) {
        let _ = store::save_record(user_store, record.clone()).await;
    }
    state.cache_record(record.clone());
    let record_json =
        serde_json::to_value(record).expect("invariant: PartitionedCrawlRecord encodes");
    let _ = peer
        .call_lead_extension(PEER_REGISTER.to_owned(), record_json.clone())
        .await?;
    for slot in &record.map.slots {
        if slot.member != peer.member_name() {
            let _ = peer
                .call_member_extension(
                    slot.member.clone(),
                    slot.peer_address.clone(),
                    PEER_REGISTER.to_owned(),
                    record_json.clone(),
                )
                .await;
        }
    }
    Ok(())
}

fn distribute_initial_seeds(
    params: &CrawlStartParams,
    map: &PartitionMap,
    domain_keys: &DomainKeys,
) -> BTreeMap<u32, Vec<Seed>> {
    let mut seeds_by_partition: BTreeMap<u32, Vec<Seed>> = BTreeMap::new();
    for slot in &map.slots {
        seeds_by_partition.insert(slot.partition, Vec::new());
    }
    for seed in params.seeds.iter() {
        let url = match seed {
            Seed::Request(req) => &req.url,
            Seed::Sitemap(sm) => &sm.sitemap,
        };
        let host = host_of(url);
        let target = if host.is_empty() {
            0
        } else {
            map.owner_of(&domain_keys.key(host)).partition
        };
        seeds_by_partition
            .entry(target)
            .or_default()
            .push(seed.clone());
    }
    seeds_by_partition
}

async fn launch_partitions(
    peer: &Arc<dyn GroupPeer>,
    crawl_id: &RunId,
    map: &PartitionMap,
    params: &CrawlStartParams,
    mut seeds_by_partition: BTreeMap<u32, Vec<Seed>>,
) -> Result<Vec<StartedPartition>, CallError> {
    let code_free = params.extraction.is_some() || params.declaration.is_some();
    let mut started_partitions = Vec::with_capacity(map.slots.len());
    for slot in &map.slots {
        let initial_seeds = seeds_by_partition
            .remove(&slot.partition)
            .unwrap_or_default();
        let prepared = PreparedPartition {
            crawl_id: crawl_id.clone(),
            partition: slot.partition,
            run_id: slot.run_id.clone(),
            map: map.clone(),
            initial_seeds,
            initial_held_domains: Vec::new(),
        };
        peer.call_member_extension(
            slot.member.clone(),
            slot.peer_address.clone(),
            PEER_PREPARE_MEMBER.to_owned(),
            serde_json::to_value(&prepared).expect("invariant: PreparedPartition encodes"),
        )
        .await?;

        let mut member_params = params.clone();
        member_params.run_id = Some(slot.run_id.as_str().to_owned());
        member_params.partitions = None;
        member_params.group_member = None;
        member_params.limits.clear();
        if code_free {
            member_params.detached = true;
        }
        let rpc_params =
            serde_json::to_value(&member_params).expect("invariant: CrawlStartParams encodes");
        let _ = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                CRAWL_START.name.to_owned(),
                rpc_params,
            )
            .await?;

        started_partitions.push(StartedPartition {
            partition: slot.partition,
            run_id: slot.run_id.clone(),
            member: slot.member.clone(),
        });
    }
    Ok(started_partitions)
}

async fn handle_report(
    state: &State,
    record: &PartitionedCrawlRecord,
    context: &CallContext,
) -> Result<RunReport, CallError> {
    let peer = require_peer(context)?;
    // Wait until every Partition has ended before calling `run.report` on each
    // member, so long-running Partitions never hit the 10s peer-link RPC
    // timeout while waiting for completion.
    let clock = context.engine().clock();
    loop {
        let current = state
            .get_known(&record.crawl_id)
            .unwrap_or_else(|| record.clone());
        let mut any_running = false;
        for slot in &current.map.slots {
            if let Ok(val) = peer
                .call_member_rpc(
                    slot.member.clone(),
                    slot.peer_address.clone(),
                    RUN_PROGRESS_READ.name.to_owned(),
                    json!({ "run_id": slot.run_id }),
                )
                .await
                && let Ok(progress) = serde_json::from_value::<RunProgress>(val)
                && progress.lifecycle == Lifecycle::Running
            {
                any_running = true;
                break;
            }
        }
        if !any_running {
            break;
        }
        tokio::select! {
            () = context.cancellation().cancelled() => {
                return Err(CallError::protocol(
                    ProtocolFailure::InternalError,
                    "call cancelled while waiting for Partitioned crawl to end",
                ));
            }
            () = clock.sleep(Duration::from_millis(50)) => {}
        }
    }

    let current = state
        .get_known(&record.crawl_id)
        .unwrap_or_else(|| record.clone());
    let mut reports = Vec::with_capacity(current.map.slots.len());
    for slot in &current.map.slots {
        let res = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                RUN_REPORT.name.to_owned(),
                json!({ "run_id": slot.run_id }),
            )
            .await;
        if let Ok(val) = res {
            let report: RunReport = serde_json::from_value(val).map_err(|error| {
                CallError::protocol(
                    ProtocolFailure::InternalError,
                    format!("decoding Partition report failed: {error}"),
                )
            })?;
            reports.push(report);
        } else {
            let usage = fetch_partition_usage(&peer, slot).await.unwrap_or_default();
            reports.push(running_placeholder(
                &slot.run_id,
                SystemTime::UNIX_EPOCH
                    + Duration::from_millis(u64::try_from(record.created_at).unwrap_or(0)),
                usage,
            ));
        }
    }
    Ok(aggregate_reports(&record.crawl_id, &reports))
}

async fn handle_usage(
    record: &PartitionedCrawlRecord,
    context: &CallContext,
) -> Result<RunUsage, CallError> {
    let peer = require_peer(context)?;
    let mut usages = Vec::with_capacity(record.map.slots.len());
    for slot in &record.map.slots {
        usages.push(fetch_partition_usage(&peer, slot).await?);
    }
    Ok(sum_usage(usages.iter()))
}

async fn fetch_partition_usage(
    peer: &Arc<dyn GroupPeer>,
    slot: &PartitionSlot,
) -> Result<RunUsage, CallError> {
    let val = peer
        .call_member_rpc(
            slot.member.clone(),
            slot.peer_address.clone(),
            RUN_USAGE.name.to_owned(),
            json!({ "run_id": slot.run_id }),
        )
        .await?;
    serde_json::from_value(val).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("decoding Partition usage failed: {error}"),
        )
    })
}

async fn handle_progress(
    record: &PartitionedCrawlRecord,
    context: &CallContext,
) -> Result<RunProgress, CallError> {
    let peer = require_peer(context)?;
    let mut progresses = Vec::with_capacity(record.map.slots.len());
    for slot in &record.map.slots {
        let val = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                RUN_PROGRESS_READ.name.to_owned(),
                json!({ "run_id": slot.run_id }),
            )
            .await?;
        let progress: RunProgress = serde_json::from_value(val).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding Partition progress failed: {error}"),
            )
        })?;
        progresses.push(progress);
    }
    let lifecycle = aggregate_lifecycle(progresses.iter().map(|p| p.lifecycle));
    let last = progresses.iter().all(|p| p.last);
    let position = progresses.iter().map(|p| p.position).sum();
    let mut first = progresses
        .into_iter()
        .next()
        .expect("invariant: at least one Partition");
    first.run_id = record.crawl_id.clone();
    first.lifecycle = lifecycle;
    first.last = last;
    first.position = position;
    Ok(first)
}

async fn handle_stop(
    record: &PartitionedCrawlRecord,
    params: RunStopParams,
    context: &CallContext,
) -> Result<RunReport, CallError> {
    let peer = require_peer(context)?;
    let mut reports = Vec::with_capacity(record.map.slots.len());
    for slot in &record.map.slots {
        let mut p = params.clone();
        p.run_id = slot.run_id.clone();
        let val = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                RUN_STOP.name.to_owned(),
                serde_json::to_value(&p).expect("invariant: RunStopParams encodes"),
            )
            .await?;
        let report: RunReport = serde_json::from_value(val).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding Partition stop report failed: {error}"),
            )
        })?;
        reports.push(report);
    }
    Ok(aggregate_reports(&record.crawl_id, &reports))
}

async fn handle_fanout_report(
    record: &PartitionedCrawlRecord,
    method: &str,
    context: &CallContext,
) -> Result<RunReport, CallError> {
    let peer = require_peer(context)?;
    let mut reports = Vec::with_capacity(record.map.slots.len());
    for slot in &record.map.slots {
        let val = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                method.to_owned(),
                json!({ "run_id": slot.run_id }),
            )
            .await?;
        let report: RunReport = serde_json::from_value(val).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding Partition `{method}` report failed: {error}"),
            )
        })?;
        reports.push(report);
    }
    Ok(aggregate_reports(&record.crawl_id, &reports))
}

async fn handle_abort(
    record: &PartitionedCrawlRecord,
    params: RunAbortParams,
    context: &CallContext,
) -> Result<RunReport, CallError> {
    let peer = require_peer(context)?;
    let mut reports = Vec::with_capacity(record.map.slots.len());
    for slot in &record.map.slots {
        let mut p = params.clone();
        p.run_id = slot.run_id.clone();
        let val = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                RUN_ABORT.name.to_owned(),
                serde_json::to_value(&p).expect("invariant: RunAbortParams encodes"),
            )
            .await?;
        let report: RunReport = serde_json::from_value(val).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding Partition abort report failed: {error}"),
            )
        })?;
        reports.push(report);
    }
    Ok(aggregate_reports(&record.crawl_id, &reports))
}

async fn handle_checkpoint(
    record: &PartitionedCrawlRecord,
    params: RunCheckpointParams,
    context: &CallContext,
) -> Result<Checkpoint, CallError> {
    let peer = require_peer(context)?;
    let mut checkpoints = Vec::with_capacity(record.map.slots.len());
    for slot in &record.map.slots {
        let mut p = params.clone();
        p.run_id = slot.run_id.clone();
        let val = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                RUN_CHECKPOINT.name.to_owned(),
                serde_json::to_value(&p).expect("invariant: RunCheckpointParams encodes"),
            )
            .await?;
        let cp: Checkpoint = serde_json::from_value(val).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding Partition checkpoint failed: {error}"),
            )
        })?;
        checkpoints.push(cp);
    }
    let total_size: u64 = checkpoints.iter().map(|c| c.size).sum();
    let mut first = checkpoints
        .into_iter()
        .next()
        .expect("invariant: at least one Partition");
    first.run_id = record.crawl_id.clone();
    first.size = total_size;
    Ok(first)
}

async fn handle_resume(
    record: &PartitionedCrawlRecord,
    params: RunResumeParams,
    context: &CallContext,
) -> Result<RunStarted, CallError> {
    let peer = require_peer(context)?;
    let mut lifecycles = Vec::with_capacity(record.map.slots.len());
    let mut partitions = Vec::with_capacity(record.map.slots.len());
    let mut any_resumed = false;
    for slot in &record.map.slots {
        let mut p = params.clone();
        p.run_id = slot.run_id.clone();
        let val = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                "run.resume".to_owned(),
                serde_json::to_value(&p).expect("invariant: RunResumeParams encodes"),
            )
            .await?;
        let started: RunStarted = serde_json::from_value(val).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding Partition resume failed: {error}"),
            )
        })?;
        if started.started_as == StartedAs::Resumed {
            any_resumed = true;
        }
        lifecycles.push(started.lifecycle);
        partitions.push(StartedPartition {
            partition: slot.partition,
            run_id: slot.run_id.clone(),
            member: slot.member.clone(),
        });
    }
    Ok(RunStarted {
        run_id: record.crawl_id.clone(),
        started_as: if any_resumed {
            StartedAs::Resumed
        } else {
            StartedAs::Ended
        },
        lifecycle: aggregate_lifecycle(lifecycles),
        needs: Product::Foxhound,
        partitions,
    })
}

async fn fetch_union_records(
    record: &PartitionedCrawlRecord,
    dataset: &str,
    wall: Option<bool>,
    peer: &Arc<dyn GroupPeer>,
) -> Result<Vec<DatasetRecord>, CallError> {
    let mut union = Vec::new();
    let mut number = 0_u64;
    for slot in &record.map.slots {
        let req = ReadRecordsRequest {
            run_id: slot.run_id.clone(),
            dataset: dataset.to_owned(),
            after: 0,
            through: None,
            wall,
        };
        let raw = peer
            .call_member_extension(
                slot.member.clone(),
                slot.peer_address.clone(),
                PEER_READ_RECORDS.to_owned(),
                serde_json::to_value(&req).expect("invariant: ReadRecordsRequest encodes"),
            )
            .await?;
        let resp: ReadRecordsResponse = serde_json::from_value(raw).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding Partition records failed: {error}"),
            )
        })?;
        for mut rec in resp.records {
            number += 1;
            rec.number = number;
            union.push(rec);
        }
    }
    Ok(union)
}

async fn handle_dataset_page(
    record: &PartitionedCrawlRecord,
    params: DatasetPageParams,
    context: &CallContext,
) -> Result<DatasetPage, CallError> {
    let peer = require_peer(context)?;
    let wall = params.filter.as_ref().and_then(|f| {
        f.wall.map(|w| {
            matches!(
                w,
                goodfetch_protocol::foxhound::dataset_view::WallFilter::With
            )
        })
    });
    let all = fetch_union_records(record, &params.dataset, wall, &peer).await?;
    let total = u64::try_from(all.len()).unwrap_or(u64::MAX);
    let through = params.through.unwrap_or(total);
    let after = params.after.unwrap_or(0);
    let limit = usize::try_from(params.limit.unwrap_or(DATASET_PAGE_DEFAULT)).unwrap_or(100);
    let mut page_records = Vec::new();
    let mut has_more = false;
    for rec in all {
        if rec.number <= after || rec.number > through {
            continue;
        }
        if page_records.len() >= limit {
            has_more = true;
            break;
        }
        page_records.push(rec);
    }
    let next = if has_more {
        page_records.last().map(|r| r.number)
    } else {
        None
    };
    Ok(DatasetPage {
        records: page_records,
        through,
        next,
    })
}

async fn handle_dataset_list(
    record: &PartitionedCrawlRecord,
    context: &CallContext,
) -> Result<DatasetList, CallError> {
    let peer = require_peer(context)?;
    let mut combined: Option<DatasetList> = None;
    for slot in &record.map.slots {
        let val = peer
            .call_member_rpc(
                slot.member.clone(),
                slot.peer_address.clone(),
                DATASET_LIST.name.to_owned(),
                json!({ "run_id": slot.run_id }),
            )
            .await?;
        let part: DatasetList = serde_json::from_value(val).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("decoding Partition dataset list failed: {error}"),
            )
        })?;
        match &mut combined {
            None => combined = Some(part),
            Some(total) => {
                for incoming in part.datasets {
                    if let Some(existing) =
                        total.datasets.iter_mut().find(|d| d.name == incoming.name)
                    {
                        existing.committed = existing.committed.saturating_add(incoming.committed);
                        existing.dropped = existing.dropped.saturating_add(incoming.dropped);
                        existing.rejected = existing.rejected.saturating_add(incoming.rejected);
                    } else {
                        total.datasets.push(incoming);
                    }
                }
            }
        }
    }
    Ok(combined.unwrap_or(DatasetList {
        datasets: Vec::new(),
    }))
}

async fn handle_records(
    record: &PartitionedCrawlRecord,
    params: RunRecordsParams,
    context: &CallContext,
) -> Result<RecordsRead, CallError> {
    let peer = require_peer(context)?;
    let stream = context.stream();
    let after = params.after.unwrap_or_default();
    let all = fetch_union_records(record, &params.dataset, params.wall, &peer).await?;
    let mut read = RecordsRead {
        count: 0,
        last: after,
    };

    for rec in all {
        if rec.number <= after {
            continue;
        }
        if let Some(through) = params.through
            && rec.number > through
        {
            return Ok(read);
        }
        let number = rec.number;
        let _ = stream.spend(1).await;
        let sent = stream
            .notify(
                RECORD,
                &RecordRead {
                    id: stream.id().clone(),
                    record: rec,
                },
            )
            .await;
        if !sent {
            return Ok(read);
        }
        read.count += 1;
        read.last = number;
    }
    Ok(read)
}

fn require_peer(context: &CallContext) -> Result<Arc<dyn GroupPeer>, CallError> {
    context.engine().group_peer().ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            "Partitioned crawl operation requires an active Daemon group peer link",
        )
    })
}

async fn open_settings_for_check(
    params: &CrawlStartParams,
    context: &CallContext,
) -> Result<goodfetch_engine::settings::RunSettings, CallError> {
    let (settings, _) = context
        .engine()
        .settings()
        .open(Opening {
            run: Some(Product::Foxhound),
            project: params.project.clone(),
            recipe: None,
            settings: run_layer(&params.settings, context)?,
            origin: Some(RunOrigin::Settings),
            registered: context.registered_settings().clone(),
            acting_for_model: context.surface().acts_for_model(),
            exposed: context.exposed(),
        })
        .await
        .map_err(|error| settings_error(&error, context))?;
    Ok(settings)
}

fn domain_keys_from_map(settings: &Map<String, Value>) -> DomainKeys {
    let _ = settings;
    DomainKeys::default()
}

fn epoch_millis(now: SystemTime) -> i64 {
    i64::try_from(
        now.duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis(),
    )
    .unwrap_or(0)
}

fn generate_crawl_id(now: SystemTime) -> RunId {
    let mut bytes = [0_u8; 8];
    let nanos = now
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    bytes.copy_from_slice(&nanos.to_le_bytes()[..8]);
    let value = u64::from_le_bytes(bytes);
    RunId::parse(&format!("pc-{value:016x}"))
        .expect("invariant: valid generated Partitioned crawl ID")
}
