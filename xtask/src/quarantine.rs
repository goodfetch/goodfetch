//! The test quarantine list (ADR 0064; TESTING.md, A flaky test found after
//! merge): `quarantine.toml` at the root, one `[[test]]` entry per flaky test
//! with its runner, its ID as that runner names it, its open `flaky-test`
//! issue and the day it was quarantined. The check refuses an entry whose
//! issue is not an open `flaky-test` issue, or whose test its runner no
//! longer lists; `exclude` gives each runner the arguments that skip the
//! listed tests, which every suite command appends. `xtask release` refuses
//! while the list holds any entry (ADR 0064), reading it through [`read`].
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::num::NonZeroU64;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use toml::value::Datetime;

use crate::check::Finding;
use crate::environment::Environment;
use crate::github::{self, Api};
use crate::issues::{self, Label as IssueLabel};
use crate::output;
use crate::runners::{self, Runner, TestId, listing};

/// The list's file, at the workspace's root.
pub(crate) const LIST: &str = "quarantine.toml";

/// One quarantined test.
pub(crate) struct Entry {
    pub(crate) runner: Runner,
    pub(crate) test: TestId,
    pub(crate) issue: NonZeroU64,
    /// The day it was quarantined.
    pub(crate) date: Datetime,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListFile {
    #[serde(default, rename = "test")]
    tests: Vec<RawEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    runner: Runner,
    id: String,
    issue: NonZeroU64,
    date: Datetime,
}

impl TryFrom<RawEntry> for Entry {
    type Error = anyhow::Error;

    fn try_from(raw: RawEntry) -> Result<Self> {
        let test = TestId::parse(raw.runner, &raw.id)?;
        if raw.date.date.is_none() || raw.date.time.is_some() {
            bail!(
                "test `{}` has a date with a time; give the day only",
                raw.id
            );
        }
        Ok(Self {
            runner: raw.runner,
            test,
            issue: raw.issue,
            date: raw.date,
        })
    }
}

/// The list's entries, in the file's order.
pub(crate) fn read(root: &Path) -> Result<Vec<Entry>> {
    let path = root.join(LIST);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    parse(&text).with_context(|| format!("reading {}", path.display()))
}

/// The entries of a list's text, such as a commit's `quarantine.toml`.
pub(crate) fn parse(text: &str) -> Result<Vec<Entry>> {
    let file: ListFile = toml::from_str(text).context("parsing the quarantine list")?;
    file.tests.into_iter().map(Entry::try_from).collect()
}

/// Where the check reads issues and lists tests.
pub(crate) struct Sources<'a> {
    pub(crate) api: Api<'a>,
    /// The repository whose issues the entries name, as `owner/name`.
    pub(crate) repository: &'a str,
    pub(crate) cargo: &'a OsStr,
    /// The Cargo profile nextest builds the listing with, to share the suite's build.
    pub(crate) cargo_profile: Option<&'a str>,
}

pub(crate) fn check(root: &Path, sources: &Sources<'_>) -> Result<Vec<Finding>> {
    let entries = read(root)?;
    let mut findings = issue_findings(&entries, &sources.api, sources.repository)?;

    let runners: BTreeSet<Runner> = entries.iter().map(|entry| entry.runner).collect();
    for runner in runners {
        let listed = listing::tests(runner, root, sources.cargo, sources.cargo_profile)?;
        for entry in entries.iter().filter(|entry| entry.runner == runner) {
            if !listed.contains(&entry.test) {
                findings.push(Finding::new(
                    "quarantined-test-missing",
                    entry.test.to_string(),
                    format!(
                        "{} lists no such test, quarantined on {}; the change that removed or renamed it takes it off the list",
                        runner.name(),
                        entry.date
                    ),
                ));
            } else if runner == Runner::Nextest {
                let name = entry.test.nextest_name();
                let others = listed
                    .iter()
                    .filter(|other| *other != &entry.test && other.nextest_name().contains(name))
                    .count();
                if others > 0 {
                    findings.push(Finding::new(
                        "quarantine-skips-other-tests",
                        entry.test.to_string(),
                        format!(
                            "nextest's `--skip {name}` matches by substring and would also skip {others} other test(s)"
                        ),
                    ));
                }
            }
        }
    }
    Ok(findings)
}

/// What lint's check reads from GitHub, read now, for `xtask gate plan`
/// before it reuses a green lint on the same tree: `Ok` when each test the
/// list quarantines has its open `flaky-test` issue, or why not. An empty
/// list reads nothing.
pub(crate) fn issues_hold(
    root: &Path,
    github: &github::Arguments,
    token: Option<&str>,
) -> Result<(), String> {
    let entries = read(root).map_err(|error| format!("{error:#}"))?;
    let findings = issue_findings(&entries, &github.api(token), &github.repository)
        .map_err(|error| format!("the quarantined tests' issues cannot be read: {error:#}"))?;
    match findings.first() {
        None => Ok(()),
        Some(finding) => Err(format!(
            "lint's quarantine check: {}: {}",
            finding.subject, finding.message
        )),
    }
}

