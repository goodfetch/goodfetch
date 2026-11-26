//! `cache.test`, the Response cache's host (ADR 0046; pitfall H19), which
//! belongs to no task. Each page says how a response may be stored and
//! which answer of its path it is, counted since `serve` started, so a
//! case tells a response a cache served from one the site sent; the
//! observations record each request's validators and each answer's status.
//! A case gives each of its pages a name of its own.
//!
//! - `/fresh/<name>` may be kept for an hour (`Cache-Control: max-age=3600`).
//! - `/stale/<name>` is stale at once (`max-age=0`), with the validators
//!   `ETag: "<name>-v1"` and a `Last-Modified` in the past: a request
//!   naming either, in `If-None-Match` or `If-Modified-Since`, is answered
//!   `304` with no body.
//! - `/vary/<name>` may be kept for an hour and varies on `Cookie`
//!   (`Vary: Cookie`): it names the cookie it was sent.
//! - `/cookie/<name>` sets the cookie `visitor=<name>` and redirects
//!   (`302`) to `/vary/<name>`, never stored (`no-store`).
//! - `/api/<name>` answers any method, never to be stored (`no-store`),
//!   which only `replay` keeps: it names the method and how many bytes of
//!   body it was sent.
//!
//! The host has no robots.txt.

use crate::raw::{Answer, Page, Request};

/// The host name the pages are served on.
pub(super) const HOST: &str = "cache.test";

/// When every stale page was last modified: a date before any run.
const LAST_MODIFIED: &str = "Thu, 01 Jan 2026 00:00:00 GMT";

/// The answer to `request`, its path asked for `earlier` times before.
pub(super) fn answer(request: &Request, earlier: u64) -> Answer {
    let path = request.path();
    let answer = earlier + 1;
    if path.starts_with("/fresh/") {
        return Answer::Page(page(path, answer, "").with("cache-control", "max-age=3600"));
    }
    if let Some(name) = path.strip_prefix("/stale/") {
        let etag = format!("\"{name}-v1\"");
        let validated = request.header("if-none-match") == Some(etag.as_str())
            || request.header("if-modified-since") == Some(LAST_MODIFIED);
        let page = if validated {
            Page::untyped(304, Vec::new())
        } else {
            page(path, answer, "")
        };
        return Answer::Page(
            page.with("cache-control", "max-age=0")
                .with("etag", etag)
                .with("last-modified", LAST_MODIFIED),
        );
    }
    if path.starts_with("/vary/") {
        let cookie = request.header("cookie").unwrap_or("none");
        return Answer::Page(
            page(path, answer, &format!("<p>Cookie: {cookie}.</p>"))
                .with("cache-control", "max-age=3600")
                .with("vary", "Cookie"),
        );
    }
    if path.starts_with("/api/") {
        let said = format!(
            "<p>{} with {} bytes.</p>",
            request.method,
            request.body.len()
        );
        return Answer::Page(page(path, answer, &said).with("cache-control", "no-store"));
    }
    if let Some(name) = path.strip_prefix("/cookie/") {
        return Answer::Page(
            Page::text(302, "")
                .with("location", format!("/vary/{name}"))
                .with("set-cookie", format!("visitor={name}; Path=/"))
                .with("cache-control", "no-store"),
        );
    }
    Answer::Page(Page::text(404, "not found"))
}

/// The page at `path`, the site's answer `answer` to it, with `more` in
/// its body.
fn page(path: &str, answer: u64, more: &str) -> Page {
    Page::html(
        200,
        format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>{path}</title></head><body><main><h1>{path}</h1><p>Answer {answer} of {path}.</p>{more}</main></body></html>"
        ),
    )
}
