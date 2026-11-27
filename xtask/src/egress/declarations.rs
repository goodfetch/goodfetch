//! The Quick-suite check that every workflow job is declared under or outside
//! the deny, and that a job under it denies before its first test step, runs
//! the Egress canary right after, and checks the deny's log at its end
//! (ADR 0043). A test step is one `workflow::is_test_step` names. A job that
//! reaches named endpoints runs on a schedule, by hand or from another
//! workflow, never in a workflow a pull request or the merge queue starts,
//! except a fixture's licence endpoint, which a fixture of any tier may need.

use std::path::Path;

use anyhow::Result;

use super::endpoints::{ENDPOINTS, Endpoints, Exception, JobKey, Rule};
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
const UNDER_DENY: [&str; 5] = [
    "ci/vm/exec.sh ",
    "ci/vm/visual.sh ",
    "ci/hosted/exec.sh ",
    "ci/hosted/visual.sh ",
    "ci/egress/under-deny.sh ",
];

/// The triggers that run a pull request's code.
const PULL_REQUEST_EVENTS: [&str; 3] = ["pull_request", "pull_request_target", "merge_group"];

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
            if let Rule::Named {
                exception,
                endpoints,
            } = &declaration.rule
            {
                if *exception != Exception::FixtureLicence
                    && let Some(event) = PULL_REQUEST_EVENTS
                        .iter()
                        .find(|event| workflow.events.contains(**event))
                {
                    findings.push(Finding::new(
                        "named-job-on-pull-request",
                        subject.clone(),
                        format!(
                            "a job that reaches named endpoints runs on a schedule or by hand, never on `{event}`"
                        ),
                    ));
                }
                if *exception == Exception::IntegrationAccount {
                    if !job.condition.as_deref().is_some_and(requires_repo_variable) {
                        findings.push(Finding::new(
                            "integration-account-unprotected",
                            subject.clone(),
                            "an integration-account job stays off unless a repository variable (`vars.<NAME> == 'true'`) is set (ADRs 0070 and 0076)",
                        ));
                    }
                    if endpoints
                        .iter()
                        .any(|endpoint| is_goodfetch_dev(&endpoint.host))
                    {
                        findings.push(Finding::new(
                            "named-tunnel-host",
                            subject.clone(),
                            "an integration-account job reaches Cloudflare's Quick Tunnel service, never a named `goodfetch.dev` tunnel host (ADR 0076)",
                        ));
                    }
                }
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

/// Whether `condition` requires a repository variable (`vars.<NAME> == 'true'`)
/// as a top-level conjunct, with no top-level `||` that could bypass it.
fn requires_repo_variable(condition: &str) -> bool {
    let mut trimmed = condition.trim();
    if let Some(inner) = trimmed
        .strip_prefix("${{")
        .and_then(|rest| rest.strip_suffix("}}"))
    {
        trimmed = inner.trim();
    }
    trimmed = strip_outer_parens(trimmed);
    let Some(conjuncts) = split_top_level_and(trimmed) else {
        return false;
    };
    conjuncts.into_iter().any(is_repo_variable_enabled)
}

/// Strips parentheses that wrap the entire expression.
fn strip_outer_parens(mut text: &str) -> &str {
    while let Some(inner) = text
        .strip_prefix('(')
        .and_then(|rest| rest.strip_suffix(')'))
    {
        if scan_expression(inner, false).is_none() {
            break;
        }
        text = inner.trim();
    }
    text
}

/// Splits `expression` on top-level `&&`, returning `None` if a top-level `||`
/// or unbalanced parentheses/quotes appear.
fn split_top_level_and(expression: &str) -> Option<Vec<&str>> {
    scan_expression(expression, true)
}

/// Walks `expression` tracking single-quoted strings (`''`-escaped) and
/// parenthesis depth. Returns `None` on unbalanced parens/quotes, or when
/// `split_and` is true and a top-level `||` appears.
fn scan_expression(expression: &str, split_and: bool) -> Option<Vec<&str>> {
    let bytes = expression.as_bytes();
    let mut parts = Vec::new();
    let mut start = 0;
    let mut depth: usize = 0;
    let mut in_quote = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if in_quote {
            if byte == b'\'' {
                if bytes.get(index + 1) == Some(&b'\'') {
                    index += 2;
                    continue;
                }
                in_quote = false;
            }
            index += 1;
            continue;
        }
        match byte {
            b'\'' => in_quote = true,
            b'(' => depth += 1,
            b')' => depth = depth.checked_sub(1)?,
            b'|' if split_and && depth == 0 && bytes.get(index + 1) == Some(&b'|') => return None,
            b'&' if split_and && depth == 0 && bytes.get(index + 1) == Some(&b'&') => {
                parts.push(expression[start..index].trim());
                index += 2;
                start = index;
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    if depth != 0 || in_quote {
        return None;
    }
    if split_and {
        parts.push(expression[start..].trim());
    }
    Some(parts)
}

/// Whether `term` is `vars.<IDENT> == 'true'` or `'true' == vars.<IDENT>`
/// (stripping matching outer parentheses).
fn is_repo_variable_enabled(term: &str) -> bool {
    let term = strip_outer_parens(term);
    let Some((left, right)) = term.split_once("==") else {
        return false;
    };
    let (left, right) = (left.trim(), right.trim());
    (is_vars_ident(left) && right == "'true'") || (left == "'true'" && is_vars_ident(right))
}

fn is_vars_ident(text: &str) -> bool {
    text.strip_prefix("vars.").is_some_and(|name| {
        !name.is_empty()
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
    })
}

fn is_goodfetch_dev(host: &str) -> bool {
    let host = host.trim_end_matches('.');
    host == "goodfetch.dev" || host.ends_with(".goodfetch.dev")
}
