//! Windows Smart App Control executable admission reporting (ADR 0039,
//! issue 873): checks whether Smart App Control admits each executable
//! Goodfetch starts but did not build — a toy Plugin, a cached interpreter,
//! the pinned Chrome for Testing build, and Lightpanda — and records a Smart
//! App Control block as the named outcome
//! `executable_blocked_by_smart_app_control` naming the file rather than
//! assuming admission.
#![expect(
    clippy::disallowed_methods,
    reason = "the synchronous conformance harness spawns target executables outside any async worker (ADR 0060)"
)]

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::executables::{
    EXECUTABLE_BLOCKED_BY_SMART_APP_CONTROL, is_smart_app_control_os_error,
};
use goodfetch_protocol::result_document::{
    CaseCheck, ConformanceDocument, ConformanceHeader, Evidence, ProvingGroundCopy, Target,
    TargetIdentity,
};
use serde_json::json;

use crate::case_list::CaseList;
use crate::host::{RUNTIME, platform};
use crate::writer::{WriteError, write_document};

/// `conformance/smart-app-control/cases.toml`, embedded so every runner uses
/// the same declared case list.
const CASES: &str = include_str!("../../../conformance/smart-app-control/cases.toml");

/// The Smart App Control executable admission suite's declared case list.
#[must_use]
pub fn smart_app_control_case_list() -> CaseList {
    toml::from_str(CASES).expect("invariant: embedded smart-app-control case list is valid")
}

/// The named outcome of attempting to start an external executable on a host
/// that may enforce Windows Smart App Control (ADR 0039, issue 873).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecutableAdmission {
    /// The OS admitted and started the executable without a Smart App Control
    /// refusal.
    Admitted { file: PathBuf },
    /// Windows' Smart App Control or another App Control policy blocked the
    /// executable from starting, reported as
    /// `executable_blocked_by_smart_app_control` naming `file`.
    BlockedBySmartAppControl {
        file: PathBuf,
        error_code: &'static str,
        os_error: i32,
    },
    /// This platform has no pinned build of this executable (for example,
    /// Lightpanda currently pins no Windows binary in `pins.json`).
    NotPinnedForPlatform { name: String },
    /// Starting the executable failed for an unrelated reason (missing file,
    /// permission error, etc.): this fails the conformance check.
    Failed { file: PathBuf, reason: String },
}

impl ExecutableAdmission {
    /// Classifies an OS spawn result for `file`: a Smart App Control refusal
    /// code (`4551`, `4556..=4559`, `4580..=4582`) becomes
    /// [`Self::BlockedBySmartAppControl`], never [`Self::Failed`] and never an
    /// assumed admission.
    #[must_use]
    pub fn from_spawn_result(file: &Path, result: io::Result<()>) -> Self {
        match result {
            Ok(()) => Self::Admitted {
                file: file.to_path_buf(),
            },
            Err(error) => {
                if let Some(os_error) = error
                    .raw_os_error()
                    .filter(|&code| is_smart_app_control_os_error(code))
                {
                    Self::BlockedBySmartAppControl {
                        file: file.to_path_buf(),
                        error_code: EXECUTABLE_BLOCKED_BY_SMART_APP_CONTROL.name,
                        os_error,
                    }
                } else {
                    Self::Failed {
                        file: file.to_path_buf(),
                        reason: error.to_string(),
                    }
                }
            }
        }
    }

    /// The named outcome string written into conformance checks and evidence.
    #[must_use]
    pub const fn outcome_name(&self) -> &'static str {
        match self {
            Self::Admitted { .. } => "admitted",
            Self::BlockedBySmartAppControl { error_code, .. } => error_code,
            Self::NotPinnedForPlatform { .. } => "not_pinned_for_platform",
            Self::Failed { .. } => "failed",
        }
    }
}

/// Probes whether the host admits `executable` by spawning it with `args`,
/// classifying any Windows Smart App Control refusal as
/// [`ExecutableAdmission::BlockedBySmartAppControl`] naming `executable`.
///
/// The child's stdout and stderr are discarded and it is killed and reaped
/// immediately after spawning if it stays alive (e.g. a stdio Plugin awaiting
/// `hello`), so proving admission never hangs.
#[must_use]
pub fn probe_executable(executable: &Path, args: &[&str]) -> ExecutableAdmission {
    let spawned = Command::new(executable)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|mut child| {
            let _ = child.kill();
            let _ = child.wait();
        });
    ExecutableAdmission::from_spawn_result(executable, spawned)
}

