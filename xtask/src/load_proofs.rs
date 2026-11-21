//! An issue's acceptance criteria ask no proof under artificial load
//! (TESTING.md, A flaky test found after merge; owner, 2026-11-18): a flaky
//! or timing fix is proven by its cause named and removed, one normal run and
//! the gate, never by `stress-ng`, busy loops or a count of runs in a row
//! (ten or more, or `just repeat` with such a count: a product's own rules
//! count fewer).
//! `just load-proofs <issue>` checks one issue before it takes
//! `ready-for-agent` and before a run or relay takes it; the sweep checks
//! every open issue.
//!
//! Only what an issue asks for is checked: the lines of a section whose
//! heading names its acceptance criteria, down to the next heading as high
//! or higher, and every task-list item. A description of what happened under
//! load asks for nothing.
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
use crate::github::{self, List};

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
            let issue: Issue = github
                .authenticated(environment.token())?
                .get(&format!("/repos/{}/issues/{issue}", github.repository))?;
            if issue.pull_request.is_some() {
                bail!("#{} is a pull request, not an issue", issue.number);
            }
            Ok(check::report(&issue.findings()))
        }
        Command::Sweep { github } => {
            let issues: Vec<Issue> = github.authenticated(environment.token())?.get_all(
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

/// A count of ten or more, in digits or in the words an issue spells one
/// with: a product's own rules count fewer ("retries three times in a row").
const COUNT: &str = r"(?:[1-9]\d+|ten|twenty|fifty|a hundred|hundred)";

/// What a passing run may be called between its count and its noun; a run
/// called anything else ("failed runs in a row") counts a product's runs.
const PASSING: &str = r"(?:(?:passing|green|clean|successful|normal)\s+)?";

static PROOF: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"(?i)\bstress-ng\b|\bbusy[- ]?(?:loops?|wait(?:s|ing)?)\b|\b{COUNT}\s+{PASSING}(?:runs?|passes|iterations|times)\s+(?:in\s+a\s+row|back[- ]to[- ]back)\b|\b{COUNT}\s+consecutive\s+{PASSING}(?:runs?|passes|iterations)\b|\bjust\s+repeat\s+{COUNT}\b"
    ))
    .expect("invariant: a valid pattern")
});

/// A Markdown heading, its `#`s captured, or a line that is only bold text,
/// as `**Acceptance criteria**` heads a section in older issues.
static HEADING: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"^(?:(#{1,6})\s+.*|\*\*[^*]+\*\*:?)\s*$").expect("invariant: a valid pattern")
});

/// A bold-only line heads a section below every Markdown heading.
const BOLD_LEVEL: usize = 7;

static TASK: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*[-*+]\s+\[[ xX]\]").expect("invariant: a valid pattern"));

/// Each line of `body` that asks for a proof under artificial load.
fn asks(body: &str) -> Vec<Ask> {
    // The level of the heading that opened the acceptance criteria, while
    // their section lasts.
    let mut acceptance: Option<usize> = None;
    let mut in_code = false;
    let mut asks = Vec::new();
    for (index, line) in body.lines().enumerate() {
        if line.trim_start().starts_with("```") {
            in_code = !in_code;
        } else if !in_code && let Some(heading) = HEADING.captures(line) {
            let level = heading.get(1).map_or(BOLD_LEVEL, |hashes| hashes.len());
            if acceptance.is_some_and(|opened| level <= opened) {
                acceptance = None;
            }
            if acceptance.is_none() && line.to_lowercase().contains("acceptance") {
                acceptance = Some(level);
            }
            continue;
        }
        if !(acceptance.is_some() || TASK.is_match(line)) {
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

    /// A heading that names the acceptance criteria starts their section, a
    /// heading as high or higher ends it, a deeper one or a line in a code
    /// block does not, and a bold-only line is the deepest heading.
    #[rstest::rstest]
    #[case::the_next_heading("### Acceptance criteria\n\nIt passes 50 runs in a row.\n\n### Notes\n\nIt failed 50 runs in a row.\n", &[3])]
    #[case::a_deeper_heading("## Acceptance criteria\n\n### Timing\n\nIt passes 50 runs in a row.\n\n# Notes\n\nIt failed 50 runs in a row.\n", &[5])]
    #[case::a_code_block("## Acceptance criteria\n\n```sh\n# run it\n```\n\nIt passes 50 runs in a row.\n", &[7])]
    #[case::bold_headings("**Acceptance criteria**\n\nIt passes 50 runs in a row.\n\n**Notes**\n\nIt failed 50 runs in a row.\n", &[3])]
    #[case::bold_inside("## Acceptance criteria\n\n**Timing**\n\nIt passes 50 runs in a row.\n", &[5])]
    fn the_acceptance_section_holds_its_lines(#[case] body: &str, #[case] lines: &[usize]) {
        let found: Vec<usize> = asks(body).iter().map(|ask| ask.line).collect();

        assert_eq!(found, lines);
    }
}
