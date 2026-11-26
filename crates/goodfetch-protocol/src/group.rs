//! Daemon groups (ADR 0055): the peer protocol Group members speak to one
//! another over the peer link, and its refusals.
//!
//! The peer protocol is newline-delimited JSON-RPC 2.0 over the peer link's
//! mutual TLS, versioned on its own (ADR 0066). The Group member that
//! connects sends [`HELLO`] first, with its peer protocol version, its
//! Product and the highest lead epoch it has recorded; the other settles on
//! the lower minor of their major ([`Rule::LowerMinor`](crate::formats::Rule)),
//! refuses another major with `protocol_version_unsupported`, and refuses a
//! Group member of the other Product with `not_in_product` (ADR 0095): runs
//! move between Group members, so they run one Product, never one that
//! contains the other.
//!
//! Every message of either side carries the group's lead epoch, and a Group
//! member refuses one carrying a lower epoch than the highest it has
//! recorded with [`LEAD_EPOCH_STALE`], so a former Lead Daemon that comes
//! back is fenced off. [`LEAD_EPOCH`] exchanges lead epochs, which a Daemon
//! configured as Lead Daemon does with a majority of the Group members it
//! last knew before it acts.

use std::fmt;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};
use crate::connection::ProductVersion;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The peer protocol's version, `major.minor` (ADRs 0055 and 0066).
pub const PEER_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The method of the peer protocol's first message.
pub const HELLO: &str = "hello";

/// The method that exchanges lead epochs.
pub const LEAD_EPOCH: &str = "group.lead_epoch";

/// A Daemon group's lead epoch (ADRs 0003 and 0055): each Lead Daemon
/// promoted takes a higher one, and a Group member refuses a peer message
/// carrying a lower epoch than the highest it has recorded.
#[derive(
    Clone,
    Copy,
    Debug,
    Default,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Serialize,
    Deserialize,
    JsonSchema,
)]
#[serde(transparent)]
pub struct LeadEpoch(u64);

impl LeadEpoch {
    /// The epoch numbered `number`.
    #[must_use]
    pub const fn new(number: u64) -> Self {
        Self(number)
    }

    /// Its number.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for LeadEpoch {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// What the Group member that connects says in its `hello`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerHelloParams {
    /// The peer protocol version it speaks, the newest of its major.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The Product it runs, which must be the group's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// The highest lead epoch it has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
}

/// What a Group member answers to a `hello` it accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerHelloResult {
    /// The peer protocol version both speak from now on: the lower of the
    /// two.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch it has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
}

/// What `group.lead_epoch` says: the sender's highest lead epoch.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LeadEpochParams {
    /// The highest lead epoch the sender has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
}

/// What a Group member answers to `group.lead_epoch`: the highest lead
/// epoch it has recorded, the sender's included.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LeadEpochResult {
    /// The highest lead epoch it has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
}

/// A peer message carrying a lower lead epoch than the receiver has
/// recorded (ADR 0055).
pub const LEAD_EPOCH_STALE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "lead_epoch_stale",
    description: "A Group member received a peer message carrying a lower lead epoch than the highest it has recorded, so the sender is a former Lead Daemon, or a Group member that has not yet learnt of the current one, and the message is refused. The sender records the higher epoch from the details and acts as an ordinary Group member.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `lead_epoch_stale` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LeadEpochStaleDetails {
    /// The lead epoch the refused message carried.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The highest lead epoch the receiver has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub highest_lead_epoch: LeadEpoch,
}

/// The Error codes of the peer protocol.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[LEAD_EPOCH_STALE],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
