//! Fenced domain handoff when Group members change, per-domain state transfer
//! (including Wall allowance refetch stops, ADR 0052), TLA+ trace emission
//! (`goodfetch::partition`, ADR 0060), and `runs adopt` completion from a
//! removed Group member's kept volume (ADR 0054 and ADR 0055).

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_crawl::partition::{DomainHandoffItem, DomainHandoffSnapshot};
use goodfetch_engine::{
    AdoptHookOutcome, AdoptPeerClient, CallError, RefetchStopped, RunAdoptHook, RunIndex,
};
use goodfetch_protocol::foxhound::frontier::{HttpBody, HttpMethod};
use goodfetch_protocol::home::RUN_FILE;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::run::{AdoptedRun, RunId};
use rusqlite::{Connection, OpenFlags};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rendezvous::{PartitionMap, PartitionSlot};
use crate::store::read_partition_membership;

/// Tracing target for `PartitionHandoff.tla` / `TracePartitionHandoff.tla`
/// transitions (ADRs 0054 and 0060).
pub const TARGET: &str = "goodfetch::partition";

/// Peer extension method broadcasting a newly published [`PartitionMap`] at a
/// higher `partition_map_epoch` and fencing moving domains on old/new owners.
pub const PEER_PUBLISH_MAP: &str = "partition.publish_map";

/// Peer extension method instructing an old owner Partition to fence, wait for
/// in-flight requests, seal its Run-state batch, and hand off its moving
/// domains to their new owners.
pub const PEER_DRAIN_DOMAINS: &str = "partition.drain_domains";

/// Peer extension method delivering one moving domain's pending URLs, seen
/// fingerprints, Wall allowance refetch stops, and attached per-domain state to
/// its new owner Partition (returning an acknowledgement before the new owner
/// begins admitting).
pub const PEER_HANDOFF_DOMAIN: &str = "partition.handoff_domain";

/// Peer extension method lifting the handoff fence on the new owner once the
/// handoff has been acknowledged.
pub const PEER_ACTIVATE_DOMAIN: &str = "partition.activate_domain";

/// Peer extension method on the Lead Daemon used by a leaving member or
/// `runs adopt` to remove a departed member from a crawl's [`PartitionMap`]
/// under a higher `partition_map_epoch` and return the current map.
pub const PEER_REMOVE_MEMBER_MAP: &str = "partition.remove_member_map";

/// Serializable representation of a Wall allowance [`RefetchStopped`] entry
/// moved with its domain key during Partition handoff or `runs adopt`
/// (ADRs 0052 and 0054).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransferredRefetchStop {
    /// Registrable domain key.
    pub domain: String,
    /// Unix epoch milliseconds when refetch first stopped.
    pub first_stopped_ms: u64,
    /// Last Run ID where refetch stopped (rebound to the new owner Partition on
    /// install).
    pub last_run: RunId,
    /// Cause string recorded by the Wall allowance.
    pub cause: String,
    /// Project configuration file that supplied `wall.refetch_domains`, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project_file: Option<String>,
}

impl TransferredRefetchStop {
    /// Converts a [`RefetchStopped`] from the Run index into its wire form.
    #[must_use]
    pub fn from_index(stop: RefetchStopped) -> Self {
        let first_stopped_ms = u64::try_from(
            stop.first_stopped
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis(),
        )
        .unwrap_or(0);
        Self {
            domain: stop.domain,
            first_stopped_ms,
            last_run: stop.last_run,
            cause: stop.cause,
            project_file: stop.project_file,
        }
    }

    /// Converts back into a [`RefetchStopped`] bound to `target_run_id` for the
    /// receiving Partition's Run index.
    #[must_use]
    pub fn into_index(self, target_run_id: RunId) -> RefetchStopped {
        RefetchStopped {
            domain: self.domain,
            first_stopped: UNIX_EPOCH + Duration::from_millis(self.first_stopped_ms),
            last_run: target_run_id,
            cause: self.cause,
            project_file: self.project_file,
        }
    }
}

