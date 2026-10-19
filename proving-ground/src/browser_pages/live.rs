//! The `live` family: a board its script keeps busy with a WebSocket
//! (`/socket`), a server-sent events stream (`/events`) and periodic
//! requests (`/updates/<n>`), each request starting when the one before
//! it is answered and held in flight [`HOLD`] by the server. On task 1's
//! page the periodic requests stop after [`UPDATES`] and the board says it
//! settled; on task 2's they never stop. Nothing else differs, so
//! `networkidle` (no request in flight for 500 ms, WebSocket connections
//! and server-sent events left out) comes on the first and never on the second,
//! and a tool that reads the board before its last update reads a stale one.

use std::time::Duration;

use hyper::StatusCode;

use super::document;
use crate::exchange::Reply;
use crate::generator;
use crate::push::Push;
use crate::task::{self, Entry, Expected, Kind, Record, Task};

pub(super) const HOST: &str = "live.test";
pub(super) const FAMILY: &str = "live";
pub(super) const TASKS: u32 = 2;

/// How many periodic requests task 1's page makes before it settles.
const UPDATES: u64 = 6;

/// The most periodic requests task 2's page is answered, past which it is
/// answered `404`: a never-settling page left open for days.
const MOST_UPDATES: u64 = 10_000_000;

/// How long the server holds each periodic request in flight.
const HOLD: Duration = Duration::from_millis(100);

/// The generator place of the family's values.
const FAMILY_PLACE: u64 = 21;

const SCRIPT: &str = include_str!("../../pages/browser/live.js");

/// How many periodic requests task `ordinal`'s page makes; `None` when it
/// never stops.
fn updates(ordinal: u32) -> Option<u64> {
    (ordinal == 1).then_some(UPDATES)
}

/// The reading update `number` of task `ordinal`'s page shows.
fn reading(seed: u64, ordinal: u32, number: u64) -> String {
    let value = generator::below(seed, &[FAMILY_PLACE, u64::from(ordinal), number], 10_000);
    format!("R-{value:04}")
}

/// The number of the periodic request at `path`, if it is one task
/// `ordinal`'s page makes.
fn update(ordinal: u32, path: &str) -> Option<u64> {
    let digits = path.strip_prefix("/updates/")?;
    if digits.starts_with('0') || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let number = digits.parse::<u64>().ok()?;
    let last = updates(ordinal).unwrap_or(MOST_UPDATES);
    (1..=last).contains(&number).then_some(number)
}

/// A `GET` of `path` on task `ordinal`'s host.
pub(super) fn page(seed: u64, ordinal: u32, path: &str) -> Option<Reply> {
    if path == "/board" {
        return Some(Reply::html(board(ordinal)));
    }
    let number = update(ordinal, path)?;
    Some(Reply::json(
        StatusCode::OK,
        &serde_json::json!({ "update": number, "reading": reading(seed, ordinal, number) }),
    ))
}

/// The connection a `GET` of `path` holds open, if it asks for one.
pub(super) fn push(path: &str) -> Option<Push> {
    match path {
        "/events" => Some(Push::Events),
        "/socket" => Some(Push::Socket),
        _ => None,
    }
}

/// The hold of a periodic request.
pub(super) fn hold(path: &str) -> Option<Duration> {
    path.starts_with("/updates/").then_some(HOLD)
}

/// The board of task `ordinal`.
fn board(ordinal: u32) -> String {
    let updates =
        updates(ordinal).map_or_else(|| "unbounded".to_owned(), |count| count.to_string());
    document(
        "Live board",
        "",
        "<h1>Live board</h1>\n<dl class=\"board\" data-id=\"board\">\n<dt>State</dt><dd id=\"state\">loading</dd>\n<dt>Updates</dt><dd id=\"updates\">0</dd>\n<dt>Reading</dt><dd id=\"reading\">none</dd>\n<dt>Socket messages</dt><dd id=\"socket\">0</dd>\n<dt>Server-sent events</dt><dd id=\"events\">0</dd>\n</dl>\n",
        &format!("<script data-updates=\"{updates}\">\n{SCRIPT}</script>\n"),
    )
}

/// The family's tasks for `seed`. Ordinals are append-only (ADR 0063).
pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let task = |ordinal: u32, description: &'static str, expected: Expected| {
        let host = task::task_host(ordinal, HOST);
        Task {
            entry: Entry {
                id: format!("{FAMILY}/{ordinal}"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("{}/board", task::origin(&host, port)),
                description,
                account: None,
                time_steps: None,
            },
            host,
            expected,
        }
    };
    vec![
        task(
            1,
            "Wait until this page's network is idle (no request in flight for 500 ms, its \
             WebSocket and server-sent events left out), then extract its board as one Record \
             with the fields `id` (the board's `data-id`), `state`, `updates` and `reading`. \
             The page holds a WebSocket and a server-sent events stream open for as long as it \
             is open, and makes periodic requests, one after another, until it settles.",
            Expected::records(vec![Record::from([
                ("id", "board".to_owned()),
                ("state", "settled".to_owned()),
                ("updates", UPDATES.to_string()),
                ("reading", reading(seed, 1, UPDATES)),
            ])]),
        ),
        task(
            2,
            "Wait until this page's network is idle (no request in flight for 500 ms, its \
             WebSocket and server-sent events left out), then extract its board as one Record \
             with the fields `id` (the board's `data-id`), `state`, `updates` and `reading`. \
             The page holds a WebSocket and a server-sent events stream open for as long as it \
             is open, and makes periodic requests, one after another, that never stop: its \
             network never becomes idle, so the only right result is a failure.",
            Expected {
                refusal: true,
                ..Expected::default()
            },
        ),
    ]
}
