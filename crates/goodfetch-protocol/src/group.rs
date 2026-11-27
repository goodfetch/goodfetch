//! Daemon groups (ADR 0055): the peer protocol Group members speak to one
//! another over the peer link, and its refusals; and the operations that
//! form a group, [`GROUP_INIT`], [`GROUP_TOKEN`] and [`GROUP_JOIN`], and
//! remove a Group member from it, [`GROUP_LEAVE`] and [`GROUP_REVOKE`].
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
//!
//! A Daemon that joins has no certificate yet: it opens a peer link to the
//! Lead Daemon without one, checks that the Lead Daemon's certificate
//! authority has the public key [`GROUP_JOIN`] pins, and sends [`JOIN`] in
//! place of `hello`, the only message such a link carries. A Group member
//! whose certificate nears its expiry opens a peer link to the Lead Daemon
//! with it and sends [`RENEW`] in place of `hello`, likewise alone, and one
//! that leaves the group sends [`LEAVE`] the same way.

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
use crate::run::RunIndexRow;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The peer protocol's version, `major.minor` (ADRs 0055 and 0066).
pub const PEER_PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// Whether `name` is a valid Group member name: one RFC 1123 DNS label of
/// 1 to 63 lowercase ASCII letters, digits or hyphens, starting and ending
/// with a letter or digit.
#[must_use]
pub fn is_group_member_name(name: &str) -> bool {
    let allowed = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    let bytes = name.as_bytes();
    (1..=63).contains(&bytes.len())
        && bytes.first().copied().is_some_and(allowed)
        && bytes.last().copied().is_some_and(allowed)
        && bytes.iter().all(|&byte| allowed(byte) || byte == b'-')
}

/// The method of the peer protocol's first message.
pub const HELLO: &str = "hello";

/// The method that exchanges lead epochs.
pub const LEAD_EPOCH: &str = "group.lead_epoch";

/// The method a Daemon that joins sends the Lead Daemon, on a peer link it
/// opens without a certificate, in place of [`HELLO`].
pub const JOIN: &str = "group.join";

/// The method a Group member sends the Lead Daemon, on a peer link it
/// opens with its certificate, in place of [`HELLO`], to have its
/// certificate renewed before it expires.
pub const RENEW: &str = "group.renew";

/// The method a Group member sends the Lead Daemon, on a peer link it
/// opens with its certificate, in place of [`HELLO`], to leave the group:
/// the Lead Daemon revokes its certificates.
pub const LEAVE: &str = "group.leave";

/// The method a Group member sends the Lead Daemon, on a peer link it
/// opens with its certificate, in place of [`HELLO`], to feed or re-report
/// its Run index rows into the group Run index (ADR 0055).
pub const RUN_INDEX_REPORT: &str = "group.run_index.report";

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

/// What a Daemon that joins sends the Lead Daemon, the one message of a
/// peer link it opens without a certificate: its join token, the name it
/// takes and its public key, which the Lead Daemon signs a Group member
/// certificate for, so no private key crosses the link.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerJoinParams {
    /// The peer protocol version the Daemon that joins speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The Product it runs, which must be the group's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// The join token `group token` minted.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: DaemonToken,
    /// The Group member name it takes: one lowercase DNS label.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its P-256 public key, an uncompressed SEC1 point in standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub public_key: String,
    /// The `host:port` its own peer listener takes, which the other Group
    /// members dial.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub peer_address: String,
}

/// What the Lead Daemon answers to a `group.join` it accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerJoinResult {
    /// The peer protocol version both speak from now on: the lower of the
    /// two.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The group's lead epoch.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The Group member certificate signed for the public key, DER in
    /// standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub certificate: String,
    /// When that certificate stops being valid, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub certificate_expires_at: SystemTime,
    /// The group's certificate authority's certificate, DER in standard
    /// base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub authority: String,
    /// The authority's latest revocation list, DER in standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revocations: String,
    /// The Lead Daemon's Group member name, which its certificate names
    /// and a Group member checks when it dials it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_name: String,
}

/// What a Group member sends the Lead Daemon to have its certificate
/// renewed, the one message of a peer link it opens with that certificate:
/// the name the certificate names and a new public key, which the Lead
/// Daemon signs a new certificate for.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRenewParams {
    /// The peer protocol version the Group member speaks.
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
    /// Its Group member name, which the certificate it presents names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its new P-256 public key, an uncompressed SEC1 point in standard
    /// base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub public_key: String,
}

/// What the Lead Daemon answers to a `group.renew` it accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRenewResult {
    /// The peer protocol version both speak: the lower of the two.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The new Group member certificate, DER in standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub certificate: String,
    /// When it stops being valid, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub certificate_expires_at: SystemTime,
    /// The authority's latest revocation list, DER in standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revocations: String,
}

