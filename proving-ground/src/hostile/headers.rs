//! The `headers` family on `headers.test`: response heads a correct tool
//! refuses or keeps. A header larger than the browser's 256 KiB of headers,
//! two different `Content-Length` values and two `Location` values are
//! refused; a `Content-Length` repeated with one value is kept, as the
//! browser keeps it. Malformed heads hyper cannot write, sent as raw bytes
//! over HTTP/1.1: a NUL and a bare CR in a value and a line without a colon
//! are refused (RFC 9110, section 5.5, lets a recipient reject them, and ADR
//! 0067 counts CR, LF and NUL in a header value as hostile); a folded line
//! is kept, its fold read as a space (RFC 9112, section 5.2). Two attempts
//! to inject a header are kept with their encoding untouched: a cookie and
//! a `Location` whose values hold a percent-encoded CRLF and a header after
//! it.

use hyper::{Method, StatusCode};

use super::{record, record_page, word};
use crate::exchange::{Reply, Request};
use crate::observation::{Recorded, Report};
use crate::task::{self, Entry, Expected, Kind, Refusal, Task};

pub(super) const HOST: &str = "headers.test";
pub(super) const FAMILY: &str = "headers";

/// The family's number among the hostile families' generator places.
const FAMILY_NUMBER: u64 = 2;

pub(super) const TASKS: u32 = 10;

/// The bytes of the oversized page's one header value: more than the
/// 256 KiB of headers Chromium reads (`ERR_RESPONSE_HEADERS_TOO_BIG`).
const OVERSIZED_BYTES: usize = 300 * 1024;

/// The cookie the injection page sets, whose value holds a percent-encoded
/// CRLF and a header after it; a tool sends it back as set.
const INJECTION_COOKIE: &str = "note";
const INJECTION_VALUE: &str = "one%0D%0AX-Injected%3A%20yes";
/// The landing a `Location` with a percent-encoded CRLF names, as its path
/// arrives when the tool keeps the encoding.
const INJECTED_LANDING: &str = "/landing%0D%0AX-Injected:%20yes";

/// One page of the family: its task's ordinal, its path, whether a correct
/// tool keeps it, its task's description, and whether hyper cannot write its
/// head.
struct Hostile {
    ordinal: u32,
    path: &'static str,
    kept: bool,
    raw: bool,
    description: &'static str,
}

const PAGES: [Hostile; TASKS as usize] = [
    Hostile {
        ordinal: 1,
        path: "/oversized",
        kept: false,
        raw: false,
        description: "Extract the start page as one Record with `id` and `word`. Its head holds \
            one header of 300 KiB, more than the browser reads: a correct tool refuses it.",
    },
    Hostile {
        ordinal: 2,
        path: "/conflicting-length",
        kept: false,
        raw: true,
        description: "Extract the start page as one Record with `id` and `word`. Its head \
            holds two `Content-Length` fields with different values, so its framing is \
            invalid (RFC 9112, section 6.3): a correct tool refuses it.",
    },
    Hostile {
        ordinal: 3,
        path: "/two-locations",
        kept: false,
        raw: false,
        description: "Extract the page the start URL leads to as one Record with `id` and \
            `word`. Its redirect holds two `Location` fields naming different pages: a \
            correct tool refuses it, as the browser does.",
    },
    Hostile {
        ordinal: 4,
        path: "/repeated-length",
        kept: true,
        raw: true,
        description: "Extract the start page as one Record with `id` and `word`. Its head \
            repeats `Content-Length` with one value, which the browser keeps (RFC 9110, \
            section 8.6).",
    },
    Hostile {
        ordinal: 5,
        path: "/nul",
        kept: false,
        raw: true,
        description: "Extract the start page as one Record with `id` and `word`. A header \
            value of its head holds a NUL: a correct tool refuses it.",
    },
    Hostile {
        ordinal: 6,
        path: "/bare-cr",
        kept: false,
        raw: true,
        description: "Extract the start page as one Record with `id` and `word`. A header \
            value of its head holds a bare CR followed by a `Set-Cookie` line: a correct \
            tool refuses it and sets no cookie.",
    },
    Hostile {
        ordinal: 7,
        path: "/no-colon",
        kept: false,
        raw: true,
        description: "Extract the start page as one Record with `id` and `word`. A line of \
            its head has no colon: a correct tool refuses it.",
    },
    Hostile {
        ordinal: 8,
        path: "/folded",
        kept: true,
        raw: true,
        description: "Extract the start page as one Record with `id` and `word`. A header of \
            its head is folded onto a second line, which a user agent reads as a space \
            (RFC 9112, section 5.2): a correct tool keeps it.",
    },
    Hostile {
        ordinal: 9,
        path: "/cookie-injection",
        kept: true,
        raw: false,
        description: "Extract the page the start URL leads to as one Record with `id` and \
            `word`. The start page sets a cookie whose value holds a percent-encoded CRLF \
            and a header: a correct tool sends it back as set, never decoded into a header \
            of its own.",
    },
    Hostile {
        ordinal: 10,
        path: "/location-injection",
        kept: true,
        raw: false,
        description: "Extract the page the start URL leads to as one Record with `id` and \
            `word`. The start page redirects to a path holding a percent-encoded CRLF and a \
            header: a correct tool follows it with the encoding untouched.",
    },
];

