//! The spec that owns what an automation issue is about (ADR 0136): a test,
//! a Live check or a benchmark belongs to the spec of the ticket whose
//! merged pull request added it, and its issue goes in that ticket's
//! milestone. The commit that added it is the first in the history to add
//! the text that names it (`git log -S`); the merged pull requests GitHub
//! lists for that commit name their tickets in `Closes #<n>` lines, alone
//! on their line as `just merge` reads them, and one that closes a single
//! ticket, the ticket's own, names it before a batch that closes several;
//! the ticket's parent is the spec. No owner is found when no commit added
//! the text, no merged pull request closes a ticket of a spec, or the
//! tickets belong to more than one spec.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::git;
use crate::github::Api;
use crate::issues::Owner;

/// The text that names what an issue is about where it is first written,
/// and the files to search, a Git pathspec; `None` searches every file.
pub(crate) struct Added {
    pub(crate) text: String,
    pub(crate) files: Option<String>,
}

#[derive(Deserialize)]
struct PullRequest {
    merged_at: Option<serde::de::IgnoredAny>,
    body: Option<String>,
}

#[derive(Deserialize)]
struct Ticket {
    milestone: Option<Milestone>,
    parent_issue_url: Option<String>,
}

#[derive(Deserialize)]
struct Milestone {
    number: u64,
}

pub(crate) struct Owners<'a> {
    api: &'a Api<'a>,
    repository: &'a str,
}

impl<'a> Owners<'a> {
    pub(crate) fn new(api: &'a Api<'a>, repository: &'a str) -> Self {
        Self { api, repository }
    }

    /// The owner of what `added` names, from the history of the checkout at
    /// `root`. Fails on a shallow clone, whose oldest commit seems to add
    /// every file.
    pub(crate) fn of_added(&self, root: &Path, added: &Added) -> Result<Option<Owner>> {
        if git::line(root, &["rev-parse", "--is-shallow-repository"])? == "true" {
            bail!(
                "{} is a shallow clone, so the commit that added `{}` cannot be found: fetch the whole history (`actions/checkout`'s `fetch-depth: 0`)",
                root.display(),
                added.text
            );
        }
        let pickaxe = format!("-S{}", added.text);
        let mut arguments = vec!["log", "--reverse", "--format=%H", &pickaxe, "HEAD", "--"];
        arguments.extend(added.files.as_deref());
        let commits = git::text(root, &arguments)?;
        let Some(commit) = commits.lines().next() else {
            return Ok(None);
        };
        let pull_requests: Vec<PullRequest> = self
            .api
            .get(&format!(
                "/repos/{}/commits/{commit}/pulls",
                self.repository
            ))
            .with_context(|| format!("reading the pull requests of commit {commit}"))?;
        let closed: Vec<BTreeSet<u64>> = pull_requests
            .iter()
            .filter(|pull_request| pull_request.merged_at.is_some())
            .map(|pull_request| closing_lines(pull_request.body.as_deref().unwrap_or_default()))
            .filter(|tickets| !tickets.is_empty())
            .collect();
        let has_own = closed.iter().any(|tickets| tickets.len() == 1);
        let tickets: BTreeSet<u64> = closed
            .iter()
            .filter(|tickets| !has_own || tickets.len() == 1)
            .flatten()
            .copied()
            .collect();
        let mut owners = BTreeSet::new();
        for ticket in tickets {
            owners.extend(self.of_ticket(ticket)?);
        }
        let specs: BTreeSet<u64> = owners.iter().map(|owner| owner.spec).collect();
        let [spec] = specs.into_iter().collect::<Vec<_>>()[..] else {
            return Ok(None);
        };
        let milestones: BTreeSet<Option<u64>> =
            owners.iter().map(|owner| owner.milestone).collect();
        let milestone = match milestones.into_iter().collect::<Vec<_>>()[..] {
            [milestone] => milestone,
            _ => None,
        };
        Ok(Some(Owner { spec, milestone }))
    }

    /// The owner through `ticket`: its parent spec, in its milestone; none
    /// when it has no parent or is no issue the token can read.
    pub(crate) fn of_ticket(&self, ticket: u64) -> Result<Option<Owner>> {
        let issue: Option<Ticket> = self
            .api
            .find(&format!("/repos/{}/issues/{ticket}", self.repository))
            .with_context(|| format!("reading ticket #{ticket}"))?;
        Ok(issue.and_then(|issue| {
            let spec = issue.parent_issue_url?.rsplit('/').next()?.parse().ok()?;
            Some(Owner {
                spec,
                milestone: issue.milestone.map(|milestone| milestone.number),
            })
        }))
    }
}

/// The tickets of a pull request body's lines that hold `Closes #<n>`
/// alone, after an optional list marker (`ci/merge-pr.sh`).
fn closing_lines(body: &str) -> BTreeSet<u64> {
    body.lines()
        .filter_map(|line| {
            let line = line.trim();
            let number = without_list_marker(line)
                .unwrap_or(line)
                .strip_prefix("Closes #")?;
            if number.is_empty() || !number.bytes().all(|byte| byte.is_ascii_digit()) {
                return None;
            }
            number.parse().ok()
        })
        .collect()
}

/// `line` after its list marker (`-`, `*`, `+`, `1.`, `1)`) and the spaces
/// that follow it; `None` when it starts with no marker.
fn without_list_marker(line: &str) -> Option<&str> {
    let rest = line.strip_prefix(['-', '*', '+']).or_else(|| {
        let after_digits = line.trim_start_matches(|character: char| character.is_ascii_digit());
        if after_digits.len() == line.len() {
            return None;
        }
        after_digits.strip_prefix(['.', ')'])
    })?;
    let text = rest.trim_start();
    (text.len() < rest.len()).then_some(text)
}