/// What a Group member sends the Lead Daemon to leave the group, the one
/// message of a peer link it opens with its certificate.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerLeaveParams {
    /// The peer protocol version the Group member speaks.
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
    /// Its Group member name, which the certificate it presents names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
}

/// What the Lead Daemon answers to a `group.leave` it accepts, once it
/// revoked the Group member's certificates.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerLeaveResult {
    /// The peer protocol version both speak: the lower of the two.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
}

/// What a Group member sends the Lead Daemon to report its Run index rows
/// into the group Run index, the one message of a peer link it opens with
/// its certificate (ADR 0055).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunIndexReportParams {
    /// The peer protocol version the Group member speaks.
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
    /// Its Group member name, which the certificate it presents names and
    /// which becomes each reported row's owner.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The Run index rows it owns to record in the group Run index.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rows: Vec<RunIndexRow>,
}

/// What the Lead Daemon answers to a `group.run_index.report` it accepts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunIndexReportResult {
    /// The peer protocol version both speak: the lower of the two.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// How many Run index rows it recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reported: u64,
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

/// A join token the Lead Daemon does not hold: unknown, used or expired.
pub const JOIN_TOKEN_REFUSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "join_token_refused",
    description: "The Lead Daemon refused the join token: it minted no such token, or the token was used or is past its 30 minutes. Mint another with `group token` on the Lead Daemon and run `group join` with it.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `group.join` under a Group member name the group has already.
pub const GROUP_MEMBER_EXISTS: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "group_member_exists",
    description: "The Daemon group has a Group member of this name already, so the Daemon joins under another. The join token stays valid for it.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The Lead Daemon's certificate authority is not the one pinned.
pub const AUTHORITY_KEY_MISMATCH: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "authority_key_mismatch",
    description: "The Daemon at the Lead Daemon's address presented no certificate authority whose public key has the SHA-256 that `group join` pins, so the Daemon sent it nothing: the address leads elsewhere, or the key was copied wrong. Take the key `group token` answers on the Lead Daemon.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The Lead Daemon's peer listener cannot be reached.
pub const GROUP_LEAD_UNREACHABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "group_lead_unreachable",
    description: "Nothing answered at the Lead Daemon's peer address, or the peer link closed before its answer: check the address and that the Lead Daemon runs, then try again.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `group.leave` or `group.revoke` of the Lead Daemon.
pub const LEAD_CANNOT_LEAVE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "lead_cannot_leave",
    description: "The Lead Daemon cannot leave its Daemon group, nor be revoked from it, while it is the Lead Daemon: promote another Group member first, then remove this one.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `group.revoke` of a name the group has no Group member of.
pub const GROUP_MEMBER_UNKNOWN: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "group_member_unknown",
    description: "The Daemon group has no Group member of this name, so nothing was revoked: it left already, was revoked, or never joined.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `group.leave` on a Daemon in no Daemon group.
pub const NOT_IN_A_GROUP: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "not_in_a_group",
    description: "This Daemon belongs to no Daemon group, so it has none to leave: form one with `group init`, or join one with `group join`.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A Lead Daemon write attempted before it has confirmed its lead epoch.
pub const LEAD_UNCONFIRMED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "lead_unconfirmed",
    description: "The Lead Daemon has not yet confirmed its lead epoch with a majority of its last-known Group members: wait for a majority to come within reach, or confirm by hand with `group lead --confirm` once you have verified no other Lead Daemon runs.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `group.join` into a group with no Politeness store and no
/// `politeness.sharing: per_daemon` in its Operator policy (ADRs 0053, 0055).
pub const GROUP_POLITENESS_UNSHARED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "group_politeness_unshared",
    description: "A Daemon group of more than one Group member requires a shared Politeness store (`politeness.store`), or `politeness.sharing: per_daemon` in its Operator policy to keep Politeness per Daemon.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `group.init`.
