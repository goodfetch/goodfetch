//! The issues the results store's readers keep: one per flaky test or Live
//! check (`xtask results scan`) and one per drifting performance figure
//! (`xtask perf drift`), found again by a marker line in its body, so its
//! title can be edited. A new one goes to
//! the current milestone, the earliest open milestone with an open issue
//! (docs/agents/issue-tracker.md, Frontier); an open one gets the current
//! results; a closed one reopens only for a result after it closed.

use anyhow::{Context as _, Result};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::github::{Api, Write};

/// GitHub's largest page.
const PAGE: usize = 100;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Label {
    FlakyTest,
    LiveCheck,
    NeedsTriage,
}

impl Label {
    fn name(self) -> &'static str {
        match self {
            Self::FlakyTest => "flaky-test",
            Self::LiveCheck => "live-check",
            Self::NeedsTriage => "needs-triage",
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

#[derive(Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
enum State {
    Open,
    Closed,
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
        let marker = format!("<!-- goodfetch-results: {} {key} -->", label.name());
        let body = format!("{marker}\n{body}");
        let existing = self
            .issues(label)?
            .iter()
            .find(|issue| {
                issue
                    .body
                    .as_deref()
                    .is_some_and(|text| text.lines().any(|line| line == marker))
            })
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

    fn issues(&mut self, label: Label) -> Result<&[Issue]> {
        if !self.listed.iter().any(|(listed, _)| *listed == label) {
            let mut issues = Vec::new();
            for page in 1.. {
                let path = format!(
                    "/repos/{}/issues?labels={}&state=all&per_page={PAGE}&page={page}",
                    self.repository,
                    label.name()
                );
                let listed: Vec<Issue> = self.api.get(&path)?;
                let full = listed.len() == PAGE;
                issues.extend(
                    listed
                        .into_iter()
                        .filter(|issue| issue.pull_request.is_none()),
                );
                if !full {
                    break;
                }
            }
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
        let mut milestones = Vec::new();
        for page in 1.. {
            let path = format!(
                "/repos/{}/milestones?state=open&per_page={PAGE}&page={page}",
                self.repository
            );
            let listed: Vec<Milestone> = self.api.get(&path).context("reading the milestones")?;
            let full = listed.len() == PAGE;
            milestones.extend(listed);
            if !full {
                break;
            }
        }
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