/// The findings on each entry's issue, which must be an open `flaky-test`
/// issue of `repository`.
fn issue_findings(entries: &[Entry], api: &Api<'_>, repository: &str) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for entry in entries {
        let issue = issues::find(api, repository, entry.issue)?;
        let problem = match issue {
            None => Some((
                "quarantine-issue-missing",
                "is no issue of the repository, or the token cannot read it",
            )),
            Some(issue) if !issue.open => Some(("quarantine-issue-closed", "is closed")),
            Some(issue)
                if !issue
                    .labels
                    .iter()
                    .any(|label| label == IssueLabel::FlakyTest.name()) =>
            {
                Some((
                    "quarantine-issue-not-flaky-test",
                    "is not labelled `flaky-test`",
                ))
            }
            Some(_) => None,
        };
        if let Some((kind, why)) = problem {
            findings.push(Finding::new(
                kind,
                entry.test.to_string(),
                format!(
                    "issue #{} {why}; a quarantined test needs its open `flaky-test` issue (ADR 0064)",
                    entry.issue
                ),
            ));
        }
    }
    Ok(findings)
}

/// Each runner's arguments that skip the listed tests, by its name, in the
/// order they go on its command line; none for a runner with no listed test.
/// Every runner has its entry, so a reader never meets a missing one.
pub(crate) fn exclusions(root: &Path) -> Result<BTreeMap<&'static str, Vec<String>>> {
    let entries = read(root)?;
    let tests = |runner: Runner| -> Vec<&TestId> {
        entries
            .iter()
            .filter(|entry| entry.runner == runner)
            .map(|entry| &entry.test)
            .collect()
    };
    let mut arguments: BTreeMap<&'static str, Vec<String>> = Runner::ALL
        .into_iter()
        .map(|runner| (runner.name(), Vec::new()))
        .collect();
    let mut push = |runner: Runner, argument: String| {
        arguments.entry(runner.name()).or_default().push(argument);
    };

    // nextest's filtersets cannot reach it through `cargo insta test`, so its
    // libtest-style `--skip` does, after `--`.
    let nextest = tests(Runner::Nextest);
    if !nextest.is_empty() {
        push(Runner::Nextest, "--".to_owned());
        for test in nextest {
            push(Runner::Nextest, "--skip".to_owned());
            push(Runner::Nextest, test.nextest_name().to_owned());
        }
    }

    // Vitest runs the tests whose full name matches; this matches every name
    // but the listed ones.
    let vitest = tests(Runner::Vitest);
    if !vitest.is_empty() {
        let names = runners::alternation(vitest.iter().map(|test| test.as_str().to_owned()));
        push(
            Runner::Vitest,
            format!("--testNamePattern=^(?!(?:{names})$)"),
        );
    }

    let playwright = tests(Runner::Playwright);
    if !playwright.is_empty() {
        push(
            Runner::Playwright,
            format!("--grep-invert={}", runners::playwright_pattern(&playwright)),
        );
    }

    for test in tests(Runner::Pytest) {
        push(Runner::Pytest, format!("--deselect={test}"));
    }
    Ok(arguments)
}

/// `xtask quarantine`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on an entry whose issue is not an open `flaky-test` issue, or
    /// whose test its runner no longer lists.
    Check {
        /// The repository whose issues the entries name, and GitHub's API.
        #[command(flatten)]
        github: github::Arguments,
        /// The Cargo profile nextest builds its listing with.
        #[arg(long)]
        cargo_profile: Option<String>,
    },
    /// Prints, as one JSON object, the arguments that make each runner skip
    /// the listed tests: each runner's name and its arguments in their
    /// order, `{"nextest": ["--", "--skip", "<test>"], "vitest": [], …}`.
    Exclude,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    match command {
        Command::Check {
            github,
            cargo_profile,
        } => {
            let sources = Sources {
                api: github.api(environment.token()),
                repository: &github.repository,
                cargo: &environment.cargo,
                cargo_profile: cargo_profile.as_deref(),
            };
            Ok(crate::check::report(&check(root, &sources)?))
        }
        Command::Exclude => {
            output::line(
                &serde_json::to_string(&exclusions(root)?)
                    .context("writing the exclusions as JSON")?,
            );
            Ok(ExitCode::SUCCESS)
        }
    }
}
