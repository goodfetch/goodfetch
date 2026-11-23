//! The workflow policy (ADR 0108's amendment of 2026-11-22, #3385), a step of
//! `just lint`: every job of `.github/workflows` runs on one of GitHub's
//! standard hosted runners, never a self-hosted or larger one; no workflow
//! runs on `pull_request_target`, or on `workflow_run` with a checkout of a
//! pull request's code, since both hold the base repository's secrets and
//! write token; every workflow starts its token with no permission, each job
//! granting its own; and every `uses:` names an Action, reusable workflow or
//! image by commit or digest.

use std::path::Path;

use anyhow::Result;

use crate::check::Finding;
use crate::rate;
use crate::workflow::{self, Job, Permissions, Runner, Workflow};

/// What a checkout's `ref:` or `repository:` names when it fetches a pull
/// request's code: the triggering run's head, the pull request's own
/// context, or a pull request's ref.
const PULL_REQUEST_CODE: [&str; 5] = [
    "github.event.workflow_run.head",
    "github.event.workflow_run.pull_requests",
    "github.event.pull_request",
    "github.head_ref",
    "refs/pull/",
];

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for workflow in workflow::read_all(root)? {
        check_workflow(&workflow, &mut findings);
        for job in &workflow.jobs {
            check_job(&workflow, job, &mut findings);
        }
    }
    Ok(findings)
}

fn check_workflow(workflow: &Workflow, findings: &mut Vec<Finding>) {
    if workflow.events.contains("pull_request_target") {
        findings.push(Finding::new(
            "pull-request-target",
            workflow.file.clone(),
            "`pull_request_target` runs with the base repository's secrets and write token; use `pull_request`",
        ));
    }
    if workflow.permissions != Permissions::Empty {
        findings.push(Finding::new(
            "top-level-permissions",
            workflow.file.clone(),
            "the workflow's top-level `permissions:` is not `{}`; grant each job its own",
        ));
    }
}

fn check_job(workflow: &Workflow, job: &Job, findings: &mut Vec<Finding>) {
    let subject = format!("{}#{}", workflow.file, job.id);
    match &job.runner {
        Runner::None => {}
        Runner::Unresolved => findings.push(Finding::new(
            "unresolved-runs-on",
            subject.clone(),
            "`runs-on` is an expression other than a matrix value, so the check cannot tell where the job runs",
        )),
        Runner::Group => findings.push(Finding::new(
            "non-standard-runner",
            subject.clone(),
            "`runs-on` names a runner group, which holds self-hosted or larger runners",
        )),
        Runner::Labels(sets) => {
            for labels in sets {
                if !matches!(labels.as_slice(), [label] if rate::is_standard_label(label)) {
                    findings.push(Finding::new(
                        "non-standard-runner",
                        subject.clone(),
                        format!(
                            "`runs-on` asks for [{}], not one label of GitHub's standard hosted runners",
                            labels.join(", ")
                        ),
                    ));
                }
            }
        }
    }
    let checks_out_pull_request = workflow.events.contains("workflow_run")
        && job.steps.iter().any(|step| {
            step.uses
                .as_deref()
                .is_some_and(|uses| uses.starts_with("actions/checkout@"))
                && ["ref", "repository"].iter().any(|input| {
                    step.with.get(*input).is_some_and(|value| {
                        PULL_REQUEST_CODE
                            .iter()
                            .any(|marker| value.contains(marker))
                    })
                })
        });
    if checks_out_pull_request {
        findings.push(Finding::new(
            "checks-out-pull-request",
            subject.clone(),
            "a `workflow_run` job holds the base repository's secrets and write token, so it checks out no pull request's code",
        ));
    }
    let uses = job
        .uses
        .iter()
        .chain(job.steps.iter().filter_map(|step| step.uses.as_ref()));
    for uses in uses {
        if !is_pinned(uses) {
            findings.push(Finding::new(
                "unpinned-uses",
                subject.clone(),
                format!("`uses: {uses}` is not pinned by a full commit SHA or an image digest"),
            ));
        }
    }
}

/// Whether `uses` names its code immutably: a path of this repository, an
/// Action or reusable workflow at a full commit SHA, or a Docker image by
/// digest.
fn is_pinned(uses: &str) -> bool {
    if uses.starts_with("./") {
        return true;
    }
    if let Some(image) = uses.strip_prefix("docker://") {
        return image
            .split_once("@sha256:")
            .is_some_and(|(name, digest)| !name.is_empty() && is_hex(digest, 64));
    }
    uses.rsplit_once('@')
        .is_some_and(|(name, reference)| name.contains('/') && is_hex(reference, 40))
}

fn is_hex(text: &str, length: usize) -> bool {
    text.len() == length
        && text
            .chars()
            .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
}
