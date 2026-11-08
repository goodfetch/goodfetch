//! The Audit log (ADRs 0016, 0055 and 0067): one per-user, append-only
//! record of the security events of every run, in Goodfetch home outside
//! the per-major folders, and of the Usage ledger's format family (ADR
//! 0066). Each entry is chained to the one before it: its hash is
//! `SHA-256(previous hash ‖ RFC 8785 canonical JSON of the entry)`, the
//! first entry's previous hash being 32 zero bytes, and the chain's head is
//! also kept in a separate head file, so an edit, a deletion, a reordering
//! or a truncation shows. `audit.verify` answers whether the log is intact,
//! or the first sequence where it is not.
//!
//! `audit.export` answers every entry with its hash, which the CLI writes
//! as JSONL, one entry per line in sequence, and an engine whose
//! `audit.otlp` is on also sends each new entry to the OTLP collector
//! `otlp.logs_endpoint` names as an OpenTelemetry event. `audit.retention`
//! prunes the log from its beginning only: the entries older than it give
//! way to an anchor entry carrying the last pruned hash, from which the
//! rest still verifies, and a pruning record chained at the tail names
//! that anchor, so an anchor rewritten or moved shows too.
//!
//! The kinds of entry are the values of the shared type `audit_kind`, each
//! marked with the Product of the module that lists it, so that every area
//! registers its own; this module lists the security area's and the
//! Audit log's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::catalog::{
    Authority, Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Setting,
    SharedType, SharedTypeValue, Stability, ValueKind, schema_of,
};
use crate::connection::Surface;
use crate::run::{PointInTime, RunId};
use crate::settings::SettingsLayer;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// Who an entry's event was done by.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Actor {
    /// A person, by the name their account or OS user goes by.
    Person {
        /// Their name.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
    },
    /// A named token (area 12), by its name.
    NamedToken {
        /// Its name.
        #[schemars(extend("arrived_in" = "0.1"))]
        name: String,
    },
    /// A Client, by the surface its `hello` named.
    ClientSurface {
        /// Its surface.
        #[schemars(extend("arrived_in" = "0.1"))]
        surface: Surface,
    },
    /// The engine itself.
    Engine,
}

/// One entry of the Audit log, as its hash covers it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditEntry {
    /// Its place in the chain, from 1.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sequence: u64,
    /// When it was recorded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub time: PointInTime,
    /// Its kind, a value of the shared type `audit_kind`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// Who did what it records.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub actor: Actor,
    /// The surface the actor acted through, such as `console` for a person
    /// in the Console; none for the engine's own events.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub surface: Option<Surface>,
    /// The settings layer its event came from, such as an Operator policy
    /// that switched a control off; none where no layer applies.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub layer: Option<SettingsLayer>,
    /// The Run it happened in; none outside a Run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<RunId>,
    /// What its kind records, such as a Secret reference and the domain it
    /// was sent to, never a secret's value.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub details: Value,
    /// Whether its hash chains it to the entry before it: `false` for an
    /// entry of a Run whose `security.audit_chain` was off, whose hash is
    /// `SHA-256` of 32 zero bytes and its own canonical JSON alone, and
    /// which `audit verify` steps over while it verifies the chain around
    /// it; left out when chained.
    #[serde(default = "chained", skip_serializing_if = "is_chained")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub chained: bool,
}

/// An entry's `chained` when left out.
const fn chained() -> bool {
    true
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes the field by reference"
)]
const fn is_chained(chained: &bool) -> bool {
    *chained
}

/// What `audit.verify` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditVerifyParams {}

/// What `audit.verify` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditVerified {
    /// Whether every entry chains to the one before it and the head file
    /// agrees with the last one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub intact: bool,
    /// How many entries the log holds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entries: u64,
    /// The first sequence that does not verify: an entry edited, missing
    /// or out of its place, or, past the last entry, a tail the head file
    /// says was there; none when intact.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub first_broken: Option<u64>,
}

/// One entry of an exported Audit log, one line of its JSONL: the entry
/// and the hash that chains it to the line before it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ExportedAuditEntry {
    /// The entry, as its hash covers it.
    #[serde(flatten)]
    pub entry: AuditEntry,
    /// `SHA-256(previous hash ‖ RFC 8785 canonical JSON of the entry)`, in
    /// lowercase hex; an anchor's is the last pruned entry's.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hash: String,
}

/// What `audit.export` takes: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct AuditExportParams {}

/// What `audit.export` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AuditExported {
    /// Every entry the log holds, in sequence, each with its hash: the
    /// anchor first when the log was pruned.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entries: Vec<ExportedAuditEntry>,
}