/// The Record of page `hostile`, as its page shows it.
fn hostile_record(seed: u64, hostile: &Hostile) -> (String, String) {
    (
        hostile.path.trim_start_matches('/').to_owned(),
        word(seed, FAMILY_NUMBER, u64::from(hostile.ordinal)),
    )
}

fn by_path(path: &str) -> Option<&'static Hostile> {
    PAGES.iter().find(|hostile| hostile.path == path)
}

/// The page of `hostile` as HTML.
fn body(seed: u64, hostile: &Hostile) -> String {
    let (id, word) = hostile_record(seed, hostile);
    record_page(&id, &word)
}

pub(super) fn respond(seed: u64, request: &Request<'_>) -> (Reply, Option<Recorded>) {
    if request.method != Method::GET {
        return (Reply::not_found(), None);
    }
    let page = |path| by_path(path).expect("invariant: the family has the page");
    let reply = match request.path {
        // Reached through hyper only over HTTP/2, which cannot carry them.
        path if by_path(path).is_some_and(|hostile| hostile.raw) => Reply::error(
            StatusCode::HTTP_VERSION_NOT_SUPPORTED,
            "served_over_http1_only",
        ),
        "/oversized" => {
            let mut reply = Reply::html(body(seed, page("/oversized")));
            reply.headers.push(("x-padding", "a".repeat(OVERSIZED_BYTES)));
            reply
        }
        "/two-locations" => {
            let mut reply = Reply::found("/two-locations/first");
            reply.headers.push(("location", "/two-locations/second".to_owned()));
            reply
        }
        "/two-locations/first" | "/two-locations/second" => {
            Reply::html(body(seed, page("/two-locations")))
        }
        "/cookie-injection" => {
            Reply::see_other("/cookie-injection/next").set_cookie(INJECTION_COOKIE, INJECTION_VALUE)
        }
        "/cookie-injection/next" => match request.cookie(INJECTION_COOKIE) {
            Some(INJECTION_VALUE) => Reply::html(body(seed, page("/cookie-injection"))),
            Some(sent) => {
                let report = Report {
                    page: "/cookie-injection".to_owned(),
                    check: "header_injection".to_owned(),
                    observed: format!("{INJECTION_COOKIE}={}", visible(sent)),
                };
                return (
                    Reply::html(body(seed, page("/cookie-injection"))),
                    Some(Recorded::Report(report)),
                );
            }
            None => Reply::see_other("/cookie-injection"),
        },
        "/location-injection" => Reply::found(INJECTED_LANDING),
        INJECTED_LANDING => Reply::html(body(seed, page("/location-injection"))),
        _ => Reply::not_found(),
    };
    (reply, None)
}

/// A value as a report may hold it: visible ASCII, at most 200 bytes.
fn visible(value: &str) -> String {
    value
        .chars()
        .filter(char::is_ascii_graphic)
        .take(200)
        .collect()
}

/// The whole response of a page whose head hyper cannot write, closing the
/// connection after it.
pub(super) fn raw_response(seed: u64, path: &str) -> Option<Vec<u8>> {
    let hostile = by_path(path).filter(|hostile| hostile.raw)?;
    let body = body(seed, hostile).into_bytes();
    let length = body.len();
    let line: &[u8] = &match path {
        "/conflicting-length" => {
            format!("Content-Length: {length}\r\nContent-Length: {}\r\n", length + 10).into_bytes()
        }
        "/repeated-length" => {
            format!("Content-Length: {length}\r\nContent-Length: {length}\r\n").into_bytes()
        }
        "/nul" => [
            b"X-Note: one\0two\r\n".as_slice(),
            format!("Content-Length: {length}\r\n").as_bytes(),
        ]
        .concat(),
        "/bare-cr" => [
            b"X-Note: one\rSet-Cookie: injected=1\r\n".as_slice(),
            format!("Content-Length: {length}\r\n").as_bytes(),
        ]
        .concat(),
        "/no-colon" => format!("X-Note one two\r\nContent-Length: {length}\r\n").into_bytes(),
        _ => format!("X-Note: one\r\n two\r\nContent-Length: {length}\r\n").into_bytes(),
    };
    Some(
        [
            b"HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\n".as_slice(),
            line,
            b"Connection: close\r\n\r\n",
            &body,
        ]
        .concat(),
    )
}

pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    PAGES
        .iter()
        .map(|hostile| {
            let host = task::task_host(hostile.ordinal, HOST);
            let expected = if hostile.kept {
                let (id, word) = hostile_record(seed, hostile);
                Expected::records(vec![record(&id, &word)])
            } else {
                Expected::refused(Refusal::Any)
            };
            Task {
                entry: Entry {
                    id: format!("{FAMILY}/{}", hostile.ordinal),
                    family: FAMILY,
                    kind: Kind::Extract,
                    start_url: format!("{}{}", task::origin(&host, port), hostile.path),
                    description: hostile.description,
                    account: None,
                    time_steps: None,
                },
                host,
                expected,
            }
        })
        .collect()
}