/// One domain moving from `from` to `to` under `map_epoch`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainMove {
    /// Moving domain key.
    pub domain_key: String,
    /// Old owning Partition index.
    pub from_partition: u32,
    /// Old owning Group member name.
    pub from_member: String,
    /// Old owning Partition Run ID.
    pub from_run_id: RunId,
    /// New owning Partition index.
    pub to_partition: u32,
    /// New owning Group member name.
    pub to_member: String,
    /// New owning Partition Run ID.
    pub to_run_id: RunId,
    /// New owning Group member peer address.
    pub to_peer_address: String,
    /// Partition-map epoch that caused this move.
    pub map_epoch: u64,
}

/// Payload for `partition.publish_map`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PublishMapRequest {
    /// Partitioned crawl ID.
    pub crawl_id: RunId,
    /// Newly published [`PartitionMap`] at a strictly higher `map_epoch`.
    pub map: PartitionMap,
    /// Domains moving INTO this member's Partition (held fenced until handoff
    /// acknowledgement).
    #[serde(default)]
    pub hold_incoming_domains: Vec<String>,
    /// Target Partition Run ID on this member, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target_run_id: Option<RunId>,
}

/// Payload for `partition.drain_domains`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrainDomainsRequest {
    /// Partitioned crawl ID.
    pub crawl_id: RunId,
    /// Old owning Partition Run ID on the receiving member.
    pub source_run_id: RunId,
    /// Old owning Partition index.
    pub source_partition: u32,
    /// Partition-map epoch under which these domains are moving.
    pub map_epoch: u64,
    /// Moving domains and their target Partition slots.
    pub moves: Vec<DomainMove>,
}

/// Response for `partition.drain_domains`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrainDomainsResponse {
    /// Domains handed off and acknowledged by their new owners.
    pub handed_off: Vec<String>,
}

/// Payload for `partition.handoff_domain`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandoffDomainRequest {
    /// Partitioned crawl ID.
    pub crawl_id: RunId,
    /// Old owning Partition index.
    pub from_partition: u32,
    /// Old owning Group member name.
    pub from_member: String,
    /// New owning Partition index.
    pub to_partition: u32,
    /// New owning Partition Run ID.
    pub to_run_id: RunId,
    /// New owning Group member name.
    pub to_member: String,
    /// Partition-map epoch of this handoff.
    pub map_epoch: u64,
    /// Sender's run epoch (fences stale senders).
    pub sender_run_epoch: u64,
    /// Exported pending URLs, seen fingerprints, and generic per-domain state.
    pub snapshot: DomainHandoffSnapshot,
    /// Wall allowance refetch stops carried with this domain key (ADR 0052).
    #[serde(default)]
    pub refetch_stops: Vec<TransferredRefetchStop>,
    /// Whether this handoff was completed offline by `runs adopt`.
    #[serde(default)]
    pub adopted: bool,
}

/// Acknowledgement for `partition.handoff_domain`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HandoffDomainResponse {
    /// Always `true` once state is durably installed on the new owner.
    pub acked: bool,
    /// Map epoch confirmed by the new owner.
    pub map_epoch: u64,
    /// Number of pending work items installed.
    pub installed: usize,
}

/// Payload for `partition.activate_domain`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ActivateDomainRequest {
    /// Partitioned crawl ID.
    pub crawl_id: RunId,
    /// New owning Partition Run ID.
    pub target_run_id: RunId,
    /// New owning Group member name.
    pub member: String,
    /// Domain key whose handoff was acknowledged.
    pub domain_key: String,
    /// Partition-map epoch of the completed handoff.
    pub map_epoch: u64,
}

/// Payload for `partition.remove_member_map`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoveMemberMapRequest {
    /// Partitioned crawl ID.
    pub crawl_id: RunId,
    /// Removed Group member name.
    pub member: String,
}

