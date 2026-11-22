//! The records of the messages a command's Connection carries (ADR 0071's
//! protocol counts): at `trace`, one per message, with its direction, its
//! kind, its method and its bytes after the shared snapshot redactions of
//! ports, times and IDs, so the same exchange records the same bytes on
//! every run and machine. A record never holds the message's content,
//! which may hold the Daemon's token, and with `trace` off nothing is
//! parsed, redacted or compiled.
//!
//! The redactions are the patterns of the one committed file the snapshot
//! tools read, `test-support/fixtures/snapshot-filters.json`, compiled once,
//! on the first record. Its run values (the test's folder, the Product
//! version, the commit) are a test's own and are not applied here.

use std::sync::OnceLock;

use regex::Regex;
use serde::Deserialize;
use serde_json::Value;

/// The `tracing` target of the records, from which a test counts them.
const WIRE_TARGET: &str = "goodfetch::cli::wire";

/// The direction of a message the command sent.
pub(super) const SENT: &str = "sent";

/// The direction of a message the command received.
pub(super) const RECEIVED: &str = "received";

/// The committed snapshot filters' text, which the snapshot tools read too.
const COMMITTED: &str = include_str!("../../../../test-support/fixtures/snapshot-filters.json");

/// The committed filters, as far as the records read them: their patterns.
#[derive(Deserialize)]
struct SnapshotFilters {
    patterns: Vec<PatternFilter>,
}

/// A rule that replaces every match of a pattern with literal text.
#[derive(Deserialize)]
struct PatternFilter {
    pattern: String,
    replacement: String,
}

/// The committed patterns, compiled on the first record.
static PATTERNS: OnceLock<Vec<(Regex, String)>> = OnceLock::new();

/// Records one message of the Connection, `line`, at `trace`: its
/// direction, its kind (`request`, `notification`, `response` or
/// `error`), its method when it has one, and its bytes after the shared
/// redactions. A line that is no JSON message is not recorded.
pub(super) fn record_wire(direction: &str, line: &[u8]) {
    if !tracing::enabled!(target: WIRE_TARGET, tracing::Level::TRACE) {
        return;
    }
    let Ok(message) = serde_json::from_slice::<Value>(line) else {
        return;
    };
    let method = message["method"].as_str().unwrap_or_default();
    let kind = match (method.is_empty(), message.get("id").is_some()) {
        (false, true) => "request",
        (false, false) => "notification",
        (true, _) if message.get("error").is_some() => "error",
        (true, _) => "response",
    };
    tracing::trace!(
        target: WIRE_TARGET,
        direction,
        kind,
        method,
        bytes = redacted_len(&String::from_utf8_lossy(line)),
        "message"
    );
}

/// The length in bytes of `text` once each committed pattern, in the
/// file's order, has replaced its matches.
fn redacted_len(text: &str) -> usize {
    patterns()
        .iter()
        .fold(text.to_owned(), |text, (pattern, replacement)| {
            pattern
                .replace_all(&text, replacement.as_str())
                .into_owned()
        })
        .len()
}

/// The committed patterns, compiled once.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn patterns() -> &'static [(Regex, String)] {
    PATTERNS.get_or_init(|| {
        let filters: SnapshotFilters = serde_json::from_str(COMMITTED).expect(
            "invariant: the committed snapshot filters parse, which the Quick suite checks",
        );
        filters
            .patterns
            .into_iter()
            .map(|filter| {
                // `(?-u)`: a word boundary is ASCII's, as the snapshot
                // tools' is in all three languages.
                let pattern = Regex::new(&format!("(?-u){}", filter.pattern)).expect(
                    "invariant: each committed pattern compiles, which the Quick suite checks",
                );
                (pattern, filter.replacement)
            })
            .collect()
    })
}

#[cfg(test)]
mod tests;
