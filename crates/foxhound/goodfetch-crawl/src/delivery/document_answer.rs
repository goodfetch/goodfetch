//! What a site's answer to a crawl request's own document asks of its
//! domain, by the backoff classes (ADRs 0053 and 0118): its class, and the
//! `Retry-After` the engine reads on a 429, 502, 503 or 504 of the site's,
//! never on an answer the Run's proxy gave itself. The domain's ending
//! ([`super::politeness`]) waits out the time it names; the attempt notes
//! one that does not parse in the Run trace.

use std::time::SystemTime;

use goodfetch_engine::retry_after::{self, RetryAfter};
use goodfetch_frontier::backoff_classes::{Class, Met, class};
use goodfetch_protocol::page::PageResponse;

/// The header a proxy names its own errors in (RFC 9209), lower-cased as a
/// response's headers are.
const PROXY_STATUS: &str = "proxy-status";

/// What an answer to a request's own document asks of its domain.
pub(super) struct Asked<'r> {
    /// Its class by the backoff classes.
    pub(super) class: Class,
    /// Its `Retry-After`, as the site sent it and as the engine reads it,
    /// when it carries one and its class reads it: a 429 or a backoff
    /// status of the site's.
    pub(super) retry_after: Option<(&'r str, RetryAfter)>,
}

/// What `response` asks of its domain, `proxied` when the Run's Sessions
/// send through a proxy, which forwards an `http:` URL itself (ADR 0053);
/// its `Retry-After` read at `wall`, against its own `Date` when it has one
/// (ADR 0118).
pub(super) fn asked(response: &PageResponse, proxied: bool, wall: SystemTime) -> Asked<'_> {
    let header = |name: &str| response.headers.get(name).map(String::as_str);
    let class = class(Met::Response {
        status: response.status,
        proxy_status: header(PROXY_STATUS),
        forwarded: proxied && response.url.starts_with("http:"),
    });
    let reads = matches!(class, Class::TooManyRequests | Class::BackOff);
    let retry_after = header("retry-after")
        .filter(|_| reads)
        .map(|value| (value, retry_after::read(value, header("date"), wall)));
    Asked { class, retry_after }
}