/// Response for `partition.remove_member_map`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RemoveMemberMapResponse {
    /// Current [`PartitionMap`] without the removed member.
    pub map: PartitionMap,
}

/// Emits an `assign_initial` transition for `PartitionHandoff.tla`.
pub fn trace_assign_initial(domain: &str, member: &str) {
    tracing::trace!(
        target: TARGET,
        transition = "assign_initial",
        domain,
        member,
        map_epoch = 1_u64,
    );
}

/// Emits a `publish_map` transition for `PartitionHandoff.tla`.
pub fn trace_publish_map(map_epoch: u64) {
    tracing::trace!(
        target: TARGET,
        transition = "publish_map",
        map_epoch,
    );
}

/// Emits a `fence_domain` transition for `PartitionHandoff.tla`.
pub fn trace_fence_domain(domain: &str, from: &str, to: &str, map_epoch: u64) {
    tracing::trace!(
        target: TARGET,
        transition = "fence_domain",
        domain,
        from,
        to,
        map_epoch,
    );
}

/// Emits a `drain_inflight` transition for `PartitionHandoff.tla`.
pub fn trace_drain_inflight(domain: &str, member: &str, map_epoch: u64) {
    tracing::trace!(
        target: TARGET,
        transition = "drain_inflight",
        domain,
        member,
        map_epoch,
    );
}

/// Emits a `send_handoff` transition for `PartitionHandoff.tla`.
pub fn trace_send_handoff(domain: &str, from: &str, to: &str, map_epoch: u64) {
    tracing::trace!(
        target: TARGET,
        transition = "send_handoff",
        domain,
        from,
        to,
        map_epoch,
    );
}

/// Emits an `ack_handoff` transition for `PartitionHandoff.tla`.
pub fn trace_ack_handoff(domain: &str, from: &str, to: &str, map_epoch: u64) {
    tracing::trace!(
        target: TARGET,
        transition = "ack_handoff",
        domain,
        from,
        to,
        map_epoch,
    );
}

/// Emits an `adopt_handoff` transition for `PartitionHandoff.tla`.
pub fn trace_adopt_handoff(domain: &str, from: &str, to: &str, map_epoch: u64) {
    tracing::trace!(
        target: TARGET,
        transition = "adopt_handoff",
        domain,
        from,
        to,
        map_epoch,
    );
}

/// Emits an `admit_domain` transition for `PartitionHandoff.tla`.
pub fn trace_admit_domain(domain: &str, member: &str, map_epoch: u64) {
    tracing::trace!(
        target: TARGET,
        transition = "admit_domain",
        domain,
        member,
        map_epoch,
    );
}

/// Emits a `start_fetch` transition for `PartitionHandoff.tla`.
pub fn trace_start_fetch(domain: &str, member: &str) {
    tracing::trace!(
        target: TARGET,
        transition = "start_fetch",
        domain,
        member,
    );
}

/// Emits a `finish_fetch` transition for `PartitionHandoff.tla`.
pub fn trace_finish_fetch(domain: &str, member: &str) {
    tracing::trace!(
        target: TARGET,
        transition = "finish_fetch",
        domain,
        member,
    );
}

/// Emits a `reroute_forward` transition for `PartitionHandoff.tla`.
pub fn trace_reroute_forward(domain: &str, from: &str, to: &str, old_epoch: u64, map_epoch: u64) {
    tracing::trace!(
        target: TARGET,
        transition = "reroute_forward",
        domain,
        from,
        to,
        old_epoch,
        map_epoch,
    );
}

