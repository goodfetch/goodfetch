//! The Rust runner's case-log writer (ADR 0069): a Rust test that runs a
//! Client conformance case on the `sdk-rust` surface starts a
//! [`ClientCaseRun`], records its checks and evidence on it, and when the
//! run drops, at the test's end or as its panic unwinds, the run appends
//! the case's one entry to the case log, `nextest.jsonl` in the folder it
//! was given. The entry carries the case's capability from
//! `conformance/clients/cases.toml`, the mode, this host's platform, the
//! `rust` runtime, the checks and a last one, `the test passed`, so a test
//! that fails outside its checks fails its case, and no outcome: the writer
//! derives that. Every test of a nextest run appends to the one file, a
//! line per write, so `just test` empties it before the run.

use std::collections::BTreeMap;
use std::fmt;
use std::io::Write as _;
use std::panic::{self, AssertUnwindSafe};
use std::path::{Path, PathBuf};

use goodfetch_protocol::case_log::CaseLogEntry;
use goodfetch_protocol::result_document::{CaseCheck, Evidence, OracleAnswer, Platform};
use serde::Deserialize;
use serde_json::Value;

/// The Client conformance case list, whose cases' capabilities and
/// surfaces the writer reads.
const CASE_LIST: &str = include_str!("../../../conformance/clients/cases.toml");

/// The surface a Rust test runs its case on.
const SURFACE: &str = "sdk-rust";

/// The runtime every entry of the Rust runner names, as a Rust Journey
/// variant's document does.
const RUNTIME: &str = "rust";

/// The check every entry ends with.
const TEST_PASSED: &str = "the test passed";

/// How a Rust test reaches the engine (ADR 0069): the crate links it, or
/// reaches a Daemon; it never starts an Attached engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RustMode {
    /// The engine linked in the test's process.
    Linked,
    /// A Daemon the crate connects to.
    Daemon,
}

impl RustMode {
    fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Daemon => "daemon",
        }
    }
}

impl fmt::Display for RustMode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One time a Rust test runs a Client conformance case: what it records
/// becomes the case's entry in the case log when it drops.
#[derive(Debug)]
pub struct ClientCaseRun {
    /// The case log's folder; none when the test runs outside `just test`,
    /// and nothing is written.
    folder: Option<PathBuf>,
    case: String,
    capability: String,
    mode: RustMode,
    checks: Vec<CaseCheck>,
    evidence: Evidence,
}

impl ClientCaseRun {
    /// The case log's file in its folder.
    pub const FILE: &str = "nextest.jsonl";

    /// Starts running `case` in `mode`, its entry appended to the case log
    /// in `folder` when the run drops; nothing is written without a folder.
    ///
    /// # Errors
    ///
    /// [`ClientCaseError::Undeclared`] for a case the case list does not
    /// declare, [`ClientCaseError::NotOnRust`] for one that does not list
    /// the `sdk-rust` surface, [`ClientCaseError::ModeNotListed`] for a
    /// mode the case's `modes` do not give `sdk-rust`, and
    /// [`ClientCaseError::CaseList`] when the case list does not parse.
    pub fn start(
        folder: Option<&Path>,
        case: &str,
        mode: RustMode,
    ) -> Result<Self, ClientCaseError> {
        let list: CaseListFile = toml::from_str(CASE_LIST)
            .map_err(|error| ClientCaseError::CaseList(error.message().to_owned()))?;
        let (capability, declared) = list
            .capabilities
            .iter()
            .find_map(|capability| {
                capability
                    .cases
                    .iter()
                    .find(|declared| declared.id == case)
                    .map(|declared| (capability.name.as_str(), declared))
            })
            .ok_or_else(|| ClientCaseError::Undeclared {
                case: case.to_owned(),
            })?;
        if !declared.surfaces.iter().any(|surface| surface == SURFACE) {
            return Err(ClientCaseError::NotOnRust {
                case: case.to_owned(),
            });
        }
        if !declared
            .modes
            .as_ref()
            .is_none_or(|modes| modes.give_rust(mode))
        {
            return Err(ClientCaseError::ModeNotListed {
                case: case.to_owned(),
                mode,
            });
        }
        Ok(Self {
            folder: folder.map(Path::to_path_buf),
            case: case.to_owned(),
            capability: capability.to_owned(),
            mode,
            checks: Vec::new(),
            evidence: Evidence::default(),
        })
    }

