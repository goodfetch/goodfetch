//! The axe exception file (ADR 0073; TESTING.md, Accessibility):
//! `console/axe-exceptions.toml`, one `[[exception]]` entry per known false
//! positive of axe, with the rule, the selector and the pages it is excepted
//! on, the reason and its open issue. `@goodfetch/test-support`'s axe fixture
//! reads the file and fails a page the entry names where its selector matches
//! nothing; this check
//! refuses an entry missing a field, or whose issue is not an open issue of
//! the repository, which a test, under ADR 0043's outbound-traffic deny,
//! cannot read.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::num::NonZeroU64;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::environment::Environment;
use crate::github::{self, Api};
use crate::issues;

/// The file, from the workspace's root.
const FILE: &str = "console/axe-exceptions.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExceptionFile {
    #[serde(default)]
    exception: Vec<RawException>,
}

/// An entry as written; a field it lacks is a finding, not a parse error.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawException {
    rule: Option<String>,
    selector: Option<String>,
    /// The URL path patterns of the pages it is excepted on; an empty list
    /// names no page, so it counts as missing.
    pages: Option<Vec<String>>,
    reason: Option<String>,
    issue: Option<NonZeroU64>,
}

fn check(root: &Path, api: &Api<'_>, repository: &str) -> Result<Vec<Finding>> {
    let path = root.join(FILE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: ExceptionFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let mut findings = Vec::new();
    for (index, entry) in file.exception.iter().enumerate() {
        let subject = format!("exception {}", index + 1);
        // A blank string names nothing, as the axe fixture reads it.
        let blank =
            |text: &Option<String>| text.as_deref().is_none_or(|text| text.trim().is_empty());
        let missing: Vec<&str> = [
            ("rule", blank(&entry.rule)),
            ("selector", blank(&entry.selector)),
            ("pages", entry.pages.as_ref().is_none_or(Vec::is_empty)),
            ("reason", blank(&entry.reason)),
            ("issue", entry.issue.is_none()),
        ]
        .into_iter()
        .filter_map(|(field, absent)| absent.then_some(field))
        .collect();
        if !missing.is_empty() {
            findings.push(Finding::new(
                "axe-exception-field-missing",
                subject.clone(),
                format!(
                    "it has no {}; an entry names its rule, selector, pages, reason and open issue (ADR 0073)",
                    missing.join(", ")
                ),
            ));
        }
        let Some(number) = entry.issue else {
            continue;
        };
        let problem = match issues::find(api, repository, number)? {
            None => Some((
                "axe-exception-issue-missing",
                "is no issue of the repository, or the token cannot read it",
            )),
            Some(issue) if !issue.open => Some(("axe-exception-issue-closed", "is closed")),
            Some(_) => None,
        };
        if let Some((kind, why)) = problem {
            findings.push(Finding::new(
                kind,
                subject,
                format!(
                    "issue #{number} {why}; an excepted rule needs its open issue, and the fix that closes it removes the entry (ADR 0073)"
                ),
            ));
        }
    }
    Ok(findings)
}

/// `xtask axe-exceptions`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on an entry of `console/axe-exceptions.toml` missing a field, or
    /// whose issue is not an open issue of the repository.
    Check {
        /// The repository whose issues the entries name, and GitHub's API.
        #[command(flatten)]
        github: github::Arguments,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Check { github } => {
            let api = github.api(environment.token());
            Ok(crate::check::report(&check(
                &environment.root,
                &api,
                &github.repository,
            )?))
        }
    }
}
