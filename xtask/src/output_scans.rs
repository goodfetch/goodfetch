//! The output-scan check (ADR 0067): a job scans its logs, test reports,
//! snapshots and artifacts with `gitleaks` (`ci/leaks.sh`) before they
//! upload, and a hit keeps them from uploading. For every
//! `actions/upload-artifact` step it requires:
//! - an earlier step that runs `ci/leaks.sh` over each path the upload takes,
//!   or over a folder holding it, with no step between them but other
//!   uploads and scans, since any other step may write what uploads;
//! - that the scan runs after a failed step (`if: always()` or
//!   `if: "!cancelled()"`), since a job's JUnit files upload pass or fail
//!   (TESTING.md), and after a cancel too (`always()`) when the upload does;
//! - that the upload's `if:` requires the scan's success
//!   (`always() && steps.<id>.outcome == 'success'`, or the same after
//!   `!cancelled()`, a conjunction with no `||`), so a found secret never
//!   uploads.
//!
//! A job that uploads nothing has nothing to scan. Its log, which GitHub
//! receives while each step runs, cannot be scanned before it uploads; the
//! Secret canary searches of the tests that write it stand there (ADR 0067).
//!
//! It also refuses a job whose own `if:` holds `always()`: such a job still
//! starts after its run is cancelled, and holds the workflow's concurrency
//! group while it does (#3526). A job that must run after a failure uses
//! `!cancelled()`.

use std::path::Path;

use anyhow::Result;

use crate::check::Finding;
use crate::workflow::{self, Step};

/// The script every scan runs, directly or through `ci/vm/exec.sh`.
pub(crate) const SCAN: &str = "ci/leaks.sh";

const UPLOAD: &str = "actions/upload-artifact@";

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    for workflow in workflow::read_all(root)? {
        for job in &workflow.jobs {
            let subject = format!("{}#{}", workflow.file, job.id);
            if job
                .condition
                .as_deref()
                .is_some_and(|condition| condition.contains("always()"))
            {
                findings.push(Finding::new(
                    "job-runs-when-cancelled",
                    &subject,
                    "the job's `if:` holds `always()`, so it starts after its run is cancelled and holds the concurrency group: use `!cancelled()`",
                ));
            }
            for (index, step) in job.steps.iter().enumerate() {
                if !is_upload(step) {
                    continue;
                }
                if let Some(finding) = upload(&job.steps[..index], step, &subject) {
                    findings.push(finding);
                }
            }
        }
    }
    Ok(findings)
}

fn is_upload(step: &Step) -> bool {
    step.uses
        .as_deref()
        .is_some_and(|uses| uses.starts_with(UPLOAD))
}

