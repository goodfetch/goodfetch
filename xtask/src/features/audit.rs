//! The exercise audit (ADR 0072): a Rust test that declares an
//! `operation:`, `error:`, `event:`, `command:` or `exit:` Feature must have
//! reached it while it ran, so a declaration never claims what the test
//! passed by. What a test reached is its exercise record, which test support
//! writes under `just test`: one JSON-lines file per test, whose first line
//! is `{"test": "<binary id> <test name>"}` and whose other lines are
//! `tracing`'s JSON log lines, `{"target": …, "fields": {…}, …}`:
//! - `rpc`'s dispatch records (target [`DISPATCH_TARGET`]), copied from the
//!   engine's JSON log in the test's Goodfetch home or written by the test
//!   `tracing` subscriber of an in-process test: each request's `method`, its
//!   reply's `error_code`, and each `event` sent to a Client;
//! - the engine's outcome records (target [`OUTCOME_TARGET`]), written the
//!   same two ways: each `error_code` a Run recorded and no reply carries,
//!   such as a work item's `handler_lost`;
//! - the command records of test support's CLI helper (target
//!   [`COMMAND_TARGET`]): each `command`, named as its `command:` Feature
//!   names it (`browsers.install`), and its `exit_code`, which the catalog's
//!   `exit_codes` table names; or, for arguments that name no `command:`
//!   Feature, `unmatched`, the arguments joined by spaces.
//!
//! Only Rust tests' declarations are audited: a conformance case's or a
//! Journey's `covers` list leaves no exercise record of its own, and counts
//! as its kept result document gives it (ADR 0072's note).
//!
//! A command run with arguments that match no `command:` Feature is
//! reported, since no declaration can claim it. A declaring test with no
//! record is reported, never passed. Over the tests that ran, a declaring
//! test that did not run while nextest ran tests below its name (an rstest
//! form named apart from nextest) is reported as `name-unmatched`, never
//! left out as a test that did not run. The one exception is ADR 0072's: a test
//! that never reached `rpc` (its record holds no dispatch record, or it is a
//! unit test with no record) may declare the `error:` or `event:` Feature it
//! asserts itself, which no record need show. A test whose record shows it
//! dispatched is judged on every declaration, unit test or not.

#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::scan::{Declaration, Level, Source};
use crate::check::Finding;
use crate::junit;

/// The `tracing` target of `rpc`'s dispatch records.
pub(crate) const DISPATCH_TARGET: &str = "goodfetch::rpc::dispatch";

/// The target of test support's command records.
pub(crate) const COMMAND_TARGET: &str = "goodfetch::test::command";

/// The `tracing` target of the engine's records of an Error code a Run
/// recorded and no reply carries, such as a work item's `handler_lost`.
pub(crate) const OUTCOME_TARGET: &str = "goodfetch::run::outcome";

/// The catalog file `xtask codegen` emits, whose `exit_codes` name each
/// exit code.
const CATALOG: &str = "crates/goodfetch-protocol/generated/catalog.json";

/// The Feature kinds a record can show.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Operation,
    Error,
    Event,
    Command,
    Exit,
}

impl Kind {
    fn of(feature: &str) -> Option<(Self, &str)> {
        let (kind, name) = feature.split_once(':')?;
        let kind = match kind {
            "operation" => Self::Operation,
            "error" => Self::Error,
            "event" => Self::Event,
            "command" => Self::Command,
            "exit" => Self::Exit,
            _ => return None,
        };
        Some((kind, name))
    }
}

/// What one test's records show it reached.
#[derive(Default)]
struct Reached {
    methods: BTreeSet<String>,
    error_codes: BTreeSet<String>,
    events: BTreeSet<String>,
    commands: BTreeSet<String>,
    exit_codes: BTreeSet<u64>,
    /// The arguments of each command that matched no `command:` Feature.
    unmatched: BTreeSet<String>,
    /// Whether any dispatch record shows the test reached `rpc`.
    dispatched: bool,
}

impl Reached {
    fn add(&mut self, line: &Value) {
        let fields = &line["fields"];
        let text = |name: &str| fields[name].as_str().map(ToOwned::to_owned);
        match line["target"].as_str() {
            Some(DISPATCH_TARGET) => {
                self.dispatched = true;
                self.methods.extend(text("method"));
                self.error_codes.extend(text("error_code"));
                self.events.extend(text("event"));
            }
            Some(OUTCOME_TARGET) => self.error_codes.extend(text("error_code")),
            Some(COMMAND_TARGET) => {
                self.commands.extend(text("command"));
                self.unmatched.extend(text("unmatched"));
                self.exit_codes.extend(fields["exit_code"].as_u64());
            }
            _ => {}
        }
    }

    fn holds(&self, kind: Kind, name: &str, exit_codes: &BTreeMap<String, u64>) -> bool {
        match kind {
            Kind::Operation => self.methods.contains(name),
            Kind::Error => self.error_codes.contains(name),
            Kind::Event => self.events.contains(name),
            Kind::Command => self.commands.contains(name),
            Kind::Exit => exit_codes
                .get(name)
                .is_some_and(|code| self.exit_codes.contains(code)),
        }
    }
}

