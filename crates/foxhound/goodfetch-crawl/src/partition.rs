//! Partitioned-crawl F3 admission seam (ADR 0054, ADR 0140, 2026-11-07
//! Foxhound refactor amendment):
//!
//! When a crawl Run is a Partition of a Partitioned crawl, its [`Invariants`]
//! carry an `Option<Arc<dyn PartitionRouter>>` built once per
//! `partition_map_epoch`. Inside Run-state admission, any URL whose domain key
//! belongs to another Partition is skipped from the local Frontier and batched
//! to [`PartitionRouter::forward`] in one call per admission batch. Incoming
//! forwarded links enter the receiving Partition through the same Run-state
//! admission (`admit_forwarded`), never a second enqueue path.

use std::fmt;
use std::sync::Arc;

use goodfetch_protocol::foxhound::frontier::{HttpBody, HttpMethod};
use goodfetch_protocol::run::LimitScope;
use serde_json::Value;

/// One discovered URL routed to a remote Partition during Run-state admission.
#[derive(Clone, Debug)]
pub struct ForwardedItem {
    /// Absolute URL.
    pub url: String,
    /// Target Label.
    pub label: String,
    /// Optional Payload JSON.
    pub payload: Option<Value>,
    /// HTTP method.
    pub method: HttpMethod,
    /// Optional HTTP body.
    pub body: Option<HttpBody>,
    /// Optional Frontier key.
    pub key: Option<String>,
    /// Domain queue priority.
    pub priority: i64,
    /// Whether forefront.
    pub forefront: bool,
    /// Computed domain key (ADR 0004).
    pub domain_key: String,
    /// Owning remote Partition index.
    pub target_partition: u32,
    /// Partition-map epoch under which this item was routed.
    pub map_epoch: u64,
}

/// Callback implemented by `goodfetch-partitioned-crawl` and attached to a
/// Partition's Run-state [`Invariants`] once per partition-map epoch.
pub trait PartitionRouter: Send + Sync + fmt::Debug {
    /// Current `partition_map_epoch`.
    fn map_epoch(&self) -> u64;

    /// Classifies `url`: returns `None` when this Partition owns `url`'s domain
    /// key (admit locally), or `Some((domain_key, target_partition))` when
    /// another Partition owns it (forward instead of admitting locally).
    fn route_url(&self, url: &str) -> Option<(String, u32)>;

    /// Hands a batch of remote items discovered in one admission step to the
    /// Partition's credit-based peer-link forwarder.
    fn forward(&self, items: Vec<ForwardedItem>);
}

/// Optional hook run inside a Partition Run's initial SQLite transaction to
/// persist Partition membership metadata in the run file.
pub type MembershipWriter = Box<dyn FnOnce(&rusqlite::Connection) -> rusqlite::Result<()> + Send>;

/// Extra options passed when starting a Partition Run of a Partitioned crawl.
pub struct PartitionStartOptions {
    /// Additional Usage ledger Limit scopes (e.g. `partitioned_crawl`).
    pub extra_limit_scopes: Vec<LimitScope>,
    /// Partition router built once for the initial `partition_map_epoch`.
    pub router: Arc<dyn PartitionRouter>,
    /// Optional hook run inside the Run's initial SQLite transaction to persist
    /// Partition membership in the run file.
    pub membership_writer: Option<MembershipWriter>,
    /// Whether an empty initial seed list is allowed (when all initial seeds
    /// hashed to other Partitions).
    pub allow_empty_seeds: bool,
}
