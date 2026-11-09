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
//!   they have, each test's (#2233).
//!
//! Only crates record green or keep passes: Vitest, Playwright and pytest rerun
//! every member in every run (ADR 0135).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use crate::junit::{self, Ending};

/// What a failed step's files say of its crates.
pub(super) enum Attribution {
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
pub(super) type Test = String;

/// How many times each test passed and failed in the files.
#[derive(Default)]
struct Runs {
    passed: u32,
    failed: u32,
}

/// The crates among `crates`, the step's, that passed in full by the nextest
/// files in `junit` (files, or folders of `nextest*.xml` files) and, for a
/// stress run, its `selection`.
pub(super) fn attribute(
    crates: &BTreeSet<String>,
    junit: &[PathBuf],
    selection: Option<&Path>,
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
    let (failing, clean) = if let Some(selection) = selection {
        match std::fs::read_to_string(selection) {
            Ok(text) => short_of_their_counts(&text, &runs, failed),
            Err(error) => {
                return Attribution::Unattributed(format!(
                    "its selection {} could not be read: {error}",
                    selection.display()
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
