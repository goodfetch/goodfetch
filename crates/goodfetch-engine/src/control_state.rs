//! Daemon group control-state store registry and monotonic versioned
//! replication state (ADR 0055, issue 243).
//!
//! Each area declares its control-state store on its [`crate::Contribution`]
//! as either:
//! - [`ControlStateMode::SingleWriter`] at the Lead Daemon (Audit log, saved
//!   identities, Usage ledger slices and charges); or
//! - [`ControlStateMode::ReadMostlyCached`] at the Lead Daemon and cached on
//!   every Group member (Operator policy, user configuration, Usage ledger
//!   Limit rows, accounts, named tokens, certificate pins, Schedules, active
//!   major, Plugin index, Clean-up rules channel, group Run index).
//!
//! The Lead Daemon publishes every change to a read-mostly store with a
//! strictly increasing version, and [`ReplicatedState::apply`] enforces that
//! a Group member never applies an older or duplicate version even when peer
//! pushes arrive reordered, duplicated, or after a partition heals.

use std::collections::BTreeMap;
use std::sync::Arc;

use goodfetch_protocol::group::{ControlStateMode, ControlStateSnapshot, StoredReplicatedStore};

/// Registered store name for Operator policy (read-mostly, cached).
pub const STORE_OPERATOR_POLICY: &str = "operator_policy";
/// Registered store name for user configuration (read-mostly, cached).
pub const STORE_USER_CONFIGURATION: &str = "user_configuration";
/// Registered store name for Usage ledger Limit rows (read-mostly, cached).
pub const STORE_USAGE_LIMITS: &str = "usage_limits";
/// Registered store name for Usage ledger slices and charges (single writer).
pub const STORE_USAGE_LEDGER: &str = "usage_ledger";
/// Registered store name for accounts, passkeys, roles and OIDC (read-mostly, cached).
pub const STORE_ACCOUNTS: &str = "accounts";
/// Registered store name for named tokens (read-mostly, cached).
pub const STORE_NAMED_TOKENS: &str = "named_tokens";
/// Registered store name for certificate pins (read-mostly, cached).
pub const STORE_CERTIFICATE_PINS: &str = "certificate_pins";
/// Registered store name for the Schedule store (read-mostly, cached).
pub const STORE_SCHEDULES: &str = "schedules";
/// Registered store name for the active-major marker (read-mostly, cached).
pub const STORE_ACTIVE_MAJOR: &str = "active_major";
/// Registered store name for saved identities (single writer).
pub const STORE_SAVED_IDENTITIES: &str = "saved_identities";
/// Registered store name for the Plugin index and revocation list (read-mostly, cached).
pub const STORE_PLUGIN_INDEX: &str = "plugin_index";
/// Registered store name for the Clean-up rules channel (read-mostly, cached).
pub const STORE_CLEAN_UP_RULES: &str = "clean_up_rules";
/// Registered store name for the group Run index (read-mostly, cached).
pub const STORE_RUN_INDEX: &str = "run_index";
/// Registered store name for the Audit log (single writer).
pub const STORE_AUDIT_LOG: &str = "audit_log";

/// ADR 0055's built-in engine control-state store declarations.
pub const ENGINE_CONTROL_STORES: &[(&str, ControlStateMode)] = &[
    (STORE_OPERATOR_POLICY, ControlStateMode::ReadMostlyCached),
    (STORE_USER_CONFIGURATION, ControlStateMode::ReadMostlyCached),
    (STORE_USAGE_LIMITS, ControlStateMode::ReadMostlyCached),
    (STORE_USAGE_LEDGER, ControlStateMode::SingleWriter),
    (STORE_ACCOUNTS, ControlStateMode::ReadMostlyCached),
    (STORE_NAMED_TOKENS, ControlStateMode::ReadMostlyCached),
    (STORE_CERTIFICATE_PINS, ControlStateMode::ReadMostlyCached),
    (STORE_ACTIVE_MAJOR, ControlStateMode::ReadMostlyCached),
    (STORE_SAVED_IDENTITIES, ControlStateMode::SingleWriter),
    (STORE_PLUGIN_INDEX, ControlStateMode::ReadMostlyCached),
    (STORE_CLEAN_UP_RULES, ControlStateMode::ReadMostlyCached),
    (STORE_RUN_INDEX, ControlStateMode::ReadMostlyCached),
    (STORE_AUDIT_LOG, ControlStateMode::SingleWriter),
];

/// Two areas declared the same control-state store under conflicting modes.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("control-state store `{store}` was registered as both {first:?} and {second:?}")]
pub struct ControlStateConflict {
    /// The store's name.
    pub store: String,
    /// The mode registered first.
    pub first: ControlStateMode,
    /// The conflicting mode registered later.
    pub second: ControlStateMode,
}