/// Builds a new [`PartitionMap`] at `old_map.map_epoch + 1` adding `member`
/// as a new Partition slot, or returns `None` if `member` is already present.
#[must_use]
pub fn map_with_added_member(
    old_map: &PartitionMap,
    crawl_id: &RunId,
    member: String,
    peer_address: String,
) -> Option<(PartitionMap, PartitionSlot)> {
    if old_map.slots.iter().any(|slot| slot.member == member) {
        return None;
    }
    let next_partition = old_map
        .slots
        .iter()
        .map(|slot| slot.partition)
        .max()
        .map_or(0, |max| max + 1);
    let run_id = RunId::parse(&format!("{}-p{next_partition}", crawl_id.as_str()))
        .unwrap_or_else(|_| crawl_id.clone());
    let new_slot = PartitionSlot {
        partition: next_partition,
        member,
        run_id,
        peer_address,
    };
    let mut slots = old_map.slots.clone();
    slots.push(new_slot.clone());
    Some((
        PartitionMap::new(old_map.map_epoch.saturating_add(1), slots),
        new_slot,
    ))
}

/// Builds a new [`PartitionMap`] at `old_map.map_epoch + 1` removing `member`,
/// or returns `None` if `member` is not in `old_map` or is the only remaining
/// Partition.
#[must_use]
pub fn map_without_member(old_map: &PartitionMap, member: &str) -> Option<PartitionMap> {
    if !old_map.slots.iter().any(|slot| slot.member == member) {
        return None;
    }
    let remaining: Vec<PartitionSlot> = old_map
        .slots
        .iter()
        .filter(|slot| slot.member != member)
        .cloned()
        .collect();
    if remaining.is_empty() {
        return None;
    }
    Some(PartitionMap::new(
        old_map.map_epoch.saturating_add(1),
        remaining,
    ))
}

/// Computes the exact minimal set of [`DomainMove`]s between `old_map` and
/// `new_map` for `domains`: only domains whose rendezvous owner changed move
/// (ADR 0054).
#[must_use]
pub fn compute_moves(
    old_map: &PartitionMap,
    new_map: &PartitionMap,
    domains: &[String],
) -> Vec<DomainMove> {
    let mut moves = Vec::new();
    for domain_key in domains {
        let old_owner = old_map.owner_of(domain_key);
        let new_owner = new_map.owner_of(domain_key);
        if old_owner.member != new_owner.member {
            moves.push(DomainMove {
                domain_key: domain_key.clone(),
                from_partition: old_owner.partition,
                from_member: old_owner.member.clone(),
                from_run_id: old_owner.run_id.clone(),
                to_partition: new_owner.partition,
                to_member: new_owner.member.clone(),
                to_run_id: new_owner.run_id.clone(),
                to_peer_address: new_owner.peer_address.clone(),
                map_epoch: new_map.map_epoch,
            });
        }
    }
    moves
}

/// Attaches per-domain state for `area` under `domain_key` in a Partition's
/// run file (`partition_domain_state`), so it moves automatically on Partition
/// handoff and `runs adopt` (ADR 0054).
///
/// # Errors
///
/// SQLite errors while writing `partition_domain_state`.
pub fn attach_domain_state(
    connection: &Connection,
    domain_key: &str,
    area: &str,
    value: &Value,
) -> rusqlite::Result<()> {
    let encoded = serde_json::to_string(value).expect("invariant: Value encodes as JSON");
    connection
        .prepare_cached(
            "INSERT INTO partition_domain_state (domain_key, area, state_json)
             VALUES (?1, ?2, ?3)
             ON CONFLICT (domain_key, area) DO UPDATE SET state_json = excluded.state_json",
        )?
        .execute((domain_key, area, encoded))?;
    Ok(())
}

/// Reads attached per-domain state for `area` under `domain_key` from a
/// Partition's run file.
///
/// # Errors
///
/// SQLite errors while querying `partition_domain_state`.
pub fn read_domain_state(
    connection: &Connection,
    domain_key: &str,
    area: &str,
) -> rusqlite::Result<Option<Value>> {
    use rusqlite::OptionalExtension as _;
    let raw: Option<String> = connection
        .prepare_cached(
            "SELECT state_json FROM partition_domain_state WHERE domain_key = ?1 AND area = ?2",
        )?
        .query_row((domain_key, area), |row| row.get(0))
        .optional()?;
    Ok(raw.and_then(|s| serde_json::from_str(&s).ok()))
}

