//! Fingerprint consistency from HTTP to the browser (ADR 0035), judged from
//! the server's own observations: every request of one Session presents the
//! same protocol, JA4, HTTP/2 settings, header order, user agent and client
//! hints as its first, and no request's client hints claim another Chrome
//! major than its user agent.

use serde::Serialize;
use serde_json::Value;

use super::versions;
use crate::observation::{Fingerprint, Seen};
use crate::oracle::Finding;

/// The headers a header order is compared without: those HTTP/1.1 sends and
/// HTTP/2 carries otherwise (`host`, `connection`), and those that change
/// with a Session's state, not its client (`cookie`, `referer`).
const UNCOMPARED_HEADERS: [&str; 4] = ["host", "connection", "cookie", "referer"];

/// The findings of `requests`, the requests of one Session's pages in
/// arrival order: each property of a request's fingerprint that differs
/// from the first request's, and each request whose client hints claim
/// another Chrome major than its user agent.
pub(crate) fn findings(requests: &[&Seen]) -> Vec<Finding> {
    let Some(first) = requests.first() else {
        return Vec::new();
    };
    let mut findings = Vec::new();
    for (place, request) in requests.iter().enumerate() {
        if place > 0 {
            for (check, expected, observed) in differences(&first.fingerprint, &request.fingerprint)
            {
                findings.push(Finding {
                    check: check.to_owned(),
                    at: request.target.clone(),
                    expected: Some(expected),
                    observed,
                });
            }
        }
        let claimed = request
            .fingerprint
            .user_agent
            .as_deref()
            .and_then(versions::claimed_major);
        let hinted = request
            .fingerprint
            .client_hints
            .iter()
            .find(|(name, _)| name == "sec-ch-ua")
            .and_then(|(_, brands)| versions::hinted_major(brands));
        if let (Some(claimed), Some(hinted)) = (claimed, hinted)
            && claimed != hinted
        {
            findings.push(Finding {
                check: "client_hints_version".to_owned(),
                at: request.target.clone(),
                expected: Some(claimed.into()),
                observed: hinted.into(),
            });
        }
    }
    findings
}

/// Each property in which `other` differs from `first`, with the first's
/// value and the other's, in a fixed order.
fn differences(first: &Fingerprint, other: &Fingerprint) -> Vec<(&'static str, Value, Value)> {
    let header_order = |fingerprint: &Fingerprint| -> Vec<String> {
        fingerprint
            .header_order
            .iter()
            .filter(|name| !UNCOMPARED_HEADERS.contains(&name.as_str()))
            .cloned()
            .collect()
    };
    let mut differences = Vec::new();
    let mut compare = |check, first: Value, other: Value| {
        if first != other {
            differences.push((check, first, other));
        }
    };
    compare("protocol", json(&first.protocol), json(&other.protocol));
    compare("ja4", json(&first.ja4), json(&other.ja4));
    compare("http2", json(&first.http2), json(&other.http2));
    compare(
        "header_order",
        json(&header_order(first)),
        json(&header_order(other)),
    );
    compare(
        "user_agent",
        json(&first.user_agent),
        json(&other.user_agent),
    );
    compare(
        "client_hints",
        json(&first.client_hints),
        json(&other.client_hints),
    );
    differences
}

fn json(value: &impl Serialize) -> Value {
    serde_json::to_value(value).expect("invariant: a fingerprint serializes")
}
