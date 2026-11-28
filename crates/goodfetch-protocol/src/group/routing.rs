//! Group Client routing, headroom reporting, run placement, Live view
//! redirect/relay and event stream forwarding contracts (ADRs 0055, 0056 and
//! 0149, issue 244).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::value::RawValue;

use super::LeadEpoch;
use crate::catalog::{ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};
use crate::connection::{EngineInstance, Form, ProductVersion, Surface};
use crate::daemon::DaemonToken;
use crate::governor::GovernorLevel;
use crate::jsonrpc::RequestId;
use crate::live_view::LiveViewScreen;
use crate::page::SessionId;
use crate::run::{RunId, RunIndexRow};
use crate::settings::session_source::SessionSource;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The method a Group member sends another Group member over the peer link to
/// forward a unary Client call for a Run owned by that peer (ADR 0055).
pub const GROUP_RPC_FORWARD: &str = "group.rpc.forward";

/// The method a Group member sends the Lead Daemon over the peer link to look
/// up the current owning Group member (and its peer address and
/// `advertise_url`) for a Run or Session (ADR 0055).
pub const GROUP_RUN_INDEX_LOOKUP: &str = "group.run_index.lookup";

/// The method a Group member sends the Lead Daemon over the peer link when
/// its Resource governor headroom or capabilities change noticeably (ADR 0055).
pub const GROUP_HEADROOM_REPORT: &str = "group.headroom.report";

/// The method a Group member sends the Lead Daemon over the peer link to place
/// a new or resumed Run onto an eligible Group member (ADRs 0055 and 0056).
pub const GROUP_RUN_PLACE: &str = "group.run.place";

/// The method a Group member sends the owning Group member over the peer link
/// to mint a single-use Live view channel token in its memory for a redirect
/// (ADRs 0055 and 0149).
pub const GROUP_LIVE_VIEW_MINT: &str = "group.live_view.mint_token";

/// The method a Group member sends the owning Group member over the peer link
/// to relay a Live view channel when the owner has no `daemon.advertise_url`
/// (ADR 0055).
pub const GROUP_LIVE_VIEW_RELAY: &str = "group.live_view.relay";

/// The method a Group member sends the owning Group member over the peer link
/// to forward an `events.subscribe` stream when the owner has no
/// `daemon.advertise_url` (ADR 0055).
pub const GROUP_EVENTS_SUBSCRIBE: &str = "group.events.subscribe";

/// The method a draining Group member (or `runs adopt`) sends the destination
/// Group member over the peer link to hand off a Run's Checkpoint, resume it
/// at the next run epoch, and transfer ownership in the group Run index
/// (ADR 0055).
pub const GROUP_RUN_HANDOFF: &str = "group.run.handoff";

/// The method a Group member sends the Lead Daemon over the peer link when
/// starting up (restart or re-add) to list the non-ended Runs the group Run
/// index still assigns to it (ADR 0055).
pub const GROUP_RUN_INDEX_LIST_ASSIGNED: &str = "group.run_index.list_assigned";

/// The method a Group member sends the Lead Daemon over the peer link to list
/// all eligible Group members satisfying a [`PlacementRequest`] (ADRs 0054 and
/// 0055).
pub const GROUP_MEMBERS_ELIGIBLE: &str = "group.members.eligible";

/// The method a Group member sends another Group member over the peer link to
/// invoke a contribution-registered peer extension handler (ADRs 0054, 0055
/// and 0087).
pub const GROUP_PEER_EXTENSION: &str = "group.peer.extension";

/// The Error code answered by a former owner when a Run has moved to another
/// Group member (ADR 0055).
pub const RUN_MOVED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "run_moved",
    description: "The Run moved to another Group member, so this Group member no longer owns it: the receiving Group member refreshes its cached group Run index and retries once, and a Client that followed a redirect reconnects through any Group member and retries once.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `run_moved` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunMovedDetails {
    /// The Run that moved.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Group member that now owns the Run, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub owner: Option<String>,
}

/// What a Group member can run and how Clients reach it directly (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemberCapabilities {
    /// Installed browser families/names available on this Group member.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browsers: Vec<String>,
    /// Installed Plugins available on this Group member.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugins: Vec<String>,
    /// Configured local egress IP addresses on this Group member (ADR 0044).
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub local_addresses: Vec<String>,
    /// Local model servers available on this Group member.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub local_model_servers: Vec<String>,
    /// Machine-bound Session sources or machine-bound imports hosted on this
    /// Group member (ADR 0056).
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_sources: Vec<String>,
    /// This Group member's `daemon.advertise_url`, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub advertise_url: Option<String>,
    /// This Group member's Product version (for minor >= run state checks).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
}

impl Default for MemberCapabilities {
    fn default() -> Self {
        Self {
            browsers: Vec::new(),
            plugins: Vec::new(),
            local_addresses: Vec::new(),
            local_model_servers: Vec::new(),
            session_sources: Vec::new(),
            advertise_url: None,
            product_version: ProductVersion::parse("0.1.0")
                .expect("invariant: 0.1.0 is a valid Product version"),
        }
    }
}

