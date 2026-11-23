//! The Quick-suite check that every workflow job is declared under or outside
//! the deny, and that a job under it denies before its first test step, runs
//! the Egress canary right after, and checks the deny's log at its end
//! (ADR 0043). A test step is one `workflow::is_test_step` names.

use std::path::Path;

use anyhow::Result;

use super::endpoints::{ENDPOINTS, Endpoints, JobKey, Rule};
use crate::check::Finding;
use crate::workflow::{self, Step};

/// The step that puts the deny in place.
pub(crate) const DENY: &str = "ci/egress/egress.sh deny";
const CANARY: &str = "ci/egress/egress.sh canary";
const CHECK: &str = "ci/egress/egress.sh check";

/// What a test step after the deny runs its command through, so the command
/// runs where the deny applies: the job's VM on the owner's machine, the
/// hosted tier's container on a standard hosted runner, or, on a runner's own
/// system, the deny's network namespace.
const UNDER_DENY: [&str; 4] = [
    "ci/vm/exec.sh ",
    "ci/vm/visual.sh ",
    "ci/hosted/exec.sh ",
    "ci/egress/under-deny.sh ",
];

fn runs(step: &Step, script: &str) -> bool {
    step.run.as_deref().map(str::trim) == Some(script)
}

fn is_test(step: &Step) -> bool {
    step.name.as_deref().is_some_and(workflow::is_test_step)
}

fn always(step: &Step) -> bool {
    step.condition.as_deref().map(str::trim) == Some("always()")
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let endpoints = Endpoints::read(root)?;
    let mut findings = Vec::new();
    let mut seen = Vec::new();
    for workflow in workflow::read_all(root)? {
        for job in &workflow.jobs {
            let key = JobKey::new(&workflow.file, &job.id);
            let subject = key.to_string();
            seen.push(key.clone());
            let mut matching = endpoints
                .declarations
                .iter()
                .filter(|declaration| declaration.job == key);
            let declaration = match (matching.next(), matching.next()) {
                (Some(declaration), None) => declaration,
                (None, _) => {
                    findings.push(Finding::new(
                        "undeclared-job",
                        subject,
                        format!(
                            "declare the job in {ENDPOINTS}: under the deny, outside it, or named"
                        ),
                    ));
                    continue;
                }
                (Some(_), Some(_)) => {
                    findings.push(Finding::new(
                        "declared-twice",
                        subject,
                        format!("the job has more than one declaration in {ENDPOINTS}"),
                    ));
                    continue;
                }
            };
            let deny = job.steps.iter().position(|step| runs(step, DENY));
            if matches!(declaration.rule, Rule::Outside) {
                if deny.is_some() {
                    findings.push(Finding::new(
                        "deny-outside-rule",
                        subject,
                        "the job is declared outside the deny but runs it",
                    ));
                }
                continue;
            }
            findings.extend(under_deny(&job.steps, deny, &subject));
        }
    }
    for declaration in &endpoints.declarations {
        if !seen.contains(&declaration.job) {
            findings.push(Finding::new(
                "unknown-job",
                declaration.job.to_string(),
                format!("{ENDPOINTS} declares a job no workflow has"),
            ));
        }
    }
    Ok(findings)
}

/// A job under the deny: the deny before every test step, the canary right
/// after it, then the log check, run whatever happened.
fn under_deny(steps: &[Step], deny: Option<usize>, subject: &str) -> Vec<Finding> {
    let Some(deny) = deny else {
        return vec![Finding::new(
            "missing-deny",
            subject,
            format!("a job under the deny runs `{DENY}` before its first test step"),
        )];
    };
    let mut findings = Vec::new();
    if steps[..deny].iter().any(is_test) {
        findings.push(Finding::new(
            "test-before-deny",
            subject,
            "a test step runs before the deny",
        ));
    }
    let outside = |step: &Step| {
        !step.run.as_deref().is_some_and(|run| {
            UNDER_DENY
                .iter()
                .any(|wrapper| run.trim_start().starts_with(wrapper))
        })
    };
    if steps[deny..]
        .iter()
        .any(|step| is_test(step) && outside(step))
    {
        findings.push(Finding::new(
            "test-outside-deny",
            subject,
            format!(
                "a test step after the deny runs its command through one of {}",
                UNDER_DENY.map(str::trim).join(", ")
            ),
        ));
    }
    if !steps.get(deny + 1).is_some_and(|step| runs(step, CANARY)) {
        findings.push(Finding::new(
            "missing-canary",
            subject,
            format!("the step right after the deny runs `{CANARY}`"),
        ));
    }
    if !steps[deny..]
        .iter()
        .any(|step| runs(step, CHECK) && always(step))
    {
        findings.push(Finding::new(
            "missing-egress-check",
            subject,
            format!("a step after the deny runs `{CHECK}` with `if: always()`"),
        ));
    }
    findings
}
