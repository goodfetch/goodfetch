//! `xtask clients check` (ADR 0069), in the Quick suite: every Client
//! conformance case of `conformance/clients/cases.toml` is implemented on each
//! surface it lists, in each mode that surface has, and no test carries an
//! unknown case.
//!
//! A test carries a case the way a Journey test carries its variant (ADR
//! 0063): nextest's name of it holds `client_conformance::<case>`, the case's
//! ID with `_` for `-`, and ends with the mode, as the test's own name or its
//! rstest case's (`client_conformance::hello_refused::refuses::case_1_attached`).
//! The `cli` surface's tests are those of each Product's executable crate,
//! read through the Feature scanner, which starts no process. The SDK
//! surfaces' tests are read from M2, with their runners; until then a case
//! they list is only noted.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::features;
use crate::output;

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a case with no test on a surface it lists in one of that
    /// surface's modes, a test carrying an unknown case, a case on a surface
    /// or in a mode the case does not list, or no mode, two cases with one
    /// ID, a case whose modes are on no surface it lists, and a case list
    /// that does not parse. A listed surface whose tests are not read yet is
    /// noted.
    Check,
}

/// The Client conformance case list.
const CASE_LIST: &str = "conformance/clients/cases.toml";
/// The crates whose tests implement the `cli` surface's cases: each
/// Product's executable crate (ADR 0094).
const EXECUTABLE_CRATES: [&str; 2] = ["goodfetch-collie", "goodfetch-foxhound"];
/// The module in a test's path that the case's module follows.
const CASES_MODULE: &str = "client_conformance";

/// A Client surface a case applies to (ADR 0069).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "kebab-case")]
enum Surface {
    SdkRust,
    SdkTypescript,
    SdkPython,
    Cli,
}

impl Surface {
    /// The modes every case runs in on this surface: only the Rust crate
    /// links the engine, and it never starts an Attached engine.
    fn modes(self) -> &'static [Mode] {
        match self {
            Self::SdkRust => &[Mode::Linked, Mode::Daemon],
            Self::SdkTypescript | Self::SdkPython | Self::Cli => &[Mode::Attached, Mode::Daemon],
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::SdkRust => "sdk-rust",
            Self::SdkTypescript => "sdk-typescript",
            Self::SdkPython => "sdk-python",
            Self::Cli => "cli",
        }
    }
}

impl fmt::Display for Surface {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// How a case reaches the engine.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mode {
    Linked,
    Attached,
    Daemon,
}

impl Mode {
    const ALL: [Self; 3] = [Self::Linked, Self::Attached, Self::Daemon];

    fn as_str(self) -> &'static str {
        match self {
            Self::Linked => "linked",
            Self::Attached => "attached",
            Self::Daemon => "daemon",
        }
    }

    /// The mode a test's last name segment names: the segment itself, or an
    /// rstest case's description after its `case_<n>_` prefix.
    fn of(segment: &str) -> Option<Self> {
        let description = segment
            .strip_prefix("case_")
            .and_then(|rest| rest.split_once('_'))
            .filter(|(number, _)| {
                !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
            })
            .map_or(segment, |(_, description)| description);
        Self::ALL
            .into_iter()
            .find(|mode| mode.as_str() == description)
    }
}

impl fmt::Display for Mode {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// The case list, as far as this check reads it; the harness reads the rest
/// (`goodfetch-conformance`), and the Feature check its `covers` lists.
#[derive(Deserialize)]
struct CaseList {
    #[serde(default)]
    capability: Vec<Capability>,
}

#[derive(Deserialize)]
struct Capability {
    #[serde(default)]
    case: Vec<Case>,
}

#[derive(Deserialize)]
struct Case {
    id: String,
    /// The surfaces it applies to.
    surfaces: Vec<Surface>,
    /// The modes it exists in, for a capability that exists in some only;
    /// every mode of each surface otherwise.
    #[serde(default)]
    modes: Option<Vec<Mode>>,
}

impl Case {
    /// The modes it runs in on `surface`.
    fn modes_on(&self, surface: Surface) -> Vec<Mode> {
        surface
            .modes()
            .iter()
            .copied()
            .filter(|mode| self.modes.as_ref().is_none_or(|modes| modes.contains(mode)))
            .collect()
    }

