//! The `rendered` family on `rendered.test` (ADR 0025's AJAX loading,
//! infinite scroll, shadow DOM and iframes): a feed whose items its script
//! loads by AJAX, a page at a time as it is scrolled to its end, and item
//! pages whose name a script puts in a shadow root, whose price it loads by
//! AJAX and whose description sits in a frame. None of a Record's data is in
//! a page's own HTML. Every value comes from the seed, and the expected
//! results from the same seed data, never from a tool's output. An app
//! shell, `/shell`, whose content its inline script writes as the page
//! parses, is shown complete by the time it loads, its own HTML holding an
//! empty `#app` (a JavaScript page fetched over HTTP, ADR 0093).

use hyper::{Method, StatusCode};
use serde_json::json;

use crate::exchange::{Reply, Request};
use crate::generator;
use crate::task::{Entry, Expected, Kind, Record, Task, origin};

/// The host name the family is served on.
pub(crate) const HOST: &str = "rendered.test";

const FAMILY: &str = "rendered";

const FEED_PAGES: u64 = 4;
const ITEMS_PER_PAGE: u64 = 6;
const ITEMS: u64 = FEED_PAGES * ITEMS_PER_PAGE;

/// The generator places of the family's values: the family's own first, then
/// the item's number (0 for the family's own values) and the value's.
const FAMILY_PLACE: u64 = 2;
const ADJECTIVE_PLACE: u64 = 1;
const NOUN_PLACE: u64 = 2;
const PRICE_PLACE: u64 = 3;
const MATERIAL_PLACE: u64 = 4;
const FINISH_PLACE: u64 = 5;
/// Which item the extract task's page shows.
const EXTRACT_PLACE: u64 = 6;

const ADJECTIVES: [&str; 16] = [
    "Arctic", "Bold", "Coral", "Dapper", "Electric", "Fern", "Golden", "Humble", "Indigo", "Jolly",
    "Kindred", "Lofty", "Misty", "Noble", "Opal", "Quiet",
];
const NOUNS: [&str; 16] = [
    "Armchair",
    "Bench",
    "Clock",
    "Drawer",
    "Easel",
    "Footstool",
    "Grater",
    "Hourglass",
    "Ladle",
    "Mug",
    "Napkin",
    "Organizer",
    "Planter",
    "Quilt",
    "Rug",
    "Shelf",
];
const MATERIALS: [&str; 8] = [
    "oak", "walnut", "steel", "brass", "linen", "wool", "ceramic", "glass",
];
const FINISHES: [&str; 8] = [
    "matte",
    "glossy",
    "brushed",
    "oiled",
    "waxed",
    "lacquered",
    "raw",
    "polished",
];

/// Each item page's script, which defines the `item-card` element: it
/// fetches its item from its `data-source`, puts the name in its own shadow
/// root and the price in the page.
const ITEM_SCRIPT: &str = r#"customElements.define("item-card", class extends HTMLElement {
  async connectedCallback() {
    const root = this.attachShadow({ mode: "open" });
    const item = await (await fetch(this.dataset.source)).json();
    const name = document.createElement("h1");
    name.className = "name";
    name.textContent = item.name;
    root.append(name);
    const price = document.querySelector(".price");
    price.dataset.price = item.price;
    price.textContent = "$" + item.price;
  }
});
"#;

/// The feed's script: it loads the feed's first page, then the next one each
/// time the end of the list comes into view, until the feed has no next page.
const FEED_SCRIPT: &str = r#"const list = document.querySelector(".items");
const end = document.querySelector(".end");
let next = list.dataset.source;
let loading = false;
async function load() {
  if (next === null || loading) return;
  loading = true;
  const page = await (await fetch(next)).json();
  for (const item of page.items) {
    const entry = document.createElement("li");
    entry.className = "item";
    entry.dataset.id = item.id;
    const link = document.createElement("a");
    link.href = item.url;
    link.textContent = item.name;
    entry.append(link);
    list.append(entry);
  }
  next = page.next;
  loading = false;
  if (next === null) {
    observer.disconnect();
    end.textContent = "End of the feed";
  } else if (end.getBoundingClientRect().top < window.innerHeight) {
    load();
  }
}
const observer = new IntersectionObserver((entries) => {
  if (entries.some((entry) => entry.isIntersecting)) load();
});
observer.observe(end);
"#;

