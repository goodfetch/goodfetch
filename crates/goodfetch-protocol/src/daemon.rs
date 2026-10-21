//! The Daemon's contracts (ADRs 0024, 0030 and 0098): its runtime file, by
//! which a Client of its major finds it, the `daemon.*` operations a Client
//! asks it about itself with, its idle timeout, the per-user service that
//! `daemon install` registers, and the `--json` outputs of `daemon start`,
//! `stop`, `restart`, `status`, `install`, `uninstall` and `rotate-token`.
//! One Daemon per user and major serves both Products, so everything here
//! is Collie's.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::net::SocketAddr;
use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Authority, Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    Product, ProtocolVersion, Reach, Setting, Stability, ValueKind, schema_of,
};
use crate::connection::ProductVersion;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

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
    pub token: String,
    /// Its Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: ProductVersion,
    /// Its process ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pid: u32,
}

/// Where a Daemon listens (ADR 0024): a `0600` Unix socket on macOS and
/// Linux, loopback TCP on Windows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DaemonEndpoint {
    /// A Unix socket's path.
    Unix(#[schemars(with = "String")] PathBuf),
    /// A loopback address and port.
    Tcp(#[schemars(with = "String")] SocketAddr),
}

impl std::fmt::Display for DaemonEndpoint {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Unix(path) => write!(formatter, "{}", path.display()),
            Self::Tcp(address) => write!(formatter, "{address}"),
        }
    }
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

/// A `daemon.*` operation called on an engine that is no Daemon.
pub const NOT_A_DAEMON: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "not_a_daemon",
    description: "The call asks a Daemon about itself, and this engine is an Attached engine. Ask the Daemon of the major, found through its runtime file, or run `daemon status`.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

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

/// What `daemon.status` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatus {
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
    result: schema_of::<DaemonStatus>,
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

/// What `daemon status --json` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStatusOutput {
    /// The runtime file of the executable's major.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runtime_file: String,
    /// The running Daemon, or `None` when none runs.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub daemon: Option<DaemonStatus>,
    /// The per-user service `daemon install` registered for the major, or
    /// `None` when none is installed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub service: Option<InstalledService>,
}

/// What `daemon start --json` and `daemon restart --json` print.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStartOutput {
    /// Whether this command started it, rather than finding it running.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub started: bool,
    /// The running Daemon.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub daemon: DaemonStatus,
}

/// What `daemon stop --json` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonStopOutput {
    /// What the stopped Daemon answered, or `None` when none ran.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stopped: Option<DaemonStopping>,
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
    /// The versioned copy in the Browser cache that the service starts,
    /// which starts its major's pick.
    #[schemars(with = "String", extend("arrived_in" = "0.1"))]
    pub executable: PathBuf,
    /// The files that define it to the service manager.
    #[schemars(with = "Vec<String>", extend("arrived_in" = "0.1"))]
    pub files: Vec<PathBuf>,
}

/// What `daemon install --json` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonInstallOutput {
    /// The service it registered.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub service: InstalledService,
    /// What the Daemon running before answered when it was stopped so the
    /// service could take its place, or `None` when none ran.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stopped: Option<DaemonStopping>,
}

/// What `daemon uninstall --json` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonUninstallOutput {
    /// The service it removed, or `None` when none was installed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub removed: Option<InstalledService>,
}

/// What `daemon rotate-token --json` prints.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DaemonRotateTokenOutput {
    /// The file that holds the new persistent token, mode `0600`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token_file: String,
    /// What the Daemon running with the old token answered when it was
    /// stopped, or `None` when none ran.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stopped: Option<DaemonStopping>,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[STATUS, STOP],
    events: &[],
    handler_requests: &[],
    error_codes: &[NOT_A_DAEMON],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[IDLE_TIMEOUT],
    hook_stages: &[],
    shared_type_values: &[],
};
