//! The bug-report file `doctor` writes (ADR 0043, spec #564), for the user
//! to attach to an issue: it stays on the machine, and nothing sends it.
//! It holds the versions, `doctor`'s findings, the settings a layer set,
//! each credential as the Secret reference it is by its kind (ADR 0015),
//! the recent Error codes the log folder's lines carry (ADR 0122) and the
//! names of the crash reports. Each URL in it is reduced to its scheme and
//! host, unless `--include-urls` keeps it whole; its credentials go either
//! way. The file is JSON, which no Goodfetch reader opens, at
//! `bug-report.json` in Goodfetch home, replaced by each `doctor`.
#![expect(
    clippy::disallowed_methods,
    reason = "the CLI reads the log folder and writes the file before and outside any async worker (ADR 0060)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::doctor::{BugReportFile, DoctorFinding};
use goodfetch_protocol::home::BUG_REPORT;
use goodfetch_protocol::settings::SettingsLayer;
use serde::Serialize;
use serde_json::Value;

use super::super::CRASH_FOLDER;
use super::super::daemon::Machine;

/// How many Error codes the file keeps, the newest.
pub(super) const RECENT_ERRORS: usize = 50;

/// How many crash reports the file names, the newest.
const CRASH_REPORTS: usize = 20;

/// What the file does with each URL in it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Urls {
    /// Reduced to its scheme and host, by default.
    Reduced,
    /// Kept whole but its credentials, under `--include-urls`.
    Whole,
}

/// The file's content.
#[derive(Serialize)]
struct BugReport<'a> {
    product: Product,
    product_version: String,
    protocol_version: String,
    os: &'static str,
    arch: &'static str,
    written_at: String,
    urls: Urls,
    findings: &'a [DoctorFinding],
    /// The Operator policy's version, when it names one.
    policy_version: Option<String>,
    settings: Vec<ReportedSetting>,
    recent_errors: Vec<RecentError>,
    crash_reports: Vec<String>,
}

/// A setting a layer set, with that layer.
#[derive(Serialize)]
struct ReportedSetting {
    key: String,
    value: Value,
    layer: SettingsLayer,
}

/// An Error code a log line carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(super) struct RecentError {
    /// The line's time.
    pub(super) time: Option<String>,
    /// The Error code.
    pub(super) code: String,
    /// The call that failed with it, when the line names one.
    pub(super) method: Option<String>,
    /// The line's message.
    pub(super) message: Option<String>,
}

/// Writes the file for the installation `machine` runs in, with
/// `findings`, each URL as `urls` says.
pub(super) fn write(
    machine: &Machine<'_>,
    findings: &[DoctorFinding],
    urls: Urls,
) -> BugReportFile {
    let root = machine.home.root();
    let path = root.join(BUG_REPORT);
    let explained = machine
        .settings()
        .ok()
        .map(|settings| settings.explain(&[], Vec::new()));
    let (policy_version, settings) = explained.map_or((None, Vec::new()), |explained| {
        let settings = explained
            .settings
            .into_iter()
            .filter(|setting| setting.layer != SettingsLayer::Default)
            .map(|setting| ReportedSetting {
                key: setting.key,
                value: setting.value,
                layer: setting.layer,
            })
            .collect();
        (explained.policy_version, settings)
    });
    let report = BugReport {
        product: machine.identity.product,
        product_version: machine.identity.product_version.to_string(),
        protocol_version: PROTOCOL_VERSION.to_string(),
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        written_at: machine.clock.timestamp().to_string(),
        urls,
        findings,
        policy_version,
        settings,
        recent_errors: recent_errors(&machine.home.logs()),
        crash_reports: crash_reports(&root.join(CRASH_FOLDER)),
    };
    let written = serde_json::to_value(&report)
        .map(|mut value| {
            reduce(&mut value, urls);
            value
        })
        .map_err(std::io::Error::other)
        .and_then(|value| {
            let mut file = tempfile::Builder::new()
                .prefix(".bug-report")
                .tempfile_in(root)?;
            serde_json::to_writer_pretty(&mut file, &value).map_err(std::io::Error::other)?;
            file.write_all(b"\n")?;
            file.persist(&path).map_err(|error| error.error)?;
            Ok(())
        });
    match written {
        Ok(()) => BugReportFile::Written {
            path: path.display().to_string(),
        },
        Err(error) => BugReportFile::NotWritten {
            message: format!(
                "the bug-report file cannot be written at {}: {error}",
                path.display()
            ),
        },
    }
}

