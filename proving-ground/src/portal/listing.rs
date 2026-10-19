//! The portal's routing tasks, as the lab's routing sites served them: one
//! listing of a hundred products behind six kinds of rendering, so a tool
//! must choose plain HTTP, a browser or the API a page's script calls. Server
//! HTML; a script shell that fetches every product; partial hydration, eight
//! products in the HTML and the rest fetched; stale HTML whose prices a
//! script corrects; GraphQL behind a "Load more" button; and an infinite
//! feed. The APIs answer what the scripts read, prices as numbers.

use hyper::{Method, StatusCode};
use serde_json::{Value, json};

use super::{FAMILY_PLACE, Item, Spec, expected, page, push};
use crate::exchange::{Reply, Request};
use crate::generator;

const PAGES: u64 = 5;
const PER_PAGE: u64 = 20;
const PRODUCTS: u64 = PAGES * PER_PAGE;
/// How many products a partly hydrated page holds in its HTML.
const HYDRATED: u64 = 8;
/// How much a stale page's prices exceed the current ones.
const STALE_BY: u64 = 7;

const PRODUCT_PLACE: u64 = 5;

/// A routing task's rendering of the listing.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Rendering {
    /// Every product in the HTML, five pages linked by Next.
    Static,
    /// No product in the HTML: a script fetches each page's from the API.
    JsShell,
    /// Eight products in the HTML, the rest fetched by a script.
    PartialHydration,
    /// Every product in the HTML with an old price, which a script corrects.
    StaleHtml,
    /// No product in the HTML: GraphQL pages behind a "Load more" button.
    Graphql,
    /// The first twenty in the HTML, the rest fetched as the end of the feed
    /// scrolls into view.
    InfiniteScroll,
}

/// The listing's product numbered `number`, from 1.
fn product(seed: u64, number: u64) -> Item {
    Item {
        id: format!("p{number:03}"),
        price: 50 + generator::below(seed, &[FAMILY_PLACE, PRODUCT_PLACE, number], 450),
    }
}

fn name(number: u64) -> String {
    format!("Product {number}")
}

/// The routing task of `rendering`.
pub(super) fn task(seed: u64, rendering: Rendering) -> Spec {
    let (start, description) = match rendering {
        Rendering::Static => (
            "/list?page=1",
            "Crawl the listing from its first page through each Next link and return one \
             Record per product with the fields `id` and `price`.",
        ),
        Rendering::JsShell => (
            "/list?page=1",
            "Crawl the listing, whose pages a script renders, and return one Record per \
             product with the fields `id` and `price`.",
        ),
        Rendering::PartialHydration => (
            "/list?page=1",
            "Crawl the listing from its first page through each Next link; a script adds \
             most of each page's products. Return one Record per product with the fields \
             `id` and `price`.",
        ),
        Rendering::StaleHtml => (
            "/list?page=1",
            "Crawl the listing from its first page through each Next link; a script \
             corrects the prices the HTML shows. Return one Record per product with the \
             fields `id` and `price`, each at its current price.",
        ),
        Rendering::Graphql => (
            "/catalog",
            "Read the catalog, whose products load twenty at a time behind its \"Load \
             more\" button, and return one Record per product with the fields `id` and \
             `price`.",
        ),
        Rendering::InfiniteScroll => (
            "/feed",
            "Read the feed, which loads more products as its end scrolls into view, and \
             return one Record per product with the fields `id` and `price`.",
        ),
    };
    Spec {
        start,
        description,
        expected: expected((1..=PRODUCTS).map(|number| product(seed, number)), 0),
        signs_in: false,
    }
}

