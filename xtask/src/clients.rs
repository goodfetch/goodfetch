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
//! read through the Feature scanner, which starts no process. The
//! `sdk-typescript` surface's tests are the Vitest tests of the TypeScript
//! SDK packages (`sdks/typescript/`), read from Vitest's listing with
//! `--listings`, in the Pre-merge suite (`just clients-tests`): each carries
//! its case as the tag `client_case/<id>` and its mode as the tag `attached`
//! or `daemon`. The `sdk-python` surface's tests are the pytest tests of the
//! Python SDK packages (`sdks/python/`), read from pytest's listing in the
//! same folder: each carries its case and mode as the marker
//! `client_case(id="<id>", mode="attached")`. Without the listings, and for
//! the surface whose runner does not write a case log yet, `sdk-rust`, a case
//! they list is only noted.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::btree_map::Entry;
use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::fmt;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::features;
use crate::output;
use crate::products;

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a case with no test on a surface it lists in one of that
    /// surface's modes, a test carrying an unknown case, a case on a surface
    /// or in a mode the case does not list, or no mode, two cases with one
    /// ID, a case whose modes are on no surface it lists, a case giving
    /// modes of their own to a surface it does not list, and a case list
    /// that does not parse. A listed surface whose tests are not read yet is
    /// noted.
    Check {
        /// The folder of the runners' listings `xtask journeys listings`
        /// writes, whose Vitest listing gives the `sdk-typescript` surface's
        /// tests and whose pytest listing gives the `sdk-python` surface's.
        #[arg(long = "listings")]
        listings: Option<PathBuf>,
    },
}

/// The Client conformance case list.
const CASE_LIST: &str = "conformance/clients/cases.toml";
/// The crates whose tests implement the `cli` surface's cases: each
/// Product's executable crate (ADR 0094).
const EXECUTABLE_CRATES: [&str; 2] = products::PACKAGES;
/// The module in a test's path that the case's module follows.
const CASES_MODULE: &str = "client_conformance";
/// The folder of the packages whose Vitest tests implement the
/// `sdk-typescript` surface's cases: each Product's TypeScript SDK packages.
const TYPESCRIPT_PACKAGES: &str = "sdks/typescript/";
/// The prefix of the Vitest tag that carries a case's ID.
const CASE_TAG: &str = "client_case/";
/// Vitest's listing in the listings folder.
const VITEST_LISTING: &str = "vitest.json";
/// The folder of the packages whose pytest tests implement the `sdk-python`
/// surface's cases: each Product's Python SDK packages.
const PYTHON_PACKAGES: &str = "sdks/python/";
/// The pytest marker that carries a case's ID and mode.
const CASE_MARKER: &str = "client_case";
/// pytest's listing in the listings folder, with each test's markers.
const PYTEST_LISTING: &str = "pytest.json";

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
    modes: Option<Modes>,
    /// The platforms it runs on, by Rust's `cfg` name (`unix`, `windows`),
    /// for a case a platform lacks; every platform otherwise.
    #[serde(default)]
    platforms: Option<Vec<String>>,
}

/// The modes a case exists in, for a capability that exists in some only.
#[derive(Deserialize)]
#[serde(
    untagged,
    expecting = "`modes` takes a list of modes, as [\"attached\"], or a table of each surface's modes, as { sdk-rust = [\"daemon\"] }"
)]
enum Modes {
    /// The same on every surface it lists, as `modes = ["attached"]`.
    Shared(Vec<Mode>),
    /// Each surface's own, as `modes = { sdk-rust = ["daemon"] }`, for a
    /// capability whose modes differ by surface; a surface the table leaves
    /// out runs in every mode it has.
    BySurface(BTreeMap<Surface, Vec<Mode>>),
}

impl Case {
    /// The modes it runs in on `surface`.
    fn modes_on(&self, surface: Surface) -> Vec<Mode> {
        let listed = match &self.modes {
            None => None,
            Some(Modes::Shared(modes)) => Some(modes),
            Some(Modes::BySurface(by_surface)) => by_surface.get(&surface),
        };
        surface
            .modes()
            .iter()
            .copied()
            .filter(|mode| listed.is_none_or(|modes| modes.contains(mode)))
            .collect()
    }

