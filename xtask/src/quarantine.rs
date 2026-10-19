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

mod listing;

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::fmt;
use std::num::NonZeroU64;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use toml::value::Datetime;

use crate::check::Finding;
use crate::environment::Environment;
use crate::github::{self, Api};
use crate::output;

/// The list's file, at the workspace's root.
pub(crate) const LIST: &str = "quarantine.toml";

/// The label every quarantined test's issue carries (ADR 0064).
const FLAKY_TEST_LABEL: &str = "flaky-test";

/// A test runner whose tests the list may name.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Runner {
    Nextest,
    Vitest,
    Playwright,
    Pytest,
}

impl Runner {
    /// The runner of `name`, as the list writes it.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        [Self::Nextest, Self::Vitest, Self::Playwright, Self::Pytest]
            .into_iter()
            .find(|runner| runner.name() == name)
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Nextest => "nextest",
            Self::Vitest => "vitest",
            Self::Playwright => "playwright",
            Self::Pytest => "pytest",
        }
    }
}

/// A test as its runner names it:
/// - nextest: `<binary ID> <test name>`, as `cargo nextest list` prints it;
/// - Vitest: the full name, its suites and title joined by ` > `;
/// - Playwright: the file, then its suites and title, joined by ` › `;
/// - pytest: the node ID, `tests/test_page.py::test_title`.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct TestId(String);

impl TestId {
    /// The ID of a test of `runner`; a nextest ID names its binary and its test.
    pub(crate) fn parse(runner: Runner, id: &str) -> Result<Self> {
        if id.trim().is_empty() {
            bail!("a test ID is empty");
        }
        if runner == Runner::Nextest && !id.contains(' ') {
            bail!("nextest test `{id}` is not `<binary ID> <test name>`");
        }
        Ok(Self(id.to_owned()))
    }

    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The test's name within its binary, which nextest's `--skip` matches.
    pub(crate) fn nextest_name(&self) -> &str {
        self.0.split_once(' ').map_or(&self.0, |(_, name)| name)
    }

    /// The nextest binary's ID, the part before the first space.
    pub(crate) fn nextest_binary(&self) -> &str {
        self.0.split_once(' ').map_or(&self.0, |(binary, _)| binary)
    }
}

impl fmt::Display for TestId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

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
        if raw.id.trim().is_empty() {
            bail!("an entry has an empty test ID");
        }
        if raw.runner == Runner::Nextest && !raw.id.contains(' ') {
            bail!("nextest test `{}` is not `<binary ID> <test name>`", raw.id);
        }
        if raw.date.date.is_none() || raw.date.time.is_some() {
            bail!(
                "test `{}` has a date with a time; give the day only",
                raw.id
            );
        }
        Ok(Self {
            runner: raw.runner,
            test: TestId(raw.id),
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
    let file: ListFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    file.tests
        .into_iter()
        .map(Entry::try_from)
        .collect::<Result<_>>()
        .with_context(|| format!("reading {}", path.display()))
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

#[derive(Deserialize)]
struct Issue {
    state: String,
    #[serde(default)]
    labels: Vec<Label>,
    /// Present when the number is a pull request's.
    pull_request: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

pub(crate) fn check(root: &Path, sources: &Sources<'_>) -> Result<Vec<Finding>> {
    let entries = read(root)?;
    let mut findings = Vec::new();
    for entry in &entries {
        let path = format!("/repos/{}/issues/{}", sources.repository, entry.issue);
        let issue: Option<Issue> = sources.api.find(&path)?;
        let problem = match issue {
            None
            | Some(Issue {
                pull_request: Some(_),
                ..
            }) => Some((
                "quarantine-issue-missing",
                "is no issue of the repository, or the token cannot read it",
            )),
            Some(issue) if issue.state != "open" => Some(("quarantine-issue-closed", "is closed")),
            Some(issue)
                if !issue
                    .labels
                    .iter()
                    .any(|label| label.name == FLAKY_TEST_LABEL) =>
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

/// Each runner's arguments that skip the listed tests, as `(runner, argument)`
/// pairs in the order they go on its command line; none for a runner with no
/// listed test.
pub(crate) fn exclusions(root: &Path) -> Result<Vec<(&'static str, String)>> {
    let entries = read(root)?;
    let tests = |runner: Runner| -> Vec<&TestId> {
        entries
            .iter()
            .filter(|entry| entry.runner == runner)
            .map(|entry| &entry.test)
            .collect()
    };
    let mut arguments = Vec::new();

    // nextest's filtersets cannot reach it through `cargo insta test`, so its
    // libtest-style `--skip` does, after `--`.
    let nextest = tests(Runner::Nextest);
    if !nextest.is_empty() {
        arguments.push(("nextest", "--".to_owned()));
        for test in nextest {
            arguments.push(("nextest", "--skip".to_owned()));
            arguments.push(("nextest", test.nextest_name().to_owned()));
        }
    }

    // Vitest runs the tests whose full name matches; this matches every name
    // but the listed ones.
    let vitest = tests(Runner::Vitest);
    if !vitest.is_empty() {
        let names = alternation(vitest.iter().map(|test| test.0.clone()));
        arguments.push(("vitest", format!("--testNamePattern=^(?!(?:{names})$)")));
    }

    // Playwright matches its pattern against the project, the file and the
    // titles joined by spaces.
    let playwright = tests(Runner::Playwright);
    if !playwright.is_empty() {
        let paths = alternation(playwright.iter().map(|test| test.0.replace(" › ", " ")));
        arguments.push((
            "playwright",
            format!("--grep-invert=(?:^| )(?:{paths})(?: |$)"),
        ));
    }

    for test in tests(Runner::Pytest) {
        arguments.push(("pytest", format!("--deselect={test}")));
    }
    Ok(arguments)
}

/// The texts as a JavaScript regular expression's alternatives, each literal.
pub(crate) fn alternation(texts: impl Iterator<Item = String>) -> String {
    texts
        .map(|text| {
            text.chars()
                .flat_map(|character| {
                    let escaped = "\\^$.|?*+()[]{}".contains(character);
                    escaped.then_some('\\').into_iter().chain([character])
                })
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("|")
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
    /// Prints, as `<runner>\t<argument>` lines, the arguments that make each
    /// runner skip the listed tests.
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
            for (runner, argument) in exclusions(root)? {
                output::line(&format!("{runner}\t{argument}"));
            }
            Ok(ExitCode::SUCCESS)
        }
    }
}