    /// The name of its module in a test's path.
    fn module(&self) -> String {
        self.id.replace('-', "_")
    }
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    let Command::Check = command;
    let verdict = check(environment.cargo.as_os_str(), &environment.root)?;
    for note in &verdict.notes {
        output::line(&format!(
            "note[{}] {}: {}",
            note.kind, note.subject, note.message
        ));
    }
    Ok(check::report(&verdict.findings))
}

struct Verdict {
    findings: Vec<Finding>,
    notes: Vec<Finding>,
}

fn check(cargo: &OsStr, root: &Path) -> Result<Verdict> {
    let mut findings = Vec::new();
    let mut notes = Vec::new();
    let cases = match read(root)? {
        Ok(cases) => cases,
        Err(message) => {
            findings.push(Finding::new("schema", CASE_LIST, message));
            return Ok(Verdict { findings, notes });
        }
    };
    let mut by_module: BTreeMap<String, &Case> = BTreeMap::new();
    let mut unique = Vec::new();
    for case in &cases {
        if let Entry::Vacant(entry) = by_module.entry(case.module()) {
            entry.insert(case);
            unique.push(case);
        } else {
            findings.push(Finding::new(
                "duplicate-case",
                &case.id,
                "an earlier case of the list has this ID; give each case its own",
            ));
        }
    }
    let implemented = cli_tests(cargo, root, &by_module, &mut findings)?;
    for case in unique {
        for &surface in &case.surfaces {
            let modes = case.modes_on(surface);
            if modes.is_empty() {
                findings.push(Finding::new(
                    "no-mode-on-surface",
                    format!("{} {surface}", case.id),
                    "none of the case's modes is a mode of this surface",
                ));
                continue;
            }
            if surface != Surface::Cli {
                notes.push(Finding::new(
                    "unread-surface",
                    format!("{} {surface}", case.id),
                    "this surface's tests are not read yet; its runner's listing joins with its case-log writer (M2)",
                ));
                continue;
            }
            for mode in modes {
                if !implemented.contains(&(case.id.as_str(), mode)) {
                    findings.push(Finding::new(
                        "missing",
                        format!("{} {surface} {mode}", case.id),
                        format!(
                            "no test of {} carries `{CASES_MODULE}::{}` in this mode",
                            EXECUTABLE_CRATES.join(" or "),
                            case.module()
                        ),
                    ));
                }
            }
        }
    }
    findings.sort_by(|left, right| (left.kind, &left.subject).cmp(&(right.kind, &right.subject)));
    Ok(Verdict { findings, notes })
}

/// The cases of the case list, in its order, or why it does not parse; none
/// when there is no list.
fn read(root: &Path) -> Result<Result<Vec<Case>, String>> {
    let path = root.join(CASE_LIST);
    if !path.is_file() {
        return Ok(Ok(Vec::new()));
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {CASE_LIST}"))?;
    Ok(toml::from_str::<CaseList>(&text)
        .map(|list| {
            list.capability
                .into_iter()
                .flat_map(|capability| capability.case)
                .collect()
        })
        .map_err(|error| error.message().to_owned()))
}

/// Each case and mode a test of the executable crates implements on the
/// `cli` surface; a test that carries an unknown case, a case or mode the
/// case does not list, or no mode, is a finding.
fn cli_tests<'a>(
    cargo: &OsStr,
    root: &Path,
    cases: &BTreeMap<String, &'a Case>,
    findings: &mut Vec<Finding>,
) -> Result<BTreeSet<(&'a str, Mode)>> {
    let mut implemented = BTreeSet::new();
    for test in features::tests(cargo, root, Some(&EXECUTABLE_CRATES))? {
        let segments: Vec<&str> = test.name.split("::").collect();
        let Some(position) = segments.iter().position(|segment| *segment == CASES_MODULE) else {
            continue;
        };
        let subject = format!("{} {}", test.binary, test.name);
        let carried = &segments[position + 1..];
        let case = match carried {
            [module, _, ..] => cases.get(*module),
            _ => None,
        };
        let Some(case) = case else {
            findings.push(Finding::new(
                "unknown-case",
                subject,
                format!("no case of {CASE_LIST} has this module's ID"),
            ));
            continue;
        };
        let Some(mode) = carried.last().and_then(|segment| Mode::of(segment)) else {
            findings.push(Finding::new(
                "no-mode",
                subject,
                "name the test, or its rstest case, after the mode it runs in",
            ));
            continue;
        };
        if !case.surfaces.contains(&Surface::Cli) || !case.modes_on(Surface::Cli).contains(&mode) {
            findings.push(Finding::new(
                "unlisted",
                subject,
                format!("the case does not list the cli surface in the {mode} mode"),
            ));
            continue;
        }
        implemented.insert((case.id.as_str(), mode));
    }
    Ok(implemented)
}