/// Reads all distinct domain keys present in a Partition's run file
/// (`work_items`, `partition_domain_state`).
pub fn distinct_domains_in_run_file(run_file: &Path) -> Vec<String> {
    let Ok(conn) = Connection::open_with_flags(
        run_file,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) else {
        return Vec::new();
    };
    let mut domains = BTreeSet::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT DISTINCT domain_key FROM work_items WHERE domain_key IS NOT NULL AND domain_key <> ''",
    ) && let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0))
    {
        for d in rows.flatten() {
            domains.insert(d);
        }
    }
    if let Ok(mut stmt) = conn.prepare("SELECT DISTINCT domain_key FROM partition_domain_state")
        && let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0))
    {
        for d in rows.flatten() {
            domains.insert(d);
        }
    }
    domains.into_iter().collect()
}

/// Exported offline Partition state read from a kept volume for `runs adopt`.
pub struct KeptPartitionExport {
    /// Partitioned crawl ID.
    pub crawl_id: RunId,
    /// Departed Partition index.
    pub partition_index: u32,
    /// Departed Partition Run ID.
    pub run_id: RunId,
    /// Departed Group member name (inferred from saved `PartitionMap`).
    pub departed_member: String,
    /// Per-domain snapshots ready to hand off to their new owners.
    pub snapshots: Vec<DomainHandoffSnapshot>,
    /// Wall allowance refetch stops grouped by domain key.
    pub refetch_stops: BTreeMap<String, Vec<TransferredRefetchStop>>,
}

/// Inspects a kept Run `folder` on a removed Group member's volume and, if it
/// belongs to a Partition of a Partitioned crawl, extracts all remaining
/// pending work items, seen fingerprints, attached per-domain state, and Wall
/// allowance refetch stops (from the kept volume's `run-index.sqlite`) so
/// `runs adopt` can hand them to their current owners (ADRs 0052, 0054 and
/// 0055).
///
/// # Errors
///
/// SQLite errors when reading a Partition's `run.sqlite`.
pub fn read_kept_partition_for_adopt(
    folder: &Path,
) -> Result<Option<KeptPartitionExport>, CallError> {
    let run_file = folder.join(RUN_FILE);
    if !run_file.is_file() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        &run_file,
        OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!(
                "opening kept Partition run file `{}` failed: {error}",
                run_file.display()
            ),
        )
    })?;
    let has_table: bool = conn
        .prepare(
            "SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'partitioned_crawl_membership'",
        )
        .and_then(|mut s| s.query_row([], |_| Ok(true)))
        .unwrap_or(false);
    if !has_table {
        return Ok(None);
    }
    let Some((crawl_id, partition_index, saved_map)) =
        read_partition_membership(&conn).map_err(|error| {
            CallError::protocol(
                ProtocolFailure::InternalError,
                format!("reading Partition membership failed: {error}"),
            )
        })?
    else {
        return Ok(None);
    };
    let slot = saved_map.slot(partition_index).cloned().ok_or_else(|| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("Partition index {partition_index} not found in saved PartitionMap"),
        )
    })?;

    // Collect all domains with remaining pending work items or attached state.
    let mut domains = BTreeSet::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT DISTINCT domain_key FROM work_items
         WHERE status IN ('pending', 'leased') AND domain_key IS NOT NULL AND domain_key <> ''",
    ) && let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0))
    {
        for d in rows.flatten() {
            domains.insert(d);
        }
    }
    if let Ok(mut stmt) = conn.prepare("SELECT DISTINCT domain_key FROM partition_domain_state")
        && let Ok(rows) = stmt.query_map([], |row| row.get::<_, String>(0))
    {
        for d in rows.flatten() {
            domains.insert(d);
        }
    }

    // Read Wall allowance refetch stops from sibling `run-index.sqlite` on the
    // kept volume (`<per_major>/runs/<run_id>` -> `<per_major>/run-index.sqlite`).
    let mut refetch_stops: BTreeMap<String, Vec<TransferredRefetchStop>> = BTreeMap::new();
    if let Some(index_path) = folder
        .parent()
        .and_then(Path::parent)
        .map(|per_major| per_major.join("run-index.sqlite"))
        && index_path.is_file()
    {
        for stop in RunIndex::read_refetch_stops_file(&index_path, &slot.run_id) {
            domains.insert(stop.domain.clone());
            refetch_stops
                .entry(stop.domain.clone())
                .or_default()
                .push(TransferredRefetchStop::from_index(stop));
        }
    }

    let mut snapshots = Vec::with_capacity(domains.len());
    for domain_key in domains {
        snapshots.push(read_kept_domain_snapshot(&conn, domain_key));
    }

    Ok(Some(KeptPartitionExport {
        crawl_id,
        partition_index,
        run_id: slot.run_id,
        departed_member: slot.member,
        snapshots,
        refetch_stops,
    }))
}

