//! Which crates passed in full in a failed gate step run per member (ADR
//! 0135; the test-gate audit's option 4), from the step's nextest JUnit
//! files: those crates record green, every other member of the step red. A
//! failure the files do not account for records every member red, as a
//! failed step did before.
//!
//! - **The Quick suite** (its nextest file alone): the file must be whole,
//!   as many test cases as its `<testsuites>` declares, and hold a failing
//!   test, each of a crate the step ran. nextest runs every test of the run
//!   (`fail-fast = false`), and Vitest and pytest, which run after it, test
//!   no crate, so each other crate of the step passed in full.
//! - **The Pre-merge suite**, judged as the Quick suite, and handed its file
//!   only when nextest's tests alone failed: its doctests and the exercise
//!   audit, which still run after a red nextest run, passed (the test-gate
//!   audit's option 4b). Its crates holding insta snapshots stay red, since
//!   cargo-insta judges unreferenced snapshots only once every test passed
//!   (`gate`'s `snapshot_holders`).
//! - **The stress run** (its files with its selection): a crate passed in
//!   full when each of its selected nextest tests passed as many times as
//!   its count, with no failure, which is what a green stress run asks of
//!   it; a test that failed or ran fewer times keeps its crate red. A crate
//!   whose tests only ran fewer times, none failing, keeps the clean passes
//!   they have, each test's (#2233). The run says which commands went red and
//!   how each ended (`just repeat --until-red`'s `target/stress-red-commands.tsv`):
//!   the files account for a red nextest command only if it ended as a failing
//!   test ends it, with a failing test in them, so a crash or a killed run,
//!   whose report says nothing of the crate behind it, takes everything; a red
//!   command of another runner, in a process of its own, says nothing of the
//!   crates. A run that says nothing of a red command failed elsewhere.
//!
//! Only crates record green or keep passes: Vitest, Playwright and pytest rerun
//! every member in every run (ADR 0135).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use super::plan::Step;
use crate::junit::{self, Ending};
use crate::output;
use crate::runners::Runner;

/// What a failed step's files say of its crates.
enum Attribution {
    /// The files account for the failure.
    Passed {
        /// The crates that passed in full; the step's other members are red.
        crates: BTreeSet<String>,
        /// The crates short of their counts with no failing test, each with
        /// the clean passes this run gave each of its selected tests.
        clean: BTreeMap<String, BTreeMap<Test, u32>>,
    },
    /// Why the files do not account for the failure: every member is red.
    Unattributed(String),
}

/// A nextest test: its binary ID, without a stress run's `@stress-<n>`, and
/// its name, joined by a space, as the stress run's selection names it.
type Test = String;

/// The status nextest exits with when a test failed, and for no other reason.
const NEXTEST_TEST_FAILED: &str = "100";

/// What a failed stress run left beside its nextest files.
pub(super) struct StressEvidence {
    /// Its `<count>\t<runner>\t<test ID>` lines.
    pub(super) selection: PathBuf,
    /// Its `<runner>\t<status>` lines, one a command that went red.
    pub(super) red_commands: PathBuf,
}

/// A failed step's nextest files and, for the stress run, what else it left
/// and when it began, with the workspace's crates.
pub(super) struct Evidence {
    pub(super) junit: Vec<PathBuf>,
    pub(super) stress: Option<StressEvidence>,
    /// When the stress run began, in seconds since the Unix epoch.
    pub(super) started: Option<u64>,
    pub(super) crates: BTreeSet<String>,
    /// The crates a red run of the step does not judge in full whatever
    /// their tests did: after a red Pre-merge suite, those holding insta
    /// snapshots, whose unreferenced snapshots cargo-insta judges only
    /// once every test passed.
    pub(super) unjudged: BTreeSet<String>,
}

/// What a failed step's evidence says of its members.
#[derive(Default)]
pub(super) struct Judged {
    /// Those that passed in full.
    pub(super) passed: BTreeSet<String>,
    /// Those short of their counts with no failing test, with the clean passes
    /// the run gave each of their selected tests.
    pub(super) clean: BTreeMap<String, BTreeMap<String, u32>>,
}