/// Registry of every area's control-state store declaration (ADR 0055).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlStateRegistry {
    stores: BTreeMap<String, ControlStateMode>,
}

impl ControlStateRegistry {
    /// An empty registry with no stores declared yet.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// A registry pre-populated with ADR 0055's engine control-state stores
    /// and the Schedule store.
    #[must_use]
    pub fn with_defaults() -> Self {
        let mut registry = Self::new();
        for &(name, mode) in ENGINE_CONTROL_STORES {
            registry.stores.insert(name.to_owned(), mode);
        }
        registry.stores.insert(
            STORE_SCHEDULES.to_owned(),
            ControlStateMode::ReadMostlyCached,
        );
        registry
    }

    /// Declares `name` with `mode`. Re-declaring the same name with the same
    /// mode is idempotent; a conflicting mode fails with
    /// [`ControlStateConflict`].
    ///
    /// # Errors
    ///
    /// [`ControlStateConflict`] when `name` is already registered with the
    /// other mode.
    pub fn register(
        &mut self,
        name: impl Into<String>,
        mode: ControlStateMode,
    ) -> Result<(), ControlStateConflict> {
        let name = name.into();
        if let Some(&first) = self.stores.get(&name) {
            if first != mode {
                return Err(ControlStateConflict {
                    store: name,
                    first,
                    second: mode,
                });
            }
            return Ok(());
        }
        self.stores.insert(name, mode);
        Ok(())
    }

    /// The mode declared for `store`, or `None` if unregistered.
    #[must_use]
    pub fn mode(&self, store: &str) -> Option<ControlStateMode> {
        self.stores.get(store).copied()
    }

    /// Whether `store` is registered as [`ControlStateMode::ReadMostlyCached`].
    #[must_use]
    pub fn is_cached(&self, store: &str) -> bool {
        self.mode(store) == Some(ControlStateMode::ReadMostlyCached)
    }

    /// Every registered store and its declared mode.
    #[must_use]
    pub fn stores(&self) -> &BTreeMap<String, ControlStateMode> {
        &self.stores
    }
}

/// Why a control-state publish or apply was refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ControlStateError {
    /// The store was never registered in the engine's control-state registry.
    #[error("control-state store `{0}` is not registered")]
    Unregistered(String),
    /// The store is declared [`ControlStateMode::SingleWriter`] and is not
    /// replicated as a cached snapshot.
    #[error("control-state store `{0}` is single-writer at the Lead Daemon, not cached")]
    NotCached(String),
    /// A snapshot carried version 0, which is reserved for "nothing applied".
    #[error("control-state snapshot for `{0}` carries invalid version 0")]
    ZeroVersion(String),
}

/// What happened when a Group member received a [`ControlStateSnapshot`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ApplyOutcome {
    /// The snapshot had a strictly higher version and became the member's
    /// current copy.
    Applied,
    /// The snapshot's version was equal to or older than `current` and was
    /// ignored.
    Stale {
        /// The version the Group member already holds.
        current: u64,
    },
}

/// One locally held versioned copy of a read-mostly store.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReplicatedEntry {
    /// Strictly positive version (`>= 1`).
    pub version: u64,
    /// Serialized store text, or `None` when removed.
    pub content: Option<String>,
}

/// In-memory view of a Group member's replicated read-mostly control-state
/// stores, enforcing strictly monotonic version application (ADR 0055).
#[derive(Clone, Debug)]
pub struct ReplicatedState {
    registry: Arc<ControlStateRegistry>,
    entries: BTreeMap<String, ReplicatedEntry>,
}

impl ReplicatedState {
    /// Empty replicated state governed by `registry`.
    #[must_use]
    pub fn new(registry: Arc<ControlStateRegistry>) -> Self {
        Self {
            registry,
            entries: BTreeMap::new(),
        }
    }

    /// The registry governing which stores may replicate.
    #[must_use]
    pub fn registry(&self) -> &ControlStateRegistry {
        &self.registry
    }

    /// Loads durable snapshots read from `group-store.sqlite` on startup,
    /// ignoring any unregistered or older entries.
    pub fn hydrate(&mut self, stored: Vec<StoredReplicatedStore>) {
        for item in stored {
            let _ = self.apply(ControlStateSnapshot {
                store: item.store,
                version: item.version,
                content: item.content,
            });
        }
    }