fn read_kept_domain_snapshot(conn: &Connection, domain_key: String) -> DomainHandoffSnapshot {
    let mut pending = Vec::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT id, url, method, body, label, depth, priority, forefront
         FROM work_items
         WHERE domain_key = ?1 AND status IN ('pending', 'leased')
         ORDER BY position",
    ) && let Ok(rows) = stmt.query_map([domain_key.as_str()], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, Option<Vec<u8>>>(3)?,
            row.get::<_, String>(4)?,
            row.get::<_, i64>(5)?,
            row.get::<_, i64>(6)?,
            row.get::<_, Option<i64>>(7)?,
        ))
    }) {
        for (id, url, method_str, body_bytes, label, depth, priority, forefront) in rows.flatten() {
            let payload = goodfetch_frontier::payload::of_request(conn, &id)
                .ok()
                .flatten();
            let method = HttpMethod::parse(&method_str).unwrap_or_else(|_| HttpMethod::get());
            let body = body_bytes.map(HttpBody::new);
            pending.push(DomainHandoffItem {
                url,
                label,
                payload,
                method,
                body,
                key: None,
                priority,
                forefront: forefront.is_some(),
                depth: u32::try_from(depth).unwrap_or(0),
                domain_key: domain_key.clone(),
            });
        }
    }

    let mut seen = BTreeSet::new();
    if let Ok(mut stmt) = conn.prepare(
        "SELECT v.identity
         FROM visited AS v
         INNER JOIN work_items AS w ON w.id = v.crawl_request
         WHERE w.domain_key = ?1 AND w.status NOT IN ('pending', 'leased')",
    ) && let Ok(rows) = stmt.query_map([domain_key.as_str()], |row| row.get::<_, String>(0))
    {
        for id in rows.flatten() {
            seen.insert(id);
        }
    }

    let mut domain_state = BTreeMap::new();
    if let Ok(mut stmt) =
        conn.prepare("SELECT area, state_json FROM partition_domain_state WHERE domain_key = ?1")
        && let Ok(rows) = stmt.query_map([domain_key.as_str()], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })
    {
        for (area, json_str) in rows.flatten() {
            if let Ok(val) = serde_json::from_str::<Value>(&json_str) {
                domain_state.insert(area, val);
            }
        }
    }

    DomainHandoffSnapshot {
        domain_key,
        pending,
        seen_fingerprints: seen.into_iter().collect(),
        domain_state,
    }
}

/// Clears handed-off pending work items & per-domain state from the kept
/// Partition `run.sqlite` after `runs adopt` succeeds so repeating `runs adopt`
/// is idempotent.
fn mark_kept_partition_adopted(folder: &Path) {
    let run_file = folder.join(RUN_FILE);
    if let Ok(conn) = Connection::open(&run_file) {
        if let Ok(mut stmt) =
            conn.prepare_cached("DELETE FROM work_items WHERE status IN ('pending', 'leased')")
        {
            let _ = stmt.execute([]);
        }
        if let Ok(mut stmt) = conn.prepare_cached("DELETE FROM partition_domain_state") {
            let _ = stmt.execute([]);
        }
    }
}