/// The finding for one upload step, given the steps before it.
fn upload(before: &[Step], step: &Step, subject: &str) -> Option<Finding> {
    let name = step.name.as_deref().unwrap_or("an upload");
    let paths: Vec<&str> = step
        .with
        .get("path")
        .map(|path| {
            path.lines()
                .map(str::trim)
                .filter(|path| !path.is_empty() && !path.starts_with('!'))
                .collect()
        })
        .unwrap_or_default();
    // A scan counts only after the last step that may write what uploads:
    // any step but another upload or scan.
    let fresh = before
        .iter()
        .rposition(|step| !is_upload(step) && scanned(step).is_none())
        .map_or(0, |index| index + 1);
    let scans: Vec<(&Step, Vec<&str>)> = before[fresh..]
        .iter()
        .filter_map(|step| scanned(step).map(|paths| (step, paths)))
        .collect();
    let unscanned: Vec<&str> = paths
        .iter()
        .copied()
        .filter(|path| {
            !scans
                .iter()
                .any(|(_, scanned)| scanned.iter().any(|folder| covers(folder, path)))
        })
        .collect();
    if !unscanned.is_empty() {
        return Some(Finding::new(
            "unscanned-upload",
            subject,
            format!(
                "\"{name}\" uploads {} with no `{SCAN}` step before it that scans it",
                unscanned.join(", ")
            ),
        ));
    }
    let guarding: Vec<&Step> = scans
        .iter()
        .filter(|(_, scanned)| {
            paths
                .iter()
                .any(|path| scanned.iter().any(|folder| covers(folder, path)))
        })
        .map(|(scan, _)| *scan)
        .collect();
    if let Some(scan) = guarding.iter().find(|scan| !runs_after_a_failure(scan)) {
        return Some(Finding::new(
            "scan-skipped-on-failure",
            subject,
            format!(
                "\"{}\" runs `{SCAN}` without `if: always()` or `if: \"!cancelled()\"`, so a failed step skips it and \"{name}\" never uploads",
                scan.name.as_deref().unwrap_or(SCAN)
            ),
        ));
    }
    if runs_after_a_cancel(step)
        && let Some(scan) = guarding.iter().find(|scan| !runs_after_a_cancel(scan))
    {
        return Some(Finding::new(
            "scan-skipped-on-cancel",
            subject,
            format!(
                "\"{name}\" runs after a cancel, but \"{}\" runs `{SCAN}` without `if: always()`, so after a cancel it never uploads",
                scan.name.as_deref().unwrap_or(SCAN)
            ),
        ));
    }
    let condition = step.condition.as_deref().unwrap_or_default();
    let unheeded: Vec<&str> = guarding
        .iter()
        .filter(|scan| {
            !scan
                .id
                .as_deref()
                .is_some_and(|id| requires_success(condition, id))
        })
        .map(|scan| scan.name.as_deref().unwrap_or(SCAN))
        .collect();
    if !unheeded.is_empty() {
        return Some(Finding::new(
            "upload-ignores-scan",
            subject,
            format!(
                "\"{name}\" must run only when {} passed: give the scan an `id` and add `steps.<id>.outcome == 'success'` to the upload's `if:`",
                unheeded.join(", ")
            ),
        ));
    }
    None
}

/// The paths a step scans: the arguments after `ci/leaks.sh` in its script.
fn scanned(step: &Step) -> Option<Vec<&str>> {
    let script = step.run.as_deref()?;
    let (_, arguments) = script.split_once(SCAN)?;
    Some(
        arguments
            .split_whitespace()
            .take_while(|word| !matches!(*word, "&&" | "||" | ";" | "|"))
            .map(|word| word.trim_matches(|character| character == '"' || character == '\''))
            .collect(),
    )
}

/// Whether scanning `folder` scans `path`: the same path, or a path inside it.
fn covers(folder: &str, path: &str) -> bool {
    let folder = folder.trim_end_matches('/');
    let path = path.trim_end_matches('/');
    path == folder
        || path
            .strip_prefix(folder)
            .is_some_and(|rest| rest.starts_with('/'))
}

fn runs_after_a_failure(step: &Step) -> bool {
    runs_after_a_cancel(step)
        || step
            .condition
            .as_deref()
            .is_some_and(|condition| condition.contains("!cancelled()"))
}

fn runs_after_a_cancel(step: &Step) -> bool {
    step.condition
        .as_deref()
        .is_some_and(|condition| condition.contains("always()"))
}

/// Whether `condition` requires the step `id`'s success: a conjunction, with
/// no `||` anywhere, one of whose terms is `steps.<id>.outcome == 'success'`,
/// as in `always() && steps.<id>.outcome == 'success'`.
fn requires_success(condition: &str, id: &str) -> bool {
    let compact: String = condition.split_whitespace().collect();
    let compact = compact
        .strip_prefix("${{")
        .and_then(|inner| inner.strip_suffix("}}"))
        .unwrap_or(&compact);
    !compact.contains("||")
        && compact.split("&&").any(|term| {
            term == format!("steps.{id}.outcome=='success'")
                || term == format!("steps.{id}.outcome==\"success\"")
        })
}
