//! Daemon groups (ADR 0055): the peer protocol Group members speak to one
//! another over the peer link, and its refusals; and the operations that
//! form a group, [`GROUP_INIT`] and [`GROUP_TOKEN`].
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
use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind, Product,
    ProtocolVersion, Reach, Stability, schema_of,
};
use crate::connection::ProductVersion;
use crate::daemon::DaemonToken;
use crate::point_in_time::PointInTime;

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

/// `group.init` on a Daemon that belongs to a Daemon group already.
pub const GROUP_EXISTS: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "group_exists",
    description: "This Daemon belongs to a Daemon group already, so `group init` forms no other: a Daemon forms or joins one group, and stays in it until it leaves.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `group.token` on a Daemon that is not a group's Lead Daemon.
pub const NOT_LEAD_DAEMON: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "not_lead_daemon",
    description: "Only a Daemon group's Lead Daemon mints join tokens, and this Daemon is not one: run `group token` on the Lead Daemon, or `group init` to form a group with this Daemon as its Lead Daemon.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `group.init`.
pub const GROUP_INIT: Operation = Operation {
    name: "group.init",
    description: "Forms a Daemon group with this Daemon as its Lead Daemon: makes the group's certificate authority from the OS generator, keeps it in the Daemon's group store at lead epoch 1, and answers the lead epoch and the SHA-256 of the authority's public key, which a Daemon that joins pins. A Daemon in a group already fails with `group_exists`. One store transaction: a cancel stops it only before it commits.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GroupInitParams>,
    result: schema_of::<GroupFormed>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `group.token`.
pub const GROUP_TOKEN: Operation = Operation {
    name: "group.token",
    description: "Mints a join token on the Lead Daemon: 32 bytes of the OS generator, which one Daemon presents once with `group join` within 30 minutes, together with the SHA-256 of the authority's public key it pins. The group store keeps only the token's SHA-256, so the answer is the one place it shows. A Daemon that is not a group's Lead Daemon fails with `not_lead_daemon`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GroupTokenParams>,
    result: schema_of::<JoinTokenMinted>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `group.init` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupInitParams {}

/// What `group.init` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupFormed {
    /// The group's lead epoch: 1, the first.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The SHA-256 of the group's certificate authority's public key, as
    /// `sha256/` and its standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub authority_key: String,
}

/// What `group.token` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupTokenParams {}

/// What `group.token` answers: the join token, the one place it shows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JoinTokenMinted {
    /// The token, in lowercase hex.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: DaemonToken,
    /// When it stops being valid, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub expires_at: SystemTime,
    /// The SHA-256 of the group's certificate authority's public key, as
    /// `sha256/` and its standard base64, which the Daemon that joins pins.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub authority_key: String,
}

/// What a Daemon's group store holds, table by table: the schema of the
/// `group-store` format.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredGroup {
    /// The group's certificate authority, which only the Lead Daemon
    /// keeps; none on any other Daemon.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub authority: Option<StoredAuthority>,
    /// The highest lead epoch the Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The join tokens minted and not yet redeemed, each by its SHA-256.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub join_tokens: Vec<StoredJoinToken>,
}

/// The group's certificate authority, as its `authority` row holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredAuthority {
    /// Its certificate's common name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its private key, PKCS #8 DER in standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Its self-signed certificate, DER in standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub certificate: String,
    /// How many revocation lists it has signed, the last one's number.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lists_issued: u64,
}

/// A join token, as its `join_tokens` row holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredJoinToken {
    /// The token's SHA-256, in lowercase hex.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub digest: String,
    /// When it stops being valid, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub expires_at: SystemTime,
}

/// The operations and Error codes of Daemon groups.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[GROUP_INIT, GROUP_TOKEN],
    events: &[],
    handler_requests: &[],
    error_codes: &[LEAD_EPOCH_STALE, GROUP_EXISTS, NOT_LEAD_DAEMON],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
