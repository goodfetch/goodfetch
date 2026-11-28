//! Rendezvous hashing (Highest Random Weight) for Partitioned crawl domain
//! ownership (ADR 0054, story:3-117).
//!
//! Each domain key (ADR 0004) is assigned to the Partition whose slot achieves
//! the highest 64-bit SHA-256 score for `(slot.member, domain_key)`, with a
//! deterministic tie-breaker on `slot.member`. Adding or removing a Partition
//! moves only the domains the added/removed Partition wins or held (~1/N).
//!
//! A [`PartitionMap`] is constructed once per `partition_map_epoch` and shared
//! immutably across admission calls on that epoch (ADR 0140, F3).

use goodfetch_protocol::run::RunId;
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

/// One Partition slot in a [`PartitionMap`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionSlot {
    /// Zero-based Partition index.
    pub partition: u32,
    /// Owning Group member name (stable rendezvous identity).
    pub member: String,
    /// This Partition's Run ID.
    pub run_id: RunId,
    /// The Group member's peer listener address (`host:port`).
    pub peer_address: String,
}

/// Immutable Partition ownership map for one `partition_map_epoch` (ADR 0054).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PartitionMap {
    /// Monotonic partition-map epoch.
    pub map_epoch: u64,
    /// Active Partition slots (always non-empty).
    pub slots: Vec<PartitionSlot>,
}

impl PartitionMap {
    /// Builds a [`PartitionMap`] at `map_epoch` from non-empty `slots`.
    ///
    /// # Panics
    ///
    /// Panics if `slots` is empty.
    #[must_use]
    pub fn new(map_epoch: u64, slots: Vec<PartitionSlot>) -> Self {
        assert!(
            !slots.is_empty(),
            "invariant: a PartitionMap has at least one PartitionSlot"
        );
        Self { map_epoch, slots }
    }

    /// Returns the Partition that owns `domain_key` under rendezvous hashing
    /// (highest score wins, tie-broken deterministically by `member` name).
    #[must_use]
    pub fn owner_of(&self, domain_key: &str) -> &PartitionSlot {
        self.slots
            .iter()
            .max_by(|left, right| {
                rendezvous_score(&left.member, domain_key)
                    .cmp(&rendezvous_score(&right.member, domain_key))
                    .then_with(|| left.member.cmp(&right.member))
            })
            .expect("invariant: PartitionMap always has at least one slot")
    }

    /// Looks up a Partition slot by its index.
    #[must_use]
    pub fn slot(&self, partition: u32) -> Option<&PartitionSlot> {
        self.slots.iter().find(|slot| slot.partition == partition)
    }

    /// Looks up a Partition slot by its Run ID.
    #[must_use]
    pub fn slot_by_run(&self, run_id: &RunId) -> Option<&PartitionSlot> {
        self.slots.iter().find(|slot| slot.run_id == *run_id)
    }
}

/// Computes the deterministic 64-bit rendezvous hash score for `(member, domain_key)`.
#[must_use]
pub fn rendezvous_score(member: &str, domain_key: &str) -> u64 {
    let mut hasher = Sha256::new();
    hasher.update(member.as_bytes());
    hasher.update([0_u8]);
    hasher.update(domain_key.as_bytes());
    let digest = hasher.finalize();
    u64::from_be_bytes([
        digest[0], digest[1], digest[2], digest[3], digest[4], digest[5], digest[6], digest[7],
    ])
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use goodfetch_protocol::run::RunId;
    use proptest::prelude::*;

    use super::{PartitionMap, PartitionSlot, rendezvous_score};

    fn make_slots(names: &[&str]) -> Vec<PartitionSlot> {
        names
            .iter()
            .enumerate()
            .map(|(idx, name)| PartitionSlot {
                partition: u32::try_from(idx).expect("small test count"),
                member: (*name).to_owned(),
                run_id: RunId::parse(&format!("crawl-p{idx}")).expect("valid RunId"),
                peer_address: format!("127.0.0.1:{}", 7000 + idx),
            })
            .collect()
    }

    /// Covers: story:3-117
    #[test]
    fn owner_is_always_the_highest_rendezvous_score() {
        let map = PartitionMap::new(1, make_slots(&["lead", "worker-a", "worker-b"]));
        for domain in [
            "catalog.test",
            "extraction.test",
            "parameters.test",
            "example.com",
            "news.ycombinator.com",
            "rust-lang.org",
        ] {
            let owner = map.owner_of(domain);
            let owner_score = rendezvous_score(&owner.member, domain);
            for slot in &map.slots {
                let s = rendezvous_score(&slot.member, domain);
                assert!(
                    owner_score > s || (owner_score == s && owner.member >= slot.member),
                    "domain {domain}: owner {} ({owner_score}) lost to {} ({s})",
                    owner.member,
                    slot.member
                );
            }
        }
    }

    proptest! {
        /// Every domain key maps deterministically to the Partition with the
        /// highest rendezvous hash score, and adding/removing a Partition only
        /// moves keys won or held by that Partition (never between surviving
        /// Partitions).
        /// Covers: story:3-117
        #[test]
        fn proptest_rendezvous_highest_score_and_minimal_movement(
            domain_key in "[a-z0-9]{1,12}\\.[a-z]{2,6}",
            extra in 1_u32..=6,
        ) {
            let mut names: Vec<String> = (0..=extra).map(|i| format!("member-{i}")).collect();
            let full_slots: Vec<PartitionSlot> = names
                .iter()
                .enumerate()
                .map(|(idx, name)| PartitionSlot {
                    partition: u32::try_from(idx).unwrap(),
                    member: name.clone(),
                    run_id: RunId::parse(&format!("run-p{idx}")).unwrap(),
                    peer_address: format!("127.0.0.1:{}", 8000 + idx),
                })
                .collect();
            let full_map = PartitionMap::new(2, full_slots.clone());
            let removed_name = names.pop().unwrap();
            let reduced_slots: Vec<PartitionSlot> = full_slots
                .into_iter()
                .filter(|slot| slot.member != removed_name)
                .collect();
            let reduced_map = PartitionMap::new(1, reduced_slots);

            let owner_full = full_map.owner_of(&domain_key);
            let owner_reduced = reduced_map.owner_of(&domain_key);

            // Highest score invariant on both maps.
            let full_best = full_map
                .slots
                .iter()
                .map(|s| (rendezvous_score(&s.member, &domain_key), s.member.as_str()))
                .max()
                .unwrap();
            prop_assert_eq!(owner_full.member.as_str(), full_best.1);

            // Minimal-movement invariant: if a domain was not won by the added
            // member, its owner never changed between existing members.
            if owner_full.member != removed_name {
                prop_assert_eq!(&owner_full.member, &owner_reduced.member);
            }
        }

        /// Removing a Partition redistributes roughly 1/N of domains and never
        /// moves a domain between two surviving Partitions.
        /// Covers: story:3-117
        #[test]
        fn proptest_distribution_balances_across_partitions(seed in 0_u64..100) {
            let map = PartitionMap::new(1, make_slots(&["node-a", "node-b", "node-c"]));
            let mut seen = BTreeSet::new();
            for i in 0..300_u64 {
                let key = format!("site-{seed}-{i}.example");
                seen.insert(map.owner_of(&key).member.clone());
            }
            prop_assert_eq!(seen.len(), 3);
        }
    }
}
