//! What a check found, and the exit code that reports it: each finding is one
//! `error[<kind>] <subject>: <message>` line, so callers and tests match the
//! kind and subject, never the message.

use std::process::ExitCode;

use crate::output;

pub(crate) struct Finding {
    pub(crate) kind: &'static str,
    pub(crate) subject: String,
    pub(crate) message: String,
}

impl Finding {
    pub(crate) fn new(
        kind: &'static str,
        subject: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self {
            kind,
            subject: subject.into(),
            message: message.into(),
        }
    }
}

/// Writes each finding as a `note[<kind>] <subject>: <message>` line, which
/// fails nothing.
pub(crate) fn note(findings: &[Finding]) {
    for finding in findings {
        output::line(&format!(
            "note[{}] {}: {}",
            finding.kind, finding.subject, finding.message
        ));
    }
}

/// Writes every finding and returns 1 when there is any, 0 otherwise.
pub(crate) fn report(findings: &[Finding]) -> ExitCode {
    for finding in findings {
        output::line(&format!(
            "error[{}] {}: {}",
            finding.kind, finding.subject, finding.message
        ));
    }
    if findings.is_empty() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}
