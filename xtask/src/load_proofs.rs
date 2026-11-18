//! An issue's acceptance criteria ask no proof under artificial load
//! (TESTING.md, A flaky test found after merge; owner, 2026-11-18): a flaky
//! or timing fix is proven by its cause named and removed, one normal run and
//! the gate, never by `stress-ng`, busy loops or a count of runs in a row.
//! `just load-proofs <issue>` checks one issue before it takes
//! `ready-for-agent` and before a run or relay takes it; the sweep checks
//! every open issue.
//!
//! Only what an issue asks for is checked: the lines of a section whose
//! heading names its acceptance criteria, and every task-list item. A
//! description of what happened under load asks for nothing.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is a repository tool on no Shared path: ADR 0140 counts the regex compiles of product code, through CountedWork"
)]

use std::process::ExitCode;
use std::sync::LazyLock;

use anyhow::{Result, bail};
use regex::Regex;
use serde::Deserialize;
use serde::de::IgnoredAny;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::github::{self, Api, List};

/// `xtask load-proofs`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Checks one issue's acceptance criteria and task-list items: fails on
    /// each line that asks for `stress-ng`, busy loops or runs in a row.
    Check {
        /// The issue's number.
        issue: u64,
        #[command(flatten)]
        github: github::Arguments,
    },
    /// The same check over every open issue; changes nothing in the tracker.
    Sweep {
        #[command(flatten)]
        github: github::Arguments,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Check { issue, github } => {
            let issue: Issue = api(&github, environment)?
                .get(&format!("/repos/{}/issues/{issue}", github.repository))?;
            Ok(check::report(&issue.findings()))
        }
        Command::Sweep { github } => {
            let issues: Vec<Issue> = api(&github, environment)?.get_all(
                &format!("/repos/{}/issues?state=open", github.repository),
                List::Array,
            )?;
            let findings: Vec<Finding> = issues
                .iter()
                .filter(|issue| issue.pull_request.is_none())
                .flat_map(Issue::findings)
                .collect();
            Ok(check::report(&findings))
        }
    }
}

/// The API, with the token the issues are read with: without one, a private
/// repository answers 404 for every issue.
fn api<'a>(github: &'a github::Arguments, environment: &'a Environment) -> Result<Api<'a>> {
    if environment.token().is_none() {
        bail!(
            "no GitHub token to read the issues of {} with: set GITHUB_TOKEN, or log the GitHub CLI in (`gh auth login`)",
            github.repository
        );
    }
    Ok(github.api(environment.token()))
}

/// An issue as GitHub's REST interface answers it.
#[derive(Deserialize)]
struct Issue {
    number: u64,
    body: Option<String>,
    /// Present when the "issue" is a pull request.
    pull_request: Option<IgnoredAny>,
}

impl Issue {
    fn findings(&self) -> Vec<Finding> {
        asks(self.body.as_deref().unwrap_or_default())
            .into_iter()
            .map(|ask| {
                Finding::new(
                    "load-proof",
                    format!("#{} line {}", self.number, ask.line),
                    format!(
                        "asks for \"{}\": a flaky or timing fix is proven by its cause named and removed, one normal run and the gate, never under artificial load (TESTING.md, A flaky test found after merge)",
                        ask.words
                    ),
                )
            })
            .collect()
    }
}

/// One line that asks for a proof under artificial load.
#[derive(Debug, PartialEq, Eq)]
struct Ask {
    /// From 1.
    line: usize,
    /// The words that ask, as written.
    words: String,
}

/// A count, in digits or in the words an issue spells one with.
const COUNT: &str = r"(?:\d+|two|three|four|five|ten|twenty|fifty|a hundred|hundred)";

/// What a passing run may be called between its count and its noun; a run
/// called anything else ("failed runs in a row") counts a product's runs.
const PASSING: &str = r"(?:(?:passing|green|clean|successful|normal)\s+)?";

static PROOF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\bstress-ng\b|\bbusy[- ]?(?:loops?|wait(?:s|ing)?)\b|\b{COUNT}\s+{PASSING}(?:runs?|passes|iterations|times)\s+(?:in\s+a\s+row|back[- ]to[- ]back)\b|\b{COUNT}\s+consecutive\s+{PASSING}(?:runs?|passes|iterations)\b"
    ))
    .expect("invariant: a valid pattern")
});

/// A Markdown heading, or a line that is only bold text, as `**Acceptance
/// criteria**` heads a section in older issues.
static HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:#{1,6}\s+.*|\*\*[^*]+\*\*:?)\s*$").expect("invariant: a valid pattern")
});

static TASK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+\[[ xX]\]").expect("invariant: a valid pattern"));

/// Each line of `body` that asks for a proof under artificial load.
fn asks(body: &str) -> Vec<Ask> {
    let mut in_acceptance = false;
    let mut asks = Vec::new();
    for (index, line) in body.lines().enumerate() {
        if HEADING.is_match(line) {
            in_acceptance = line.to_lowercase().contains("acceptance");
            continue;
        }
        if !(in_acceptance || TASK.is_match(line)) {
            continue;
        }
        if let Some(found) = PROOF.find(line) {
            asks.push(Ask {
                line: index + 1,
                words: found.as_str().to_owned(),
            });
        }
    }
    asks
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A heading that names its acceptance criteria starts the section, and
    /// the next heading ends it.
    #[test]
    fn the_acceptance_section_ends_at_the_next_heading() {
        let body = "### Acceptance criteria\n\nIt passes 50 runs in a row.\n\n### Notes\n\nIt failed 50 runs in a row.\n";

        assert_eq!(
            asks(body),
            vec![Ask {
                line: 3,
                words: "50 runs in a row".to_owned()
            }]
        );
    }
}
