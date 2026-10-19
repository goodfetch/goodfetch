//! The portal's frontier tasks, as the lab's frontier sites served them: a
//! crawler trap, where every link carries a new session ID and tracking
//! parameters, each category has four sort orders of the same products and a
//! calendar pages on without end; and a table of orders that changes while it
//! is paged, so offset pagination loses rows and only its cursor reads every
//! row that lived through the crawl. A trap page is a pure function of the
//! seed and its URL; the table is the family's one changing state.

use std::collections::BTreeSet;

use hyper::Method;

use super::{FAMILY_PLACE, Item, Spec, expected, page, push};
use crate::exchange::{Reply, Request};
use crate::generator;
use crate::task::{Expected, Record};

const PRODUCTS: u64 = 60;
const CATEGORIES: [&str; 3] = ["a", "b", "c"];
/// The months with events, two each; every other month is empty.
const EVENT_MONTHS: [&str; 3] = ["2026-01", "2026-02", "2026-03"];
const EVENTS_PER_MONTH: u64 = 2;
const RELATED: usize = 3;

const ROWS: u64 = 1500;
const ROWS_PER_PAGE: usize = 50;
/// Every how many table requests the table changes.
const CHANGE_EVERY: u64 = 3;
/// How many rows already paged past one change deletes.
const DELETED_PER_CHANGE: u64 = 2;

const TRAP_PRODUCT_PLACE: u64 = 6;
const SESSION_PLACE: u64 = 7;
const ROW_PLACE: u64 = 8;
const VICTIM_PLACE: u64 = 9;

/// The crawler trap's task.
pub(super) fn trap_task(seed: u64) -> Spec {
    let products = (1..=PRODUCTS).map(|number| trap_product(seed, number));
    Spec {
        start: "/shop",
        description: "Crawl the shop from its start page and return one Record per product \
            and per event, with the fields `id` and `price`. Links carry session IDs and \
            tracking parameters, categories offer sort orders of the same products, and \
            the events calendar pages on without end.",
        expected: expected(products.chain(events()), 0),
        signs_in: false,
    }
}

/// The changing table's task.
pub(super) fn table_task(seed: u64) -> Spec {
    Spec {
        start: "/orders?page=1",
        description: "Read every order of the table, newest first, and return one Record \
            per order with the fields `id` and `price`. The table gains and loses rows \
            while it is paged: return every row that is there for the whole crawl, once \
            each.",
        expected: expected(Table::new(seed).rows.into_iter().map(|row| row.item), 0),
        signs_in: false,
    }
}

/// The trap's product numbered `number`, from 1, and its category.
fn trap_product(seed: u64, number: u64) -> Item {
    Item {
        id: format!("item{number:03}"),
        price: 10 + generator::below(seed, &[FAMILY_PLACE, TRAP_PRODUCT_PLACE, number], 90),
    }
}

fn category(number: u64) -> &'static str {
    CATEGORIES[usize::try_from((number - 1) % 3).expect("invariant: an index below 3")]
}

fn events() -> impl Iterator<Item = Item> {
    EVENT_MONTHS.into_iter().flat_map(|month| {
        (1..=EVENTS_PER_MONTH).map(move |k| Item {
            id: format!("ev-{month}-{k}"),
            price: 0,
        })
    })
}

/// The session IDs of one trap page's links: each a pure function of the
/// seed, the page's URL and the link's place, so the same URL gives the same
/// page and every link names a URL never seen before.
struct SessionIds {
    seed: u64,
    url: u64,
    count: u64,
}

