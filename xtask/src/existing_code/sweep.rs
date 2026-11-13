//! The sweep that sets aside a `ready-for-agent` issue whose `Existing code`
//! list no longer holds on `main` (ADR 0144; docs/agents/triage-labels.md):
//! one comment with what moved, `needs-triage` applied and `ready-for-agent`
//! removed. It runs with the merge gate's backstop.

use anyhow::{Context as _, Result};
use serde::Deserialize;
use serde_json::{Value, json};

use super::{Checker, Issue, Problem};
use crate::check;
use crate::github::{Api, List, Write};
use crate::issues::Label;
use crate::output;

#[derive(Deserialize)]
struct Comment {
    body: Option<String>,
}

/// An issue the sweep sets aside, with its comment.
struct Aside<'a> {
    issue: &'a Issue,
    comment: String,
    /// Whether the issue already holds that comment, from an earlier sweep
    /// that failed before its labels moved.
    commented: bool,
}

/// Sets aside every open `ready-for-agent` issue whose list fails, but those
/// whose spec's run has started. Every read comes before the first write, and
/// every comment before the first label write, so a tracker that fails a read
/// moves no label, and a sweep that fails after commenting leaves its issues
/// listed: the next one finds their comments and writes none again.
pub(super) fn sweep(
    checker: &Checker<'_>,
    api: &Api<'_>,
    repository: &str,
    apply: bool,
) -> Result<()> {
    let ready = Label::ReadyForAgent.name();
    let issues: Vec<Issue> = api
        .get_all(
            &format!("/repos/{repository}/issues?labels={ready}&state=open"),
            List::Array,
        )
        .with_context(|| format!("listing the open {ready} issues"))?;
    let mut examined = 0;
    let mut aside = Vec::new();
    for issue in issues.iter().filter(|issue| issue.pull_request.is_none()) {
        if let Some(spec) = issue.spec()
            && checker.started(spec)?
        {
            output::line(&format!(
                "skipped #{}: the run of spec #{spec} has started, and its implementers read the code itself",
                issue.number
            ));
            continue;
        }
        examined += 1;
        let problems = checker.check(issue)?;
        let failed: Vec<&Problem> = problems
            .iter()
            .filter(|problem| !problem.only_moved())
            .collect();
        if failed.is_empty() {
            continue;
        }
        check::note(
            &failed
                .iter()
                .map(|problem| problem.finding(issue.number))
                .collect::<Vec<_>>(),
        );
        aside.push(Aside {
            issue,
            comment: comment(&failed),
            commented: false,
        });
    }
    if apply {
        for aside in &mut aside {
            let comments: Vec<Comment> = api.get_all(
                &format!("/repos/{repository}/issues/{}/comments", aside.issue.number),
                List::Array,
            )?;
            aside.commented = comments
                .iter()
                .any(|comment| comment.body.as_deref() == Some(aside.comment.as_str()));
        }
        for aside in aside.iter().filter(|aside| !aside.commented) {
            let _: Value = api.send(
                Write::Post,
                &format!("/repos/{repository}/issues/{}/comments", aside.issue.number),
                &json!({ "body": aside.comment }),
            )?;
        }
        for Aside { issue, .. } in &aside {
            move_labels(api, repository, issue.number)?;
            output::line(&format!("set aside #{}", issue.number));
        }
    } else {
        for Aside { issue, .. } in &aside {
            output::line(&format!(
                "would set aside #{}: a dry run, which `--apply` makes",
                issue.number
            ));
        }
    }
    output::line(&format!(
        "checked {examined} {ready} issues: {} {}",
        aside.len(),
        if apply { "set aside" } else { "to set aside" }
    ));
    Ok(())
}

/// Applies `needs-triage` to issue `number`, then removes `ready-for-agent`:
/// each write touches its one label, so a label written since the listing
/// stays, and an issue left with both is listed again by the next sweep.
fn move_labels(api: &Api<'_>, repository: &str, number: u64) -> Result<()> {
    let labels = format!("/repos/{repository}/issues/{number}/labels");
    let _: Value = api.send(
        Write::Post,
        &labels,
        &json!({ "labels": [Label::NeedsTriage.name()] }),
    )?;
    api.delete(&format!("{labels}/{}", Label::ReadyForAgent.name()))
}

/// The sweep's comment: what moved, item by item, and how the label comes
/// back. It names the merge that first moved each citation, which a later
/// `main` keeps, so a later sweep of the same evidence writes the same text.
fn comment(problems: &[&Problem]) -> String {
    let lines: Vec<String> = problems
        .iter()
        .map(|problem| problem.comment_lines())
        .collect();
    let ready = Label::ReadyForAgent.name();
    format!(
        "The `Existing code` list of this issue no longer holds on `main`: the evidence moved (ADR 0144). The check reads where each statement was read, not whether it still holds; that is for whoever reads the code again.\n\n{}\n\nThe sweep applied `{}` and took `{ready}`. Read the cited code again, correct the list and its \"Read at\" commit, and put `{ready}` back (`triage`, \"Verify the claim\").",
        lines.join("\n"),
        Label::NeedsTriage.name()
    )
}
