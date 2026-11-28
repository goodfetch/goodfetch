//! Credit-based at-least-once link forwarding over the Daemon group peer link
//! (ADRs 0024, 0054 and 0055, story:3-117).
//!
//! When a Partition's Run-state admission finds a link whose domain key belongs
//! to another Partition under the current [`PartitionMap`], it hands the link
//! to [`LinkForwarder`]. The forwarder batches links per target Partition under
//! credit-based backpressure, stamps each batch with `sender_run_epoch` and
//! `partition_map_epoch`, and retries until acknowledged (at-least-once).
//! The receiving Partition deduplicates forwarded links by admitting them
//! through its own Run state's admission with its Run invariants (never a
//! second enqueue path), and re-routes any link whose domain key it no longer
//! owns under a higher `partition_map_epoch`.

use std::collections::{BTreeMap, VecDeque};

use goodfetch_protocol::foxhound::crawl::{RequestSeed, Seed};
use goodfetch_protocol::foxhound::frontier::{HttpBody, HttpMethod};
use goodfetch_protocol::run::RunId;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rendezvous::PartitionMap;

/// Peer extension method name for forwarding discovered links between
/// Partitions over the mTLS peer link.
pub const PEER_FORWARD_LINKS: &str = "partition.forward_links";

/// Default initial credit window per target Partition (ADR 0024).
pub const INITIAL_CREDITS: u32 = 64;

/// Maximum links per single peer-link forward batch.
pub const MAX_BATCH_LINKS: usize = 64;

/// One discovered link forwarded from its discovering Partition to its owning
/// Partition (ADR 0054).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardedLink {
    /// Absolute URL to admit.
    pub url: String,
    /// Target Label declared by the crawl.
    pub label: String,
    /// Optional Payload JSON.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub payload: Option<Value>,
    /// HTTP method (`GET` by default).
    #[serde(default = "HttpMethod::get")]
    pub method: HttpMethod,
    /// Optional HTTP request body.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<HttpBody>,
    /// Optional user-declared Frontier key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    /// Priority within its domain queue.
    #[serde(default)]
    pub priority: i64,
    /// Whether queued at the forefront of its domain queue.
    #[serde(default)]
    pub forefront: bool,
    /// Pre-computed domain key (ADR 0004).
    pub domain_key: String,
}

impl ForwardedLink {
    /// Converts this forwarded link into a [`Seed`] so the receiving Partition
    /// admits it through its own Run-state admission with its Run invariants
    /// (ADR 0140, F3).
    #[must_use]
    pub fn into_seed(self) -> Seed {
        Seed::Request(RequestSeed {
            url: self.url,
            label: self.label,
            payload: self.payload,
            priority: self.priority,
            forefront: self.forefront,
            method: self.method,
            headers: BTreeMap::default(),
            body: self.body,
            key: self.key,
        })
    }
}

/// Authenticated peer-link batch of links forwarded to `target_partition`
/// (ADR 0054).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardLinksRequest {
    /// The Partitioned crawl ID.
    pub partitioned_crawl_id: RunId,
    /// The destination Partition index.
    pub target_partition: u32,
    /// The destination Partition's Run ID.
    pub target_run_id: RunId,
    /// The sending Partition index.
    pub sender_partition: u32,
    /// The sender's current run epoch (fences stale senders, ADR 0003/0054).
    pub sender_run_epoch: u64,
    /// The partition-map epoch under which these links were routed (ADR 0054).
    pub partition_map_epoch: u64,
    /// How many credits the sender requests in return for flow control (ADR 0024).
    pub credits_requested: u32,
    /// The links being forwarded.
    pub links: Vec<ForwardedLink>,
}

/// Receiver's acknowledgement to [`ForwardLinksRequest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForwardLinksResponse {
    /// Number of links accepted into the receiver's Run-state admission.
    pub accepted: usize,
    /// Number of links re-routed under a higher `partition_map_epoch`.
    pub rerouted: usize,
    /// Credits granted back to the sender (ADR 0024).
    pub credits_granted: u32,
    /// The receiver's current `partition_map_epoch`.
    pub current_map_epoch: u64,
}

/// Per-target credit window and outbound buffer for at-least-once forwarding
/// (ADR 0024 and 0054).
#[derive(Clone, Debug)]
pub struct CreditWindow {
    /// Available send credits granted by the receiving Partition.
    credits: u32,
    /// Pending outbound links awaiting credits or acknowledgement.
    pending: VecDeque<ForwardedLink>,
    /// Links currently in flight (awaiting ack; requeued on failure for
    /// at-least-once delivery).
    in_flight: Vec<ForwardedLink>,
    /// Total links acknowledged by the receiver.
    acked: u64,
}

impl Default for CreditWindow {
    fn default() -> Self {
        Self::new(INITIAL_CREDITS)
    }
}

impl CreditWindow {
    /// Creates a new credit window with `initial_credits`.
    #[must_use]
    pub const fn new(initial_credits: u32) -> Self {
        Self {
            credits: initial_credits,
            pending: VecDeque::new(),
            in_flight: Vec::new(),
            acked: 0,
        }
    }

    /// Enqueues a link to send to this target Partition.
    pub fn push(&mut self, link: ForwardedLink) {
        self.pending.push_back(link);
    }

    /// Returns whether there are links waiting to be sent or awaiting ack.
    #[must_use]
    pub fn has_unacked(&self) -> bool {
        !self.pending.is_empty() || !self.in_flight.is_empty()
    }

    /// Number of available credits.
    #[must_use]
    pub const fn credits(&self) -> u32 {
        self.credits
    }

