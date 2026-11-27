//! Group-wide Usage ledger slices, the *group redirect* row, erasure
//! broadcasts, token checks and Lead Daemon reachability contracts (ADRs 0041,
//! 0046 and 0055, issue 258).

use std::time::SystemTime;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::LeadEpoch;
use crate::catalog::{
    ErrorCodeDefinition, ErrorGroup, Event, EventClass, Product, ProtocolVersion, Stability,
    schema_of,
};
use crate::connection::ProductVersion;
use crate::point_in_time::PointInTime;
use crate::run::{LimitScope, RunId, RunLimit};
use crate::usage::{Limit, LimitRaiseRefusedDetails, LimitUsage};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The method a Group member sends the Lead Daemon, on a peer link opened with
/// its certificate, in place of [`super::HELLO`], to read Limits, take or
/// return Slices, reclaim Slices, or mutate group Limits (ADRs 0041 and 0055).
pub const GROUP_LEDGER: &str = "group.ledger";

/// The method used on the peer link to broadcast or apply an erasure of a
/// discarded Run or deleted Saved identity across every Group member's local
/// stores, including the Response cache (ADRs 0046 and 0055).
pub const GROUP_ERASURE: &str = "group.erasure";

/// The method a Group member sends the Lead Daemon to check a named token's
/// SHA-256 digest (ADR 0055).
pub const GROUP_TOKEN_CHECK: &str = "group.token.check";

/// The `scope_kind` value written to the per-user Usage ledger `limits` table
/// by a Group member so engines on its machine route slices, Politeness
/// configuration and policy through it; flagged *must understand* so older
/// engines fail closed with `limit_not_understood` (ADRs 0041 and 0055).
pub const GROUP_REDIRECT_SCOPE: &str = "group_redirect";

/// The current format version carried inside a *group redirect* row's payload.
pub const GROUP_REDIRECT_FORMAT_VERSION: u32 = 1;

/// The Error code raised when the Lead Daemon (or a machine's Group member via
/// its *group redirect* row) cannot be reached for a new Run needing a Slice,
/// a new Slice, or a new/expired token check (ADR 0055).
pub const LEAD_UNREACHABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "lead_unreachable",
    description: "The Daemon group's Lead Daemon, or the local Group member named by this machine's group redirect row, cannot be reached to check or grant a Usage ledger Slice or verify a sign-in: live Runs continue spending held Slices until they need a new one, and new Runs needing a Slice or sign-in wait until the Lead Daemon or Group member returns (or `group leave` removes the redirect row).",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Payload of `group.lead_reachability_changed` (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupLeadReachabilityChanged {
    /// Whether the Lead Daemon is reachable now.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reachable: bool,
    /// The Lead Daemon's Group member name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_name: String,
    /// Why the Lead Daemon became unreachable, when `reachable` is `false`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: Option<String>,
}

/// Lossless event published whenever a Group member's view of Lead Daemon
/// reachability changes (ADRs 0024 and 0055).
pub const GROUP_LEAD_REACHABILITY_CHANGED: Event = Event {
    name: "group.lead_reachability_changed",
    description: "A Group member lost contact with its Lead Daemon or regained it (ADR 0055). Readiness on `/readyz` stays ready while the Lead Daemon is unreachable; live Runs keep spending held Slices until they need a new one.",
    class: EventClass::Lossless,
    payload: schema_of::<GroupLeadReachabilityChanged>,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Serializable wire mirror of a stored Usage ledger Limit row.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireStoredLimit {
    /// Row primary key in the Lead Daemon's ledger.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: i64,
    /// Limit scope.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope: LimitScope,
    /// Usage kind capped by this Limit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// Optional ISO 4217 currency code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// Optional period length wire name (`day`, `week`, `month`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub period: Option<String>,
    /// Optional IANA time zone name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time_zone: Option<String>,
    /// Cap amount per period (or total).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: u64,
    /// Whether unknown engines must refuse runs covered by this row.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub must_understand: bool,
    /// Who set this row (`person`, `managed_policy`, `client`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub set_by: String,
    /// Optional managed Operator policy major.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub managed_major: Option<String>,
    /// RFC 3339 timestamp when set.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub set_at: String,
}

/// Wire mirror of a Slice holder.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireHolder {
    /// Run ID holding the Slice.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: String,
    /// Run epoch under which the Slice was taken.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub epoch: u64,
    /// Run state folder path on its owning Group member.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub folder: String,
}

/// Wire mirror of a granted Slice.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireSlice {
    /// Slice row ID in the Lead Daemon's ledger.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: i64,
    /// Granted Slice size.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
}

/// Wire mirror of Slice sizing parameters.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct WireSliceSizing {
    /// Largest Slice share in thousandths of the Limit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub share: u64,
    /// Smallest Slice share in thousandths of the Limit.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub minimum: u64,
}

