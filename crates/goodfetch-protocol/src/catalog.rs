//! What a catalog entry declares: an operation, an event, an Error code, a
//! Plugin kind, a metric, a Setting, a Hook stage or a value of a shared
//! type, with the markers and versions ADRs 0030, 0038, 0050 and 0095 give
//! it. Each module that defines entries lists them in one
//! `Entries` value, which the emitter reads.

use std::borrow::Cow;
use std::fmt;
use std::str::FromStr;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

mod setting;

pub use setting::{Authority, HostList, PriceValue, ValueKind, ValueShape};

/// The protocol's version, `major.minor` (ADR 0030), written on the wire as
/// that text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProtocolVersion {
    /// Raised by a breaking change.
    pub major: u16,
    /// Raised by an additive change.
    pub minor: u16,
}

impl fmt::Display for ProtocolVersion {
    /// `major.minor`, as the catalog and the SDKs write it.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

/// Text that is no `major.minor` version.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is no `major.minor` version")]
pub struct MalformedVersion(String);

impl FromStr for ProtocolVersion {
    type Err = MalformedVersion;

    /// Two decimal numbers without sign, leading zero or space, joined by a
    /// dot.
    fn from_str(text: &str) -> Result<Self, MalformedVersion> {
        let number = |part: &str| -> Option<u16> {
            let canonical = part == "0" || (!part.starts_with('0') && !part.is_empty());
            (canonical && part.bytes().all(|byte| byte.is_ascii_digit()))
                .then(|| part.parse().ok())
                .flatten()
        };
        let malformed = || MalformedVersion(text.to_owned());
        let (major, minor) = text.split_once('.').ok_or_else(malformed)?;
        Ok(Self {
            major: number(major).ok_or_else(malformed)?,
            minor: number(minor).ok_or_else(malformed)?,
        })
    }
}

impl Serialize for ProtocolVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for ProtocolVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ProtocolVersion {
    fn schema_name() -> Cow<'static, str> {
        "ProtocolVersion".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A version `major.minor` (ADR 0030).",
            "type": "string",
            "pattern": "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)$",
        })
    }
}

/// The version this crate speaks.
pub const PROTOCOL_VERSION: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// One of Goodfetch's two Products (ADR 0084), by its mark as the catalog
/// writes it.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum Product {
    /// Browser automation; Foxhound contains it.
    Collie,
    /// Crawling, which contains Collie.
    Foxhound,
}

impl Product {
    /// The smallest Product that carries an entry defined in `module`, a
    /// `module_path!()`: Foxhound for an item of the `foxhound` module, Collie
    /// for any other (ADR 0099).
    pub fn of_module(module: &str) -> Self {
        if module.split("::").any(|segment| segment == "foxhound") {
            Self::Foxhound
        } else {
            Self::Collie
        }
    }

    /// Whether this Product carries everything `other` carries: each
    /// carries itself, and Foxhound contains Collie (ADR 0084).
    pub fn contains(self, other: Self) -> bool {
        match self {
            Self::Collie => other == Self::Collie,
            Self::Foxhound => true,
        }
    }

    /// The mark as the catalog writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Collie => "collie",
            Self::Foxhound => "foxhound",
        }
    }

    /// Its name, as prose and type names write it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Collie => "Collie",
            Self::Foxhound => "Foxhound",
        }
    }
}

/// What calling an operation does (ADR 0011).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationClass {
    /// Changes nothing.
    Read,
    /// Changes state, and calling it again changes nothing more.
    IdempotentWrite,
    /// Acts on the world, and is never treated as undone (ADR 0010).
    Effect,
}

impl OperationClass {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::IdempotentWrite => "idempotent_write",
            Self::Effect => "effect",
        }
    }
}

/// The shape an operation takes on every surface (ADR 0099), from which
/// each generator applies its one form, such as a method in each SDK or a
/// CLI command. Whether a call can be cancelled is [`Operation::cancellable`],
/// which an operation of any kind may be.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OperationKind {
    /// A call answered once with its result, which may wait (ADR 0080).
    RoundTrip,
    /// A round trip that never waits, such as ADR 0085's questions
    /// (`is_visible`, `count`), so it takes no timeout.
    Query,
    /// A read the SDK answers itself on a handle, such as `page.url` or
    /// `run.id`.
    Read,
    /// A call that opens a value the caller then names by its ID, such as a
    /// Run: its result carries the new value's ID, `<namespace>_id`.
    Opener,
    /// A call that stays in flight and sends its answer as notifications,
    /// such as an event subscription or a body read in chunks (ADR 0081).
    Sequence,
    /// A wait for an application state, armed when it is called (ADR 0080).
    Waiter,
    /// A side activity a program listens to, such as a page's console
    /// messages (ADR 0081).
    Event,
}

