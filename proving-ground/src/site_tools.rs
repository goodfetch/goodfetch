//! The `site_tools` family on `site-tools.test`: a page whose script
//! registers WebMCP tools (AI-09), for the site-tools operations. It
//! registers four, one of them read-only and answering the order the page
//! holds, one that never answers and one whose input schema is only a `$ref`
//! to a URL; half a second after the page has loaded it removes the fourth
//! and shows `Ready`. The tasks start over TLS, a secure context, as WebMCP
//! needs.

use hyper::{Method, StatusCode};
use serde::Serialize;
use serde_json::json;

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::task::{self, Entry, Expected, Kind, Record, Task};

/// The host name the family is served on.
pub(crate) const HOST: &str = "site-tools.test";

const FAMILY: &str = "site_tools";

/// The page that registers the tools.
const PATH: &str = "/tools";

/// The schema `send_message`'s input schema refers to, under the path no
/// task may request: a client never resolves a `$ref` to a URL.
const SCHEMAS: &str = "/schemas/";
const SCHEMA_PATH: &str = "/schemas/message.json";

/// The tools the page offers once it shows `Ready`, each with whether the
/// site marks it read-only. `track_parcel`, read-only too, is gone by then.
const OFFERED: [(&str, bool); 3] = [
    ("look_up_order", true),
    ("place_order", false),
    ("send_message", false),
];

/// The page's script.
const SCRIPT: &str = include_str!("../pages/site_tools/tools.js");

/// The generator places of the family's values, after the family's own.
const FAMILY_PLACE: u64 = 46;
const ORDER_PLACE: u64 = 1;
const ITEM_PLACE: u64 = 2;
const TOTAL_PLACE: u64 = 3;

const ITEMS: [&str; 8] = [
    "Brass lantern",
    "Wool blanket",
    "Cast-iron pan",
    "Linen apron",
    "Copper kettle",
    "Oak stool",
    "Clay teapot",
    "Canvas satchel",
];

/// The order the page holds, which `look_up_order` answers for its ID.
#[derive(Serialize)]
struct Order {
    id: String,
    item: &'static str,
    total: String,
}

/// The order of `seed`.
fn order(seed: u64) -> Order {
    let draw = |value, bound| generator::below(seed, &[FAMILY_PLACE, value], bound);
    let item = draw(ITEM_PLACE, ITEMS.len() as u64);
    let cents = 500 + draw(TOTAL_PLACE, 20_000);
    Order {
        id: format!("O-{:04}", draw(ORDER_PLACE, 10_000)),
        item: ITEMS[usize::try_from(item).expect("invariant: an item index is below 8")],
        total: format!("{}.{:02}", cents / 100, cents % 100),
    }
}

/// Answers a request on one of the family's host names.
pub(crate) fn respond(seed: u64, request: &Request<'_>) -> Reply {
    match (request.method, request.path) {
        (&Method::GET, PATH) => Reply::html(page(&order(seed))),
        (&Method::GET, SCHEMA_PATH) => Reply::json(
            StatusCode::OK,
            &json!({
                "type": "object",
                "properties": { "to": { "type": "string" }, "text": { "type": "string" } },
                "required": ["to", "text"],
            }),
        ),
        _ => Reply::not_found(),
    }
}

/// The page that shows `order`'s ID and registers the tools.
fn page(order: &Order) -> String {
    let held = serde_json::to_string(order).expect("invariant: an order serializes");
    let id = &order.id;
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Your order</title></head>\n<body>\n<main>\n<h1>Your order</h1>\n<p>Order <span id=\"order-id\">{id}</span></p>\n<p id=\"status\" role=\"status\">Loading</p>\n</main>\n<script type=\"application/json\" id=\"order\">{held}</script>\n<script>\n{SCRIPT}</script>\n</body>\n</html>\n"
    )
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`, over TLS. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let task = |ordinal, description, records| {
        let host = task::task_host(ordinal, HOST);
        Task {
            entry: Entry {
                id: format!("{FAMILY}/{ordinal}"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("{}{PATH}", task::secure_origin(&host, port)),
                description,
                account: None,
                time_steps: None,
            },
            host,
            expected: Expected {
                records,
                forbidden_paths: vec![SCHEMAS],
                ..Expected::default()
            },
        }
    };
    let Order { id, item, total } = order(seed);
    vec![
        task(
            1,
            "List the WebMCP tools this page offers once it shows `Ready`, one Record per \
             tool with the fields `id` (the tool's name) and `read_only` (whether the site \
             marks it read-only).",
            OFFERED
                .iter()
                .map(|(name, read_only)| {
                    Record::from([
                        ("id", (*name).to_owned()),
                        ("read_only", read_only.to_string()),
                    ])
                })
                .collect(),
        ),
        task(
            2,
            "Call this page's read-only WebMCP tool `look_up_order` with the order ID the page \
             shows, and return its answer as one Record with the fields `id`, `item` and \
             `total`.",
            vec![Record::from([
                ("id", id),
                ("item", item.to_owned()),
                ("total", total),
            ])],
        ),
    ]
}