    /// The surfaces it gives modes of their own to without listing them.
    fn unlisted_with_modes(&self) -> impl Iterator<Item = Surface> + '_ {
        let by_surface = match &self.modes {
            Some(Modes::BySurface(by_surface)) => Some(by_surface.keys().copied()),
            None | Some(Modes::Shared(_)) => None,
        };
        by_surface
            .into_iter()
            .flatten()
            .filter(|surface| !self.surfaces.contains(surface))
    }

    /// Whether a test with these gates runs wherever the case does: one
    /// with none, or one behind the `cfg` of the one platform the case
    /// lists. An `#[ignore]`d test, or one behind any other `cfg`, may never
    /// run, so it implements nothing.
    fn runs_wherever_it_does(&self, gates: &[String]) -> bool {
        match (gates, self.platforms.as_deref()) {
            ([], _) => true,
            ([gate], Some([platform])) => *gate == format!("cfg({platform})"),
            _ => false,
        }
    }

    /// The name of its module in a test's path.
    fn module(&self) -> String {
        self.id.replace('-', "_")
    }
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    let Command::Check { listings } = command;
    let root = &environment.root;
    let listings = listings.as_ref().map(|folder| root.join(folder));
    let verdict = check(environment.cargo.as_os_str(), root, listings.as_deref())?;
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

