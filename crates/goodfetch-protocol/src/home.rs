//! The Goodfetch home's layout (ADRs 0031, 0059 and 0098): the names of
//! the files and folders in it that more than one reader opens, the
//! engines of both Products, the Kennel (ADR 0141), the conformance suites,
//! test support and `xtask`, so that each is written once.
//!
//! ```text
//! <home>/                         one user's Goodfetch home
//!   <line>/                       a per-major folder, named by its
//!                                 `VersionLine`: `1` or `0.4`
//!     runs/<run id>/run.sqlite    a run folder and its run file
//!     runs/<run id>/trace/…       the run's trace files
//!     run-index.sqlite            the Run index
//!     operator-policy.toml,       the major's settings files (ADR 0098)
//!     configuration.toml
//!     daemon.json, daemon.lock, … the Daemon's files (`DaemonFile`)
//!   usage-ledger.sqlite, politeness.sqlite, audit-log.sqlite, …
//!                                 the files every major shares
//! ```
//!
//! Where the home is and what each file holds are their owners'; only the
//! names are here.

use std::path::{Path, PathBuf};

use crate::connection::ProductVersion;

/// The home's folder in the OS's usual data folder when `GOODFETCH_HOME`
/// is unset.
pub const HOME_FOLDER: &str = "goodfetch";

/// The per-major folder's own name for the run folders.
pub const RUNS: &str = "runs";

/// The Run index's file in the per-major folder.
pub const RUN_INDEX: &str = "run-index.sqlite";

/// The Operator policy's file in the per-major folder, which both
/// Products read (ADRs 0028 and 0098).
pub const OPERATOR_POLICY_FILE: &str = "operator-policy.toml";

/// User configuration's file in the per-major folder, which both Products
/// read (ADRs 0083 and 0098).
pub const USER_CONFIGURATION_FILE: &str = "configuration.toml";

/// The Usage ledger's file, outside the per-major folders (ADR 0041).
pub const USAGE_LEDGER: &str = "usage-ledger.sqlite";

/// The per-user Politeness file, outside the per-major folders.
pub const POLITENESS_FILE: &str = "politeness.sqlite";

/// The Audit log's store, outside the per-major folders (ADR 0016).
pub const AUDIT_LOG: &str = "audit-log.sqlite";

/// The Audit log's head file, beside its store.
pub const AUDIT_LOG_HEAD: &str = "audit-log-head.json";

/// The known-certificates store, beside the Usage ledger (ADR 0042).
pub const KNOWN_CERTIFICATES: &str = "known-certificates.sqlite";

/// The log folder, outside the per-major folders, where every Daemon but
/// `daemon run` writes its JSON lines (ADR 0122).
pub const LOGS: &str = "logs";

/// The encrypted local files' folder, outside the per-major folders
/// (spec #564).
pub const SECRET_FILES: &str = "secrets";

/// The lock an engine holds while it makes its own key-encryption key,
/// outside the per-major folders (spec #564).
pub const ENGINE_KEY_LOCK: &str = "engine-key.lock";

/// The list each Product's executable keeps of the installed executables,
/// outside the per-major folders (ADR 0098).
pub const INSTALLED_EXECUTABLES: &str = "installed-executables.json";

/// The bug-report file `doctor` writes for the user to attach to an issue,
/// outside the per-major folders, replaced by each `doctor` (spec #564).
pub const BUG_REPORT: &str = "bug-report.json";

/// A run folder's run file.
pub const RUN_FILE: &str = "run.sqlite";

/// The run trace's request events, in the run folder.
pub const RUN_TRACE_REQUESTS: &str = "trace/requests.jsonl";

/// The run trace's Provenance entries, in the run folder.
pub const RUN_TRACE_PROVENANCE: &str = "trace/provenance.jsonl";

/// The run trace's CDP commands, in the run folder.
pub const RUN_TRACE_CDP_COMMANDS: &str = "trace/cdp-commands.jsonl";

/// The run trace's raw calls (ADR 0099), in the run folder.
pub const RUN_TRACE_CALLS: &str = "trace/calls.jsonl";

/// The run trace's settings a resume changed (ADR 0102), in the run folder.
pub const RUN_TRACE_SETTINGS: &str = "trace/settings.jsonl";

/// The per-major folder of the Product version `version` in the home at
/// `home`.
pub fn per_major(home: &Path, version: &ProductVersion) -> PathBuf {
    home.join(version.line().to_string())
}

/// A file of the Daemon's in its per-major folder (ADRs 0024, 0030 and
/// 0059): its own while it runs, never state, so `upgrade-files` copies
/// none of them into a newer major.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DaemonFile {
    /// The runtime file, mode 0600.
    RuntimeFile,
    /// The socket, mode 0600 (macOS and Linux).
    Socket,
    /// The lock held exclusively while a Daemon serves.
    ServingLock,
    /// The lock held by whatever starts, or hands over, one.
    StartLock,
    /// The persistent token, mode 0600, of a socket-activated Daemon.
    Token,
    /// The per-user service `daemon install` registered.
    ServiceRecord,
    /// The ops token, mode 0600, which the ops listener's `/metrics` takes
    /// (ADR 0050).
    OpsToken,
    /// The HTTP listener's token, mode 0600.
    HttpListenerToken,
}

impl DaemonFile {
    /// Every one of them.
    pub const ALL: [Self; 8] = [
        Self::RuntimeFile,
        Self::Socket,
        Self::ServingLock,
        Self::StartLock,
        Self::Token,
        Self::ServiceRecord,
        Self::OpsToken,
        Self::HttpListenerToken,
    ];

    /// Its name in the per-major folder.
    pub const fn name(self) -> &'static str {
        match self {
            Self::RuntimeFile => "daemon.json",
            Self::Socket => "daemon.sock",
            Self::ServingLock => "daemon.lock",
            Self::StartLock => "daemon-start.lock",
            Self::Token => "daemon.token",
            Self::ServiceRecord => "daemon-service.json",
            Self::OpsToken => "ops.token",
            Self::HttpListenerToken => "http-listener.token",
        }
    }

    /// The Daemon's file named `name` in a per-major folder, if it is one.
    pub fn named(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|file| file.name() == name)
    }
}
