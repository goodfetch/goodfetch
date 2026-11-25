//! The workflow policy (ADR 0108's amendment of 2026-11-22, #3385), a step of
//! `just lint`: every job of `.github/workflows` runs on one of GitHub's
//! standard hosted runners, never a self-hosted or larger one; no workflow
//! runs on `pull_request_target`, or on `workflow_run` with a checkout of a
//! pull request's code, since both hold the base repository's secrets and
//! write token; every workflow starts its token with no permission, each job
//! granting its own; every `uses:` names an Action, reusable workflow or
//! image by commit or digest; and a secret reaches only the jobs
//! `ci/secrets.toml` lists for it (ADR 0076).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::rate;
use crate::workflow::{self, Job, Permissions, Runner, Workflow};

/// Each secret a workflow reads, with the jobs that may read it.
const SECRETS: &str = "ci/secrets.toml";

/// The job's own token, whose reach its `permissions:` set, so no list holds it.
const JOB_TOKEN: &str = "GITHUB_TOKEN";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SecretsFile {
    #[serde(default)]
    secret: Vec<Secret>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Secret {
    name: String,
    /// What the jobs use it for.
    #[serde(rename = "use")]
    _use: String,
    /// Each job that reads it, `<workflow file>#<job id>`.
    jobs: Vec<String>,
}

/// Every (secret, job) pair `ci/secrets.toml` allows, the secret's name in
/// upper case; none when the file is absent.
fn allowed_secrets(root: &Path) -> Result<BTreeSet<(String, String)>> {
    let path = root.join(SECRETS);
    if !path.exists() {
        return Ok(BTreeSet::new());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: SecretsFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(file
        .secret
        .into_iter()
        .flat_map(|secret| {
            let name = secret.name.to_ascii_uppercase();
            secret.jobs.into_iter().map(move |job| (name.clone(), job))
        })
        .collect())
}

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
    let allowed = allowed_secrets(root)?;
    let mut read = BTreeSet::new();
    let mut findings = Vec::new();
    for workflow in workflow::read_all(root)? {
        check_workflow(&workflow, &mut findings);
        for job in &workflow.jobs {
            check_job(&workflow, job, &mut findings);
            let subject = format!("{}#{}", workflow.file, job.id);
            for secret in job.secrets.iter().filter(|secret| *secret != JOB_TOKEN) {
                if !allowed.contains(&(secret.clone(), subject.clone())) {
                    findings.push(Finding::new(
                        "unlisted-secret",
                        subject.clone(),
                        format!("the job reads the secret `{secret}`, which {SECRETS} does not list for it"),
                    ));
                }
                read.insert((secret.clone(), subject.clone()));
            }
            if job.inherits_secrets {
                findings.push(Finding::new(
                    "inherited-secrets",
                    subject,
                    "`secrets: inherit` passes the called workflow every secret; pass the ones it needs by name",
                ));
            }
        }
    }
    for (secret, job) in allowed.difference(&read) {
        findings.push(Finding::new(
            "unread-secret",
            job.clone(),
            format!("{SECRETS} lists the job for the secret `{secret}`, which it does not read"),
        ));
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
    if workflow.secrets.iter().any(|secret| secret != JOB_TOKEN) {
        findings.push(Finding::new(
            "workflow-wide-secret",
            workflow.file.clone(),
            "an expression outside `jobs:` reads a secret, which every job then gets; read it in the jobs that need it",
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