fn check(cargo: &OsStr, root: &Path, listings: Option<&Path>) -> Result<Verdict> {
    let mut findings = Vec::new();
    let mut notes = Vec::new();
    let cases = match read(root)? {
        Ok(Some(cases)) => cases,
        Ok(None) => {
            findings.push(Finding::new(
                "missing-list",
                CASE_LIST,
                "the case list is not there, so no case would be checked",
            ));
            return Ok(Verdict { findings, notes });
        }
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
    let cli = cli_tests(cargo, root, &by_module, &mut findings)?;
    let by_id: BTreeMap<&str, &Case> = unique
        .iter()
        .map(|case| (case.id.as_str(), *case))
        .collect();
    let tested = Tested {
        cli,
        typescript: listings
            .map(|folder| typescript_tests(root, folder, &by_id, &mut findings))
            .transpose()?,
        python: listings
            .map(|folder| python_tests(folder, &by_id, &mut findings))
            .transpose()?,
    };
    for case in unique {
        check_case(case, &tested, &mut findings, &mut notes);
    }
    findings.sort_by(|left, right| (left.kind, &left.subject).cmp(&(right.kind, &right.subject)));
    Ok(Verdict { findings, notes })
}

/// Each case and mode a runner's tests run, by surface; `None` for an SDK
/// whose listing was not given.
struct Tested<'a> {
    cli: BTreeSet<(&'a str, Mode)>,
    typescript: Option<BTreeSet<(&'a str, Mode)>>,
    python: Option<BTreeSet<(&'a str, Mode)>>,
}

/// Finds each surface and mode of `case` that no test runs, and notes each
/// surface whose tests are not read.
fn check_case(
    case: &Case,
    tested: &Tested<'_>,
    findings: &mut Vec<Finding>,
    notes: &mut Vec<Finding>,
) {
    for surface in case.unlisted_with_modes() {
        findings.push(Finding::new(
            "modes-unlisted-surface",
            format!("{} {surface}", case.id),
            "the case gives this surface modes of its own but does not list it in `surfaces`",
        ));
    }
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
        let (implemented, carrier) = match (surface, &tested.typescript, &tested.python) {
            (Surface::Cli, _, _) => (
                &tested.cli,
                format!(
                    "no test of {} carries `{CASES_MODULE}::{}`",
                    EXECUTABLE_CRATES.join(" or "),
                    case.module()
                ),
            ),
            (Surface::SdkTypescript, Some(implemented), _) => (
                implemented,
                format!(
                    "no Vitest test of `{TYPESCRIPT_PACKAGES}` carries the tag `{CASE_TAG}{}`",
                    case.id
                ),
            ),
            (Surface::SdkPython, _, Some(implemented)) => (
                implemented,
                format!(
                    "no pytest test of `{PYTHON_PACKAGES}` carries `{CASE_MARKER}(id=\"{}\", mode=…)`",
                    case.id
                ),
            ),
            (Surface::SdkTypescript | Surface::SdkPython, _, _) => {
                let runner = if surface == Surface::SdkTypescript {
                    "Vitest's"
                } else {
                    "pytest's"
                };
                notes.push(Finding::new(
                        "unread-surface",
                        format!("{} {surface}", case.id),
                        format!("this surface's tests are read from {runner} listing, with --listings (`just clients-tests`)"),
                    ));
                continue;
            }
            (Surface::SdkRust, _, _) => {
                notes.push(Finding::new(
                        "unread-surface",
                        format!("{} {surface}", case.id),
                        "this surface's tests are not read yet; its runner's listing joins with its case-log writer (M2)",
                    ));
                continue;
            }
        };
        for mode in modes {
            if !implemented.contains(&(case.id.as_str(), mode)) {
                findings.push(Finding::new(
                    "missing",
                    format!("{} {surface} {mode}", case.id),
                    format!("{carrier} in this mode"),
                ));
            }
        }
    }
}

/// The cases of the case list, in its order, or why it does not parse;
/// `None` when there is no list.
fn read(root: &Path) -> Result<Result<Option<Vec<Case>>, String>> {
    let path = root.join(CASE_LIST);
    if !path.is_file() {
        return Ok(Ok(None));
    }
    let text = std::fs::read_to_string(&path).with_context(|| format!("reading {CASE_LIST}"))?;
    Ok(toml::from_str::<CaseList>(&text)
        .map(|list| {
            list.capability
                .into_iter()
                .flat_map(|capability| capability.case)
                .collect()
        })
        .map(Some)
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
        if !case.runs_wherever_it_does(&test.gates) {
            findings.push(Finding::new(
                "gated",
                subject,
                format!(
                    "behind {}, it may not run where the case does; drop the gate, or list the case's one platform in its `platforms`",
                    test.gates.join(" and ")
                ),
            ));
            continue;
        }
        implemented.insert((case.id.as_str(), mode));
    }
    Ok(implemented)
}

#[derive(Deserialize)]
struct VitestTest {
    /// The full name, its suites and title joined by ` > `.
    name: String,
    file: PathBuf,
    tags: Vec<String>,
    /// How Vitest runs it: `run`, `only`, or `skip` and `todo`, which never
    /// run; a listing written before the field counts as `run`.
    #[serde(default)]
    mode: Option<String>,
}

/// Each case and mode a Vitest test of the TypeScript SDK packages
/// implements on the `sdk-typescript` surface, from Vitest's listing in
/// `folder`; a test that carries an unknown case, a case or mode the case
/// does not list, or not exactly one mode, is a finding.
fn typescript_tests<'a>(
    root: &Path,
    folder: &Path,
    cases: &BTreeMap<&str, &'a Case>,
    findings: &mut Vec<Finding>,
) -> Result<BTreeSet<(&'a str, Mode)>> {
    let path = folder.join(VITEST_LISTING);
    let bytes = std::fs::read(&path).with_context(|| {
        format!(
            "reading {}; `xtask journeys listings` writes it",
            path.display()
        )
    })?;
    let tests: Vec<VitestTest> =
        serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))?;
    let roots = [
        root.to_path_buf(),
        root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
    ];
    let mut implemented = BTreeSet::new();
    for test in tests {
        let file = roots
            .iter()
            .find_map(|root| test.file.strip_prefix(root).ok())
            .unwrap_or(&test.file)
            .to_string_lossy()
            .replace('\\', "/");
        if !file.starts_with(TYPESCRIPT_PACKAGES) {
            continue;
        }
        let subject = format!("vitest {file} > {}", test.name);
        let modes: Vec<Mode> = test
            .tags
            .iter()
            .filter_map(|tag| Mode::ALL.into_iter().find(|mode| mode.as_str() == tag))
            .collect();
        for id in test
            .tags
            .iter()
            .filter_map(|tag| tag.strip_prefix(CASE_TAG))
        {
            let Some(case) = cases.get(id) else {
                findings.push(Finding::new(
                    "unknown-case",
                    &subject,
                    format!("no case of {CASE_LIST} has the ID `{id}`"),
                ));
                continue;
            };
            let [mode] = modes.as_slice() else {
                findings.push(Finding::new(
                    "no-mode",
                    &subject,
                    "tag the test with exactly one mode it runs in, `attached` or `daemon`",
                ));
                continue;
            };
            if !case.surfaces.contains(&Surface::SdkTypescript)
                || !case.modes_on(Surface::SdkTypescript).contains(mode)
            {
                findings.push(Finding::new(
                    "unlisted",
                    &subject,
                    format!("the case does not list the sdk-typescript surface in the {mode} mode"),
                ));
                continue;
            }
            if matches!(test.mode.as_deref(), Some("skip" | "todo")) {
                findings.push(Finding::new(
                    "gated",
                    &subject,
                    "a skipped or todo test never runs, so it implements no case",
                ));
                continue;
            }
            implemented.insert((case.id.as_str(), *mode));
        }
    }
    Ok(implemented)
}

