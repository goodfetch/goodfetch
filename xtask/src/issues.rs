//! The issues the checks keep: one per flaky test or Live check (`xtask
//! results scan`), one per drifting performance figure (`xtask perf drift`)
//! and one for a Release run past the minute ceiling (`xtask release-suite
//! plan`), each found again by a marker line in its body, so its title can
//! be edited. A new one goes to the current milestone, the earliest open
//! milestone with an open issue
//! (docs/agents/issue-tracker.md, Frontier); an open one gets the current
//! results; a closed one reopens only for a result after it closed. The
//! check of the file that names an issue (the quarantine list) reads it
//! through [`find`]. The watchlist job's issues
//! (`xtask watchlist`) are opened once, outside every milestone, and never
//! reopened: closing one declines its vendor.

use std::num::NonZeroU64;

use anyhow::{Context as _, Result, bail};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::github::{Api, List, Write};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Label {
    FlakyTest,
    LiveCheck,
    NeedsTriage,
    Watchlist,
}

impl Label {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::FlakyTest => "flaky-test",
            Self::LiveCheck => "live-check",
            Self::NeedsTriage => "needs-triage",
            Self::Watchlist => "watchlist",
        }
    }
}

#[derive(Deserialize)]
struct Issue {
    number: u64,
    state: State,
    closed_at: Option<Timestamp>,
    body: Option<String>,
    /// Present when the "issue" is a pull request.
    pull_request: Option<Value>,
}

impl Issue {
    /// Whether a line of its body is `marker`.
    fn marked(&self, marker: &str) -> bool {
        self.body
            .as_deref()
            .is_some_and(|text| text.lines().any(|line| line == marker))
    }
}

/// The line that finds the issue of `key` under `label` again.
fn marker(label: Label, key: &str) -> String {
    format!("<!-- goodfetch-results: {} {key} -->", label.name())
}

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum State {
    Open,
    Closed,
}

/// An issue a file names, as its check reads it.
pub(crate) struct NamedIssue {
    pub(crate) open: bool,
    /// Its labels' names.
    pub(crate) labels: Vec<String>,
}

#[derive(Deserialize)]
struct ReadIssue {
    state: State,
    #[serde(default)]
    labels: Vec<ReadLabel>,
    /// Present when the number is a pull request's.
    pull_request: Option<serde::de::IgnoredAny>,
}

#[derive(Deserialize)]
struct ReadLabel {
    name: String,
}

/// Issue `number` of `repository`; `None` when the number is no issue of the
/// repository (GitHub answers 404, or it is a pull request's) or the token
/// cannot read it. Without a token it fails: a private repository answers
/// 404 for every issue then, which would read as each one missing.
pub(crate) fn find(
    api: &Api<'_>,
    repository: &str,
    number: NonZeroU64,
) -> Result<Option<NamedIssue>> {
    if api.token.is_none() {
        bail!(
            "no GitHub token to read issue #{number} of {repository} with: set GITHUB_TOKEN, or log the GitHub CLI in (`gh auth login`)"
        );
    }
    let issue: Option<ReadIssue> = api.find(&format!("/repos/{repository}/issues/{number}"))?;
    Ok(issue
        .filter(|issue| issue.pull_request.is_none())
        .map(|issue| NamedIssue {
            open: issue.state == State::Open,
            labels: issue.labels.into_iter().map(|label| label.name).collect(),
        }))
}

#[derive(Deserialize)]
struct Milestone {
    number: u64,
    open_issues: u64,
    due_on: Option<Timestamp>,
}

#[derive(Deserialize)]
struct Created {
    number: u64,
}

pub(crate) struct Issues<'a> {
    api: &'a Api<'a>,
    repository: &'a str,
    /// Each label's issues, read once.
    listed: Vec<(Label, Vec<Issue>)>,
    milestone: CurrentMilestone,
    actions: Vec<String>,
}

/// The current milestone, read once, when an issue first needs it.
#[derive(Clone, Copy)]
enum CurrentMilestone {
    Unread,
    /// The milestone's number, or none when no open milestone has an open issue.
    Read(Option<u64>),
}

impl<'a> Issues<'a> {
    pub(crate) fn new(api: &'a Api<'a>, repository: &'a str) -> Self {
        Self {
            api,
            repository,
            listed: Vec::new(),
            milestone: CurrentMilestone::Unread,
            actions: Vec::new(),
        }
    }