/// One action performed against the Lead Daemon's Usage ledger (or via a
/// machine's Group member local redirect endpoint).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PeerLedgerAction {
    /// Read all Limit rows.
    Limits,
    /// Read remaining budget of `stored` at `at` (RFC 3339).
    Left {
        /// Limit row to query.
        #[schemars(extend("arrived_in" = "0.1"))]
        stored: WireStoredLimit,
        /// Timestamp (RFC 3339) locating the period.
        #[schemars(extend("arrived_in" = "0.1"))]
        at: String,
    },
    /// Take one Slice in one transaction on the Lead Daemon's ledger.
    TakeSlice {
        /// Limit row to draw from.
        #[schemars(extend("arrived_in" = "0.1"))]
        stored: WireStoredLimit,
        /// Run taking the Slice.
        #[schemars(extend("arrived_in" = "0.1"))]
        holder: WireHolder,
        /// Timestamp (RFC 3339) locating the period.
        #[schemars(extend("arrived_in" = "0.1"))]
        at: String,
        /// Slice sizing parameters.
        #[schemars(extend("arrived_in" = "0.1"))]
        sizing: WireSliceSizing,
        /// Minimum size required (for a worst-case reservation).
        #[schemars(extend("arrived_in" = "0.1"))]
        needing: u64,
    },
    /// Return unused portions of held Slices, charging what was used.
    GiveBack {
        /// `(slice_id, used)` pairs.
        #[schemars(extend("arrived_in" = "0.1"))]
        used: Vec<(i64, u64)>,
    },
    /// Read every Limit and its Usage (`usage.list`).
    Usage {
        /// Caller's Product.
        #[schemars(extend("arrived_in" = "0.1"))]
        product: Product,
        /// Timestamp (RFC 3339) locating current periods.
        #[schemars(extend("arrived_in" = "0.1"))]
        at: String,
    },
    /// Add a Limit row (`usage.set`).
    AddLimit {
        /// Limit to add.
        #[schemars(extend("arrived_in" = "0.1"))]
        limit: Limit,
        /// Caller's Product.
        #[schemars(extend("arrived_in" = "0.1"))]
        product: Product,
        /// Timestamp (RFC 3339).
        #[schemars(extend("arrived_in" = "0.1"))]
        at: String,
    },
    /// Raise or set a person Limit (`usage.raise`).
    Raise {
        /// Limit to raise/set.
        #[schemars(extend("arrived_in" = "0.1"))]
        limit: Limit,
        /// Caller's Product.
        #[schemars(extend("arrived_in" = "0.1"))]
        product: Product,
        /// Timestamp (RFC 3339).
        #[schemars(extend("arrived_in" = "0.1"))]
        at: String,
    },
    /// Check Run-level Limits against standing ones.
    CheckRunLimits {
        /// Run ID.
        #[schemars(extend("arrived_in" = "0.1"))]
        run_id: String,
        /// Given Run Limits.
        #[schemars(extend("arrived_in" = "0.1"))]
        given: Vec<RunLimit>,
    },
    /// Record Run-level Limits on start.
    GiveRunLimits {
        /// Run ID.
        #[schemars(extend("arrived_in" = "0.1"))]
        run_id: String,
        /// Given Run Limits.
        #[schemars(extend("arrived_in" = "0.1"))]
        given: Vec<RunLimit>,
        /// Whether this start began the Run.
        #[schemars(extend("arrived_in" = "0.1"))]
        began: bool,
        /// Timestamp (RFC 3339).
        #[schemars(extend("arrived_in" = "0.1"))]
        at: String,
    },
    /// Report reclaimed Slice charges from an owning Group member.
    Reclaim {
        /// `(slice_id, charged, state)` entries (`reclaimed` or `review`).
        #[schemars(extend("arrived_in" = "0.1"))]
        charges: Vec<(i64, u64, String)>,
    },
}

/// Reply payload for [`PeerLedgerAction`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PeerLedgerReply {
    /// Answer to `Limits`.
    Limits {
        /// All stored Limit rows.
        #[schemars(extend("arrived_in" = "0.1"))]
        limits: Vec<WireStoredLimit>,
    },
    /// Answer to `Left`.
    Left {
        /// Remaining budget.
        #[schemars(extend("arrived_in" = "0.1"))]
        left: u64,
    },
    /// Answer to `TakeSlice`.
    Slice {
        /// Granted Slice, or `None` when the Limit has too little left.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        slice: Option<WireSlice>,
    },
    /// Answer to `GiveBack`.
    GivenBack,
    /// Answer to `Usage`.
    Usage {
        /// Every Limit with its current Usage.
        #[schemars(extend("arrived_in" = "0.1"))]
        limits: Vec<LimitUsage>,
    },
    /// Answer to `AddLimit` or `Raise`.
    Limit {
        /// The added or updated Limit with its Usage.
        #[schemars(extend("arrived_in" = "0.1"))]
        limit: LimitUsage,
    },
    /// Answer to `CheckRunLimits` or `GiveRunLimits`.
    RunLimits {
        /// Optional refusal when a Run Limit was raised.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        refused: Option<LimitRaiseRefusedDetails>,
    },
    /// Answer to `Reclaim`.
    Reclaimed,
}

/// Parameters for `group.ledger` over mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerLedgerParams {
    /// Peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// Calling Group member's Product.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Calling Group member's Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// Highest lead epoch recorded by the caller.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Calling Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// Ledger operation to execute on the Lead Daemon.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub action: PeerLedgerAction,
}

