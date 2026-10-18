//! Workflow files and composite actions: no step retries a test through a
//! retry action, and no test step or job with one carries
//! `continue-on-error`. Their commands and variables are read as commands
//! are everywhere else. A test step's name starts with `Test` (TESTING.md, No
//! retries), or its script runs a test runner.

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::File;
use super::commands::{self, Reading};
use crate::check::Finding;

/// The prefix that marks a test step.
const TEST_STEP_PREFIX: &str = "Test";

pub(super) fn check(file: &File, findings: &mut Vec<Finding>) -> Result<()> {
    let document: Value =
        serde_saphyr::from_str(&file.text).with_context(|| format!("parsing {}", file.path))?;
    commands::check_document(
        &file.path,
        &document,
        Reading {
            runners: &[],
            scripts: true,
        },
        findings,
    );
    let jobs = document
        .get("jobs")
        .and_then(Value::as_object)
        .into_iter()
        .flat_map(|jobs| jobs.iter())
        .map(|(id, job)| (id.as_str(), job));
    // A composite action's steps, as one job without an id.
    let action = document.get("runs").map(|runs| ("", runs));
    for (job_id, job) in jobs.chain(action) {
        let steps = job.get("steps").and_then(Value::as_array);
        let mut has_test_step = false;
        for step in steps.into_iter().flatten() {
            let test_step = is_test_step(step);
            has_test_step |= test_step;
            if let Some(action) = step.get("uses").and_then(Value::as_str)
                && is_retry_action(action)
            {
                findings.push(Finding::new(
                    "retry-action",
                    &file.path,
                    format!("`{action}` runs its command again on failure (ADR 0064)"),
                ));
            }
            if test_step && continues_on_error(step) {
                findings.push(Finding::new(
                    "continue-on-error",
                    &file.path,
                    format!(
                        "test step `{}` of job `{job_id}` continues on error, which passes a failed test (ADR 0064)",
                        step_name(step)
                    ),
                ));
            }
        }
        if has_test_step && continues_on_error(job) {
            findings.push(Finding::new(
                "continue-on-error",
                &file.path,
                format!(
                    "job `{job_id}` runs tests and continues on error, which passes a failed test (ADR 0064)"
                ),
            ));
        }
    }
    Ok(())
}

fn step_name(step: &Value) -> &str {
    step.get("name").and_then(Value::as_str).unwrap_or_default()
}

fn is_test_step(step: &Value) -> bool {
    step_name(step).starts_with(TEST_STEP_PREFIX)
        || step
            .get("run")
            .and_then(Value::as_str)
            .is_some_and(commands::script_runs_test)
}

/// An action whose repository's name holds `retry`, such as `nick-fields/retry`.
fn is_retry_action(uses: &str) -> bool {
    let repository = uses.split('@').next().unwrap_or(uses);
    repository.to_ascii_lowercase().contains("retry")
}

/// `continue-on-error` set to anything but `false`; an expression may be true.
fn continues_on_error(entry: &Value) -> bool {
    match entry.get("continue-on-error") {
        None | Some(Value::Bool(false)) => false,
        Some(Value::String(text)) => text.trim() != "false",
        Some(_) => true,
    }
}