    /// Publishes a new Lead Daemon state for `store`: returns `Ok(Some(snapshot))`
    /// with the next strictly higher version if `content` differs from the
    /// current entry (or `Ok(None)` when unchanged, including an absent file
    /// that was never published).
    ///
    /// # Errors
    ///
    /// [`ControlStateError::Unregistered`] or [`ControlStateError::NotCached`].
    pub fn publish(
        &mut self,
        store: &str,
        content: Option<String>,
    ) -> Result<Option<ControlStateSnapshot>, ControlStateError> {
        self.require_cached(store)?;
        match self.entries.get(store) {
            None if content.is_none() => return Ok(None),
            Some(existing) if existing.content == content => return Ok(None),
            _ => {}
        }
        let next = self
            .entries
            .get(store)
            .map_or(1, |entry| entry.version.saturating_add(1));
        self.entries.insert(
            store.to_owned(),
            ReplicatedEntry {
                version: next,
                content: content.clone(),
            },
        );
        Ok(Some(ControlStateSnapshot {
            store: store.to_owned(),
            version: next,
            content,
        }))
    }

    /// Applies `snapshot` received from the Lead Daemon if and only if
    /// `snapshot.version` is strictly greater than the version already held
    /// for `snapshot.store`.
    ///
    /// # Errors
    ///
    /// [`ControlStateError::Unregistered`], [`ControlStateError::NotCached`],
    /// or [`ControlStateError::ZeroVersion`].
    pub fn apply(
        &mut self,
        snapshot: ControlStateSnapshot,
    ) -> Result<ApplyOutcome, ControlStateError> {
        self.require_cached(&snapshot.store)?;
        if snapshot.version == 0 {
            return Err(ControlStateError::ZeroVersion(snapshot.store));
        }
        if let Some(existing) = self.entries.get(&snapshot.store)
            && snapshot.version <= existing.version
        {
            return Ok(ApplyOutcome::Stale {
                current: existing.version,
            });
        }
        self.entries.insert(
            snapshot.store,
            ReplicatedEntry {
                version: snapshot.version,
                content: snapshot.content,
            },
        );
        Ok(ApplyOutcome::Applied)
    }

    /// The current entry for `store`, if any version has been published or
    /// applied.
    #[must_use]
    pub fn get(&self, store: &str) -> Option<&ReplicatedEntry> {
        self.entries.get(store)
    }

    /// The current version held for `store`, or `0` if none.
    #[must_use]
    pub fn version(&self, store: &str) -> u64 {
        self.entries.get(store).map_or(0, |entry| entry.version)
    }

    /// Map of every locally held store name to its highest version.
    #[must_use]
    pub fn versions(&self) -> BTreeMap<String, u64> {
        self.entries
            .iter()
            .map(|(store, entry)| (store.clone(), entry.version))
            .collect()
    }

    /// Every local snapshot strictly newer than the version recorded in
    /// `peer_versions` (treating absent stores as version 0).
    #[must_use]
    pub fn snapshots_newer_than(
        &self,
        peer_versions: &BTreeMap<String, u64>,
    ) -> Vec<ControlStateSnapshot> {
        self.entries
            .iter()
            .filter_map(|(store, entry)| {
                let peer_version = peer_versions.get(store).copied().unwrap_or(0);
                (entry.version > peer_version).then(|| ControlStateSnapshot {
                    store: store.clone(),
                    version: entry.version,
                    content: entry.content.clone(),
                })
            })
            .collect()
    }

