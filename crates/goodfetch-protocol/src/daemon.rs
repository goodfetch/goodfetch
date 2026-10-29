//! The Daemon's contracts (ADRs 0024, 0030 and 0098): its runtime file, by
//! which a Client of its major finds it, the `daemon.*` operations a Client
//! asks it about itself with, its idle timeout, the per-user service that
//! `daemon install` registers, its health as its ops listener's probes and
//! `daemon health` check it (ADR 0050), and the `--json` outputs of
//! `daemon start`, `stop`, `restart`, `status`, `install`, `run`,
//! `uninstall`, `rotate-token`, `http-listener-token` and `health`.
//! One Daemon per user and major serves both Products, so everything here
//! is Collie's.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    Product, ProtocolVersion, Reach, Setting, Stability, ValueKind, schema_of,
};
pub use crate::connection::DaemonEndpoint;
use crate::connection::ProductVersion;
use crate::console::{ConsoleFailure, ConsoleState};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A token a Daemon or its HTTP listener takes, which only the command
/// whose output it is prints: plain text in JSON and in its schema, and
/// `[REDACTED]` in `Debug`, so no log line or failed assertion shows it
/// (STYLE.md).
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent, inline)]
pub struct DaemonToken(String);

impl DaemonToken {
    /// The token `text`.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Its text, for the `hello` or the output that carries it.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for DaemonToken {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("DaemonToken([REDACTED])")
    }
}

/// The Daemon's runtime file, `daemon.json` in its per-major folder, mode
/// `0600` (ADR 0024): where it listens, the token its Clients' `hello`
/// carries, its Product version and its process.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DaemonRuntimeFile {
    /// Where it listens.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub endpoint: DaemonEndpoint,
    /// The per-launch token every Client's `hello` carries.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: DaemonToken,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: ProductVersion,
    /// Its process ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pid: u32,
    /// Where its Console is served, with the port it bound when
    /// `console.address` asks for port 0.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub console: ConsoleState,
}

fn ten_minutes() -> serde_json::Value {
    serde_json::json!(10_u64 * 60 * 1000)
}

/// `daemon.idle_timeout` (ADR 0024): how long a Daemon with no connected
/// Client waits before it stops.
pub const IDLE_TIMEOUT: Setting = Setting {
    name: "daemon.idle_timeout",
    description: "How long a Daemon with no connected Client, a visible Console tab included, waits before it stops: 10 minutes. `daemon start --keep-alive`, a held Schedule and an installed service keep it running. Set in user configuration or an Operator policy only.",
    kind: ValueKind::Duration,
    default: ten_minutes,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// The foreground Daemon's ports: a release major's is this plus the major.
const RELEASE_PORT_BASE: u16 = 7900;
/// A `0.y` preview's is this plus the minor: each preview minor counts as a
/// major (ADR 0030).
const PREVIEW_PORT_BASE: u16 = 7600;

/// Where `daemon run` of `version`'s major listens by default: `127.0.0.1`
/// on port 7900 plus the major, or 7600 plus the minor during `0.y`
/// previews, so the foreground Daemons of two majors never share a port.
pub fn default_address(version: &ProductVersion) -> SocketAddr {
    version.loopback_of_its_major(RELEASE_PORT_BASE, PREVIEW_PORT_BASE)
}

fn address_of_this_version() -> serde_json::Value {
    let address = ProductVersion::parse(env!("CARGO_PKG_VERSION")).map_or_else(
        |_| SocketAddr::from((Ipv4Addr::LOCALHOST, RELEASE_PORT_BASE)),
        |version| default_address(&version),
    );
    serde_json::Value::String(address.to_string())
}

/// `daemon.address` (ADR 0049): where the foreground Daemon listens.
pub const ADDRESS: Setting = Setting {
    name: "daemon.address",
    description: "Where `daemon run`, the foreground Daemon, listens, as `host:port`: `127.0.0.1` on port 7900 plus the major (7600 plus the minor during `0.y` previews). Port 0 takes a free port, which its log names. Only a loopback address, until the Daemon terminates TLS. Set in user configuration or an Operator policy, or for `daemon run` through `GOODFETCH_DAEMON_ADDRESS`.",
    kind: ValueKind::Text,
    default: address_of_this_version,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

fn hundred_and_ten_seconds() -> serde_json::Value {
    serde_json::json!(110_u64 * 1000)
}

/// `daemon.drain_grace` (ADR 0049): how long the foreground Daemon drains.
pub const DRAIN_GRACE: Setting = Setting {
    name: "daemon.drain_grace",
    description: "How long `daemon run`, the foreground Daemon, drains on `SIGTERM` before it exits: 110 seconds, below the 120-second `terminationGracePeriodSeconds` of the Helm chart, which must exceed it. Each run that reaches its next safe point in that time stops with a Checkpoint, resumable by its Run ID; one that does not stays resumable from its run file. Set in user configuration or an Operator policy, or for `daemon run` through `GOODFETCH_DAEMON_DRAIN_GRACE` in milliseconds.",
    kind: ValueKind::Duration,
    default: hundred_and_ten_seconds,
    clearable: false,
    authority: Authority::Machine,
    weakens: None,
    arrived_in: ARRIVED_IN,
};

/// A run started or resumed on a draining foreground Daemon.
pub const DAEMON_DRAINING: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "daemon_draining",
    description: "The foreground Daemon is draining on `SIGTERM`: it takes no run and checkpoints every live run as resumable, then exits. Start or resume the run on the Daemon that replaces it.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `daemon_draining` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonDrainingDetails {
    /// The Run ID of the run refused, when the call named one.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: Option<String>,
}

/// A `daemon.*` operation called on an engine that is no Daemon.
pub const NOT_A_DAEMON: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "not_a_daemon",
    description: "The call asks a Daemon about itself, and this engine is an Attached engine. Ask the Daemon of the major, found through its runtime file, or run `daemon status`.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// No Daemon a CLI command could reach (ADRs 0024 and 0069): nothing
/// answers where `GOODFETCH_URL` or the runtime file says it listens, it
/// closed the Connection before it answered `hello`, it went away while
/// the command connected, or its files cannot be read or its pick
/// started.
pub const DAEMON_UNREACHABLE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "daemon_unreachable",
    description: "No Daemon could be reached, so nothing was called: nothing answers where `GOODFETCH_URL` or its runtime file says it listens, it closed the Connection before it answered `hello`, as one does for a wrong `GOODFETCH_TOKEN`, it went away while the command connected, or its files cannot be read or its major's pick cannot be started. The message says which, and the `fix` what to do.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `daemon_unreachable` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonUnreachableDetails {
    /// What to do, such as starting the Daemon `GOODFETCH_URL` names or
    /// checking `GOODFETCH_TOKEN`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
}