/// [`RunAdoptHook`] registered by `goodfetch-partitioned-crawl`: completes any
/// Partition handoff that the removal drain grace period did not cover by
/// handing the remaining domains (with their pending URLs, seen fingerprints,
/// Wall allowance refetch stops, and attached per-domain state) to their owners
/// under the current [`PartitionMap`] rather than resuming the Partition as a
/// standalone Run (ADRs 0052, 0054 and 0055).
pub struct PartitionAdoptHook;

impl RunAdoptHook for PartitionAdoptHook {
    fn try_adopt_folder(
        &self,
        folder: PathBuf,
        from_member: Option<String>,
        peer: Arc<dyn AdoptPeerClient>,
    ) -> Pin<Box<dyn Future<Output = Result<AdoptHookOutcome, CallError>> + Send>> {
        Box::pin(async move {
            let Some(mut export) = read_kept_partition_for_adopt(&folder)? else {
                return Ok(AdoptHookOutcome::NotMine);
            };
            if export.snapshots.is_empty() && export.refetch_stops.is_empty() {
                return Ok(AdoptHookOutcome::Handled {
                    adopted: Vec::new(),
                    skipped: vec![export.run_id.as_str().to_owned()],
                });
            }
            let departed = from_member.unwrap_or_else(|| export.departed_member.clone());
            let map_resp_raw = peer
                .call_lead_extension(
                    PEER_REMOVE_MEMBER_MAP.to_owned(),
                    serde_json::to_value(RemoveMemberMapRequest {
                        crawl_id: export.crawl_id.clone(),
                        member: departed.clone(),
                    })
                    .expect("invariant: RemoveMemberMapRequest encodes"),
                )
                .await?;
            let map_resp: RemoveMemberMapResponse =
                serde_json::from_value(map_resp_raw).map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        format!("decoding `partition.remove_member_map` failed: {error}"),
                    )
                })?;