    fn require_cached(&self, store: &str) -> Result<(), ControlStateError> {
        match self.registry.mode(store) {
            Some(ControlStateMode::ReadMostlyCached) => Ok(()),
            Some(ControlStateMode::SingleWriter) => {
                Err(ControlStateError::NotCached(store.to_owned()))
            }
            None => Err(ControlStateError::Unregistered(store.to_owned())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Areas register stores as either single-writer at the Lead Daemon or
    /// read-mostly and cached, and conflicting declarations are refused.
    ///
    /// Covers: story:3-93, story:3-107
    #[test]
    fn registry_declares_modes_and_refuses_conflicts() {
        let mut registry = ControlStateRegistry::with_defaults();
        assert_eq!(
            registry.mode(STORE_OPERATOR_POLICY),
            Some(ControlStateMode::ReadMostlyCached)
        );
        assert_eq!(
            registry.mode(STORE_USER_CONFIGURATION),
            Some(ControlStateMode::ReadMostlyCached)
        );
        assert_eq!(
            registry.mode(STORE_AUDIT_LOG),
            Some(ControlStateMode::SingleWriter)
        );
        assert_eq!(
            registry.mode(STORE_SAVED_IDENTITIES),
            Some(ControlStateMode::SingleWriter)
        );
        assert_eq!(
            registry.mode(STORE_USAGE_LEDGER),
            Some(ControlStateMode::SingleWriter)
        );

        // Same mode is idempotent; conflicting mode fails.
        assert_eq!(
            registry.register("custom_area_cache", ControlStateMode::ReadMostlyCached),
            Ok(())
        );
        assert_eq!(
            registry.register("custom_area_cache", ControlStateMode::ReadMostlyCached),
            Ok(())
        );
        assert_eq!(
            registry.register("custom_area_cache", ControlStateMode::SingleWriter),
            Err(ControlStateConflict {
                store: "custom_area_cache".to_owned(),
                first: ControlStateMode::ReadMostlyCached,
                second: ControlStateMode::SingleWriter,
            })
        );
    }

    /// Single-writer and unregistered stores cannot be published or applied
    /// through the read-mostly replication path.
    ///
    /// Covers: story:3-93
    #[test]
    fn single_writer_and_unregistered_stores_are_not_replicated() {
        let registry = Arc::new(ControlStateRegistry::with_defaults());
        let mut state = ReplicatedState::new(registry);

        assert_eq!(
            state.publish(STORE_AUDIT_LOG, Some("entry".to_owned())),
            Err(ControlStateError::NotCached(STORE_AUDIT_LOG.to_owned()))
        );
        assert_eq!(
            state.publish("unknown_store", Some("x".to_owned())),
            Err(ControlStateError::Unregistered("unknown_store".to_owned()))
        );
        assert_eq!(
            state.apply(ControlStateSnapshot {
                store: STORE_USAGE_LEDGER.to_owned(),
                version: 1,
                content: Some("x".to_owned()),
            }),
            Err(ControlStateError::NotCached(STORE_USAGE_LEDGER.to_owned()))
        );
    }

    /// Reordered, duplicate, or older snapshots are never applied, and
    /// `snapshots_newer_than` returns only strictly newer versions.
    ///
    /// Covers: story:3-93, story:3-107
    #[test]
    fn replicated_state_applies_strictly_monotonically() {
        let registry = Arc::new(ControlStateRegistry::with_defaults());
        let mut lead = ReplicatedState::new(Arc::clone(&registry));
        let mut worker = ReplicatedState::new(registry);

        // Publishing None for a never-published store is a no-op.
        assert_eq!(lead.publish(STORE_OPERATOR_POLICY, None).unwrap(), None);

        let v1 = lead
            .publish(STORE_OPERATOR_POLICY, Some("v1".to_owned()))
            .unwrap()
            .unwrap();
        assert_eq!(v1.version, 1);
        // Publishing identical content does not bump the version.
        assert_eq!(
            lead.publish(STORE_OPERATOR_POLICY, Some("v1".to_owned()))
                .unwrap(),
            None
        );

        let v2 = lead
            .publish(STORE_OPERATOR_POLICY, Some("v2".to_owned()))
            .unwrap()
            .unwrap();
        let v3 = lead
            .publish(STORE_OPERATOR_POLICY, Some("v3".to_owned()))
            .unwrap()
            .unwrap();
        assert_eq!((v2.version, v3.version), (2, 3));

        // Deliver v3 first, then stale v1, stale v2, duplicate v3.
        assert_eq!(worker.apply(v3.clone()).unwrap(), ApplyOutcome::Applied);
        assert_eq!(
            worker.apply(v1).unwrap(),
            ApplyOutcome::Stale { current: 3 }
        );
        assert_eq!(
            worker.apply(v2).unwrap(),
            ApplyOutcome::Stale { current: 3 }
        );
        assert_eq!(
            worker.apply(v3).unwrap(),
            ApplyOutcome::Stale { current: 3 }
        );
        assert_eq!(
            worker.get(STORE_OPERATOR_POLICY),
            Some(&ReplicatedEntry {
                version: 3,
                content: Some("v3".to_owned()),
            })
        );

        // Zero version is always rejected.
        assert_eq!(
            worker.apply(ControlStateSnapshot {
                store: STORE_USER_CONFIGURATION.to_owned(),
                version: 0,
                content: Some("bad".to_owned()),
            }),
            Err(ControlStateError::ZeroVersion(
                STORE_USER_CONFIGURATION.to_owned()
            ))
        );

        // `snapshots_newer_than` returns only what the peer lacks.
        let _cfg_v1 = lead
            .publish(STORE_USER_CONFIGURATION, Some("cfg1".to_owned()))
            .unwrap()
            .unwrap();
        let diff = lead.snapshots_newer_than(&worker.versions());
        assert_eq!(diff.len(), 1);
        assert_eq!(diff[0].store, STORE_USER_CONFIGURATION);
        assert_eq!(diff[0].version, 1);
    }
}