/// What `not_a_daemon` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotADaemonDetails {
    /// The operation called.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub operation: String,
}

/// What `daemon.status` and `daemon.stop` take: nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct DaemonParams {}

/// What `daemon.status` answers: the running Daemon's state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonState {
    /// The Product the Daemon runs as.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// Its process ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pid: u32,
    /// Where it listens.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub endpoint: DaemonEndpoint,
    /// How many Clients are connected, the caller and Consoles included.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub clients: u32,
    /// How many of its Runs are live.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub live_runs: u32,
    /// Whether it never stops idle: `--keep-alive`, a held Schedule or an
    /// installed service.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kept_alive: bool,
    /// Its `daemon.idle_timeout`, in milliseconds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub idle_timeout: u64,
    /// Where its Console is served, or why it is not.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub console: ConsoleState,
}

/// What `daemon.stop` answers, before the Daemon stops.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStopping {
    /// How many live Runs it stops, each resumable from its own state.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub live_runs: u32,
}

/// `daemon.status`.
pub const STATUS: Operation = Operation {
    name: "daemon.status",
    description: "Says what the Daemon runs as, where it listens, how many Clients and live Runs it has, and when it stops idle.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<DaemonParams>,
    result: schema_of::<DaemonState>,
    events: &[],
    cancellable: false,
    stability: Stability::Stable,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `daemon.stop`.
pub const STOP: Operation = Operation {
    name: "daemon.stop",
    description: "Stops the Daemon once it answered: its live Runs stop, each resumable from its own state, and its runtime file and socket are removed.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<DaemonParams>,
    result: schema_of::<DaemonStopping>,
    events: &[],
    cancellable: false,
    stability: Stability::Stable,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Why a Daemon cannot take a new Run now, as its ops listener's
/// `/readyz` and `daemon health` check it (ADR 0050).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum Unready {
    /// It drains, taking no Run (ADR 0049).
    Draining,
    /// Its run state cannot be written.
    RunStateNotWritable,
    /// A browser its configuration uses is not installed.
    BrowsersMissing,
    /// Its Resource governor is at its last resort: memory stayed critical
    /// at the floor past `governor.last_resort_after` (ADR 0040).
    GovernorLastResort,
    /// The filesystem holding its run state, or the Goodfetch home's, is
    /// at the `critical` storage level, where a new Run's start is refused
    /// (ADR 0122).
    StorageCritical,
}

/// What `daemon.health` answers: what the ops listener's `/livez` and
/// `/readyz` check (ADR 0050). In a Daemon group, readiness ignores the
/// Lead Daemon's reachability.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonHealth {
    /// Whether it is live: its runtime and its Runs' registry, which writes
    /// their run state, respond.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub live: bool,
    /// Whether it is ready: live, and able to take a new Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ready: bool,
    /// Why it is not ready, when it is not; empty when it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub unready: Vec<Unready>,
}