/// One item, as its pages show it once their scripts have run.
struct Item {
    id: String,
    name: String,
    /// In dollars, with two decimals.
    price: String,
    description: String,
}

/// The item numbered `number` (from 1) for `seed`.
fn item(seed: u64, number: u64) -> Item {
    let draw = |value, bound: usize| {
        let bound = u64::try_from(bound).expect("invariant: a word list is short");
        usize::try_from(generator::below(
            seed,
            &[FAMILY_PLACE, number, value],
            bound,
        ))
        .expect("invariant: a word index is below 16")
    };
    let cents = 100 + generator::below(seed, &[FAMILY_PLACE, number, PRICE_PLACE], 99_900);
    Item {
        id: item_id(number),
        name: format!(
            "{} {}",
            ADJECTIVES[draw(ADJECTIVE_PLACE, ADJECTIVES.len())],
            NOUNS[draw(NOUN_PLACE, NOUNS.len())]
        ),
        price: format!("{}.{:02}", cents / 100, cents % 100),
        description: format!(
            "Made of {}, with a {} finish.",
            MATERIALS[draw(MATERIAL_PLACE, MATERIALS.len())],
            FINISHES[draw(FINISH_PLACE, FINISHES.len())]
        ),
    }
}

fn item_id(number: u64) -> String {
    format!("I-{number:04}")
}

/// The item number an ID such as `I-0001` names, if the family has it.
fn item_number(id: &str) -> Option<u64> {
    let digits = id.strip_prefix("I-")?;
    let number = digits.parse::<u64>().ok()?;
    (digits.len() == 4 && digits.bytes().all(|byte| byte.is_ascii_digit()))
        .then_some(number)
        .filter(|number| (1..=ITEMS).contains(number))
}

/// A number written without a sign or leading zeros, so each page has one URL.
fn canonical_number(text: &str) -> Option<u64> {
    let number = text.parse::<u64>().ok()?;
    (number.to_string() == text).then_some(number)
}

/// Answers a request on the family's host name.
pub(crate) fn respond(seed: u64, request: &Request<'_>) -> Reply {
    if request.method != Method::GET {
        return Reply::not_found();
    }
    let path = request.path;
    if path == "/feed" {
        return Reply::html(feed_page());
    }
    if path == "/shell" {
        return Reply::html(SHELL_PAGE.to_owned());
    }
    if let Some(page) = path.strip_prefix("/api/feed/") {
        return match canonical_number(page).filter(|page| (1..=FEED_PAGES).contains(page)) {
            Some(page) => feed_data(seed, page),
            None => Reply::not_found(),
        };
    }
    if let Some(id) = path.strip_prefix("/api/items/") {
        return match item_number(id) {
            Some(number) => {
                let Item {
                    id, name, price, ..
                } = item(seed, number);
                Reply::json(
                    StatusCode::OK,
                    &json!({ "id": id, "name": name, "price": price }),
                )
            }
            None => Reply::not_found(),
        };
    }
    if let Some(rest) = path.strip_prefix("/items/") {
        let (id, details) = match rest.strip_suffix("/details") {
            Some(id) => (id, true),
            None => (rest, false),
        };
        return match (item_number(id), details) {
            (Some(number), false) => Reply::html(item_page(&item_id(number))),
            (Some(number), true) => Reply::html(details_page(&item(seed, number).description)),
            (None, _) => Reply::not_found(),
        };
    }
    Reply::not_found()
}

/// The app shell: its inline script writes its heading into the empty
/// `#app` before the page loads, so a browser shows it at `load` and the
/// HTML alone never does.
const SHELL_PAGE: &str = "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Shell</title></head>\n<body>\n<main id=\"app\"></main>\n<script>\ndocument.getElementById(\"app\").innerHTML = \"<h1 class=\\\"title\\\">Rendered shell</h1><p>Written by its script.</p>\";\n</script>\n</body>\n</html>\n";