/// Answers a request to a routing task.
pub(super) fn respond(seed: u64, rendering: Rendering, request: &Request<'_>) -> Reply {
    let number = request
        .parameter("page")
        .and_then(|page| page.parse::<u64>().ok())
        .map_or(1, |page| page.clamp(1, PAGES));
    match (request.method, request.path, rendering) {
        (&Method::GET, "/list", Rendering::Static) => Reply::html(page(
            &format!("List {number}"),
            &format!(
                "<main id=\"records\" data-total=\"{PRODUCTS}\">{}</main>{}",
                articles(seed, number, PER_PAGE, false),
                next_link(number)
            ),
        )),
        (&Method::GET, "/list", Rendering::JsShell) => Reply::html(shell(
            &format!("List {number}"),
            &format!(
                "fetch('/api/list?page={number}').then(r=>r.json()).then(d=>{{render(d.items);if(d.next){{const a=document.createElement('a');a.className='next';a.href='/list?page='+d.next;a.textContent='Next';document.body.append(a)}}}})"
            ),
            "",
            "",
        )),
        (&Method::GET, "/list", Rendering::PartialHydration) => Reply::html(shell(
            &format!("List {number}"),
            &format!(
                "fetch('/api/list?page={number}&offset={HYDRATED}').then(r=>r.json()).then(d=>render(d.items))"
            ),
            &articles(seed, number, HYDRATED, false),
            &next_link(number),
        )),
        (&Method::GET, "/list", Rendering::StaleHtml) => Reply::html(shell(
            &format!("List {number}"),
            &format!(
                "fetch('/api/prices?page={number}').then(r=>r.json()).then(d=>{{for(const p of d.prices){{const a=main.querySelector('[data-id=\"'+p.id+'\"]');if(a)a.dataset.price=p.price}}}})"
            ),
            &articles(seed, number, PER_PAGE, true),
            &next_link(number),
        )),
        (&Method::GET, "/api/list", _) => {
            let offset = request
                .parameter("offset")
                .and_then(|offset| offset.parse::<u64>().ok())
                .unwrap_or(0);
            Reply::json(
                StatusCode::OK,
                &json!({
                    "items": items(seed, (number - 1) * PER_PAGE + 1 + offset..=number * PER_PAGE),
                    "next": (number < PAGES).then_some(number + 1),
                }),
            )
        }
        (&Method::GET, "/api/prices", _) => {
            let prices: Vec<Value> = ((number - 1) * PER_PAGE + 1..=number * PER_PAGE)
                .map(|n| {
                    let item = product(seed, n);
                    json!({ "id": item.id, "price": item.price })
                })
                .collect();
            Reply::json(StatusCode::OK, &json!({ "prices": prices }))
        }
        (&Method::GET, "/catalog", Rendering::Graphql) => Reply::html(shell(
            "Catalog",
            "const more=document.querySelector('button.more');let after=null;const load=()=>fetch('/graphql',{method:'POST',headers:{'content-type':'application/json'},body:JSON.stringify({query:'query Products($after:String){products(first:20,after:$after){nodes{id name price} pageInfo{endCursor hasNextPage}}}',variables:{after}})}).then(r=>r.json()).then(d=>{const p=d.data.products;render(p.nodes);after=p.pageInfo.endCursor;more.hidden=!p.pageInfo.hasNextPage});more.addEventListener('click',load);load()",
            "",
            "<button class=\"more\" hidden>Load more</button>",
        )),
        (&Method::POST, "/graphql", Rendering::Graphql) => graphql(seed, request.body),
        (&Method::GET, "/feed", Rendering::InfiniteScroll) => Reply::html(
            shell(
                "Feed",
                "let cursor=main.dataset.cursor,busy=false;const more=()=>{if(busy||!cursor||innerHeight+scrollY<document.body.scrollHeight-200)return;busy=true;fetch('/api/feed?cursor='+cursor).then(r=>r.json()).then(d=>{render(d.items);cursor=d.nextCursor;main.dataset.cursor=cursor??'';busy=false;more()})};addEventListener('scroll',more)",
                &articles(seed, 1, PER_PAGE, false),
                "",
            )
            .replace(
                &format!("data-total=\"{PRODUCTS}\""),
                &format!("data-total=\"{PRODUCTS}\" data-cursor=\"{}\"", cursor(PER_PAGE)),
            ),
        ),
        (&Method::GET, "/api/feed", Rendering::InfiniteScroll) => {
            let Some(start) = request.parameter("cursor").as_deref().and_then(index) else {
                return Reply::error(StatusCode::BAD_REQUEST, "malformed_cursor");
            };
            let end = (start + PER_PAGE).min(PRODUCTS);
            Reply::json(
                StatusCode::OK,
                &json!({
                    "items": items(seed, start + 1..=end),
                    "nextCursor": (end < PRODUCTS).then(|| cursor(end)),
                }),
            )
        }
        _ => Reply::not_found(),
    }
}

