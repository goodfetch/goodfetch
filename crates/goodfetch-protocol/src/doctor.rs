//! What `doctor --json` prints (ADR 0043, specs #564 and #608): one finding for
//! each part of the installation the command checks, Goodfetch home, the
//! installed executables, the Daemon, the versions they run and the
//! browser builds, each with its status and, when it is not `ok`, the fix,
//! and where it wrote the bug-report file, which it sends nowhere (spec
//! #564). `doctor` exits `0` whatever it finds.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// What `doctor --json` prints: its findings, in the order of
/// [`FindingId`], and the bug-report file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoctorReport {
    /// One finding for each part checked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub findings: Vec<DoctorFinding>,
    /// The bug-report file `doctor` wrote for the user to attach to an
    /// issue, or why it could not.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bug_report: BugReportFile,
}

/// The bug-report file `doctor` writes on the machine and never sends
/// (ADR 0043): versions, the findings, the settings with each credential
/// as its Secret reference, and recent Error codes, each URL reduced to its
/// scheme and host unless `--include-urls` is given.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum BugReportFile {
    /// It was written.
    Written {
        /// Its path.
        #[schemars(extend("arrived_in" = "0.1"))]
        path: String,
    },
    /// It could not be written.
    NotWritten {
        /// Why, for people.
        #[schemars(extend("arrived_in" = "0.1"))]
        message: String,
    },
}

/// What `doctor` found about one part of the installation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DoctorFinding {
    /// The part checked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: FindingId,
    /// Whether it is in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: FindingStatus,
    /// What was found, for people; never matched on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// What to do about it; present whenever the status is not `ok`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: Option<String>,
}

/// A part of the installation `doctor` checks.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingId {
    /// Goodfetch home: whether it can be written, and whether its Operator
    /// policy and user configuration can be read.
    Home,
    /// The list of installed executables, in which this one records
    /// itself (ADR 0098).
    Executables,
    /// The Daemon of this executable's major: whether it runs and answers.
    Daemon,
    /// Whether the running Daemon is of this executable's version.
    VersionSkew,
    /// Whether the browser build a Run launches by default is in the
    /// Browser cache.
    Browsers,
}

/// Whether a part of the installation is in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum FindingStatus {
    /// It is.
    Ok,
    /// It works, but something may fail or slow down later.
    Warning,
    /// Something that needs it fails until the fix is applied.
    Failing,
}