            let mut adopted = Vec::new();
            let mut handed_to_members = BTreeSet::new();
            for snapshot in export.snapshots {
                let domain_key = snapshot.domain_key.clone();
                let owner = map_resp.map.owner_of(&domain_key).clone();
                let stops = export.refetch_stops.remove(&domain_key).unwrap_or_default();
                let req = HandoffDomainRequest {
                    crawl_id: export.crawl_id.clone(),
                    from_partition: export.partition_index,
                    from_member: departed.clone(),
                    to_partition: owner.partition,
                    to_run_id: owner.run_id.clone(),
                    to_member: owner.member.clone(),
                    map_epoch: map_resp.map.map_epoch,
                    sender_run_epoch: 1,
                    snapshot,
                    refetch_stops: stops,
                    adopted: true,
                };
                let raw = peer
                    .call_member_extension(
                        owner.member.clone(),
                        owner.peer_address.clone(),
                        PEER_HANDOFF_DOMAIN.to_owned(),
                        serde_json::to_value(&req)
                            .expect("invariant: HandoffDomainRequest encodes"),
                    )
                    .await?;
                let ack: HandoffDomainResponse = serde_json::from_value(raw).map_err(|error| {
                    CallError::protocol(
                        ProtocolFailure::InternalError,
                        format!("decoding handoff ack during `runs adopt` failed: {error}"),
                    )
                })?;
                if ack.acked {
                    trace_adopt_handoff(
                        &domain_key,
                        &departed,
                        &owner.member,
                        map_resp.map.map_epoch,
                    );
                    let activate = ActivateDomainRequest {
                        crawl_id: export.crawl_id.clone(),
                        target_run_id: owner.run_id.clone(),
                        member: owner.member.clone(),
                        domain_key: domain_key.clone(),
                        map_epoch: map_resp.map.map_epoch,
                    };
                    let _ = peer
                        .call_member_extension(
                            owner.member.clone(),
                            owner.peer_address.clone(),
                            PEER_ACTIVATE_DOMAIN.to_owned(),
                            serde_json::to_value(&activate)
                                .expect("invariant: ActivateDomainRequest encodes"),
                        )
                        .await?;
                    if handed_to_members.insert(owner.run_id.clone()) {
                        adopted.push(AdoptedRun {
                            run_id: owner.run_id.clone(),
                            owner: owner.member.clone(),
                            run_epoch: map_resp.map.map_epoch,
                        });
                    }
                }
            }
            mark_kept_partition_adopted(&folder);
            Ok(AdoptHookOutcome::Handled {
                adopted,
                skipped: Vec::new(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::run::RunId;
    use serde_json::json;

    use super::{
        attach_domain_state, compute_moves, map_with_added_member, map_without_member,
        read_domain_state,
    };
    use crate::rendezvous::{PartitionMap, PartitionSlot};
    use crate::store::RUN_STATE_TABLES;

    fn sample_map() -> PartitionMap {
        PartitionMap::new(
            1,
            vec![
                PartitionSlot {
                    partition: 0,
                    member: "lead".to_owned(),
                    run_id: RunId::parse("crawl-1-p0").unwrap(),
                    peer_address: "127.0.0.1:7000".to_owned(),
                },
                PartitionSlot {
                    partition: 1,
                    member: "worker-a".to_owned(),
                    run_id: RunId::parse("crawl-1-p1").unwrap(),
                    peer_address: "127.0.0.1:7001".to_owned(),
                },
            ],
        )
    }

    /// Adding a Group member publishes a map at epoch+1 and moves ONLY domains
    /// won by the new member; removing a Group member publishes a map at
    /// epoch+1 and moves ONLY domains held by the removed member.
    /// Covers: story:3-118
    #[test]
    fn membership_change_bumps_epoch_and_moves_only_affected_domains() {
        let crawl_id = RunId::parse("crawl-1").unwrap();
        let map1 = sample_map();
        let domains: Vec<String> = (0..40).map(|i| format!("site-{i}.test")).collect();

        // Add worker-b -> epoch 2.
        let (map2, added_slot) = map_with_added_member(
            &map1,
            &crawl_id,
            "worker-b".to_owned(),
            "127.0.0.1:7002".to_owned(),
        )
        .expect("worker-b added");
        assert_eq!(map2.map_epoch, 2);
        assert_eq!(added_slot.partition, 2);

        let add_moves = compute_moves(&map1, &map2, &domains);
        assert!(!add_moves.is_empty(), "new member should win some domains");
        assert!(
            add_moves.len() < domains.len(),
            "existing members should keep their unaffected domains"
        );
        for mv in &add_moves {
            assert_eq!(mv.to_member, "worker-b");
            assert_eq!(mv.map_epoch, 2);
        }

        // Remove worker-a -> epoch 3.
        let map3 = map_without_member(&map2, "worker-a").expect("worker-a removed");
        assert_eq!(map3.map_epoch, 3);
        let remove_moves = compute_moves(&map2, &map3, &domains);
        assert_ne!(remove_moves.len(), 0);
        for mv in &remove_moves {
            assert_eq!(mv.from_member, "worker-a");
            assert_ne!(mv.to_member, "worker-a");
            assert_eq!(mv.map_epoch, 3);
        }
    }

    /// Per-domain state attached to a domain key persists and round-trips in
    /// `partition_domain_state`.
    /// Covers: story:3-118
    #[test]
    fn per_domain_state_attaches_and_reads_round_trip() {
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        for stmt in RUN_STATE_TABLES.statements {
            conn.execute_batch(stmt).unwrap();
        }
        attach_domain_state(
            &conn,
            "walled.test",
            "wall_allowance",
            &json!({ "refetch_stopped": true, "cause": "challenge" }),
        )
        .unwrap();
        let read = read_domain_state(&conn, "walled.test", "wall_allowance")
            .unwrap()
            .expect("attached state present");
        assert_eq!(read["cause"], "challenge");
    }
}