pub const GROUP_INIT: Operation = Operation {
    name: "group.init",
    description: "Forms a Daemon group with this Daemon as its Lead Daemon: makes the group's certificate authority from the OS generator and signs this Daemon's own Group member certificate with it, keeps both in the Daemon's group store at lead epoch 1, listens for peer links at `peer_address`, and answers the lead epoch, the address the peer listener took and the SHA-256 of the authority's public key, which a Daemon that joins pins. A Daemon in a group already fails with `group_exists`. One store transaction: a cancel stops it only before it commits.",
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

/// `group.join`.
pub const GROUP_JOIN: Operation = Operation {
    name: "group.join",
    description: "Joins this Daemon to the Daemon group whose Lead Daemon listens at `lead`: makes the Daemon's key from the OS generator, opens a peer link that checks the Lead Daemon's certificate authority against `authority_key`, sends the join token, the name and the public key, and keeps the certificate the Lead Daemon signs, then listens for peer links at `peer_address`. The token works once, so a refusal of anything but the token leaves it valid. A Daemon in a group already fails with `group_exists`; a token the Lead Daemon does not hold with `join_token_refused`; a name taken with `group_member_exists`; another authority with `authority_key_mismatch`; a Lead Daemon out of reach with `group_lead_unreachable`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GroupJoinParams>,
    result: schema_of::<GroupJoined>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `group.leave`.
pub const GROUP_LEAVE: Operation = Operation {
    name: "group.leave",
    description: "Takes this Daemon out of its Daemon group: opens a peer link to the Lead Daemon with its certificate and sends `group.leave`, after which the Lead Daemon revokes every certificate it signed for it and no Group member accepts it again; then forgets its group and closes its peer listener. A Daemon in no group fails with `not_in_a_group`; the Lead Daemon with `lead_cannot_leave`; a Lead Daemon out of reach with `group_lead_unreachable`, after which the Daemon is in its group still.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GroupLeaveParams>,
    result: schema_of::<GroupLeft>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `group.revoke`.
pub const GROUP_REVOKE: Operation = Operation {
    name: "group.revoke",
    description: "Removes the Group member `name` from the Daemon group, on the Lead Daemon: revokes every certificate the authority signed for it in a new revocation list, which the Lead Daemon's peer listener enforces from its next link and hands every Group member that renews, so no Group member accepts it again, and forgets it. One store transaction. A Daemon that is not a group's Lead Daemon fails with `not_lead_daemon`; the Lead Daemon's own name with `lead_cannot_leave`; a name with no Group member with `group_member_unknown`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GroupRevokeParams>,
    result: schema_of::<GroupMemberRevoked>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `group.lead`.
pub const GROUP_LEAD: Operation = Operation {
    name: "group.lead",
    description: "Reports whether this Lead Daemon has confirmed its lead epoch, and under `--confirm` confirms it by hand when no majority of its last-known Group members can be reached (ADR 0055). Hand confirmation warns that the former Lead Daemon must not restart with its lead configuration. A Daemon that is not a group's Lead Daemon fails with `not_lead_daemon`; one fenced off by a higher lead epoch with `lead_epoch_stale`.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<GroupLeadParams>,
    result: schema_of::<GroupLeadStatus>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// What `group.lead` takes.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupLeadParams {
    /// Confirms this Lead Daemon's lead epoch by hand when no majority of its
    /// last-known Group members can be reached.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub confirm: bool,
}

/// What `group.lead` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupLeadStatus {
    /// The Lead Daemon's Group member name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The lead epoch this Lead Daemon leads at.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Whether this Lead Daemon has confirmed its lead epoch and may act.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub confirmed: bool,
    /// Whether an operator confirmed it with `--confirm` rather than a
    /// majority of its last-known Group members.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub confirmed_by_operator: bool,
    /// The warning ADR 0055 requires after hand confirmation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub warning: Option<String>,
}

/// What `group.leave` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupLeaveParams {}

/// What `group.leave` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupLeft {
    /// The Group member name this Daemon had.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
}

/// What `group.revoke` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupRevokeParams {
    /// The Group member name to remove.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
}

/// What `group.revoke` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupMemberRevoked {
    /// The Group member name removed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The number of the revocation list that revokes its certificates.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revocation_list: u64,
}

/// What `group.init` takes: the Lead Daemon's Group member name and where
/// its peer listener listens.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupInitParams {
    /// The Lead Daemon's Group member name: one lowercase DNS label, such
    /// as its host's or pod's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The `host:port` its peer listener takes, which the Group members
    /// dial; port 0 takes a free one, which the answer names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub peer_address: String,
}

/// What `group.init` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupFormed {
    /// The group's lead epoch: 1, the first.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The `host:port` the peer listener took, which `group join` dials.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub peer_address: String,
    /// The SHA-256 of the group's certificate authority's public key, as
    /// `sha256/` and its standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub authority_key: String,
}

/// What `group.join` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GroupJoinParams {
    /// The `host:port` of the Lead Daemon's peer listener.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead: String,
    /// The join token `group token` minted on the Lead Daemon.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: DaemonToken,
    /// The SHA-256 of the group's certificate authority's public key, as
    /// `group token` answered it: `sha256/` and its standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub authority_key: String,
    /// The Group member name this Daemon takes: one lowercase DNS label,
    /// such as its host's or pod's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The `host:port` its own peer listener takes, which the other Group
    /// members dial; port 0 takes a free one, which the answer names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub peer_address: String,
}