    /// Total acknowledged links.
    #[must_use]
    pub const fn acked(&self) -> u64 {
        self.acked
    }

    /// Pops up to `min(credits, MAX_BATCH_LINKS)` links as the next in-flight
    /// batch if no batch is currently in flight and credits > 0.
    pub fn take_batch(&mut self) -> Option<Vec<ForwardedLink>> {
        if !self.in_flight.is_empty() || self.pending.is_empty() || self.credits == 0 {
            return None;
        }
        let budget = (self.credits as usize).min(MAX_BATCH_LINKS);
        let count = budget.min(self.pending.len());
        let mut batch = Vec::with_capacity(count);
        for _ in 0..count {
            if let Some(link) = self.pending.pop_front() {
                batch.push(link);
            }
        }
        self.credits = self
            .credits
            .saturating_sub(u32::try_from(batch.len()).unwrap_or(u32::MAX));
        self.in_flight.clone_from(&batch);
        Some(batch)
    }

    /// Marks the current in-flight batch acknowledged and adds `credits_granted`.
    pub fn acknowledge(&mut self, credits_granted: u32) {
        let count = u64::try_from(self.in_flight.len()).unwrap_or(u64::MAX);
        self.in_flight.clear();
        self.acked = self.acked.saturating_add(count);
        self.credits = self.credits.saturating_add(credits_granted);
    }

    /// Requeues the in-flight batch at the front on transient failure so
    /// delivery is at-least-once (ADR 0054), restoring the spent credits.
    pub fn fail_in_flight(&mut self) {
        let failed = std::mem::take(&mut self.in_flight);
        let restored = u32::try_from(failed.len()).unwrap_or(u32::MAX);
        self.credits = self.credits.saturating_add(restored);
        for link in failed.into_iter().rev() {
            self.pending.push_front(link);
        }
    }
}

/// Partitions a batch of incoming forwarded links against `local_partition`
/// under `current_map`: links owned by `local_partition` stay for local
/// Run-state admission, and links owned by another Partition (e.g. routed under
/// an older `partition_map_epoch`) are grouped by their current owning
/// Partition for re-routing (ADR 0054).
#[must_use]
pub fn classify_incoming(
    current_map: &PartitionMap,
    local_partition: u32,
    links: Vec<ForwardedLink>,
) -> (Vec<ForwardedLink>, BTreeMap<u32, Vec<ForwardedLink>>) {
    let mut local = Vec::new();
    let mut reroute: BTreeMap<u32, Vec<ForwardedLink>> = BTreeMap::new();
    for link in links {
        let owner = current_map.owner_of(&link.domain_key);
        if owner.partition == local_partition {
            local.push(link);
        } else {
            reroute.entry(owner.partition).or_default().push(link);
        }
    }
    (local, reroute)
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::foxhound::frontier::HttpMethod;
    use goodfetch_protocol::run::RunId;

    use super::{CreditWindow, ForwardedLink, classify_incoming};
    use crate::rendezvous::{PartitionMap, PartitionSlot};

    fn link(url: &str, domain_key: &str) -> ForwardedLink {
        ForwardedLink {
            url: url.to_owned(),
            label: "page".to_owned(),
            payload: None,
            method: HttpMethod::get(),
            body: None,
            key: None,
            priority: 0,
            forefront: false,
            domain_key: domain_key.to_owned(),
        }
    }

    /// Covers: story:3-117
    #[test]
    fn credit_window_enforces_backpressure_and_at_least_once_retry() {
        let mut window = CreditWindow::new(2);
        window.push(link("https://a.test/1", "a.test"));
        window.push(link("https://a.test/2", "a.test"));
        window.push(link("https://a.test/3", "a.test"));

        // Takes at most 2 (limited by credits).
        let first = window.take_batch().expect("first batch");
        assert_eq!(first.len(), 2);
        assert_eq!(window.credits(), 0);
        // No second batch while in flight or out of credits.
        assert!(window.take_batch().is_none());

        // Simulated transient peer failure requeues in-flight links (at-least-once).
        window.fail_in_flight();
        assert_eq!(window.credits(), 2);
        let retried = window.take_batch().expect("retried batch");
        assert_eq!(retried, first);

        // Ack with 0 credits pauses until receiver grants more; ack with 1 allows the 3rd link.
        window.acknowledge(1);
        assert_eq!(window.acked(), 2);
        let second = window.take_batch().expect("second batch");
        assert_eq!(second.len(), 1);
        assert_eq!(second[0].url, "https://a.test/3");
        window.acknowledge(1);
        assert!(!window.has_unacked());
        assert_eq!(window.acked(), 3);
    }

    /// Covers: story:3-117
    #[test]
    fn classify_incoming_reroutes_links_from_stale_map_epoch() {
        let map = PartitionMap::new(
            2,
            vec![
                PartitionSlot {
                    partition: 0,
                    member: "lead".to_owned(),
                    run_id: RunId::parse("crawl-p0").unwrap(),
                    peer_address: "127.0.0.1:7000".to_owned(),
                },
                PartitionSlot {
                    partition: 1,
                    member: "worker-a".to_owned(),
                    run_id: RunId::parse("crawl-p1").unwrap(),
                    peer_address: "127.0.0.1:7001".to_owned(),
                },
            ],
        );
        let domain_a = "alpha.test";
        let owner_a = map.owner_of(domain_a).partition;
        let other = 1 - owner_a;

        let (local, reroute) =
            classify_incoming(&map, other, vec![link("https://alpha.test/1", domain_a)]);
        assert_eq!(local.len(), 0);
        assert_eq!(reroute.get(&owner_a).map(Vec::len), Some(1));
    }
}