/// `audit.export`.
pub const AUDIT_EXPORT: Operation = Operation {
    name: "audit.export",
    description: "Exports the user's Audit log: every entry in sequence, each with the hash that chains it to the one before it, the anchor first when `audit.retention` pruned its beginning. The CLI's `audit export` writes them as JSONL, one entry per line, which `audit verify --file` verifies off the machine.",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::AllSurfaces,
    params: schema_of::<AuditExportParams>,
    result: schema_of::<AuditExported>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn none() -> Value {
    Value::Null
}

fn off() -> Value {
    Value::Bool(false)
}

/// `audit.retention` (ADR 0016).
pub const AUDIT_RETENTION: Setting = Setting {
    name: "audit.retention",
    description: "How long the user's Audit log keeps an entry, in milliseconds, such as 15552000000 for the six months the EU AI Act asks of high-risk deployers. Unset, the default, nothing is ever pruned. Set, each append first prunes the entries older than it from the log's beginning only, leaving an anchor entry that carries the last pruned hash, from which `audit verify` checks the rest, and a pruning record at the tail. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Duration,
    default: none,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `audit.otlp` (ADR 0016).
pub const AUDIT_OTLP: Setting = Setting {
    name: "audit.otlp",
    description: "Whether an engine process also sends each new Audit log entry, the hash with it, as an OpenTelemetry event to the OTLP/HTTP collector `otlp.logs_endpoint` names: off by default, and nothing is sent while `otlp.logs_endpoint` is unset. Read when the process starts; set in user configuration or an Operator policy only.",
    kind: ValueKind::Boolean,
    default: off,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `audit.verify`.
pub const AUDIT_VERIFY: Operation = Operation {
    name: "audit.verify",
    description: "Verifies the user's Audit log: every entry's hash chains it to the one before it, and the head file names the last one. Answers that it is intact, or the first sequence that does not verify, so an edited, deleted, reordered or truncated entry, a truncated tail included, shows.",
    class: OperationClass::Read,
    kind: OperationKind::Query,
    reach: Reach::AllSurfaces,
    params: schema_of::<AuditVerifyParams>,
    result: schema_of::<AuditVerified>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// A kind of Audit log entry: a value of the shared type `audit_kind`.
const fn kind(name: &'static str, description: &'static str) -> SharedTypeValue {
    SharedTypeValue {
        shared_type: SharedType::AuditKind,
        name,
        description,
        arrived_in: ARRIVED_IN,
    }
}

/// A Secret reference resolved and used.
pub const SECRET_REFERENCE_USED: SharedTypeValue = kind(
    "secret_reference_used",
    "A Secret reference was resolved and used: its placeholder, its reference and the domain that received it, never its value.",
);

/// A security control switched off.
pub const SECURITY_SWITCH_TURNED_OFF: SharedTypeValue = kind(
    "security_switch_turned_off",
    "A security control was switched off for a run, once per run, with its setting and the layer that set it (ADR 0016).",
);

/// Private addresses allowed.
pub const PRIVATE_ADDRESSES_ALLOWED: SharedTypeValue = kind(
    "private_addresses_allowed",
    "A run allowed private addresses through `egress.private_ranges`, with the ranges it listed.",
);

/// A listener of the endpoint table (ADR 0067), by the name an
/// `endpoint_authentication_failed` entry and its log record give it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Listener {
    /// An Attached engine's stdio.
    Stdio,
    /// A Daemon's socket or named pipe.
    DaemonSocket,
    /// A Daemon's exposed address (`daemon.address`).
    ExposedDaemon,
    /// An Attached engine's private endpoint.
    PrivateEndpoint,
    /// A Daemon's WebSocket listener.
    DaemonWebsocket,
    /// The foreground Daemon's ops listener (ADR 0050).
    OpsListener,
    /// The Console's listener.
    Console,
    /// The Console's content origin.
    ContentOrigin,
    /// A Session's egress relay.
    EgressRelay,
    /// A Session's upstream proxy relay.
    ProxyRelay,
}

impl Listener {
    /// Every value, in declaration order.
    pub const ALL: [Self; 10] = [
        Self::Stdio,
        Self::DaemonSocket,
        Self::ExposedDaemon,
        Self::PrivateEndpoint,
        Self::DaemonWebsocket,
        Self::OpsListener,
        Self::Console,
        Self::ContentOrigin,
        Self::EgressRelay,
        Self::ProxyRelay,
    ];

    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// Why a listener refused an attempt, by its endpoint table's rule.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum RefusalReason {
    /// A Connection's first message was no `hello`.
    HelloNotFirst,
    /// No token came.
    TokenMissing,
    /// The token is not the listener's.
    TokenWrong,
    /// The token has expired.
    TokenExpired,
    /// The request's `Host` names no address or `localhost`.
    HostForeign,
    /// The request carries an `Origin` the listener does not allow.
    OriginForeign,
    /// The request carries no `Origin` where one is required.
    OriginMissing,
    /// The address tried to sign in more often than allowed.
    SignInRateExceeded,
}

impl RefusalReason {
    /// Every value, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::HelloNotFirst,
        Self::TokenMissing,
        Self::TokenWrong,
        Self::TokenExpired,
        Self::HostForeign,
        Self::OriginForeign,
        Self::OriginMissing,
        Self::SignInRateExceeded,
    ];

    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// The details of an `endpoint_authentication_failed` entry: which
/// listener refused, and why. They never hold a token.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EndpointRefusal {
    /// The listener that refused.
    pub listener: Listener,
    /// Why.
    pub reason: RefusalReason,
}

/// A listening endpoint refused a request.
pub const ENDPOINT_AUTHENTICATION_FAILED: SharedTypeValue = kind(
    "endpoint_authentication_failed",
    "A listening endpoint refused a request for a missing, wrong, expired or revoked token, a foreign `Host` or a foreign `Origin`.",
);

/// A raw CDP client attached.
pub const RAW_CDP_ATTACHED: SharedTypeValue = kind(
    "raw_cdp_attached",
    "A raw CDP client attached to a Session's browser.",
);

/// A Lent page attached.
pub const LENT_PAGE_ATTACHED: SharedTypeValue = kind(
    "lent_page_attached",
    "A page was lent to the program's own Playwright client (ADR 0089).",
);

/// A certificate pinned on first use.
pub const CERTIFICATE_PINNED: SharedTypeValue = kind(
    "certificate_pinned",
    "A host's certificate was pinned on first use under its domain's exception, with the SHA-256 of its public key (ADR 0042).",
);

/// A changed certificate accepted.
pub const CERTIFICATE_CHANGE_ACCEPTED: SharedTypeValue = kind(
    "certificate_change_accepted",
    "A person accepted a pinned host's changed certificate, with both hashes (ADR 0042).",
);

/// A pinned host forgotten.
pub const CERTIFICATE_PIN_FORGOTTEN: SharedTypeValue = kind(
    "certificate_pin_forgotten",
    "A person removed their pin of a host, and any change waiting for it, so its next connection under trust on first use pins again (ADR 0042).",
);

/// Any certificate accepted for a domain.
pub const CERTIFICATE_ANY_USED: SharedTypeValue = kind(
    "certificate_any_used",
    "A run reached a host whose domain accepts any certificate, `any` (ADR 0042).",
);

/// The certificate exceptions one run used.
pub const CERTIFICATE_EXCEPTIONS_USED: SharedTypeValue = kind(
    "certificate_exceptions_used",
    "The certificate exceptions a run used, once per run, by host and kind (ADR 0042).",
);

/// A Session exported.
pub const SESSION_EXPORTED: SharedTypeValue = kind(
    "session_exported",
    "A Session was exported to an encrypted bundle.",
);

/// A Session imported.
pub const SESSION_IMPORTED: SharedTypeValue = kind(
    "session_imported",
    "A Session was imported from an encrypted bundle.",
);

/// TLS terminated upstream of an Exposed Daemon.
pub const TLS_TERMINATED_UPSTREAM: SharedTypeValue = kind(
    "tls_terminated_upstream",
    "An Exposed Daemon started with `daemon.tls.upstream`, TLS terminated in front of it (ADR 0049).",
);

/// An Exposed Daemon set up.
pub const EXPOSED_DAEMON_SET_UP: SharedTypeValue = kind(
    "exposed_daemon_set_up",
    "An Exposed Daemon's setup completed through its setup link (ADR 0049).",
);

/// A value that presses a site harder than its default.
pub const PRESSING_HARDER_VALUE_SET: SharedTypeValue = kind(
    "pressing_harder_value_set",
    "A run set a value that presses a site harder than its default, with the setting and its value (ADR 0086).",
);

/// The anchor a pruning leaves.
pub const AUDIT_LOG_ANCHOR: SharedTypeValue = kind(
    "audit_log_anchor",
    "The first entry of a pruned Audit log, in place of the entries `audit.retention` pruned: the last pruned sequence, its hash, from which the rest chains, and the time and layer of the pruning that left it.",
);

/// A pruning of the Audit log's beginning.
pub const AUDIT_LOG_PRUNED: SharedTypeValue = kind(
    "audit_log_pruned",
    "`audit.retention` pruned the Audit log's beginning: the last pruned sequence and its hash, which the anchor carries, and the retention, with the layer that set it.",
);

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[AUDIT_EXPORT, AUDIT_VERIFY],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[AUDIT_RETENTION, AUDIT_OTLP],
    hook_stages: &[],
    shared_type_values: &[
        AUDIT_LOG_ANCHOR,
        AUDIT_LOG_PRUNED,
        SECRET_REFERENCE_USED,
        SECURITY_SWITCH_TURNED_OFF,
        PRIVATE_ADDRESSES_ALLOWED,
        ENDPOINT_AUTHENTICATION_FAILED,
        RAW_CDP_ATTACHED,
        LENT_PAGE_ATTACHED,
        CERTIFICATE_PINNED,
        CERTIFICATE_CHANGE_ACCEPTED,
        CERTIFICATE_PIN_FORGOTTEN,
        CERTIFICATE_ANY_USED,
        CERTIFICATE_EXCEPTIONS_USED,
        SESSION_EXPORTED,
        SESSION_IMPORTED,
        TLS_TERMINATED_UPSTREAM,
        EXPOSED_DAEMON_SET_UP,
        PRESSING_HARDER_VALUE_SET,
    ],
};
