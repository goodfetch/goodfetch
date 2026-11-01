//! The responses that ask a crawl to slow down (ADRs 0021, 0053, 0086,
//! 0102 and 0118; spec #579's `backoff_classes`), and whose they are. Pure:
//! no I/O and no task.
//!
//! - **Proxy or site:** an answer is the proxy's when the proxy answered
//!   the tunnel's `CONNECT` itself (a failure whose network error is a
//!   proxy's, such as `net::ERR_TUNNEL_CONNECTION_FAILED`), or when it
//!   forwarded the request itself and its response's `Proxy-Status` names
//!   an error of its own; any other answer is the site's. It is the rule
//!   Collie's page applies to the browser ([`goodfetch_page::party`]),
//!   applied to every tier's answer alike.
//! - **The site's:** a 429 halves its domain's concurrency and backs it
//!   off; a 502, 503 or 504, a refused or timed-out connection and an
//!   HTTP/2 `ENHANCE_YOUR_CALM` back it off. A 500 does not: it usually
//!   belongs to one page. Chromium names an `ENHANCE_YOUR_CALM` reset as it
//!   names every HTTP/2 reset it has no error of its own for,
//!   `net::ERR_HTTP2_PROTOCOL_ERROR`, so that error backs the domain off on
//!   every tier.
//! - **The proxy's:** its own 429 or 502, or a tunnel it refused, lowers
//!   its learned ceiling and backs no domain off (ADR 0053); its other
//!   answers, a 407 or a proxy nothing answers on, do neither.

use goodfetch_page::{Party, ProxyAnswer, party};

/// What a request met: a response, or a network error and no response.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Met<'a> {
    /// A response.
    Response {
        /// Its status.
        status: u16,
        /// Its `Proxy-Status` header (RFC 9209), if it carries one.
        proxy_status: Option<&'a str>,
        /// Whether a proxy forwarded the request itself: an `http:` URL
        /// sent to an HTTP or HTTPS proxy. Only then can the proxy write
        /// its own member of `Proxy-Status`; over a tunnel the field is
        /// the site's.
        forwarded: bool,
    },
    /// No response: the network error, as Chrome names it, such as
    /// `net::ERR_CONNECTION_REFUSED`.
    Failure {
        /// The network error.
        error: &'a str,
    },
}

/// What a request's answer asks of its domain or its proxy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    /// Nothing: the domain may go on.
    Done,
    /// The site asks its domain to back off.
    BackOff,
    /// The site answered too many requests came: its domain's concurrency
    /// is halved, and it backs off.
    TooManyRequests,
    /// The proxy refused a request its plan does not admit: its ceiling is
    /// lowered, and no domain backs off.
    ProxyRefusal,
}

/// The network errors of a site that failed for now: a refused or
/// timed-out connection, and an HTTP/2 reset such as `ENHANCE_YOUR_CALM`.
const BACKS_OFF: [&str; 4] = [
    "ERR_CONNECTION_REFUSED",
    "ERR_CONNECTION_TIMED_OUT",
    "ERR_TIMED_OUT",
    "ERR_HTTP2_PROTOCOL_ERROR",
];

/// Who gave `met`: the proxy or the site.
pub fn party_of(met: Met<'_>) -> Party {
    party(answer(met))
}

/// What `met` asks of its domain or its proxy.
pub fn class(met: Met<'_>) -> Class {
    match (party_of(met), met) {
        (
            Party::Proxy,
            Met::Response {
                status: 429 | 502, ..
            },
        ) => Class::ProxyRefusal,
        (Party::Proxy, Met::Failure { error }) if name(error) == "ERR_TUNNEL_CONNECTION_FAILED" => {
            Class::ProxyRefusal
        }
        (Party::Site, Met::Response { status: 429, .. }) => Class::TooManyRequests,
        (
            Party::Site,
            Met::Response {
                status: 502..=504, ..
            },
        ) => Class::BackOff,
        (Party::Site, Met::Failure { error }) if BACKS_OFF.contains(&name(error)) => Class::BackOff,
        (Party::Proxy | Party::Site, _) => Class::Done,
    }
}

/// `met` as the proxy-or-site rule reads it.
fn answer(met: Met<'_>) -> ProxyAnswer<'_> {
    match met {
        Met::Response {
            proxy_status,
            forwarded,
            ..
        } => ProxyAnswer::Response {
            proxy_status,
            forwarded,
        },
        Met::Failure { error } => ProxyAnswer::Failure { error },
    }
}

/// A network error's name without Chrome's `net::` prefix.
fn name(error: &str) -> &str {
    error.strip_prefix("net::").unwrap_or(error)
}

#[cfg(test)]
mod tests;