/// `daemon.health`.
pub const HEALTH: Operation = Operation {
    name: "daemon.health",
    description: "Says whether the Daemon is live and ready to take a new Run, and why not: what its ops listener's `/livez` and `/readyz` answer with a status alone.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<DaemonParams>,
    result: schema_of::<DaemonHealth>,
    events: &[],
    cancellable: false,
    stability: Stability::Stable,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Which of the ops listener's probes `daemon health` answers.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum HealthProbe {
    /// `/livez`'s: whether the Daemon is live.
    Live,
    /// `/readyz`'s: whether it is ready to take a new Run.
    Ready,
}

/// What `daemon health --json` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonHealthOutput {
    /// The probe it answers.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub probe: HealthProbe,
    /// Its answer, which its exit code repeats: 0 when `true`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub healthy: bool,
    /// What the Daemon answered, or `None` when no Daemon of the major
    /// answered.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub daemon: Option<DaemonHealth>,
}

/// What `daemon start`, `restart`, `stop`, `status`, `install` and `run`
/// print under `--json` (spec #608): the Daemon of the executable's major
/// that runs once the command is done, each field `None` but `major` when
/// none runs, as after `daemon stop`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatus {
    /// Its process ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pid: Option<u32>,
    /// The executable's major, whose Daemon it is, such as `1` or `0.4`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub major: String,
    /// The Product it runs as.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Option<Product>,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: Option<ProductVersion>,
    /// Where it listens: its socket's path, or `host:port`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub address: Option<String>,
    /// Where its Console is served, or `None` when it serves none, as
    /// `daemon run`'s, or its Console did not start.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub console_url: Option<String>,
    /// Why its Console did not start, the failure `daemon start` exits
    /// by; absent while it serves one or carries none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub console_error: Option<ConsoleFailure>,
    /// The per-user service `daemon install` registered for the major;
    /// absent when none is installed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub service: Option<InstalledService>,
}

/// The OS's service manager a Daemon is installed with (ADR 0024).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum ServiceManager {
    /// macOS's launchd, as a launch agent with socket activation.
    Launchd,
    /// Linux's systemd, as a user unit with socket activation.
    Systemd,
    /// Windows' Task Scheduler, as a logon task.
    TaskScheduler,
}

/// The per-user service of one major that `daemon install` registered,
/// recorded in `daemon-service.json` in the major's per-major folder (ADRs
/// 0024, 0031 and 0098): what starts the major's pick, from which versioned
/// copy, and the socket its service manager listens on for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct InstalledService {
    /// The service manager it is registered with.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub manager: ServiceManager,
    /// Its name there: a launch agent's label, a user unit's name without
    /// its suffix, or a logon task's path.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The socket the service manager listens on and hands the Daemon it
    /// starts at the first connection, or `None` where the OS has no socket
    /// activation (Windows).
    #[schemars(with = "Option<String>", extend("arrived_in" = "0.1"))]
    pub socket: Option<PathBuf>,
    /// The Console's `host:port` the service manager also listens on, so
    /// the first visit to the Console starts the Daemon (ADR 0031); `None`
    /// without socket activation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub console: Option<String>,
    /// The versioned copy in the Browser cache that the service starts,
    /// which starts its major's pick.
    #[schemars(with = "String", extend("arrived_in" = "0.1"))]
    pub executable: PathBuf,
    /// The files that define it to the service manager.
    #[schemars(with = "Vec<String>", extend("arrived_in" = "0.1"))]
    pub files: Vec<PathBuf>,
}

/// What `daemon uninstall --json` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonUninstallOutput {
    /// The service it removed, or `None` when none was installed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub removed: Option<InstalledService>,
}

/// What `daemon rotate-token --json` prints: when it replaced the
/// persistent token of the major's Daemon and the HTTP listener's token.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TokenRotated {
    /// When the tokens were replaced, in RFC 3339, UTC.
    #[schemars(with = "String", extend("format" = "date-time", "arrived_in" = "0.1"))]
    pub rotated_at: jiff::Timestamp,
}

/// What `daemon http-listener-token --json` prints: the token the
/// Daemon's HTTP listener takes, the one credential a command prints, on
/// stdout alone (spec #608).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HttpListenerToken {
    /// The token.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: DaemonToken,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[STATUS, STOP, HEALTH],
    events: &[],
    handler_requests: &[],
    error_codes: &[NOT_A_DAEMON, DAEMON_DRAINING, DAEMON_UNREACHABLE],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[IDLE_TIMEOUT, ADDRESS, DRAIN_GRACE],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
