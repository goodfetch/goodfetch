//! The self-hosted allowlist (ADRs 0108 and 0135): the committed list of every
//! job that may reach a self-hosted runner, the Quick-suite check that refuses
//! any other job naming one, and the flat copy the owner installs for the
//! runner's pre-job script.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::workflow::{self, Runner};

pub(crate) const ALLOWLIST: &str = ".github/self-hosted-allowlist.toml";

/// The labels that only a self-hosted runner carries (the owner's machine's
/// labels, ADR 0108), compared without case as GitHub does.
const SELF_HOSTED_LABELS: [&str; 4] = ["self-hosted", "macos", "arm64", "owner-machine"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Allowlist {
    /// The only repository whose jobs the runner admits.
    repository: String,
    #[serde(rename = "job", default)]
    jobs: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    /// The workflow's file name under `.github/workflows/`.
    workflow: String,
    job: String,
    events: BTreeSet<String>,
    /// Patterns of `GITHUB_REF`, where `*` matches any text.
    refs: Vec<String>,
    /// The job's exact `if:`.
    #[serde(rename = "if")]
    condition: String,
}

fn read_allowlist(root: &Path) -> Result<Allowlist> {
    let path = root.join(ALLOWLIST);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

fn reaches_self_hosted(runner: &Runner) -> bool {
    match runner {
        Runner::None => false,
        Runner::Group | Runner::Unresolved => true,
        Runner::Labels(sets) => sets.iter().flatten().any(|label| {
            SELF_HOSTED_LABELS
                .iter()
                .any(|self_hosted| label.eq_ignore_ascii_case(self_hosted))
        }),
    }
}

/// Every job of `<root>/.github/workflows` that may reach a self-hosted runner
/// must be on the allowlist, with the same events and the same `if:`.
pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let allowlist = read_allowlist(root)?;
    let mut findings = Vec::new();
    for workflow in workflow::read_all(root)? {
        for job in &workflow.jobs {
            let subject = format!("{}#{}", workflow.file, job.id);
            if matches!(job.runner, Runner::Unresolved) {
                findings.push(Finding::new(
                    "unresolved-runs-on",
                    subject,
                    "`runs-on` is an expression other than a matrix value, so the check cannot tell where the job runs",
                ));
                continue;
            }
            if !reaches_self_hosted(&job.runner) {
                continue;
            }
            let entry = allowlist
                .jobs
                .iter()
                .find(|entry| entry.workflow == workflow.file && entry.job == job.id);
            match entry {
                None => findings.push(Finding::new(
                    "off-allowlist",
                    subject,
                    format!("the job may reach a self-hosted runner but is not on {ALLOWLIST}"),
                )),
                Some(entry)
                    if entry.events != workflow.events
                        || Some(entry.condition.trim())
                            != job.condition.as_deref().map(str::trim) =>
                {
                    findings.push(Finding::new(
                        "allowlist-mismatch",
                        subject,
                        format!("the workflow's events or the job's `if:` differ from its entry in {ALLOWLIST}"),
                    ));
                }
                Some(_) => {}
            }
        }
    }
    Ok(findings)
}

/// The flat copy the pre-job script reads: a `repository` line, then one
/// line per workflow, job, event and ref pattern, separated by tabs.
pub(crate) fn export(root: &Path) -> Result<String> {
    let allowlist = read_allowlist(root)?;
    let mut lines = vec![format!("repository\t{}", allowlist.repository)];
    for entry in &allowlist.jobs {
        for event in &entry.events {
            for pattern in &entry.refs {
                lines.push(format!(
                    "job\t{}\t{}\t{event}\t{pattern}",
                    entry.workflow, entry.job
                ));
            }
        }
    }
    Ok(lines.join("\n"))
}