/// Every string in `value`, each URL in it as `urls` says.
fn reduce(value: &mut Value, urls: Urls) {
    match value {
        Value::String(text) => *text = urls_in(text, urls),
        Value::Array(items) => items.iter_mut().for_each(|item| reduce(item, urls)),
        Value::Object(fields) => fields.values_mut().for_each(|field| reduce(field, urls)),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// `text` with each URL in it, `scheme://` and what follows up to a space,
/// a quote or an angle bracket, but the punctuation that ends a sentence,
/// as `urls` says.
pub(super) fn urls_in(text: &str, urls: Urls) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let start = rest[..at]
            .bytes()
            .rposition(|byte| !(byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.')))
            .map_or(0, |before| before + 1);
        let after = at + "://".len();
        if !rest
            .as_bytes()
            .get(start)
            .is_some_and(u8::is_ascii_alphabetic)
            || start == at
        {
            out.push_str(&rest[..after]);
            rest = &rest[after..];
            continue;
        }
        let tail = &rest[after..];
        let mut end = after
            + tail
                .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
                .unwrap_or(tail.len());
        while end > after && rest[..end].ends_with(['.', ',', ';', ':', '!', '?', ')']) {
            end -= 1;
        }
        out.push_str(&rest[..start]);
        out.push_str(&url(&rest[start..end], &rest[start..at], urls));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// `candidate`, a URL of `scheme`, as `urls` says: one that does not parse
/// keeps its scheme alone either way.
fn url(candidate: &str, scheme: &str, urls: Urls) -> String {
    let Ok(mut parsed) = url::Url::parse(candidate) else {
        return format!("{scheme}://");
    };
    match urls {
        Urls::Reduced => match parsed.host_str() {
            Some(host) => format!("{}://{host}", parsed.scheme()),
            None => format!("{}://", parsed.scheme()),
        },
        Urls::Whole if parsed.username().is_empty() && parsed.password().is_none() => {
            candidate.to_owned()
        }
        Urls::Whole => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string()
        }
    }
}

/// The Error codes the log lines in `folder` carry, from every file, oldest
/// first by their time, the newest [`RECENT_ERRORS`] kept.
pub(super) fn recent_errors(folder: &Path) -> Vec<RecentError> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut errors: Vec<RecentError> = entries
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "jsonl")
        })
        .filter_map(|path| std::fs::File::open(path).ok())
        .flat_map(|file| BufReader::new(file).lines().map_while(Result::ok))
        .filter_map(|line| error_of(&serde_json::from_str(&line).ok()?))
        .collect();
    errors.sort_by(|left, right| left.time.cmp(&right.time));
    let kept = errors.len().saturating_sub(RECENT_ERRORS);
    errors.split_off(kept)
}

/// The Error code `line` carries, when it carries one.
fn error_of(line: &Value) -> Option<RecentError> {
    let fields = &line["fields"];
    let text = |value: &Value| value.as_str().map(str::to_owned);
    Some(RecentError {
        time: text(&line["timestamp"]),
        code: text(&fields["error_code"]).or_else(|| text(&fields["code"]))?,
        method: text(&fields["method"]),
        message: text(&fields["message"]),
    })
}

/// The names of the newest [`CRASH_REPORTS`] crash reports in `folder`,
/// oldest first.
fn crash_reports(folder: &Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(|entry| entry.ok()?.file_name().into_string().ok())
        .collect();
    names.sort();
    let kept = names.len().saturating_sub(CRASH_REPORTS);
    names.split_off(kept)
}

#[cfg(test)]
mod tests;