/// The findings of the audit of `declarations` against the records in
/// `records`, over the tests in `ran` or, without it, over every declaring
/// test.
pub(crate) fn audit(
    root: &Path,
    declarations: &[Declaration],
    records: &Path,
    ran: Option<&BTreeSet<String>>,
) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let reached = read_records(root, records, &mut findings)?;
    let exit_codes = exit_codes(root)?;
    for (test, record) in &reached {
        for arguments in &record.unmatched {
            findings.push(Finding::new(
                "command-unmatched",
                format!("`{arguments}` in {test}"),
                "the CLI helper ran a command whose arguments name no `command:` Feature of the inventory",
            ));
        }
    }
    let mut by_test: BTreeMap<&str, Vec<(&Declaration, Kind, &str)>> = BTreeMap::new();
    let mut unmatched = BTreeSet::new();
    for declaration in declarations {
        if declaration.source != Source::Nextest {
            continue;
        }
        if let Some(ran) = ran
            && !ran.contains(&declaration.test)
        {
            // nextest ran tests below this name but not this one: the
            // scanner named the test apart from nextest, so its
            // declarations would go unaudited, not merely unrun.
            let below = format!("{}::", declaration.test);
            if ran
                .range::<str, _>((Bound::Excluded(declaration.test.as_str()), Bound::Unbounded))
                .next()
                .is_some_and(|next| next.starts_with(&below))
                && unmatched.insert(declaration.test.as_str())
            {
                findings.push(Finding::new(
                    "name-unmatched",
                    &declaration.test,
                    "nextest ran tests below this declaring test's name but not the name itself; its declarations name no test that ran",
                ));
            }
            continue;
        }
        let Some((kind, name)) = Kind::of(&declaration.feature) else {
            continue;
        };
        let record = reached.get(declaration.test.as_str());
        // ADR 0072: a test that never reaches `rpc` and asserts a module's
        // own Error code or event through its interface, which only the
        // review sees: one whose record holds no dispatch record, such as a
        // module's test on a loopback stand-in, or a unit test that left no
        // record. A test whose record shows it dispatched is judged at every
        // level.
        let own = matches!(kind, Kind::Error | Kind::Event)
            && record.map_or(declaration.level == Level::Unit, |record| {
                !record.dispatched
            });
        if !own {
            by_test
                .entry(declaration.test.as_str())
                .or_default()
                .push((declaration, kind, name));
        }
    }
    for (test, audited) in by_test {
        let Some(record) = reached.get(test) else {
            let features: Vec<&str> = audited
                .iter()
                .map(|(declaration, ..)| declaration.feature.as_str())
                .collect();
            findings.push(Finding::new(
                "record-missing",
                test,
                format!(
                    "the test declares {} but left no exercise record; run it through `just test`",
                    features.join(", ")
                ),
            ));
            continue;
        };
        for (declaration, kind, name) in audited {
            if !record.holds(kind, name, &exit_codes) {
                findings.push(Finding::new(
                    "unreached",
                    format!("{} in {test}", declaration.feature),
                    "the test declares it, but its exercise record never shows it reached",
                ));
            }
        }
    }
    Ok(findings)
}

/// Every test's records in the folder, by test; a file that is no record is
/// a finding.
fn read_records(
    root: &Path,
    folder: &Path,
    findings: &mut Vec<Finding>,
) -> Result<BTreeMap<String, Reached>> {
    let mut reached: BTreeMap<String, Reached> = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Ok(reached);
    };
    let mut files: Vec<_> = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<_, _>>()
        .with_context(|| format!("listing {}", folder.display()))?;
    files.retain(|path| {
        path.extension()
            .is_some_and(|extension| extension == "jsonl")
    });
    files.sort();
    for path in files {
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let shown = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let mut lines = contents
            .lines()
            .filter(|line| !line.trim().is_empty())
            .map(serde_json::from_str::<Value>);
        let test = lines
            .next()
            .and_then(Result::ok)
            .and_then(|first| first["test"].as_str().map(ToOwned::to_owned));
        let Some(test) = test else {
            findings.push(Finding::new(
                "record-unreadable",
                shown,
                "an exercise record's first line names its test, as `{\"test\": \"<binary id> <test name>\"}`",
            ));
            continue;
        };
        let record = reached.entry(test).or_default();
        let mut unreadable = false;
        for line in lines {
            match line {
                Ok(line) => record.add(&line),
                Err(_) => unreadable = true,
            }
        }
        if unreadable {
            findings.push(Finding::new(
                "record-unreadable",
                shown,
                "an exercise record holds JSON lines only",
            ));
        }
    }
    Ok(reached)
}

/// The catalog's exit codes by name; none when the catalog has no table.
fn exit_codes(root: &Path) -> Result<BTreeMap<String, u64>> {
    let path = root.join(CATALOG);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(BTreeMap::new());
    };
    let catalog: Value =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(catalog["exit_codes"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| Some((entry["name"].as_str()?.to_owned(), entry["code"].as_u64()?)))
        .collect())
}

/// The tests nextest's JUnit files show ran, passed or failed.
pub(crate) fn tests_that_ran(root: &Path, files: &[PathBuf]) -> Result<BTreeSet<String>> {
    let mut ran = BTreeSet::new();
    for file in files {
        let path = root.join(file);
        let contents = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let cases = junit::test_cases("nextest", &contents)
            .with_context(|| format!("reading {}", path.display()))?;
        ran.extend(
            cases
                .into_iter()
                .filter(|case| case.ending != junit::Ending::Skipped)
                .map(|case| case.test),
        );
    }
    Ok(ran)
}