/// A Group member's Resource governor headroom snapshot (ADRs 0040 and 0055).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemberHeadroom {
    /// Available HTTP fetch concurrency slots (`target - active`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub http_available: u32,
    /// Available browser page concurrency slots (`target - active`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_available: u32,
    /// Available model call concurrency slots (`target - active`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub model_available: u32,
    /// Current governor pressure level.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub level: GovernorLevel,
    /// Whether this Group member is ready to accept new runs.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ready: bool,
}

impl Default for MemberHeadroom {
    fn default() -> Self {
        Self {
            http_available: 0,
            browser_available: 0,
            model_available: 0,
            level: GovernorLevel::Normal,
            ready: true,
        }
    }
}

/// What a Group member sends in `group.headroom.report` when its headroom or
/// capabilities change noticeably (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerHeadroomReportParams {
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
    /// Its Group member name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// Current governor headroom.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub headroom: MemberHeadroom,
    /// Current capabilities and `advertise_url`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub capabilities: MemberCapabilities,
}

/// What the Lead Daemon answers to `group.headroom.report`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerHeadroomReportResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
}

/// What a Group member asks the Lead Daemon in `group.run_index.lookup`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunIndexLookupParams {
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
    /// The requesting Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The Run ID being looked up, when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// The Session ID being looked up, when looking up a Live view Session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_id: Option<SessionId>,
}

/// Routing metadata for one Group member.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemberRoute {
    /// The owning Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The `host:port` of the owning Group member's peer listener.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub peer_address: String,
    /// The owning Group member's `daemon.advertise_url`, when set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub advertise_url: Option<String>,
}

/// What the Lead Daemon answers to `group.run_index.lookup`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunIndexLookupResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The owning Group member's route, or `None` when no Group member owns it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub route: Option<MemberRoute>,
}

/// Inputs used by the Lead Daemon to place a Run (ADRs 0055 and 0056).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlacementRequest {
    /// The Group member holding the Client/SDK Connection that received the
    /// start or resume call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub receiving_member: String,
    /// Explicit `group_member` pin requested by the caller.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub group_member: Option<String>,
    /// Minimum Product version required by the run state for a resumed or
    /// handed-off run (minor must be at least this version's minor within the
    /// same major).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_state_version: Option<ProductVersion>,
    /// Installed browsers required by this run.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub required_browsers: Vec<String>,
    /// Installed Plugins required by this run.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub required_plugins: Vec<String>,
    /// Local egress addresses required by this run (ADR 0044).
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub required_local_addresses: Vec<String>,
    /// Local model servers required by this run.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub required_local_model_servers: Vec<String>,
    /// Session source requested by this run, if any (ADR 0056).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session_source: Option<SessionSource>,
    /// Whether the Session source is served by a person's Browser host (which
    /// allows placing the run on any Group member).
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_host_served: bool,
    /// Group member name owning a machine-bound import or machine-bound source
    /// when known ahead of placement.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub machine_bound_member: Option<String>,
    /// Whether the run uses `client_browser` (`run.borrow()`), which must run
    /// on the Group member holding the SDK's connection (ADR 0056).
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub client_browser: bool,
    /// Group members excluded from placement (for placing distinct Partitions).
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exclude_members: Vec<String>,
}

/// What a Group member sends in `group.run.place`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunPlaceParams {
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
    /// The requesting Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// Placement constraints and hints.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request: PlacementRequest,
}

/// What the Lead Daemon answers to `group.run.place`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunPlaceResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The selected Group member and its route.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub placed: MemberRoute,
}

/// What a Group member sends in `group.members.eligible`: the same placement
/// request payload as [`PeerRunPlaceParams`].
pub type PeerMembersEligibleParams = PeerRunPlaceParams;

/// What the Lead Daemon answers to `group.members.eligible`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerMembersEligibleResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// All eligible Group members ordered by headroom (best first).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub members: Vec<MemberRoute>,
}

/// What a Group member sends in `group.peer.extension`: an authenticated,
/// lead-epoch-fenced envelope carrying a contribution's peer method and JSON
/// payload (ADR 0087).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerExtensionParams {
    /// The peer protocol version the sender speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The Product it runs, which must be the group's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// The highest lead epoch the sender has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The sending Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The contribution-registered peer method name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub extension_method: String,
    /// The JSON payload for the extension handler.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub payload: serde_json::Value,
}

/// What a Group member answers to `group.peer.extension`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerExtensionResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the receiver has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The JSON response returned by the extension handler.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub result: serde_json::Value,
}