#[derive(Deserialize)]
struct PytestListing {
    tests: Vec<PytestTest>,
}

#[derive(Deserialize)]
struct PytestTest {
    /// The node ID, its file's path from the workspace's folder first.
    id: String,
    /// The keyword arguments of each of its `client_case` markers; none in
    /// a listing written before the field.
    #[serde(default)]
    client_case: Vec<serde_json::Map<String, serde_json::Value>>,
    /// Whether a `skip` or `skipif` marker may keep it from running.
    #[serde(default)]
    skipped: bool,
}

/// Each case and mode a pytest test of the Python SDK packages implements
/// on the `sdk-python` surface, from pytest's listing in `folder`; a test
/// whose `client_case` marker is not `client_case(id=…, mode=…)`, or that
/// carries an unknown case, a case or mode the case does not list, or no
/// mode, is a finding.
fn python_tests<'a>(
    folder: &Path,
    cases: &BTreeMap<&str, &'a Case>,
    findings: &mut Vec<Finding>,
) -> Result<BTreeSet<(&'a str, Mode)>> {
    let path = folder.join(PYTEST_LISTING);
    let bytes = std::fs::read(&path).with_context(|| {
        format!(
            "reading {}; `xtask journeys listings` writes it",
            path.display()
        )
    })?;
    let listing: PytestListing =
        serde_json::from_slice(&bytes).with_context(|| format!("reading {}", path.display()))?;
    let mut implemented = BTreeSet::new();
    for test in listing.tests {
        if !test.id.starts_with(PYTHON_PACKAGES) {
            continue;
        }
        let subject = format!("pytest {}", test.id);
        for marker in &test.client_case {
            let id = marker.get("id").and_then(serde_json::Value::as_str);
            let known = marker.keys().all(|key| key == "id" || key == "mode");
            let (Some(id), true) = (id, known) else {
                findings.push(Finding::new(
                    "malformed",
                    &subject,
                    format!("its `{CASE_MARKER}` marker is not `{CASE_MARKER}(id=\"<case>\", mode=\"attached\")`"),
                ));
                continue;
            };
            let Some(case) = cases.get(id) else {
                findings.push(Finding::new(
                    "unknown-case",
                    &subject,
                    format!("no case of {CASE_LIST} has the ID `{id}`"),
                ));
                continue;
            };
            let mode = marker
                .get("mode")
                .and_then(serde_json::Value::as_str)
                .and_then(|name| Mode::ALL.into_iter().find(|mode| mode.as_str() == name));
            let Some(mode) = mode else {
                findings.push(Finding::new(
                    "no-mode",
                    &subject,
                    format!("give its `{CASE_MARKER}` marker the mode it runs in, `mode=\"attached\"` or `mode=\"daemon\"`"),
                ));
                continue;
            };
            if !case.surfaces.contains(&Surface::SdkPython)
                || !case.modes_on(Surface::SdkPython).contains(&mode)
            {
                findings.push(Finding::new(
                    "unlisted",
                    &subject,
                    format!("the case does not list the sdk-python surface in the {mode} mode"),
                ));
                continue;
            }
            if test.skipped {
                findings.push(Finding::new(
                    "gated",
                    &subject,
                    "a test behind `skip` or `skipif` may never run, so it implements no case",
                ));
                continue;
            }
            implemented.insert((case.id.as_str(), mode));
        }
    }
    Ok(implemented)
}