impl OperationKind {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::RoundTrip => "round_trip",
            Self::Query => "query",
            Self::Read => "read",
            Self::Opener => "opener",
            Self::Sequence => "sequence",
            Self::Waiter => "waiter",
            Self::Event => "event",
        }
    }
}

/// Which surfaces offer an operation (ADRs 0011, 0089, 0092, 0099 and 0106).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reach {
    /// Every surface of its Product: the SDKs, the CLI, MCP, skills, Code
    /// mode and the Framework adapters.
    AllSurfaces,
    /// The SDKs only, to the program's own code: a Session's cookies and a
    /// page's storage (ADR 0092), `page.lend` (ADR 0089), `effect.open` and
    /// `effect.close` (ADR 0106), and the routes and their answers, whose
    /// handler is the program's own code (ADR 0085). No CLI command, MCP
    /// tool, skill, Code mode global or Framework adapter offers it.
    SdkOnly,
    /// A person's answer, given in the Console, the Kennel or an interactive
    /// CLI prompt only (ADR 0011's list, such as a certificate's acceptance
    /// or a Limit raise; ADR 0141): no agent surface offers it, and the CLI
    /// offers it as a prompt, never as a generated command.
    PersonOnly,
}

impl Reach {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::AllSurfaces => "all_surfaces",
            Self::SdkOnly => "sdk_only",
            Self::PersonOnly => "person_only",
        }
    }
}

/// The canonical names of the entries ADRs 0089, 0092 and 0106 keep to the
/// SDKs, which the catalog marks [`Reach::SdkOnly`] when it defines them.
/// Each surface generator other than the SDKs' refuses to offer an entry of
/// these names, so one added without the marker fails its checks.
pub const SDK_ONLY: [&str; 8] = [
    "session.cookies",
    "session.add_cookies",
    "session.clear_cookies",
    "page.local_storage",
    "page.session_storage",
    "page.lend",
    "effect.open",
    "effect.close",
];

/// Whether an entry is under semver's promise (ADR 0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stability {
    /// Under semver.
    Stable,
    /// Outside semver until it is promoted.
    Experimental,
}

impl Stability {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Experimental => "experimental",
        }
    }
}

/// An entry's deprecation (ADR 0030).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Deprecation {
    /// The version that deprecated it.
    pub since: ProtocolVersion,
    /// What to use instead, or why it goes.
    pub note: &'static str,
}

/// Registers a type's JSON Schema with the bundle's generator and returns
/// the reference to it.
pub type SchemaOf = fn(&mut SchemaGenerator) -> Schema;

/// The [`SchemaOf`] of `T`.
pub fn schema_of<T: JsonSchema>(generator: &mut SchemaGenerator) -> Schema {
    generator.subschema_for::<T>()
}

/// An operation of the Operation catalog (ADR 0038).
pub struct Operation {
    /// `<namespace>.<member>` in `snake_case` words (ADR 0099).
    pub name: &'static str,
    /// The one text of its SDK docstrings, MCP tool description and
    /// reference page.
    pub description: &'static str,
    /// What calling it does.
    pub class: OperationClass,
    /// The shape it takes on every surface.
    pub kind: OperationKind,
    /// The surfaces that offer it.
    pub reach: Reach,
    /// The type of its params.
    pub params: SchemaOf,
    /// The type of its result.
    pub result: SchemaOf,
    /// The events a call may send, each one of its module's or another's
    /// `Entries::events`.
    pub events: &'static [&'static Event],
    /// Whether a Client may cancel a call in flight.
    pub cancellable: bool,
    /// Its semver status.
    pub stability: Stability,
    /// Its deprecation, if any.
    pub deprecated: Option<Deprecation>,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// How an event travels when a subscriber reads slower than the engine
/// produces (ADR 0024).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EventClass {
    /// Never dropped: while a subscription's queue is full, its producer
    /// waits (axis changes, "Needs you" items).
    Lossless,
    /// Coalesced: a newer event of the same kind and scope replaces an
    /// unsent one, which the one sent counts as `dropped` (progress, logs).
    Telemetry,
}

impl EventClass {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Lossless => "lossless",
            Self::Telemetry => "telemetry",
        }
    }
}