fn feed_page() -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Feed</title>\n<style>.item {{ min-height: 200px; }}</style></head>\n<body>\n<main>\n<h1>Feed</h1>\n<ul class=\"items\" data-source=\"/api/feed/1\"></ul>\n<p class=\"end\">Loading more items</p>\n</main>\n<script>\n{FEED_SCRIPT}</script>\n</body>\n</html>\n"
    )
}

/// One page of the feed's items, with the path of the next page or `null`.
fn feed_data(seed: u64, page: u64) -> Reply {
    let items: Vec<_> = ((page - 1) * ITEMS_PER_PAGE + 1..=page * ITEMS_PER_PAGE)
        .map(|number| {
            let Item { id, name, .. } = item(seed, number);
            json!({ "id": id, "name": name, "url": format!("/items/{id}") })
        })
        .collect();
    let next = (page < FEED_PAGES).then(|| format!("/api/feed/{}", page + 1));
    Reply::json(StatusCode::OK, &json!({ "items": items, "next": next }))
}

fn item_page(id: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Item {id}</title></head>\n<body>\n<main>\n<item-card data-id=\"{id}\" data-source=\"/api/items/{id}\"></item-card>\n<p>Price: <span class=\"price\"></span></p>\n<iframe class=\"details\" src=\"/items/{id}/details\" title=\"Details\"></iframe>\n<a href=\"/feed\">Back to the feed</a>\n</main>\n<script>\n{ITEM_SCRIPT}</script>\n</body>\n</html>\n"
    )
}

fn details_page(description: &str) -> String {
    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head><meta charset=\"utf-8\"><title>Details</title></head>\n<body>\n<p class=\"description\">{description}</p>\n</body>\n</html>\n"
    )
}

/// The host name of the family's task `ordinal`, on which the task's pages
/// are served: `1.rendered.test` for `rendered/1`.
fn task_host(ordinal: u32) -> String {
    format!("{ordinal}.{HOST}")
}

/// The family's tasks for `seed`, their start URLs on their own host names
/// and `port`. Ordinals are append-only (ADR 0063).
pub(crate) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let origin = |ordinal| origin(&task_host(ordinal), port);
    let extracted = 1 + generator::below(seed, &[FAMILY_PLACE, 0, EXTRACT_PLACE], ITEMS);
    vec![
        Task {
            host: task_host(1),
            entry: Entry {
                id: format!("{FAMILY}/1"),
                family: FAMILY,
                kind: Kind::Crawl,
                start_url: format!("{}/feed", origin(1)),
                description: "Crawl the feed, scrolling it to its end as its script loads more \
                    items, and return one Record per item, read from its item page once the \
                    page's scripts have run, with the fields `id`, `name` (in the page's shadow \
                    root), `price` and `description` (in the page's frame).",
                account: None,
                time_steps: None,
            },
            expected: Expected::records((1..=ITEMS).map(|number| record(seed, number)).collect()),
        },
        Task {
            host: task_host(2),
            entry: Entry {
                id: format!("{FAMILY}/2"),
                family: FAMILY,
                kind: Kind::Extract,
                start_url: format!("{}/items/{}", origin(2), item_id(extracted)),
                description: "Extract the item this page shows once its scripts have run, as one \
                    Record with the fields `id`, `name` (in the page's shadow root), `price` and \
                    `description` (in the page's frame).",
                account: None,
                time_steps: None,
            },
            expected: Expected::records(vec![record(seed, extracted)]),
        },
    ]
}

/// The expected Record of an item: its fields as its pages show them, the
/// price without its currency sign.
fn record(seed: u64, number: u64) -> Record {
    let Item {
        id,
        name,
        price,
        description,
    } = item(seed, number);
    Record::from([
        ("id", id),
        ("name", name),
        ("price", price),
        ("description", description),
    ])
}