/// What a Group member sends to the owning Group member in `group.rpc.forward`
/// (ADRs 0055, 0132 and 0140): raw JSON params are passed straight through
/// without decoding into a `Value` copy, keeping `_meta` Trace context intact.
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PeerRpcForwardParams {
    /// The peer protocol version the sender speaks.
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
    /// The forwarding Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The Client's JSON-RPC request ID, preserved on the owner's response.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request_id: RequestId,
    /// The JSON-RPC operation method being forwarded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub method: String,
    /// The Run ID targeted by this forwarded call, when forwarding for an
    /// existing Run (used by the receiver to answer `run_moved` if ownership
    /// moved away).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// The Client's raw JSON params object, passed verbatim with `_meta` intact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Value>", extend("arrived_in" = "0.1"))]
    pub params: Option<Box<RawValue>>,
    /// The Client's surface.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub surface: Surface,
    /// The Client's error-name spelling form.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub form: Form,
    /// Whether the Client reached the forwarding Group member behind a gateway.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub behind_gateway: bool,
    /// Whether the forwarding Connection answers as a person.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub answers_as_a_person: bool,
}

/// What the owning Group member answers to `group.rpc.forward`: the raw
/// JSON-RPC reply object produced by its local dispatch, written straight back
/// to the Client without decoding into a copy or encoding again (ADR 0140).
#[derive(Clone, Debug, Serialize, Deserialize, JsonSchema)]
pub struct PeerRpcForwardResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the owner has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The owner's raw JSON-RPC response object (`result` or `error`) bearing
    /// the caller's `request_id`.
    #[schemars(with = "serde_json::Value", extend("arrived_in" = "0.1"))]
    pub response: Box<RawValue>,
}

/// What a Group member sends the owning Group member in
/// `group.live_view.mint_token` to obtain a single-use Live view redirect token
/// (ADRs 0055 and 0149).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerLiveViewMintParams {
    /// The peer protocol version the sender speaks.
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
    /// The requesting Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The Session the viewer wants to watch.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session: SessionId,
}

/// What the owning Group member answers to `group.live_view.mint_token`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerLiveViewMintResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the owner has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The single-use channel token valid in the owner's memory for `session`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: DaemonToken,
    /// The owner's `daemon.advertise_url`, if set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub advertise_url: Option<String>,
}

/// What a Group member sends to open a relayed Live view stream over the peer
/// link when the owner has no `daemon.advertise_url` (ADR 0055).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct PeerLiveViewRelayParams {
    /// The peer protocol version the sender speaks.
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
    /// The forwarding Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The Session to watch.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub session: SessionId,
    /// The viewer's initial screen dimensions.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub screen: LiveViewScreen,
}

/// What a Group member sends to forward an `events.subscribe` stream over the
/// peer link when the owner has no `daemon.advertise_url` (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerEventsSubscribeParams {
    /// The peer protocol version the sender speaks.
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
    /// The forwarding Group member's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The Run whose events are being subscribed to, if scoped.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// Engine instance of the last event read, when resuming a subscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after_engine_instance: Option<EngineInstance>,
    /// Position of the last event read, when resuming a subscription.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after_position: Option<u64>,
    /// Whether to include `engine.system_info` events.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub system_info: bool,
}

/// What a draining Group member or `runs adopt` sends over `group.run.handoff`
/// to transfer a Run's Checkpoint to the destination Group member (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunHandoffParams {
    /// The peer protocol version the sender speaks.
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
    /// The sending Group member's authenticated name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The Run being handed off.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The Group member that previously owned the Run (for audit/logging).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source_member: Option<String>,
    /// The self-contained SQLite Checkpoint bytes, standard Base64 encoded
    /// (its run data key stays wrapped by the shared Secret provider, ADR 0055).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub checkpoint_base64: String,
    /// Lowercase hex SHA-256 digest of the decoded Checkpoint bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sha256: String,
    /// Whether this handoff comes from `runs adopt` rather than a live drain.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub adopt: bool,
}

/// What the destination Group member answers to `group.run.handoff` once it
/// has installed the Checkpoint, resumed the Run at the next run epoch, and
/// claimed ownership in the Lead Daemon's group Run index (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunHandoffResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the destination Group member has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// The Run that was resumed and claimed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The destination Group member's name (the Run's new owner).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub owner: String,
    /// The new run epoch taken when the destination Group member resumed it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_epoch: u64,
}

/// What a Group member sends the Lead Daemon on `group.run_index.list_assigned`
/// on startup to learn which non-ended Runs the group Run index still assigns
/// to it (ADR 0055).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunIndexListAssignedParams {
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
    /// The calling Group member's authenticated name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub member: String,
    /// The Group member name whose assigned non-ended Runs to return.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub for_member: String,
}

/// What the Lead Daemon answers to `group.run_index.list_assigned`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PeerRunIndexListAssignedResult {
    /// The settled peer protocol version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The highest lead epoch the Lead Daemon has recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lead_epoch: LeadEpoch,
    /// Non-ended Run index rows currently assigned to `for_member`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rows: Vec<RunIndexRow>,
}