/// An event the engine sends.
pub struct Event {
    /// Its `snake_case` name, dotted by namespace.
    pub name: &'static str,
    /// What it reports.
    pub description: &'static str,
    /// How it travels to a slow subscriber.
    pub class: EventClass,
    /// The type of its payload.
    pub payload: SchemaOf,
    /// Its semver status.
    pub stability: Stability,
    /// Its deprecation, if any.
    pub deprecated: Option<Deprecation>,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// A request the engine sends to a handler of the program over the
/// Connection that serves it, whose reply is the handler's answer (ADRs 0024
/// and 0069): the program's own code, never a Plugin.
pub struct HandlerRequest {
    /// `<namespace>.<member>` in `snake_case` words (ADR 0099).
    pub name: &'static str,
    /// What the handler is asked to do.
    pub description: &'static str,
    /// The type of its params.
    pub params: SchemaOf,
    /// The type of the handler's answer.
    pub result: SchemaOf,
    /// Its semver status.
    pub stability: Stability,
    /// Its deprecation, if any.
    pub deprecated: Option<Deprecation>,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// The seven groups of Error codes, named for their remedy (ADR 0079).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ErrorGroup {
    /// The program is wrong.
    Program,
    /// A policy, a Limit, a lock or a person refused.
    Refused,
    /// The page or site never reached the state the call needed.
    Page,
    /// The Model provider's answer was unusable.
    Model,
    /// The deadline passed.
    DeadlineExceeded,
    /// Something Goodfetch depends on is unreachable or gone.
    Unavailable,
    /// A bug in Goodfetch.
    Internal,
}

impl ErrorGroup {
    /// Every group, in the order ADR 0079 lists them.
    pub const ALL: [Self; 7] = [
        Self::Program,
        Self::Refused,
        Self::Page,
        Self::Model,
        Self::DeadlineExceeded,
        Self::Unavailable,
        Self::Internal,
    ];

    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Program => "program",
            Self::Refused => "refused",
            Self::Page => "page",
            Self::Model => "model",
            Self::DeadlineExceeded => "deadline_exceeded",
            Self::Unavailable => "unavailable",
            Self::Internal => "internal",
        }
    }

    /// Its JSON-RPC numeric class, in the range JSON-RPC 2.0 leaves to
    /// servers (ADRs 0038 and 0079).
    pub fn json_rpc_class(self) -> i32 {
        match self {
            Self::Program => -32001,
            Self::Refused => -32002,
            Self::Page => -32003,
            Self::Model => -32004,
            Self::DeadlineExceeded => -32005,
            Self::Unavailable => -32006,
            Self::Internal => -32007,
        }
    }

    /// Its row of ADR 0069's exit-code table: the exit code of a CLI command
    /// that fails with one of its codes that has no row of its own.
    pub fn exit_code(self) -> u8 {
        match self {
            Self::Internal => 1,
            Self::Program => 3,
            Self::Refused => 4,
            Self::Page => 5,
            Self::Model => 6,
            Self::DeadlineExceeded => 7,
            Self::Unavailable => 8,
        }
    }

    /// What it means, for the generated error classes' docs.
    pub fn description(self) -> &'static str {
        match self {
            Self::Program => {
                "The program is wrong: its call, its settings or its own handler code. Fix the program."
            }
            Self::Refused => {
                "A policy, a Limit, a lock or a person refused. Change a setting, raise the Limit or ask the operator."
            }
            Self::Page => {
                "The page or site never reached the state the call needed. Read the Run trace; change the step, the wait or the Access mode."
            }
            Self::Model => {
                "The Model provider's answer was unusable. Change the model or the strategy."
            }
            Self::DeadlineExceeded => "The deadline passed. Raise it, or wait for the right thing.",
            Self::Unavailable => {
                "Something Goodfetch depends on is unreachable or gone. Retry, or fix the setup."
            }
            Self::Internal => "A bug in Goodfetch. Report it.",
        }
    }
}

/// An Error code (ADR 0038).
pub struct ErrorCodeDefinition {
    /// The code's `snake_case` string.
    pub name: &'static str,
    /// What went wrong.
    pub description: &'static str,
    /// Its group, whose numeric class the wire carries.
    pub group: ErrorGroup,
    /// Whether the same call may succeed when made again.
    pub retryable: bool,
    /// Its own row of ADR 0069's exit-code table, the exit code of a CLI
    /// command that fails with it; `None` takes its group's.
    pub exit_code: Option<u8>,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// One request the engine sends to an implementation of a Plugin kind.
pub struct PluginMethod {
    /// Its `snake_case` name.
    pub name: &'static str,
    /// What the implementation does.
    pub description: &'static str,
    /// The type of its params.
    pub params: SchemaOf,
    /// The type of its result.
    pub result: SchemaOf,
}

/// A Plugin kind (ADR 0033), whose SDK handlers get a generated base class.
pub struct PluginKind {
    /// Its `snake_case` name.
    pub name: &'static str,
    /// What its implementations do.
    pub description: &'static str,
    /// The requests the engine sends to an implementation.
    pub methods: &'static [PluginMethod],
    /// Its semver status.
    pub stability: Stability,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// The kind of OpenTelemetry instrument a metric is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Instrument {
    /// A sum that only grows.
    Counter,
    /// A sum that grows and shrinks.
    UpDownCounter,
    /// A distribution of measurements.
    Histogram,
    /// A value sampled when read.
    Gauge,
}

impl Instrument {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Counter => "counter",
            Self::UpDownCounter => "up_down_counter",
            Self::Histogram => "histogram",
            Self::Gauge => "gauge",
        }
    }
}