    /// What the scan did, one line each.
    pub(crate) fn actions(&self) -> &[String] {
        &self.actions
    }

    /// Opens the issue of `key` under `label`, updates its body while it is
    /// open, or reopens it when `latest` came after it closed.
    pub(crate) fn keep(
        &mut self,
        label: Label,
        key: &str,
        title: &str,
        body: &str,
        latest: Timestamp,
    ) -> Result<()> {
        let marker = marker(label, key);
        let body = format!("{marker}\n{body}");
        let existing = self
            .issues(label)?
            .iter()
            .find(|issue| issue.marked(&marker))
            .map(|issue| {
                (
                    issue.number,
                    issue.state,
                    issue.closed_at,
                    issue.body.clone(),
                )
            });
        let issues = format!("/repos/{}/issues", self.repository);
        match existing {
            None => {
                let milestone = self.current_milestone()?;
                let created: Created = self.api.send(
                    Write::Post,
                    &issues,
                    &json!({
                        "title": title,
                        "body": body,
                        "labels": [label.name()],
                        "milestone": milestone,
                    }),
                )?;
                self.actions
                    .push(format!("opened #{}: {title}", created.number));
            }
            Some((number, State::Open, _, current)) => {
                if current.as_deref() != Some(body.as_str()) {
                    let _: Value = self.api.send(
                        Write::Patch,
                        &format!("{issues}/{number}"),
                        &json!({ "body": body }),
                    )?;
                    self.actions.push(format!("updated #{number}: {title}"));
                }
            }
            Some((number, State::Closed, closed_at, _)) => {
                if closed_at.is_none_or(|closed_at| latest > closed_at) {
                    let milestone = self.current_milestone()?;
                    let _: Value = self.api.send(
                        Write::Patch,
                        &format!("{issues}/{number}"),
                        &json!({ "state": "open", "body": body, "milestone": milestone }),
                    )?;
                    self.actions.push(format!("reopened #{number}: {title}"));
                }
            }
        }
        Ok(())
    }

    /// Opens the issue of `key` under `label` and `needs-triage`, outside
    /// every milestone, unless one exists, open or closed; true when it
    /// opened one.
    pub(crate) fn open_once(
        &mut self,
        label: Label,
        key: &str,
        title: &str,
        body: &str,
    ) -> Result<bool> {
        let marker = marker(label, key);
        if self
            .issues(label)?
            .iter()
            .any(|issue| issue.marked(&marker))
        {
            return Ok(false);
        }
        let created: Created = self.api.send(
            Write::Post,
            &format!("/repos/{}/issues", self.repository),
            &json!({
                "title": title,
                "body": format!("{marker}\n{body}"),
                "labels": [label.name(), Label::NeedsTriage.name()],
            }),
        )?;
        self.actions
            .push(format!("opened #{}: {title}", created.number));
        Ok(true)
    }

    fn issues(&mut self, label: Label) -> Result<&[Issue]> {
        if !self.listed.iter().any(|(listed, _)| *listed == label) {
            let path = format!(
                "/repos/{}/issues?labels={}&state=all",
                self.repository,
                label.name()
            );
            let mut issues: Vec<Issue> = self.api.get_all(&path, List::Array)?;
            issues.retain(|issue| issue.pull_request.is_none());
            self.listed.push((label, issues));
        }
        Ok(self
            .listed
            .iter()
            .find(|(listed, _)| *listed == label)
            .map_or(&[], |(_, issues)| issues.as_slice()))
    }

    /// The earliest open milestone that still has an open issue, by due date
    /// and then by number; `None` when there is none.
    fn current_milestone(&mut self) -> Result<Option<u64>> {
        if let CurrentMilestone::Read(milestone) = self.milestone {
            return Ok(milestone);
        }
        let milestones: Vec<Milestone> = self
            .api
            .get_all(
                &format!("/repos/{}/milestones?state=open", self.repository),
                List::Array,
            )
            .context("reading the milestones")?;
        let current = milestones
            .iter()
            .filter(|milestone| milestone.open_issues > 0)
            .min_by_key(|milestone| {
                (
                    milestone.due_on.is_none(),
                    milestone.due_on,
                    milestone.number,
                )
            })
            .map(|milestone| milestone.number);
        self.milestone = CurrentMilestone::Read(current);
        Ok(current)
    }
}
