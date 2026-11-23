//! The Google-host stand-in (#328, ADR 0074): the Proving ground answers on
//! Google's own host names, which a test maps to it with `resolve` (ADR
//! 0059), so the quiet-browser test of the Goodfetch Chromium reaches no
//! real Google server and still sees what the browser sends there. Chrome
//! attaches the `X-Client-Data` header of its variations seed only to
//! Google's domains over HTTPS (`variations_http_headers.cc`): the
//! `*.google.<TLD>` and `*.youtube.<TLD>` hosts, `*.doubleclick.net`,
//! `*.googlesyndication.com` and exactly `www.googleadservices.com`, one
//! host of each kind here. Every request to them is observed with its
//! headers in the order sent, `X-Client-Data` among them (`GET
//! /observations`), so the test checks the shipped seed's header (ADR
//! 0043).
//!
//! Any path answers: a `GET` or `HEAD` a small page, any other method
//! `204`, as no request the browser sends there should fail on a missing
//! path.

use hyper::Method;

use crate::exchange::{Reply, Request};

/// The Google host names the stand-in answers on, one of each kind Chrome
/// sends `X-Client-Data` to.
pub(crate) const HOSTS: [&str; 5] = [
    "www.google.com",
    "www.youtube.com",
    "googleads.g.doubleclick.net",
    "pagead2.googlesyndication.com",
    "www.googleadservices.com",
];

/// The stand-in's page.
const PAGE: &str = "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Google-host stand-in</title></head>\n<body><p id=\"stand-in\">The Proving ground's Google-host stand-in.</p></body>\n</html>\n";

/// The stand-in's reply to `request` on `name`; `None` when `name` is not
/// one of [`HOSTS`].
pub(crate) fn respond(name: &str, request: &Request<'_>) -> Option<Reply> {
    if !HOSTS.contains(&name) {
        return None;
    }
    Some(match *request.method {
        Method::GET | Method::HEAD => Reply::html(PAGE.to_owned()),
        _ => Reply::no_content(),
    })
}
