//! Replicated read-mostly control-state wire types for Daemon groups
//! (ADR 0055, issue 243).

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::LeadEpoch;
use crate::catalog::{Product, ProtocolVersion};
use crate::connection::ProductVersion;

/// The method the Lead Daemon sends a Group member, on a peer link it
/// opens with its certificate, in place of [`super::HELLO`], to push versioned
/// snapshots of read-mostly control-state stores (ADR 0055).
pub const CONTROL_STATE_PUSH: &str = "group.control_state.push";

/// The method a Group member sends the Lead Daemon, on a peer link it
/// opens with its certificate, in place of [`super::HELLO`], to request any
/// read-mostly control-state snapshots newer than the versions it holds
/// (ADR 0055).
pub const CONTROL_STATE_SYNC: &str = "group.control_state.sync";

/// How one area's control-state store is kept in a Daemon group (ADR 0055).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ControlStateMode {
    /// Single writer at the Lead Daemon; Group members reach the Lead Daemon
    /// directly and do not cache replicated snapshots of it (the Audit log,
    /// saved identities, the Usage ledger's slices and charges).
    SingleWriter,
    /// Single writer at the Lead Daemon, cached on every Group member: the
    /// Lead Daemon pushes each versioned change over the peer link and a
    /// Group member never applies an older version.
    ReadMostlyCached,
}

/// One versioned snapshot of a read-mostly control-state store pushed from
/// the Lead Daemon to a Group member (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ControlStateSnapshot {
    /// The registered store name (for example `operator_policy` or
    /// `user_configuration`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub store: String,
    /// Strictly increasing version assigned by the Lead Daemon (`>= 1`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: u64,
    /// The store's serialized text at `version`, or `None` when the store's
    /// file was removed on the Lead Daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content: Option<String>,
}

/// What the Lead Daemon sends a Group member to push versioned control-state
/// snapshots over mutual TLS (`group.control_state.push`, ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerControlStatePushParams {
    /// The peer protocol version the Lead Daemon speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The Product it runs, which must be the group's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// The Lead Daemon's lead epoch.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The versioned snapshots to apply monotonically.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub snapshots: Vec<ControlStateSnapshot>,
}

/// What a Group member answers to a `group.control_state.push` it accepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerControlStatePushResult {
    /// The peer protocol version both speak: the lower of the two.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Group member has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The highest version this Group member now holds for each store.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub versions: BTreeMap<String, u64>,
}

/// What a Group member sends the Lead Daemon to catch up on any read-mostly
/// control-state snapshots newer than `versions` (`group.control_state.sync`,
/// ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerControlStateSyncParams {
    /// The peer protocol version the Group member speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The Product it runs, which must be the group's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// The highest lead epoch the Group member has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The highest version the Group member holds for each store.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub versions: BTreeMap<String, u64>,
}

/// What the Lead Daemon answers to a `group.control_state.sync` it accepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerControlStateSyncResult {
    /// The peer protocol version both speak: the lower of the two.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Every read-mostly control-state snapshot strictly newer than the
    /// versions the Group member reported.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub snapshots: Vec<ControlStateSnapshot>,
}

/// One persisted read-mostly control-state store snapshot, as the
/// `replicated_stores` row holds it (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredReplicatedStore {
    /// The registered store name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub store: String,
    /// The highest version recorded (`>= 1`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: u64,
    /// The store's serialized text at `version`, or `None` when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub content: Option<String>,
}
