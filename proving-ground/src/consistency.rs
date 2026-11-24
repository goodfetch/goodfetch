//! The consistency probe pages on `consistency.test` and
//! `consistency-frame.test` (ADR 0074): the Goodfetch Chromium's identity
//! consistency test reads each surface of its Identity profile from every
//! context a page can run script in, and compares each with the profile and
//! with the HTTP headers the same context sent. No task: every context
//! loads `/probe.js`, reads each surface and posts one report per surface
//! to its own origin's `/reports?page=<page>`, so the observations
//! (`GET /observations`) keep each value, as JSON text, beside the request
//! that carried it and that request's headers.
//!
//! - `/` on `consistency.test`: the main frame, page `/`. It embeds
//!   `consistency-frame.test`'s `/frame` (a cross-origin iframe, page
//!   `/frame`) and starts `/dedicated-worker.js`, `/shared-worker.js` and
//!   `/service-worker.js`, each reporting as its script's path. Once every
//!   context has reported, it shows each one's values as JSON in
//!   `#contexts` and sets `data-probe="done"` on its root element. A
//!   service worker and the user-agent client hints need a secure context.
//! - `/probe.js`: the surfaces' reader, the same in a window and a worker.

use hyper::{Method, StatusCode};

use crate::exchange::{Reply, Request};
use crate::observation::{Recorded, Report};

/// The host name of the main frame and the workers.
pub(crate) const HOST: &str = "consistency.test";

/// The host name of the iframe, another site than [`HOST`], so its frame
/// is out of process.
pub(crate) const FRAME_HOST: &str = "consistency-frame.test";

/// The main frame.
const MAIN: &str = include_str!("../pages/consistency/main.html");

/// The cross-origin iframe.
const FRAME: &str = include_str!("../pages/consistency/frame.html");

/// Each script a context loads, by its path.
const SCRIPTS: [(&str, &str); 4] = [
    ("/probe.js", include_str!("../pages/consistency/probe.js")),
    (
        "/dedicated-worker.js",
        include_str!("../pages/consistency/dedicated-worker.js"),
    ),
    (
        "/shared-worker.js",
        include_str!("../pages/consistency/shared-worker.js"),
    ),
    (
        "/service-worker.js",
        include_str!("../pages/consistency/service-worker.js"),
    ),
];

/// Answers a request to the probe pages, with the report it carried, if
/// any; `None` on another host.
pub(crate) fn respond(name: &str, request: &Request<'_>) -> Option<(Reply, Option<Recorded>)> {
    if name != HOST && name != FRAME_HOST {
        return None;
    }
    Some(match (request.method, name == FRAME_HOST, request.path) {
        (&Method::POST, _, "/reports") => match Report::parse(request.body) {
            Some(report) => (Reply::no_content(), Some(Recorded::Report(report))),
            None => (
                Reply::error(StatusCode::BAD_REQUEST, "malformed_report"),
                None,
            ),
        },
        (&Method::GET, false, "/") => (Reply::html(MAIN.to_owned()), None),
        (&Method::GET, true, "/frame") => (Reply::html(FRAME.to_owned()), None),
        (&Method::GET, _, path) => match SCRIPTS.iter().find(|(script, _)| *script == path) {
            Some((_, source)) => (
                Reply {
                    status: StatusCode::OK,
                    content_type: "application/javascript",
                    headers: Vec::new(),
                    body: source.as_bytes().into(),
                },
                None,
            ),
            None => (Reply::not_found(), None),
        },
        _ => (Reply::not_found(), None),
    })
}