/// A metric, whose name is a versioned contract (ADR 0050).
pub struct Metric {
    /// Its OpenTelemetry name under `goodfetch.`.
    pub name: &'static str,
    /// What it measures.
    pub description: &'static str,
    /// Its instrument.
    pub instrument: Instrument,
    /// Its UCUM unit, such as `s` or `{request}`.
    pub unit: &'static str,
    /// Its labels (OpenTelemetry's attributes), each with the bounded set
    /// of values it takes (ADR 0050).
    pub labels: &'static [MetricLabel],
    /// Its semver status.
    pub stability: Stability,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// A label of a metric and every value it takes: a bounded set, never a
/// URL, a domain or a Run ID (ADR 0050).
pub struct MetricLabel {
    /// Its `snake_case` name.
    pub name: &'static str,
    /// Every value it takes.
    pub values: &'static [&'static str],
}

/// A Setting (ADR 0028), named by its key, which ADR 0087 refuses on a
/// Product that lacks it: the kind of its values, its built-in default and
/// who may set it, from which the settings schema is generated (ADR 0083).
pub struct Setting {
    /// Its key, a dotted `snake_case` path with its area first, such as
    /// `crawl.max_pages` (ADR 0083).
    pub name: &'static str,
    /// What the setting shapes.
    pub description: &'static str,
    /// The kind of its values.
    pub kind: ValueKind,
    /// Its built-in default, the lowest layer; `null` when it has none.
    pub default: fn() -> serde_json::Value,
    /// Whether `null` clears it (ADR 0083's three states).
    pub clearable: bool,
    /// Which layers may set it, and how.
    pub authority: Authority,
    /// The values that weaken a guarantee class or switch off a costly
    /// control, which the results it touches are marked with (ADR 0028);
    /// `None` when no value does.
    pub weakens: Option<fn(&serde_json::Value) -> bool>,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// A Hook stage: a fixed stage of a request, a navigation, a page, a Record
/// or a run's lifecycle where Hooks run (ADR 0033).
pub struct HookStage {
    /// Its `snake_case` name.
    pub name: &'static str,
    /// The stage the Hooks run at.
    pub description: &'static str,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// A type both Products share, to which each may add values (ADR 0095).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SharedType {
    /// What Usage counts.
    UsageKind,
    /// What a Limit applies to.
    LimitScope,
    /// What a Notification reports.
    NotificationEvent,
    /// Why a site refused a page (ADR 0035).
    BlockCause,
    /// What an Audit log entry records (ADR 0016).
    AuditKind,
    /// What a Run report's warning says (ADR 0102).
    RunWarningCode,
    /// Why a Run is not `complete` (ADRs 0029 and 0102).
    CompletenessReasonCode,
    /// What waits for a person in "Needs you" (ADR 0031).
    NeedsYouKind,
}

impl SharedType {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UsageKind => "usage_kind",
            Self::LimitScope => "limit_scope",
            Self::NotificationEvent => "notification_event",
            Self::BlockCause => "block_cause",
            Self::AuditKind => "audit_kind",
            Self::RunWarningCode => "run_warning_code",
            Self::CompletenessReasonCode => "completeness_reason_code",
            Self::NeedsYouKind => "needs_you_kind",
        }
    }
}

/// A value of a type both Products share, such as a Usage kind.
pub struct SharedTypeValue {
    /// The type it is a value of.
    pub shared_type: SharedType,
    /// The value's `snake_case` name.
    pub name: &'static str,
    /// What it means.
    pub description: &'static str,
    /// The version that added it.
    pub arrived_in: ProtocolVersion,
}

/// Every entry one module defines.
pub struct Entries {
    /// The `module_path!()` of the module, which gives each entry's Product
    /// (ADR 0099).
    pub defined_in: &'static str,
    /// Its operations.
    pub operations: &'static [Operation],
    /// Its events.
    pub events: &'static [Event],
    /// The requests the engine sends to the program's handlers.
    pub handler_requests: &'static [HandlerRequest],
    /// Its Error codes.
    pub error_codes: &'static [ErrorCodeDefinition],
    /// Its Plugin kinds.
    pub plugin_kinds: &'static [PluginKind],
    /// Its metrics.
    pub metrics: &'static [Metric],
    /// Its Settings.
    pub settings: &'static [Setting],
    /// Its Hook stages.
    pub hook_stages: &'static [HookStage],
    /// The values it adds to types both Products share.
    pub shared_type_values: &'static [SharedTypeValue],
}

#[cfg(test)]
mod tests;