/// The four executable admission observations for one conformance report
/// (ADR 0039, issue 873).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SmartAppControlObservations {
    pub toy_plugin: ExecutableAdmission,
    pub cached_interpreter: ExecutableAdmission,
    pub chrome_for_testing: ExecutableAdmission,
    pub lightpanda: ExecutableAdmission,
}

/// Turns `observations` into a `ConformanceDocument` against
/// `conformance/smart-app-control/cases.toml`.
///
/// Each case records whether Smart App Control admitted the executable or
/// refused it with `executable_blocked_by_smart_app_control` naming the file:
/// both `admitted` and `executable_blocked_by_smart_app_control` (as well as
/// `not_pinned_for_platform`) are named outcomes that pass the check, while an
/// unrelated spawn error (`Failed`) fails the case.
///
/// # Errors
///
/// Returns [`WriteError`] if the document cannot be constructed.
pub fn write_smart_app_control_report(
    observations: &SmartAppControlObservations,
    proving_ground: ProvingGroundCopy,
) -> Result<ConformanceDocument, WriteError> {
    let cases = smart_app_control_case_list();
    let entries = [
        (
            "plugins.executables",
            "toy-plugin-executable-admission",
            "toy Plugin",
            &observations.toy_plugin,
        ),
        (
            "plugins.executables",
            "cached-interpreter-admission",
            "cached interpreter",
            &observations.cached_interpreter,
        ),
        (
            "browsers.executables",
            "chrome-for-testing-admission",
            "Chrome for Testing",
            &observations.chrome_for_testing,
        ),
        (
            "browsers.executables",
            "lightpanda-admission",
            "Lightpanda",
            &observations.lightpanda,
        ),
    ];

    let plat = platform();
    let log: Vec<CaseLogEntry> = entries
        .into_iter()
        .map(|(capability, case_id, subject, admission)| {
            admission_entry(capability, case_id, subject, admission, &plat)
        })
        .collect();

    let header = ConformanceHeader {
        suite: cases.suite.clone(),
        target: Target {
            name: "goodfetch-executables".to_owned(),
            identity: TargetIdentity::Version(env!("CARGO_PKG_VERSION").to_owned()),
            product: None,
        },
        platform: plat,
        runtime: RUNTIME.to_owned(),
        proving_ground,
        online: false,
        gate: false,
    };
    write_document(header, &cases, &log)
}

fn admission_entry(
    capability: &str,
    case_id: &str,
    subject: &str,
    admission: &ExecutableAdmission,
    plat: &goodfetch_protocol::result_document::Platform,
) -> CaseLogEntry {
    let (check_name, passed, detail) = match admission {
        ExecutableAdmission::Admitted { file } => (
            format!("Smart App Control admits {subject} ({})", file.display()),
            true,
            json!({
                "subject": subject,
                "outcome": "admitted",
                "file": file.display().to_string(),
            }),
        ),
        ExecutableAdmission::BlockedBySmartAppControl {
            file,
            error_code,
            os_error,
        } => (
            format!(
                "Smart App Control refused {subject} ({}) with {error_code}",
                file.display()
            ),
            true,
            json!({
                "subject": subject,
                "outcome": error_code,
                "error_code": error_code,
                "retryable": false,
                "file": file.display().to_string(),
                "os_error": os_error,
            }),
        ),
        ExecutableAdmission::NotPinnedForPlatform { name } => (
            format!("{subject} ({name}) has no pinned build for this platform"),
            true,
            json!({
                "subject": subject,
                "outcome": "not_pinned_for_platform",
                "name": name,
            }),
        ),
        ExecutableAdmission::Failed { file, reason } => (
            format!(
                "{subject} ({}) failed to start outside Smart App Control: {reason}",
                file.display()
            ),
            false,
            json!({
                "subject": subject,
                "outcome": "failed",
                "file": file.display().to_string(),
                "reason": reason,
            }),
        ),
    };
    CaseLogEntry {
        case: case_id.to_owned(),
        capability: capability.to_owned(),
        mode: None,
        blocking: None,
        unreachable: Vec::new(),
        platform: plat.clone(),
        runtime: RUNTIME.to_owned(),
        checks: vec![CaseCheck {
            name: check_name,
            passed,
        }],
        evidence: Evidence {
            messages: vec![detail],
            ..Evidence::default()
        },
    }
}