impl Evidence {
    /// What the evidence shows of the crates among `members`, the failed
    /// `step`'s, or nothing when it does not account for the failure. Says
    /// which in the run's log.
    pub(super) fn judge(&self, step: Step, members: &[String]) -> Judged {
        let crates: BTreeSet<String> = members
            .iter()
            .filter(|member| self.crates.contains(*member))
            .cloned()
            .collect();
        match attribute(&crates, &self.junit, self.stress.as_ref()) {
            Attribution::Passed {
                crates: passed,
                clean,
            } => {
                let unjudged: BTreeSet<String> =
                    passed.intersection(&self.unjudged).cloned().collect();
                let passed: BTreeSet<String> = passed.difference(&unjudged).cloned().collect();
                let failed: BTreeSet<String> = crates
                    .difference(&passed)
                    .filter(|krate| !unjudged.contains(*krate))
                    .cloned()
                    .collect();
                output::error(&format!(
                    "Gate: {} failed in {}; its other crates passed in full and are recorded green: {}",
                    step.name(),
                    listed(&failed),
                    listed(&passed)
                ));
                if !unjudged.is_empty() {
                    output::error(&format!(
                        "Gate: {} hold insta snapshots, which cargo-insta judges unreferenced only once every test passed, so they are recorded red",
                        listed(&unjudged)
                    ));
                }
                Judged { passed, clean }
            }
            Attribution::Unattributed(why) => {
                output::error(&format!(
                    "Gate: {} failed and {why}, so each of its members is recorded red",
                    step.name()
                ));
                Judged::default()
            }
        }
    }
}

/// Crates as the log lists them: joined by spaces, or `none`.
fn listed(crates: &BTreeSet<String>) -> String {
    if crates.is_empty() {
        "none".to_owned()
    } else {
        crates.iter().cloned().collect::<Vec<_>>().join(" ")
    }
}

/// How many times each test passed and failed in the files.
#[derive(Default)]
struct Runs {
    passed: u32,
    failed: u32,
}

/// The crates among `crates`, the step's, that passed in full by the nextest
/// files in `junit` (files, or folders of `nextest*.xml` files) and, for a
/// stress run, the files it left.
fn attribute(
    crates: &BTreeSet<String>,
    junit: &[PathBuf],
    stress: Option<&StressEvidence>,
) -> Attribution {
    let runs = match tally(junit) {
        Ok(runs) => runs,
        Err(why) => return Attribution::Unattributed(why),
    };
    let failed: BTreeSet<String> = runs
        .iter()
        .filter(|(_, runs)| runs.failed > 0)
        .map(|(test, _)| crate_of(test).to_owned())
        .collect();
    // A stress run's crates are judged by its selection; the Quick suite's
    // by its failing tests, of which there must be one.
    let (failing, clean) = if let Some(stress) = stress {
        if let Err(why) = red_commands_accounted_for(&stress.red_commands, &failed) {
            return Attribution::Unattributed(why);
        }
        match std::fs::read_to_string(&stress.selection) {
            Ok(text) => short_of_their_counts(&text, &runs, failed),
            Err(error) => {
                return Attribution::Unattributed(format!(
                    "its selection {} could not be read: {error}",
                    stress.selection.display()
                ));
            }
        }
    } else {
        if failed.is_empty() {
            return Attribution::Unattributed(
                "no nextest test failed in it, so the failure was elsewhere".to_owned(),
            );
        }
        (failed, BTreeMap::new())
    };
    if let Some(stranger) = failing.iter().find(|krate| !crates.contains(*krate)) {
        return Attribution::Unattributed(format!(
            "a failing test is of {stranger}, a crate the step did not run"
        ));
    }
    Attribution::Passed {
        crates: crates.difference(&failing).cloned().collect(),
        clean,
    }
}

/// Why the nextest files, where `failed` are the crates of their failing tests,
/// do not account for the red commands the stress run lists in `file`, if so.
fn red_commands_accounted_for(file: &Path, failed: &BTreeSet<String>) -> Result<(), String> {
    let text = std::fs::read_to_string(file).map_err(|error| {
        format!(
            "it names no red command of its runners in {} ({error}), so it failed elsewhere",
            file.display()
        )
    })?;
    let mut nextest_went_red = false;
    for line in text.lines() {
        let Some((runner, status)) = line.split_once('\t') else {
            return Err(format!("{} holds {line:?}", file.display()));
        };
        if runner == Runner::Nextest.name() {
            nextest_went_red = true;
            if status != NEXTEST_TEST_FAILED {
                return Err(format!(
                    "nextest exited {status}, which no failing test gives, so its files say nothing of the crate behind it"
                ));
            }
        }
    }
    if text.trim().is_empty() {
        return Err(format!("{} names no red command", file.display()));
    }
    if nextest_went_red && failed.is_empty() {
        return Err("nextest went red and no test in its files failed".to_owned());
    }
    Ok(())
}

