//! The families whose state only a browser reaches (#1536): pages for the
//! input calls, printing, layout, live updates and key events, each with an
//! expected result the oracle reads from the page's own state, as a tool
//! returns it, and from the Proving ground's own record of what the page's
//! script reported and what was requested.
//!
//! - `live` on `live.test`: a board kept busy by a WebSocket, a server-sent
//!   events stream and periodic requests; the periodic requests stop on one
//!   task's page and never on the other's, which is all the two differ in.
//! - `canvas` on `canvas.test`: a button drawn on a `<canvas>`, with no
//!   element, role or text for it, and a drawing surface read back through
//!   `evaluate`.
//! - `print` on `print.test`: a report whose print styles show and hide
//!   other entries than its screen styles, saved as print, as screen and as
//!   one MHTML file.
//! - `responsive` on `responsive.test`: a layout whose values change at
//!   viewport widths and device pixel ratios.
//! - `keys` on `keys.test`: a field that records every key, input and
//!   composition event it receives, in order.

mod canvas;
mod keys;
mod live;
mod print;
mod responsive;

use std::time::Duration;

use hyper::Method;

use crate::exchange::{Reply, Request};
use crate::observation::{Recorded, Seen};
use crate::oracle::Finding;
use crate::push::Push;
use crate::task::{self, Effect, Reported, Task};

/// The families, each with its host name and task count.
const FAMILIES: [(&str, &str, u32); 5] = [
    (live::FAMILY, live::HOST, live::TASKS),
    (canvas::FAMILY, canvas::HOST, canvas::TASKS),
    (print::FAMILY, print::HOST, print::TASKS),
    (responsive::FAMILY, responsive::HOST, responsive::TASKS),
    (keys::FAMILY, keys::HOST, keys::TASKS),
];

/// Every host name the families are served on, each with its task.
pub(crate) fn hosts() -> Vec<(String, Option<String>)> {
    FAMILIES
        .iter()
        .flat_map(|(family, host, tasks)| task::family_hosts(family, host, *tasks))
        .collect()
}

/// The families' tasks for `seed`, their start URLs on `port`.
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let mut tasks = live::tasks(seed, port);
    tasks.extend(canvas::tasks(seed, port));
    tasks.extend(print::tasks(seed, port));
    tasks.extend(responsive::tasks(seed, port));
    tasks.extend(keys::tasks(seed, port));
    tasks
}

/// Where the vision rung clicks on task `id`'s page, in the page's CSS
/// pixels; `None` when its page draws nothing to point at.
pub(crate) fn point(seed: u64, id: &str) -> Option<(u64, u64)> {
    let ordinal = id
        .strip_prefix(canvas::FAMILY)?
        .strip_prefix('/')?
        .parse()
        .ok()?;
    canvas::point(seed, ordinal)
}

/// The family of `name`, a host name without its port, and the ordinal of
/// its task; `None` when it is no task's host of these families.
fn task_of(name: &str) -> Option<(&'static str, u32)> {
    FAMILIES.iter().find_map(|(family, host, tasks)| {
        let ordinal = name.strip_suffix(host)?.strip_suffix('.')?;
        if ordinal.starts_with('0') {
            return None;
        }
        let ordinal = ordinal.parse::<u32>().ok()?;
        (1..=*tasks)
            .contains(&ordinal)
            .then_some((*family, ordinal))
    })
}

/// Answers a request to one of the families' task hosts, `name` without
/// its port: the page, and the Effect its script reported, if any; `None`
/// when `name` is not one.
pub(crate) fn respond(
    seed: u64,
    name: &str,
    request: &Request<'_>,
) -> Option<(Reply, Option<Recorded>)> {
    let (family, ordinal) = task_of(name)?;
    if request.method == Method::POST && request.path == EFFECTS_PATH {
        return Some(match serde_json::from_slice::<Reported>(request.body) {
            Ok(reported) => (
                Reply::no_content(),
                Some(Recorded::Effect(Effect::from(reported))),
            ),
            Err(_) => (
                Reply::error(hyper::StatusCode::BAD_REQUEST, "malformed_effect"),
                None,
            ),
        });
    }
    if request.method != Method::GET {
        return Some((Reply::not_found(), None));
    }
    let reply = match family {
        live::FAMILY => live::page(seed, ordinal, request.path),
        canvas::FAMILY => canvas::page(seed, ordinal, request.path),
        print::FAMILY => print::page(seed, request.path),
        responsive::FAMILY => responsive::page(seed, request.path),
        _ => keys::page(seed, ordinal, request.path),
    };
    Some((reply.unwrap_or_else(Reply::not_found), None))
}

/// The connection a request to `name` and `path` asks the server to hold
/// open, if it is one of the `live` family's.
pub(crate) fn push(name: &str, method: &Method, path: &str) -> Option<Push> {
    let (family, _) = task_of(name)?;
    (family == live::FAMILY && method == Method::GET)
        .then(|| live::push(path))
        .flatten()
}

/// How long the server holds its answer to a request to `name` and `path`
/// before it sends it, keeping the request in flight; `None` for no hold.
pub(crate) fn hold(name: &str, path: &str) -> Option<Duration> {
    let (family, _) = task_of(name)?;
    (family == live::FAMILY).then(|| live::hold(path)).flatten()
}

/// What the task's own requests refute, as observed: an image of another
/// device pixel ratio than the task's, for a `responsive` task.
pub(crate) fn findings(task: &str, requests: &[Seen]) -> Vec<Finding> {
    responsive::findings(task, requests)
}

/// Where the families' pages post the Effects their scripts report, each
/// script counting its posts in flight in the root element's
/// `data-posting`, so a tool knows when the site has them all.
const EFFECTS_PATH: &str = "/effects";

/// A whole page: its title, its `<main>`, its styles and its script, if any.
fn document(title: &str, head: &str, main: &str, script: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n<link rel=\"icon\" href=\"data:,\">\n<title>{title}</title>\n{head}</head>\n<body>\n<main>\n{main}</main>\n{script}</body>\n</html>\n"
    )
}
