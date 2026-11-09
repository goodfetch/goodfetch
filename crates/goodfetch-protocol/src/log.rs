//! The engine's logs and traces as its Clients and operators reach them
//! (ADRs 0078, 0122 and 0132; RT-17, RT-18): the level a Connection asks for
//! in `hello` and changes with `connection.set_log_level`, the `$/log`
//! notifications the engine sends it at that level, telemetry that counts
//! the ones it dropped, and the settings the executable reads before it logs
//! (ADR 0122): `log.level`, `log.output`, `log.format`, `log.file.max_size`
//! and `log.file.max_files`, and `otlp.traces_endpoint`, which turns the
//! OTLP export of the engine's spans on.
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
    /// Every value, in declaration order.
    pub const ALL: [Self; 6] = [
        Self::Off,
        Self::Error,
        Self::Warn,
        Self::Info,
        Self::Debug,
        Self::Trace,
    ];

    /// Its `snake_case` name.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
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

fn info() -> Value {
    Value::String("info".to_owned())
}

fn ten_mebibytes() -> Value {
    serde_json::json!(10 * 1024 * 1024)
}

fn five() -> Value {
    serde_json::json!(5)
}

/// `log.level` (ADR 0122).
pub const LOG_LEVEL: Setting = Setting {
    name: "log.level",
    description: "Which of its logs an engine process writes: `info`, the default, which logs state changes only (a Run started, resumed or stopped, a domain backing off, a storage level changing, a Daemon draining) and never one line per request, or any `tracing` filter directive, such as `warn` or `info,goodfetch::rpc=debug`. Read when the process starts; set in user configuration or an Operator policy only.",
    kind: ValueKind::Text,
    default: info,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// The values of `log.output`.
pub const LOG_OUTPUTS: &[&str] = &["stdout", "file", "none"];

/// `log.output` (ADR 0122).
pub const LOG_OUTPUT: Setting = Setting {
    name: "log.output",
    description: "Where an engine process writes its logs beside stderr: `stdout`, JSON lines for the platform to collect and rotate, `file`, JSON lines in its own files of the Goodfetch home's log folder, rotated at `log.file.max_size` and kept to `log.file.max_files`, or `none`. Unset, it is `stdout` under `daemon run`, `file` for every other Daemon and `none` for an Attached engine, whose logs go to its Client as log events; `stdout` on an Attached engine, whose stdout is its protocol, fails with `setting_not_applicable`. Read when the process starts; set in user configuration or an Operator policy only.",
    kind: ValueKind::Choice(LOG_OUTPUTS),
    default: null,
    clearable: true,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `log.file.max_size` (ADR 0122).
pub const LOG_FILE_MAX_SIZE: Setting = Setting {
    name: "log.file.max_size",
    description: "The size, in bytes, at which an engine process's log file rotates where `log.output` is `file`: 10 MiB. A file holds at most this much and one line more. Read when the process starts; set in user configuration or an Operator policy only.",
    kind: ValueKind::Count,
    default: ten_mebibytes,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// `log.file.max_files` (ADR 0122).
pub const LOG_FILE_MAX_FILES: Setting = Setting {
    name: "log.file.max_files",
    description: "How many log files an engine process keeps where `log.output` is `file`, the one it writes included: 5, so a Daemon's logs stay under about 50 MiB. The oldest goes at each rotation. A process other than the Daemon writes a set of files of its own, and at its start keeps as many of its executable's sets, its own included, the ones written last. Read when the process starts; set in user configuration or an Operator policy only.",
    kind: ValueKind::Count,
    default: five,
    clearable: false,
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

/// `otlp.logs_endpoint` (ADRs 0016 and 0043).
pub const OTLP_LOGS_ENDPOINT: Setting = Setting {
    name: "otlp.logs_endpoint",
    description: "The OTLP/HTTP endpoint an engine process sends its OpenTelemetry events to, such as `http://localhost:4318/v1/logs`: today the Audit log's entries, while `audit.otlp` is on. Unset, the default, sends nothing, and nothing leaves the machine. Read when the process starts; set in user configuration or an Operator policy only.",
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
    settings: &[
        LOG_LEVEL,
        LOG_OUTPUT,
        LOG_FORMAT,
        LOG_FILE_MAX_SIZE,
        LOG_FILE_MAX_FILES,
        OTLP_TRACES_ENDPOINT,
        OTLP_LOGS_ENDPOINT,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};
