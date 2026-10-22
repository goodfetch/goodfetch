//! The engine's logs and traces as its Clients and operators reach them
//! (ADRs 0078, 0122 and 0132; RT-17, RT-18): the level a Connection asks for
//! in `hello` and changes with `connection.set_log_level`, the `$/log`
//! notifications the engine sends it at that level, telemetry that counts
//! the ones it dropped, and the settings the executable reads before it logs:
//! `log.format` and `otlp.traces_endpoint`, which turns the OTLP export of
//! the engine's spans on.
//!
//! Span names, span fields and the engine's `tracing` targets are
//! Experimental, outside semver like log text (ADR 0132).
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::{
    Authority, Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Setting,
    Stability, ValueKind, schema_of,
};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The notification that carries one of the engine's log events.
pub const LOG: &str = "$/log";

/// The level of the engine's log events a Connection receives: those at
/// it and above; `off`, the default, receives none.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum LogLevel {
    /// No log event.
    #[default]
    Off,
    /// Errors only.
    Error,
    /// Warnings and errors.
    Warn,
    /// State changes, warnings and errors: what `info` logs.
    Info,
    /// Also a line per request, fetch or command.
    Debug,
    /// Everything the engine logs.
    Trace,
}

impl LogLevel {
    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Error => "error",
            Self::Warn => "warn",
            Self::Info => "info",
            Self::Debug => "debug",
            Self::Trace => "trace",
        }
    }
}

/// What `$/log` carries: one log event of the engine.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogEvent {
    /// Its level, never `off`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub level: LogLevel,
    /// Where in the engine it comes from, such as `goodfetch::rpc::dispatch`:
    /// Experimental (ADR 0132).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub target: String,
    /// Its message.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// Its fields, by name: Experimental (ADR 0132).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fields: Map<String, Value>,
    /// How many log events the Connection was too slow to receive since the
    /// one before: logs are telemetry, and never slow a Run.
    #[serde(default, skip_serializing_if = "is_zero")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dropped: u64,
}

#[expect(
    clippy::trivially_copy_pass_by_ref,
    reason = "serde's `skip_serializing_if` passes a reference"
)]
fn is_zero(count: &u64) -> bool {
    *count == 0
}

/// What `connection.set_log_level` takes.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SetLogLevelParams {
    /// The Connection's new level.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub log_level: LogLevel,
}

/// What `connection.set_log_level` answers: nothing more.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct LogLevelSet {}

/// `connection.set_log_level`.
pub const CONNECTION_SET_LOG_LEVEL: Operation = Operation {
    name: "connection.set_log_level",
    description: "Sets the level of the engine's log events this Connection receives as `$/log` notifications, from the next one on: `off`, `error`, `warn`, `info`, `debug` or `trace`, as `hello`'s `log_level` set it first. Each Connection has its own. Log events are telemetry: one the Connection is too slow to receive is dropped, and the next counts it in `dropped`.",
    class: OperationClass::IdempotentWrite,
    kind: OperationKind::RoundTrip,
    reach: Reach::SdkOnly,
    params: schema_of::<SetLogLevelParams>,
    result: schema_of::<LogLevelSet>,
    events: &[],
    cancellable: false,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

fn null() -> Value {
    Value::Null
}

/// `log.format` (ADR 0122).
pub const LOG_FORMAT: Setting = Setting {
    name: "log.format",
    description: "How an engine process writes its logs on stderr, or on stdout under `daemon run`: `text`, lines for a person reading a terminal, or `json`, JSON lines. Unset, the logs are text on stderr and JSON lines on stdout under `daemon run`; a log file holds JSON lines whatever it says. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Choice(&["text", "json"]),
    default: null,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `otlp.traces_endpoint` (ADRs 0043 and 0132; RT-18).
pub const OTLP_TRACES_ENDPOINT: Setting = Setting {
    name: "otlp.traces_endpoint",
    description: "The OTLP/HTTP endpoint an engine process exports its `tracing` spans to, such as `http://localhost:4318/v1/traces`; unset, the default, exports nothing, and nothing leaves the machine. The spans cover each run, page and CDP command, with glossary IDs and bounded values as fields and a URL as its origin. Span names, span fields and the engine's targets are Experimental: a minor may change them, and its release notes list the change. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Text,
    default: null,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[CONNECTION_SET_LOG_LEVEL],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[LOG_FORMAT, OTLP_TRACES_ENDPOINT],
    hook_stages: &[],
    shared_type_values: &[],
};