impl SessionIds {
    fn new(seed: u64, request: &Request<'_>) -> Self {
        // FNV-1a over the path and query.
        let url = format!("{}?{}", request.path, request.query)
            .bytes()
            .fold(0xcbf2_9ce4_8422_2325_u64, |hash, byte| {
                (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
            });
        Self {
            seed,
            url,
            count: 0,
        }
    }

    fn next(&mut self) -> String {
        self.count += 1;
        let id = generator::draw(
            self.seed,
            &[FAMILY_PLACE, SESSION_PLACE, self.url, self.count],
        );
        format!("sid={id:016x}")
    }
}

/// Answers a request to the crawler trap.
pub(super) fn trap(seed: u64, request: &Request<'_>) -> Reply {
    if request.method != Method::GET {
        return Reply::not_found();
    }
    let mut sid = SessionIds::new(seed, request);
    if request.path == "/shop" {
        let mut nav = String::from("<nav>");
        for name in CATEGORIES {
            push(
                &mut nav,
                format_args!(
                    "<a href=\"/shop/c/{name}?{}\">Category {name}</a><a href=\"/shop/c/{name}?utm_source=nav&amp;{}\">{name}</a>",
                    sid.next(),
                    sid.next()
                ),
            );
        }
        push(
            &mut nav,
            format_args!(
                "<a href=\"/events?month={}&amp;{}\">Events</a></nav>",
                EVENT_MONTHS[0],
                sid.next()
            ),
        );
        return Reply::html(page("Shop", &nav));
    }
    if let Some(name) = request.path.strip_prefix("/shop/c/") {
        return CATEGORIES
            .contains(&name)
            .then(|| category_page(seed, name, request, &mut sid))
            .map_or_else(Reply::not_found, Reply::html);
    }
    if let Some(id) = request.path.strip_prefix("/p/") {
        return (1..=PRODUCTS)
            .find(|number| trap_product(seed, *number).id == id)
            .map_or_else(Reply::not_found, |number| {
                Reply::html(product_page(seed, number, &mut sid))
            });
    }
    if request.path == "/events" {
        let month = request
            .parameter("month")
            .and_then(|month| Month::parse(&month))
            .unwrap_or(Month {
                year: 2026,
                month: 1,
            });
        return Reply::html(events_page(month, &mut sid));
    }
    Reply::not_found()
}

fn category_page(seed: u64, name: &str, request: &Request<'_>, sid: &mut SessionIds) -> String {
    let mut products: Vec<(u64, Item)> = (1..=PRODUCTS)
        .filter(|number| category(*number) == name)
        .map(|number| (number, trap_product(seed, number)))
        .collect();
    match request.parameter("sort").as_deref() {
        Some("price") => products.sort_by_key(|(_, item)| item.price),
        Some("name") => products.sort_by(|a, b| b.1.id.cmp(&a.1.id)),
        _ => {}
    }
    if request.parameter("order").as_deref() == Some("desc") {
        products.reverse();
    }
    let mut body = String::from("<ul>");
    for (_, item) in &products {
        push(
            &mut body,
            format_args!(
                "<li><a href=\"/p/{}?{}&amp;utm_source=cat\">{}</a></li>",
                item.id,
                sid.next(),
                item.id
            ),
        );
    }
    body.push_str("</ul><nav>");
    for sort in ["price", "name"] {
        for order in ["asc", "desc"] {
            push(
                &mut body,
                format_args!(
                    "<a href=\"/shop/c/{name}?sort={sort}&amp;order={order}&amp;{}\">{sort} {order}</a>",
                    sid.next()
                ),
            );
        }
    }
    push(
        &mut body,
        format_args!("</nav><a href=\"/shop?{}\">Shop</a>", sid.next()),
    );
    page(&format!("Category {name}"), &body)
}

fn product_page(seed: u64, number: u64, sid: &mut SessionIds) -> String {
    let item = trap_product(seed, number);
    let mut body = item.article(&item.id, item.price);
    body.push_str("<aside>");
    let related = (1..=PRODUCTS)
        .filter(|other| *other != number && category(*other) == category(number))
        .take(RELATED);
    for other in related {
        let id = trap_product(seed, other).id;
        push(
            &mut body,
            format_args!("<a href=\"/p/{id}?{}\">{id}</a>", sid.next()),
        );
    }
    push(
        &mut body,
        format_args!(
            "</aside><a href=\"/shop/c/{}?{}\">Back</a>",
            category(number),
            sid.next()
        ),
    );
    page(&item.id, &body)
}

/// A calendar month, `YYYY-MM`.
#[derive(Clone, Copy)]
struct Month {
    year: u32,
    month: u32,
}

impl Month {
    fn parse(text: &str) -> Option<Self> {
        let (year, month) = text.split_once('-')?;
        let digits = |part: &str, length| {
            part.len() == length && part.bytes().all(|byte| byte.is_ascii_digit())
        };
        let parsed = Self {
            year: year.parse().ok().filter(|_| digits(year, 4))?,
            month: month.parse().ok().filter(|_| digits(month, 2))?,
        };
        (1..=12).contains(&parsed.month).then_some(parsed)
    }

    /// The month `delta` (one either way) from this one, if it has four
    /// digits.
    fn shifted(self, forward: bool) -> Option<Self> {
        let index = self.year * 12 + self.month - 1;
        let index = if forward {
            index + 1
        } else {
            index.checked_sub(1)?
        };
        let shifted = Self {
            year: index / 12,
            month: index % 12 + 1,
        };
        (shifted.year <= 9999).then_some(shifted)
    }