/// Result of `group.ledger` over mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerLedgerResult {
    /// Settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// Highest lead epoch recorded by the responder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Reply payload.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reply: PeerLedgerReply,
}

/// What a group erasure deletes across every Group member's local stores
/// (ADRs 0046 and 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ErasureTarget {
    /// A discarded Run: remove its Response cache entries and local Run index
    /// presence on every Group member.
    Run {
        /// Discarded Run ID.
        #[schemars(extend("arrived_in" = "0.1"))]
        run_id: RunId,
    },
    /// A deleted Saved identity: remove its Response cache partition and any
    /// local cached copy on every Group member.
    SavedIdentity {
        /// Deleted Saved identity name.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
    },
}

/// Parameters for `group.erasure` over mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerErasureParams {
    /// Peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// Calling Group member's Product.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Calling Group member's Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// Highest lead epoch recorded by the caller.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Calling Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// `true` when a Group member asks the Lead Daemon to broadcast the
    /// erasure to every member; `false` when the Lead Daemon instructs a Group
    /// member to apply the erasure locally and acknowledge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub broadcast: bool,
    /// What to erase.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: ErasureTarget,
}

/// Result of `group.erasure` over mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerErasureResult {
    /// Settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// Highest lead epoch recorded by the responder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Group members that have applied and acknowledged the erasure.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub acknowledged: Vec<String>,
    /// Group members that were unreachable and remain pending delivery.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pending: Vec<String>,
}

/// Parameters for `group.token.check` over mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerTokenCheckParams {
    /// Peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// Calling Group member's Product.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Calling Group member's Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// Highest lead epoch recorded by the caller.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Calling Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// Lowercase hex SHA-256 of the named token to check.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token_sha256: String,
}

/// Result of `group.token.check` over mutual TLS.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerTokenCheckResult {
    /// Settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// Highest lead epoch recorded by the responder.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Whether the token is currently valid.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub valid: bool,
    /// Named token's name, when valid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token_name: Option<String>,
    /// Optional expiry of the token, in RFC 3339, UTC.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[serde(with = "crate::point_in_time::nullable")]
    #[schemars(with = "Option<PointInTime>", extend("arrived_in" = "0.1"))]
    pub expires_at: Option<SystemTime>,
}

/// Payload stored inside the `scope_id` column of a *group redirect* row in
/// `usage-ledger.sqlite` (ADRs 0041 and 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct GroupRedirectRow {
    /// Format version of this redirect payload (`1`). Any newer version fails
    /// closed with `limit_not_understood`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub format_version: u32,
    /// Product of the Group member (ADR 0095).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Product major line (for example `"0.0"`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_major: String,
    /// Name of the Group member writing this row.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member_name: String,
    /// Local loopback `host:port` endpoint served by the Group member.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub endpoint: String,
    /// Path to the `0600` bearer token file authenticating local redirect calls.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token_file: String,
    /// Last renewed heartbeat timestamp (RFC 3339, UTC).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub heartbeat_at: String,
    /// Maximum age of `heartbeat_at` before Attached engines fail closed with
    /// `lead_unreachable`, in milliseconds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub heartbeat_ttl_ms: u64,
}

/// Configuration snapshot served by a Group member to Attached engines on its
/// machine over the local redirect endpoint (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RedirectConfig {
    /// Group member's name (for `[group_member.<name>]` resolution).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member_name: String,
    /// Cached Operator policy TOML text, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub operator_policy: Option<String>,
    /// Cached user configuration TOML text, if present.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub user_configuration: Option<String>,
    /// Effective Politeness sharing mode, if configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub politeness_sharing: Option<String>,
    /// Effective Politeness store URL, if configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub politeness_store: Option<String>,
}

/// Request sent by an Attached engine to its machine's Group member over the
/// local redirect endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RedirectRequest {
    /// Bearer token read from the `0600` token file named by the redirect row.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: String,
    /// Action requested.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub action: RedirectAction,
}

/// Action carried in [`RedirectRequest`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedirectAction {
    /// Forward a Usage ledger action to the group's ledger.
    Ledger {
        /// Ledger action to execute.
        #[schemars(extend("arrived_in" = "0.1"))]
        action: Box<PeerLedgerAction>,
    },
    /// Fetch the Group member's cached policy and Politeness configuration.
    Config,
}

/// Reply returned by a Group member's local redirect endpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RedirectReply {
    /// Ledger reply.
    Ledger {
        /// Reply payload.
        #[schemars(extend("arrived_in" = "0.1"))]
        reply: PeerLedgerReply,
    },
    /// Configuration reply.
    Config {
        /// Cached policy and Politeness configuration.
        #[schemars(extend("arrived_in" = "0.1"))]
        config: RedirectConfig,
    },
    /// Refusal carrying a Goodfetch Error code (`lead_unreachable`, etc.).
    Refused {
        /// Error code name.
        #[schemars(extend("arrived_in" = "0.1"))]
        code: String,
        /// Human-readable message.
        #[schemars(extend("arrived_in" = "0.1"))]
        message: String,
    },
}
