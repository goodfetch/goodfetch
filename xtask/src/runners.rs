//! The workspace's four test runners, nextest, Vitest, Playwright and
//! pytest, as every command that names a test knows them: a test's ID as its
//! runner names it, each runner's listing of the workspace's tests
//! ([`listing`]), and the patterns that select tests on its command line.
//! The quarantine list, the stress run, the Journey check, the gate and
//! `xtask repeat` all name tests this way.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

pub(crate) mod listing;

use std::fmt;

use anyhow::{Result, bail};
use serde::Deserialize;

/// A test runner: the quarantine list, the stress run and the gate name
/// tests by it.
#[derive(Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Runner {
    Nextest,
    Vitest,
    Playwright,
    Pytest,
}

impl Runner {
    pub(crate) const ALL: [Self; 4] = [Self::Nextest, Self::Vitest, Self::Playwright, Self::Pytest];

    /// The runner of `name`, as the list writes it.
    pub(crate) fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|runner| runner.name() == name)
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

/// The pattern that matches exactly `tests` of Playwright, which matches a
/// pattern against the project, the file and the titles joined by spaces.
pub(crate) fn playwright_pattern(tests: &[&TestId]) -> String {
    let (before, after) = PLAYWRIGHT_AROUND;
    let paths = tests
        .iter()
        .map(|test| playwright_alternative(test))
        .collect::<Vec<_>>()
        .join("|");
    format!("{before}{paths}{after}")
}

/// What a Playwright pattern of tests puts before and after their
/// alternatives, joined by `|`, so that each matches a whole path.
pub(crate) const PLAYWRIGHT_AROUND: (&str, &str) = ("(?:^| )(?:", ")(?: |$)");

/// A Playwright test's alternative in such a pattern: its path, literal.
pub(crate) fn playwright_alternative(test: &TestId) -> String {
    alternation(std::iter::once(test.0.replace(" › ", " ")))
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