    /// Runs `assertion` as the check `name` and records whether it held: a
    /// panic in it is recorded as failed and goes on unwinding, so it still
    /// fails the test.
    pub fn check<T>(&mut self, name: &str, assertion: impl FnOnce() -> T) -> T {
        let held = panic::catch_unwind(AssertUnwindSafe(assertion));
        self.checks.push(CaseCheck {
            name: name.to_owned(),
            passed: held.is_ok(),
        });
        held.unwrap_or_else(|failure| panic::resume_unwind(failure))
    }

    /// Records a scripted message the case exchanged.
    pub fn message(&mut self, value: Value) {
        self.evidence.messages.push(value);
    }

    /// Records an oracle's answer the case received.
    pub fn oracle_answer(&mut self, answer: OracleAnswer) {
        self.evidence.oracle_answers.push(answer);
    }

    /// Records what the Proving ground observed on the server side.
    pub fn observation(&mut self, value: Value) {
        self.evidence.observations.push(value);
    }

    /// The run's entry, ending with whether the test passed.
    fn entry(&mut self, passed: bool) -> CaseLogEntry {
        let mut checks = std::mem::take(&mut self.checks);
        checks.push(CaseCheck {
            name: TEST_PASSED.to_owned(),
            passed,
        });
        CaseLogEntry {
            case: self.case.clone(),
            capability: self.capability.clone(),
            mode: Some(self.mode.as_str().to_owned()),
            blocking: None,
            unreachable: Vec::new(),
            platform: Platform {
                os: std::env::consts::OS.to_owned(),
                arch: std::env::consts::ARCH.to_owned(),
            },
            runtime: RUNTIME.to_owned(),
            checks,
            evidence: std::mem::take(&mut self.evidence),
        }
    }
}

impl Drop for ClientCaseRun {
    /// Appends the run's entry to the case log; a log that cannot be
    /// written fails a test that was passing.
    fn drop(&mut self) {
        let Some(folder) = self.folder.take() else {
            return;
        };
        let failing = std::thread::panicking();
        let entry = self.entry(!failing);
        let path = folder.join(Self::FILE);
        if let Err(error) = append(&folder, &path, &entry)
            && !failing
        {
            panic!("the case log {} cannot be written: {error}", path.display());
        }
    }
}

/// Appends `entry` to the case log at `path` as one line, in one write.
#[expect(
    clippy::disallowed_methods,
    reason = "a test writes the case log, outside the work a Shared path counts"
)]
fn append(folder: &Path, path: &Path, entry: &CaseLogEntry) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(entry)?;
    line.push(b'\n');
    std::fs::create_dir_all(folder)?;
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?
        .write_all(&line)
}

/// Why a Rust test cannot run a Client conformance case.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ClientCaseError {
    /// The case list does not declare the case.
    #[error("the case list does not declare the case `{case}`")]
    Undeclared {
        /// The case's ID.
        case: String,
    },
    /// The case does not list the `sdk-rust` surface.
    #[error("the case `{case}` does not list the sdk-rust surface")]
    NotOnRust {
        /// The case's ID.
        case: String,
    },
    /// The case's `modes` do not give `sdk-rust` the mode.
    #[error("the case `{case}` does not list the {mode} mode for sdk-rust")]
    ModeNotListed {
        /// The case's ID.
        case: String,
        /// The mode the test runs it in.
        mode: RustMode,
    },
    /// The case list does not parse.
    #[error("the case list does not parse: {0}")]
    CaseList(String),
}

/// The case list, as far as the writer reads it.
#[derive(Deserialize)]
struct CaseListFile {
    #[serde(rename = "capability")]
    capabilities: Vec<CapabilityEntry>,
}

#[derive(Deserialize)]
struct CapabilityEntry {
    name: String,
    #[serde(rename = "case", default)]
    cases: Vec<CaseEntry>,
}

#[derive(Deserialize)]
struct CaseEntry {
    id: String,
    surfaces: Vec<String>,
    /// Left out, the case runs in every mode of each surface.
    modes: Option<CaseModes>,
}

/// A case's `modes`: one list for every surface, or a table where a surface
/// it leaves out takes all its modes.
#[derive(Deserialize)]
#[serde(untagged)]
enum CaseModes {
    Shared(Vec<String>),
    BySurface(BTreeMap<String, Vec<String>>),
}

impl CaseModes {
    /// Whether these modes give `sdk-rust` the mode `mode`.
    fn give_rust(&self, mode: RustMode) -> bool {
        let listed = match self {
            Self::Shared(modes) => Some(modes),
            Self::BySurface(by_surface) => by_surface.get(SURFACE),
        };
        listed.is_none_or(|modes| modes.iter().any(|listed| listed == mode.as_str()))
    }
}