    fn text(self) -> String {
        format!("{:04}-{:02}", self.year, self.month)
    }
}

fn events_page(month: Month, sid: &mut SessionIds) -> String {
    let text = month.text();
    let mut body = String::from("<main>");
    for event in events().filter(|event| event.id.starts_with(&format!("ev-{text}-"))) {
        body.push_str(&event.article(&event.id, event.price));
    }
    body.push_str("</main>");
    for (forward, label) in [(false, "Previous"), (true, "Next")] {
        if let Some(other) = month.shifted(forward) {
            push(
                &mut body,
                format_args!(
                    "<a href=\"/events?month={}&amp;{}\">{label}</a>",
                    other.text(),
                    sid.next()
                ),
            );
        }
    }
    page(&format!("Events {text}"), &body)
}

/// One row of the orders table.
#[derive(Clone)]
struct Row {
    number: u64,
    item: Item,
}

fn row(seed: u64, number: u64) -> Row {
    Row {
        number,
        item: Item {
            id: format!("o{number:05}"),
            price: 1 + generator::below(seed, &[FAMILY_PLACE, ROW_PLACE, number], 999),
        },
    }
}

/// The orders table, newest first, and what changed in it.
pub(super) struct Table {
    seed: u64,
    rows: Vec<Row>,
    initial: BTreeSet<String>,
    deleted: Vec<Row>,
    next_number: u64,
    requests: u64,
    changes: u64,
}

impl Table {
    pub(super) fn new(seed: u64) -> Self {
        let rows: Vec<Row> = (1..=ROWS).rev().map(|number| row(seed, number)).collect();
        Self {
            seed,
            initial: rows.iter().map(|row| row.item.id.clone()).collect(),
            rows,
            deleted: Vec::new(),
            next_number: ROWS + 1,
            requests: 0,
            changes: 0,
        }
    }

    /// Answers a request for a page of the table, `?page=N` from 1 or
    /// `?after=<id>`, then changes the table every third request: it deletes
    /// two rows already paged past and adds one on top.
    pub(super) fn respond(&mut self, request: &Request<'_>) -> Reply {
        if request.method != Method::GET || request.path != "/orders" {
            return Reply::not_found();
        }
        let after = request.parameter("after");
        let start = match &after {
            Some(after) => {
                let Some(number) = after.strip_prefix('o').and_then(|n| n.parse::<u64>().ok())
                else {
                    return Reply::not_found();
                };
                self.rows.iter().position(|row| row.number < number)
            }
            None => request
                .parameter("page")
                .and_then(|page| page.parse::<usize>().ok())
                .filter(|page| *page >= 1)
                .map(|page| (page - 1) * ROWS_PER_PAGE),
        };
        let slice = start.map_or(&[][..], |start| {
            &self.rows[start.min(self.rows.len())..(start + ROWS_PER_PAGE).min(self.rows.len())]
        });
        let mut head = String::new();
        if let (Some(start), Some(last)) = (start, slice.last())
            && slice.len() == ROWS_PER_PAGE
            && start + ROWS_PER_PAGE < self.rows.len()
        {
            push(
                &mut head,
                format_args!(
                    "<link rel=\"next\" href=\"/orders?after={}\">",
                    last.item.id
                ),
            );
        }
        let mut body = String::from("<main id=\"records\">");
        for row in slice {
            body.push_str(&row.item.article(&row.item.id, row.item.price));
        }
        body.push_str("</main>");
        if after.is_none()
            && let Some(start) = start
            && start + ROWS_PER_PAGE < self.rows.len()
        {
            push(
                &mut body,
                format_args!(
                    "<a class=\"next\" href=\"/orders?page={}\">Next</a>",
                    start / ROWS_PER_PAGE + 2
                ),
            );
        }
        let html = format!(
            "<!doctype html><html><head><meta charset=\"utf-8\"><title>Orders</title>{head}</head><body>{body}</body></html>"
        );
        self.change(start.unwrap_or(0));
        Reply::html(html)
    }

    fn change(&mut self, offset: usize) {
        self.requests += 1;
        if !self.requests.is_multiple_of(CHANGE_EVERY) {
            return;
        }
        self.changes += 1;
        for k in 0..DELETED_PER_CHANGE {
            let bound = offset.min(self.rows.len());
            if bound == 0 {
                break;
            }
            let place = [FAMILY_PLACE, VICTIM_PLACE, self.changes, k];
            let bound = u64::try_from(bound).expect("invariant: a table length fits in 64 bits");
            let victim = generator::below(self.seed, &place, bound);
            let victim =
                usize::try_from(victim).expect("invariant: an index below the table's length");
            self.deleted.push(self.rows.remove(victim));
        }
        self.rows.insert(0, row(self.seed, self.next_number));
        self.next_number += 1;
    }

    /// The expected result now: every initial row not deleted, and the rows
    /// added or deleted as tolerated.
    pub(super) fn expected(&self) -> Expected {
        let records: Vec<Record> = self
            .rows
            .iter()
            .filter(|row| self.initial.contains(&row.item.id))
            .map(|row| row.item.record())
            .collect();
        let tolerated = self
            .rows
            .iter()
            .filter(|row| !self.initial.contains(&row.item.id))
            .chain(&self.deleted)
            .map(|row| row.item.record())
            .collect();
        Expected {
            records,
            tolerated,
            ..Expected::default()
        }
    }
}
