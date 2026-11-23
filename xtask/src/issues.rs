//! The issues the checks keep: one per flaky test or Live check (`xtask
//! results scan`), one per drifting performance figure (`xtask perf drift`)
//! and one for a Release run past the minute ceiling (`xtask release-suite
//! plan`), each found again by a marker line in its body, so its title can
//! be edited. A new one goes to the spec that owns what it is about, as a
//! sub-issue in the milestone of that spec's ticket (ADR 0136,
//! [`crate::owners`]); with no owner found, it opens under `needs-triage`
//! outside every milestone. An open one gets the current results; a closed
//! one reopens only for a result after it closed, and keeps its parent and
//! milestone. The check of the file that names an issue (the quarantine
//! list) reads it through [`find`]. The watchlist job's issues (`xtask
//! watchlist`) are opened once, outside every milestone, and never reopened:
//! closing one declines its vendor.

use std::num::NonZeroU64;

use anyhow::{Result, bail};
use jiff::Timestamp;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::github::{Api, List, Write};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Label {
    FlakyTest,
    LiveCheck,
    NeedsTriage,
    ReadyForAgent,
    Watchlist,
}

impl Label {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::FlakyTest => "flaky-test",
            Self::LiveCheck => "live-check",
            Self::NeedsTriage => "needs-triage",
            Self::ReadyForAgent => "ready-for-agent",
            Self::Watchlist => "watchlist",
        }
    }
}

/// The spec that owns what an issue is about, and the milestone of its
/// ticket that added it, when its tickets agree on one (ADR 0136).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Owner {
    pub(crate) spec: u64,
    pub(crate) milestone: Option<u64>,
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
struct Created {
    number: u64,
}

/// A created issue that becomes a sub-issue.
#[derive(Deserialize)]
struct CreatedChild {
    number: u64,
    /// The issue's id, which the sub-issues endpoint takes.
    id: u64,
}

pub(crate) struct Issues<'a> {
    api: &'a Api<'a>,
    repository: &'a str,
    /// Each label's issues, read once.
    listed: Vec<(Label, Vec<Issue>)>,
    actions: Vec<String>,
}

impl<'a> Issues<'a> {
    pub(crate) fn new(api: &'a Api<'a>, repository: &'a str) -> Self {
        Self {
            api,
            repository,
            listed: Vec::new(),
            actions: Vec::new(),
        }
    }

    /// What the scan did, one line each.
    pub(crate) fn actions(&self) -> &[String] {
        &self.actions
    }

    /// Opens the issue of `key` under `label`, updates its body while it is
    /// open, or reopens it when `latest` came after it closed. `owner` is
    /// read only to open one: a sub-issue of its spec in its milestone, or,
    /// with none, an issue under `needs-triage` outside every milestone. One
    /// that opens but cannot join its spec gets `needs-triage`, and the call
    /// fails.
    pub(crate) fn keep(
        &mut self,
        label: Label,
        key: &str,
        title: &str,
        body: &str,
        latest: Timestamp,
        owner: impl FnOnce() -> Result<Option<Owner>>,
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
                if let Some(owner) = owner()? {
                    let created: CreatedChild = self.api.send(
                        Write::Post,
                        &issues,
                        &json!({
                            "title": title,
                            "body": body,
                            "labels": [label.name()],
                            "milestone": owner.milestone,
                        }),
                    )?;
                    let joined: Result<Value> = self.api.send(
                        Write::Post,
                        &format!("{issues}/{}/sub_issues", owner.spec),
                        &json!({ "sub_issue_id": created.id }),
                    );
                    if let Err(error) = joined {
                        // Left outside its spec, it waits for triage.
                        let _: Value = self.api.send(
                            Write::Post,
                            &format!("{issues}/{}/labels", created.number),
                            &json!({ "labels": [Label::NeedsTriage.name()] }),
                        )?;
                        return Err(error.context(format!(
                            "adding #{} to spec #{}; it is labelled {} instead",
                            created.number,
                            owner.spec,
                            Label::NeedsTriage.name()
                        )));
                    }
                    self.actions.push(format!(
                        "opened #{} under spec #{}: {title}",
                        created.number, owner.spec
                    ));
                } else {
                    let mut labels = vec![label.name()];
                    if label != Label::NeedsTriage {
                        labels.push(Label::NeedsTriage.name());
                    }
                    let created: Created = self.api.send(
                        Write::Post,
                        &issues,
                        &json!({ "title": title, "body": body, "labels": labels }),
                    )?;
                    self.actions.push(format!(
                        "opened #{} for triage, no owning spec found: {title}",
                        created.number
                    ));
                }
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
                    let _: Value = self.api.send(
                        Write::Patch,
                        &format!("{issues}/{number}"),
                        &json!({ "state": "open", "body": body }),
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
}