/// `failed`, the crates of the tests that failed, with those of the stress
/// run's selected nextest tests, `<count>\t<runner>\t<test>` lines, that did
/// not pass `count` times; and, of the latter, those with no failing test,
/// each with its selected tests' clean passes.
fn short_of_their_counts(
    selection: &str,
    runs: &BTreeMap<Test, Runs>,
    failed: BTreeSet<String>,
) -> (BTreeSet<String>, BTreeMap<String, BTreeMap<Test, u32>>) {
    let mut short = BTreeSet::new();
    let mut kept: BTreeMap<String, BTreeMap<Test, u32>> = BTreeMap::new();
    for line in selection.lines() {
        let mut fields = line.splitn(3, '\t');
        let (Some(count), Some("nextest"), Some(test)) =
            (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let passed = runs.get(test).map_or(0, |runs| runs.passed);
        let krate = crate_of(test).to_owned();
        if count.parse::<u32>().ok() != Some(passed) {
            short.insert(krate.clone());
        }
        kept.entry(krate)
            .or_default()
            .insert(test.to_owned(), passed);
    }
    kept.retain(|krate, _| short.contains(krate) && !failed.contains(krate));
    short.extend(failed);
    (short, kept)
}

/// How many times each test passed and failed in the nextest files of
/// `paths`, or why they cannot say: a path missing, a file unreadable or
/// unfinished.
fn tally(paths: &[PathBuf]) -> Result<BTreeMap<Test, Runs>, String> {
    let mut files = Vec::new();
    for path in paths {
        if path.is_dir() {
            let entries = std::fs::read_dir(path)
                .map_err(|error| format!("{} could not be listed: {error}", path.display()))?;
            for entry in entries {
                let file = entry
                    .map_err(|error| format!("{} could not be listed: {error}", path.display()))?
                    .path();
                let name = file
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("");
                let xml = file
                    .extension()
                    .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"));
                if xml && junit::runner(name) == "nextest" {
                    files.push(file);
                }
            }
        } else if path.is_file() {
            files.push(path.clone());
        } else {
            return Err(format!("its nextest file {} is missing", path.display()));
        }
    }
    let mut runs: BTreeMap<Test, Runs> = BTreeMap::new();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .map_err(|error| format!("{} could not be read: {error}", file.display()))?;
        let cases = junit::test_cases("nextest", &text)
            .map_err(|error| format!("{} could not be read: {error:#}", file.display()))?;
        let declared = junit::declared_tests(&text)
            .map_err(|error| format!("{} could not be read: {error:#}", file.display()))?;
        if declared != Some(cases.len()) {
            return Err(format!(
                "{} is unfinished: it holds {} test cases of {}",
                file.display(),
                cases.len(),
                declared.map_or_else(
                    || "an undeclared count".to_owned(),
                    |count| count.to_string()
                )
            ));
        }
        for case in cases {
            let entry = runs.entry(without_iteration(&case.test)).or_default();
            match case.ending {
                Ending::Passed => entry.passed += 1,
                Ending::Failed => entry.failed += 1,
                Ending::Skipped => {}
            }
        }
    }
    Ok(runs)
}

/// A test's ID without the `@stress-<n>` a stress run's iteration adds to
/// its binary ID: `crate::it@stress-3 a::test` is `crate::it a::test`.
fn without_iteration(test: &str) -> Test {
    let (binary, name) = test.split_once(' ').unwrap_or((test, ""));
    let binary = binary.split_once('@').map_or(binary, |(binary, _)| binary);
    format!("{binary} {name}")
}

/// The crate of a nextest test: its binary ID up to `::`, its package's
/// name (`goodfetch-engine::it` is `goodfetch-engine`'s).
fn crate_of(test: &str) -> &str {
    let binary = test.split_once(' ').map_or(test, |(binary, _)| binary);
    binary.split_once("::").map_or(binary, |(krate, _)| krate)
}