/// What `group.join` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupJoined {
    /// The group's lead epoch.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The `host:port` the peer listener took.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub peer_address: String,
    /// When the Group member certificate the Lead Daemon signed stops
    /// being valid, in RFC 3339, UTC; the Daemon renews it before.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub certificate_expires_at: SystemTime,
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
    /// The highest lead epoch the Daemon has recorded; none until it forms
    /// or joins a group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: Option<LeadEpoch>,
    /// The join tokens minted and not yet redeemed, each by its SHA-256.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub join_tokens: Vec<StoredJoinToken>,
    /// This Daemon's own Group member identity; none until it forms or
    /// joins a group.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub identity: Option<StoredIdentity>,
    /// The Group members the Lead Daemon admitted, itself included; none
    /// on any other Daemon.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub members: Vec<StoredMember>,
    /// The Group member certificates the Lead Daemon signed and that have
    /// not expired, revoked or not; none on any other Daemon.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub certificates: Vec<StoredCertificate>,
}

/// A Group member certificate the Lead Daemon signed, as its
/// `certificates` row holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredCertificate {
    /// Its serial number, in lowercase hex.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub serial: String,
    /// The Group member name it names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// When it stops being valid, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub expires_at: SystemTime,
    /// When the authority revoked it, in RFC 3339, UTC; none while it is
    /// valid.
    #[serde(
        with = "crate::point_in_time::nullable",
        default,
        skip_serializing_if = "Option::is_none"
    )]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub revoked_at: Option<SystemTime>,
}

/// A Daemon's own Group member identity, as its `identity` row holds it.
/// Its `Debug` leaves the private key out.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredIdentity {
    /// Its Group member name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its private key, PKCS #8 DER in standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// Its Group member certificate, DER in standard base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub certificate: String,
    /// The group's certificate authority's certificate, DER in standard
    /// base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub authority: String,
    /// The authority's latest revocation list it holds, DER in standard
    /// base64.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub revocations: String,
    /// The `host:port` its peer listener takes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub peer_address: String,
    /// The `host:port` of the Lead Daemon's peer listener, its own on the
    /// Lead Daemon.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_address: String,
    /// The Lead Daemon's Group member name, its own on the Lead Daemon.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_name: String,
    /// When its certificate stops being valid, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub certificate_expires_at: SystemTime,
    /// When it asks for a new certificate: two thirds of the way from
    /// receiving this one to its expiry, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub renew_at: SystemTime,
}

impl std::fmt::Debug for StoredIdentity {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredIdentity")
            .field("name", &self.name)
            .field("key", &"[REDACTED]")
            .field("certificate", &self.certificate)
            .field("authority", &self.authority)
            .field("revocations", &self.revocations)
            .field("peer_address", &self.peer_address)
            .field("lead_address", &self.lead_address)
            .field("lead_name", &self.lead_name)
            .field("certificate_expires_at", &self.certificate_expires_at)
            .field("renew_at", &self.renew_at)
            .finish()
    }
}

/// A Group member the Lead Daemon admitted, as its `members` row holds it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct StoredMember {
    /// Its Group member name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The `host:port` of its peer listener.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub peer_address: String,
    /// When it joined, in RFC 3339, UTC.
    #[serde(with = "crate::point_in_time")]
    #[schemars(with = "PointInTime", extend("arrived_in" = "0.1"))]
    pub joined_at: SystemTime,
}

/// The group's certificate authority, as its `authority` row holds it. Its
/// `Debug` leaves the private key out.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
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

impl std::fmt::Debug for StoredAuthority {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("StoredAuthority")
            .field("name", &self.name)
            .field("key", &"[REDACTED]")
            .field("certificate", &self.certificate)
            .field("lists_issued", &self.lists_issued)
            .finish()
    }
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
    operations: &[
        GROUP_INIT,
        GROUP_TOKEN,
        GROUP_JOIN,
        GROUP_LEAVE,
        GROUP_REVOKE,
        GROUP_LEAD,
    ],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        LEAD_EPOCH_STALE,
        GROUP_EXISTS,
        NOT_LEAD_DAEMON,
        JOIN_TOKEN_REFUSED,
        GROUP_MEMBER_EXISTS,
        AUTHORITY_KEY_MISMATCH,
        GROUP_LEAD_UNREACHABLE,
        LEAD_CANNOT_LEAVE,
        GROUP_MEMBER_UNKNOWN,
        NOT_IN_A_GROUP,
        LEAD_UNCONFIRMED,
        GROUP_POLITENESS_UNSHARED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