/// The GraphQL answer to `products(first: 20, after: $after)`.
fn graphql(seed: u64, body: &[u8]) -> Reply {
    let Ok(query) = serde_json::from_slice::<Value>(body) else {
        return Reply::error(StatusCode::BAD_REQUEST, "malformed_query");
    };
    let start = match &query["variables"]["after"] {
        Value::Null => Some(0),
        Value::String(after) => index(after),
        _ => None,
    };
    let Some(start) = start else {
        return Reply::error(StatusCode::BAD_REQUEST, "malformed_cursor");
    };
    let end = (start + PER_PAGE).min(PRODUCTS);
    Reply::json(
        StatusCode::OK,
        &json!({ "data": { "products": {
            "nodes": items(seed, start + 1..=end),
            "pageInfo": { "endCursor": cursor(end), "hasNextPage": end < PRODUCTS },
        } } }),
    )
}

/// The API's items numbered in `numbers`, prices as numbers.
fn items(seed: u64, numbers: std::ops::RangeInclusive<u64>) -> Vec<Value> {
    numbers
        .filter(|number| *number <= PRODUCTS)
        .map(|number| {
            let item = product(seed, number);
            json!({ "id": item.id, "name": name(number), "price": item.price })
        })
        .collect()
}

/// The first `shown` articles of page `number`, with stale prices if
/// `stale`.
fn articles(seed: u64, number: u64, shown: u64, stale: bool) -> String {
    let mut html = String::new();
    for n in (number - 1) * PER_PAGE + 1..=(number - 1) * PER_PAGE + shown {
        let item = product(seed, n);
        let price = if stale {
            item.price + STALE_BY
        } else {
            item.price
        };
        html.push_str(&item.article(&name(n), price));
    }
    html
}

fn next_link(number: u64) -> String {
    let mut link = String::new();
    if number < PAGES {
        push(
            &mut link,
            format_args!(
                "<a class=\"next\" href=\"/list?page={}\">Next</a>",
                number + 1
            ),
        );
    }
    link
}

/// A page whose script renders the API's items into `#records`.
fn shell(title: &str, script: &str, inner: &str, after: &str) -> String {
    page(
        title,
        &format!(
            "<main id=\"records\" data-total=\"{PRODUCTS}\">{inner}</main>{after}<script>const main=document.querySelector('#records');const render=items=>{{for(const r of items){{const a=document.createElement('article');a.dataset.id=r.id;a.dataset.price=r.price;a.innerHTML='<h2>'+r.name+'</h2>';main.append(a)}}}};{script}</script>"
        ),
    )
}

/// The opaque cursor after the first `index` products: `after:<index>` in
/// lowercase hex.
fn cursor(index: u64) -> String {
    format!("after:{index}")
        .bytes()
        .fold(String::new(), |mut hex, byte| {
            push(&mut hex, format_args!("{byte:02x}"));
            hex
        })
}

/// The index a cursor names, if it is one.
fn index(cursor: &str) -> Option<u64> {
    let bytes = (0..cursor.len())
        .step_by(2)
        .map(|at| u8::from_str_radix(cursor.get(at..at + 2)?, 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let index = String::from_utf8(bytes)
        .ok()?
        .strip_prefix("after:")?
        .parse()
        .ok()?;
    (index <= PRODUCTS).then_some(index)
}
