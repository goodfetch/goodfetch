//! A crawl's Frontier on Foxhound's executable over stdio (spec #581's
//! seam: the protocol against the Proving ground; ADR 0094): each seed is
//! answered as an enqueue is, added with its ID or not added with its
//! reason, `run.seeds` lists the answers, so a crawl of nothing says why,
//! and a handler's `crawl_request.enqueue` resolves a relative URL against
//! its document's base URL, after a `<base href>` and after a redirect, on
//! the Proving ground's `links` family. A request's identity is its method,
//! its URL's key and its body's hash, or its declared `key`: tracking
//! parameters, parameter order, `;jsessionid=` and the fragment never split
//! a page, which is fetched at its own URL. A priority and `forefront`
//! order a domain's requests and never the domains' turns.

#![expect(
    clippy::disallowed_methods,
    reason = "a test's own statements, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the test's handler fetches the Proving ground's pages from outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{Read as _, Write as _};

use goodfetch_test_client::Attached;
use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::site::Site;

/// The Proving ground's seed.
const SEED: u64 = 828;

/// A page of the Proving ground as a handler reads it.
pub(crate) struct Fetched {
    /// The URL it was reached at, after its redirects.
    pub(crate) url: String,
    pub(crate) status: u16,
    pub(crate) body: String,
}

/// Fetches `url`, a page of the Proving ground, following its redirects,
/// as the crawl's handler does: its fragment never sent.
pub(crate) fn fetch(proving_ground: &ProvingGround, url: &str) -> Fetched {
    let authority_and_path = url.strip_prefix("http://").unwrap();
    let (authority, path) = authority_and_path.split_at(authority_and_path.find('/').unwrap());
    let path = path.split('#').next().unwrap();
    let mut stream = std::net::TcpStream::connect(proving_ground.address()).unwrap();
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {authority}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    let (head, body) = response.split_once("\r\n\r\n").unwrap();
    let status: u16 = head.split(' ').nth(1).unwrap().parse().unwrap();
    let location = head.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        name.eq_ignore_ascii_case("location")
            .then(|| value.trim().to_owned())
    });
    match location {
        Some(path) if (300..400).contains(&status) => {
            fetch(proving_ground, &format!("http://{authority}{path}"))
        }
        _ => Fetched {
            url: url.to_owned(),
            status,
            body: body.to_owned(),
        },
    }
}

/// The value of each `name="…"` attribute of `html`, in order.
pub(crate) fn attributes(html: &str, name: &str) -> Vec<String> {
    let opening = format!("{name}=\"");
    html.match_indices(&opening)
        .map(|(at, _)| {
            let value = &html[at + opening.len()..];
            value[..value.find('"').unwrap()].to_owned()
        })
        .collect()
}

/// Crawls `seeds` with one request in flight at a time, answering each
/// `crawl_request.handle` with `handler`, which may call the engine
/// itself through the `Inside` it gets; answers `run.finish`'s reply.
pub(crate) fn crawl(
    engine: &mut Attached,
    run_id: &str,
    seeds: &[Value],
    handler: &mut impl FnMut(&mut Inside<'_>, &Value) -> Value,
) -> Value {
    crawl_under(engine, run_id, seeds, &json!({}), handler)
}

/// [`crawl`] under the Run's `crawl` settings `settings` beside the
/// handler's concurrency of 1.
pub(crate) fn crawl_under(
    engine: &mut Attached,
    run_id: &str,
    seeds: &[Value],
    settings: &Value,
    handler: &mut impl FnMut(&mut Inside<'_>, &Value) -> Value,
) -> Value {
    crawl_with(
        engine,
        run_id,
        seeds,
        &json!({ "crawl": settings }),
        handler,
    )
}

/// [`crawl`] under the Run's settings `settings`, every key and not only
/// `crawl`'s, beside the handler's concurrency of 1. The crawl declares
/// the Labels its seeds name, each seed's and each sitemap rule's, with no
/// settings: a test whose handler enqueues another Label declares it with
/// [`crawl_declaring`].
pub(crate) fn crawl_with(
    engine: &mut Attached,
    run_id: &str,
    seeds: &[Value],
    settings: &Value,
    handler: &mut impl FnMut(&mut Inside<'_>, &Value) -> Value,
) -> Value {
    crawl_declaring(
        engine,
        run_id,
        seeds,
        &seed_labels(seeds),
        settings,
        handler,
    )
}

/// The Labels `seeds` name, each seed's and each sitemap rule's, each
/// declared with no settings, as `crawl.start`'s `labels` takes them.
pub(crate) fn seed_labels(seeds: &[Value]) -> Value {
    let rules = seeds
        .iter()
        .filter_map(|seed| seed["rules"].as_array())
        .flatten();
    seeds
        .iter()
        .chain(rules)
        .filter_map(|named| named["label"].as_str())
        .map(|label| (label.to_owned(), json!({})))
        .collect::<serde_json::Map<_, _>>()
        .into()
}

/// [`crawl_with`] for a crawl that declares the Labels `labels`, each with
/// its settings, as `crawl.start` takes them.
pub(crate) fn crawl_declaring(
    engine: &mut Attached,
    run_id: &str,
    seeds: &[Value],
    labels: &Value,
    settings: &Value,
    handler: &mut impl FnMut(&mut Inside<'_>, &Value) -> Value,
) -> Value {
    crawl_from(
        engine,
        &json!({ "run_id": run_id, "seeds": seeds, "labels": labels, "settings": settings }),
        handler,
    )
}

/// [`crawl`] from the `crawl.start` params `params`, its settings beside
/// the handler's concurrency of 1. A crawl that declares Hooks in code has
/// `handler` answer each `crawl_hook.handle` too, whose params name the
/// `hook` and its `stage`.
pub(crate) fn crawl_from(
    engine: &mut Attached,
    params: &Value,
    handler: &mut impl FnMut(&mut Inside<'_>, &Value) -> Value,
) -> Value {
    let mut params = params.clone();
    params["settings"]["crawl"]["handler"] = json!({ "concurrency": 1 });
    let run_id = params["run_id"].clone();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start", "params": params,
    }));
    let mut calls = 0;
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the Run ended");
        if message["method"] == "crawl_request.handle" || message["method"] == "crawl_hook.handle" {
            let mut inside = Inside {
                engine,
                calls: &mut calls,
            };
            let mut reply = handler(&mut inside, &message["params"]);
            reply["jsonrpc"] = json!("2.0");
            reply["id"] = message["id"].clone();
            engine.send(&reply);
        } else if message["id"] == "start" {
            assert_eq!(message["result"]["run_id"], run_id, "{message}");
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "finish", "method": "run.finish", "params": { "run_id": run_id },
            }));
        } else if message["id"] == "finish" {
            return message;
        }
    }
}

/// The engine as a handler reaches it while it handles one request.
pub(crate) struct Inside<'a> {
    engine: &'a mut Attached,
    calls: &'a mut u32,
}

impl Inside<'_> {
    /// The engine itself, for a handler that sends calls of its own while
    /// another is in flight.
    pub(crate) fn engine(&mut self) -> &mut Attached {
        self.engine
    }

    /// Calls `method` and answers its reply; with one request in flight,
    /// nothing else comes meanwhile.
    pub(crate) fn call(&mut self, method: &str, params: &Value) -> Value {
        *self.calls += 1;
        let id = format!("inside-{}", self.calls);
        self.engine
            .send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let reply = self.engine.next().unwrap();
        assert_eq!(reply["id"], id, "{reply}");
        reply
    }

    /// `ctx.enqueue(url, label)` from the attempt `handle` gave, read on
    /// `document` when given; answers its `Enqueued` or its error.
    pub(crate) fn enqueue(
        &mut self,
        handle: &Value,
        url: &str,
        label: &str,
        document: Option<Value>,
    ) -> Value {
        let mut params = enqueue_params(handle, url, label);
        if let Some(document) = document {
            params["document"] = document;
        }
        let reply = self.call("crawl_request.enqueue", &params);
        reply
            .get("result")
            .unwrap_or_else(|| &reply["error"]["data"]["code"])
            .clone()
    }
}

/// The params of `crawl_request.enqueue` for `url` under `label`, from the
/// attempt `handle` gave.
pub(crate) fn enqueue_params(handle: &Value, url: &str, label: &str) -> Value {
    json!({
        "run_id": handle["run_id"],
        "crawl_request_id": handle["crawl_request"]["id"],
        "lease_epoch": handle["lease_epoch"],
        "url": url,
        "label": label,
    })
}

pub(crate) fn added(id: &str, url: &str) -> Value {
    json!({ "kind": "added", "id": id, "url": url })
}

pub(crate) fn not_added(reason: &str, url: &str) -> Value {
    json!({ "kind": "not_added", "reason": reason, "url": url })
}

/// Only `http` and `https` URLs enter the Frontier: a seed or an enqueue
/// of `ftp:`, `file:`, `gopher:` or `data:` is `unsupported_scheme`, and one
/// that does not parse `invalid_url` (ADRs 0118 and 0120, SC-03).
///
/// Covers: story:6-201, story:6-202, story:6-211, story:4-100, operation:run.seeds, operation:crawl.start, operation:crawl_request.enqueue
#[test]
fn each_seed_is_answered_as_an_enqueue_is_and_run_seeds_lists_the_answers() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let shelf = proving_ground.url("1.links.test", "/guide/");
    let unfetchable = shelf.replacen("http:", "ftp:", 1);
    let seeds: Vec<Value> = [
        shelf.as_str(),
        &shelf.replacen("1.links.test", "1.LINKS.test", 1),
        "/guide/",
        &unfetchable,
        "http://",
        "file:///etc/passwd",
        "gopher://1.links.test/1",
        "data:text/html,hi",
    ]
    .into_iter()
    .map(|url| json!({ "url": url, "label": "shelf" }))
    .collect();
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let mut enqueued = Vec::new();
    let report = crawl(&mut engine, "seeds", &seeds, &mut |inside, handle| {
        let page = fetch(
            &proving_ground,
            handle["crawl_request"]["url"].as_str().unwrap(),
        );
        for seed in &seeds {
            enqueued.push(inside.enqueue(handle, seed["url"].as_str().unwrap(), "shelf", None));
        }
        assert_eq!(page.status, 200);
        json!({ "result": { "records": [] } })
    });
    let answers = engine.call(
        "seeds",
        "run.seeds",
        &json!({ "run_id": "seeds" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &answers["result"]["seeds"],
            Value::Array(enqueued)
        ),
        (
            &json!("finished"),
            &json!([
                added("1", &shelf),
                not_added(
                    "duplicate",
                    &shelf.replacen("1.links.test", "1.LINKS.test", 1)
                ),
                not_added("invalid_url", "/guide/"),
                not_added("unsupported_scheme", &unfetchable),
                not_added("invalid_url", "http://"),
                not_added("unsupported_scheme", "file:///etc/passwd"),
                not_added("unsupported_scheme", "gopher://1.links.test/1"),
                not_added("unsupported_scheme", "data:text/html,hi"),
            ]),
            json!([
                not_added("duplicate", &shelf),
                not_added(
                    "duplicate",
                    &shelf.replacen("1.links.test", "1.LINKS.test", 1)
                ),
                not_added("duplicate", "/guide/"),
                not_added("unsupported_scheme", &unfetchable),
                not_added("invalid_url", "http://"),
                not_added("unsupported_scheme", "file:///etc/passwd"),
                not_added("unsupported_scheme", "gopher://1.links.test/1"),
                not_added("unsupported_scheme", "data:text/html,hi"),
            ]),
        )
    );
}

/// Covers: story:6-201, story:6-202, operation:run.seeds, operation:crawl.start
#[test]
fn a_crawl_whose_every_seed_is_refused_finishes_at_once_and_says_why() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds: Vec<Value> = [
        "/catalog/1",
        "statuses.test/articles",
        "mailto:books@statuses.test",
        "http://[::1/",
    ]
    .into_iter()
    .map(|url| json!({ "url": url, "label": "page" }))
    .collect();
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let mut handled = 0;
    let report = crawl(&mut engine, "nothing", &seeds, &mut |_, _| {
        handled += 1;
        json!({ "result": { "records": [] } })
    });
    let answers = engine.call(
        "seeds",
        "run.seeds",
        &json!({ "run_id": "nothing" }),
        &mut |_| None,
    );
    let requested = proving_ground.received_requests().unwrap().every().count();
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            handled,
            requested,
            &answers["result"]["seeds"]
        ),
        (
            &json!("finished"),
            0,
            0,
            &json!([
                not_added("invalid_url", "/catalog/1"),
                not_added("invalid_url", "statuses.test/articles"),
                not_added("unsupported_scheme", "mailto:books@statuses.test"),
                not_added("invalid_url", "http://[::1/"),
            ])
        )
    );
}

/// Covers: story:6-231, story:6-211, operation:crawl_request.enqueue
#[test]
fn an_enqueue_resolves_against_its_documents_base_href_and_the_url_it_was_redirected_to() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [
        json!({ "url": proving_ground.url("1.links.test", "/guide/"), "label": "shelf" }),
        json!({ "url": proving_ground.url("2.links.test", "/old/shelf"), "label": "shelf" }),
    ];
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let mut enqueued = Vec::new();
    let report = crawl_declaring(
        &mut engine,
        "links",
        &seeds,
        &json!({ "shelf": {}, "book": {} }),
        &json!({}),
        &mut |inside, handle| {
            let page = fetch(
                &proving_ground,
                handle["crawl_request"]["url"].as_str().unwrap(),
            );
            if handle["crawl_request"]["label"] == "book" {
                let id = attributes(&page.body, "data-id");
                return json!({ "result": { "records": [{ "dataset": "books", "data": { "id": id, "status": page.status } }] } });
            }
            let document = json!({
                "url": page.url,
                "base_href": attributes(&page.body, "base href").first(),
            });
            for href in attributes(&page.body, "a href") {
                let answer = inside.enqueue(handle, &href, "book", Some(document.clone()));
                enqueued.push(answer["url"].as_str().unwrap().to_owned());
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let books = Connection::open(HomeLayout::of(env.home()).run_file("links"))
        .unwrap()
        .query_row(
            "SELECT count(*) FROM records WHERE dataset = 'books' AND data ->> '$.status' = 200",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap();
    let expected: Vec<String> = (1..=8)
        .map(|book| proving_ground.url("1.links.test", &format!("/library/shelves/book/{book}")))
        .chain(
            (1..=8)
                .map(|book| proving_ground.url("2.links.test", &format!("/new/shelf/book/{book}"))),
        )
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            enqueued,
            books,
            &observations["findings"]
        ),
        (&json!("finished"), expected, 16, &json!([]))
    );
}

/// Covers: error:attempt_ended, operation:crawl_request.enqueue
#[test]
fn an_enqueue_from_an_attempt_that_ended_fails_with_attempt_ended() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let seeds = [json!({ "url": site.page("/a"), "label": "page" })];
    let mut engine = site.engine(&env);

    let mut handles = Vec::new();
    let mut answers = Vec::new();
    crawl(&mut engine, "ended", &seeds, &mut |inside, handle| {
        let mut stale = handle.clone();
        stale["lease_epoch"] = json!(handle["lease_epoch"].as_u64().unwrap() + 1);
        answers.push(inside.enqueue(&stale, &site.page("/b"), "page", None));
        handles.push(handle.clone());
        json!({ "result": { "records": [] } })
    });
    let after = engine.call(
        "late",
        "crawl_request.enqueue",
        &json!({
            "run_id": "ended", "crawl_request_id": handles[0]["crawl_request"]["id"],
            "lease_epoch": handles[0]["lease_epoch"], "url": site.page("/c"), "label": "page",
        }),
        &mut |_| None,
    );
    assert!(engine.finished());

    assert_eq!(
        (answers, &after["error"]["data"]["code"]),
        (vec![json!("attempt_ended")], &json!("attempt_ended"))
    );
}

/// The URL identity conformance cases of this slice (ADR 0120): tracking
/// parameters, parameters in another order, `;jsessionid=` and the
/// fragment make one request, seeded or enqueued, while the request is
/// fetched at its own URL: the Proving ground receives it with its
/// parameters in their order and its tracking parameters kept (pitfalls
/// U04, U06).
///
/// Covers: story:6-213, operation:run.seeds, operation:crawl.start, operation:crawl_request.enqueue
#[test]
fn tracking_parameters_order_sessions_and_fragments_never_split_a_page_fetched_at_its_own_url() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let guide = proving_ground.url("1.links.test", "/guide/");
    let given = [
        format!("{guide}?utm_source=news&b=2&a=1#top"),
        format!("{guide}?a=1&b=2"),
        format!("{guide};JSESSIONID=0A1B?b=2&gclid=x&a=1&&"),
        format!("{guide}?a=2&b=1"),
    ];
    let seeds: Vec<Value> = given
        .iter()
        .map(|url| json!({ "url": url, "label": "guide" }))
        .collect();
    let linked = format!("{guide}?fbclid=y&a=1&b=2#reviews");
    let mut engine = crate::site::engine_on(&env, &proving_ground);

    let mut enqueued = Vec::new();
    let report = crawl(&mut engine, "keys", &seeds, &mut |inside, handle| {
        enqueued.push(inside.enqueue(handle, &linked, "guide", None));
        json!({ "result": { "records": [] } })
    });
    let answers = engine.call(
        "seeds",
        "run.seeds",
        &json!({ "run_id": "keys" }),
        &mut |_| None,
    );
    let received: Vec<Value> = proving_ground
        .received_requests()
        .unwrap()
        .every()
        .map(|request| json!([request.host, request.target]))
        .collect();
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            &answers["result"]["seeds"],
            Value::Array(enqueued),
            received
        ),
        (
            &json!("finished"),
            &json!([
                added("1", &given[0]),
                not_added("duplicate", &given[1]),
                not_added("duplicate", &given[2]),
                added("2", &given[3]),
            ]),
            json!([
                not_added("duplicate", &linked),
                not_added("duplicate", &linked)
            ]),
            vec![
                json!(["1.links.test", "/robots.txt"]),
                json!(["1.links.test", "/guide/?utm_source=news&b=2&a=1"]),
                json!(["1.links.test", "/guide/?a=2&b=1"]),
            ]
        )
    );
}

/// A request's identity is its method, its URL's key and its body's hash
/// (ADR 0093): a POST to one URL with two bodies is two requests, the same
/// body twice is one, and a GET of that URL another; a declared `key`
/// replaces the identity (ADR 0005). Each request reaches its handler with
/// the method and body it is sent with.
///
/// Covers: story:6-212, story:6-214, operation:run.seeds, operation:crawl.start
#[test]
fn a_post_with_two_bodies_is_two_requests_and_a_declared_key_replaces_the_identity() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let search = site.url("api.test", "/search");
    let (first, second) = ("eyJwYWdlIjoxfQ==", "eyJwYWdlIjoyfQ==");
    let seeds = [
        json!({ "url": search, "label": "search", "method": "POST", "body": first }),
        json!({ "url": search, "label": "search", "method": "post", "body": second }),
        json!({ "url": search, "label": "search", "method": "POST", "body": first }),
        json!({ "url": search, "label": "search" }),
        json!({ "url": site.url("api.test", "/item/1?ref=home"), "label": "item", "key": "item-1" }),
        json!({ "url": site.url("api.test", "/item/1?ref=search"), "label": "item", "key": "item-1" }),
    ];
    let mut engine = site.engine(&env);

    let mut handled = Vec::new();
    crawl(&mut engine, "bodies", &seeds, &mut |_, handle| {
        let request = &handle["crawl_request"];
        handled.push(json!([request["url"], request["method"], request["body"]]));
        json!({ "result": { "records": [] } })
    });
    let answers = engine.call(
        "seeds",
        "run.seeds",
        &json!({ "run_id": "bodies" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    assert_eq!(
        (&answers["result"]["seeds"], handled),
        (
            &json!([
                added("1", &search),
                added("2", &search),
                not_added("duplicate", &search),
                added("3", &search),
                added("4", &site.url("api.test", "/item/1?ref=home")),
                not_added("duplicate", &site.url("api.test", "/item/1?ref=search")),
            ]),
            vec![
                json!([search, "POST", first]),
                json!([search, "POST", second]),
                json!([search, "GET", null]),
                json!([site.url("api.test", "/item/1?ref=home"), "GET", null]),
            ]
        )
    );
}

/// A handler's enqueues with a priority and `forefront`, crawled one
/// request at a time: within `a.test`, the higher priority goes before the
/// lower and the `forefront` request ahead of every one pending, whatever
/// their priority; `b.test`'s urgent request goes first in its own domain
/// only, so the domains still take turns as they would with no priority.
///
/// Covers: story:6-215, operation:crawl_request.enqueue
#[test]
fn priority_and_forefront_order_a_domains_requests_and_never_the_domains_turns() {
    let env = TestEnv::new();
    let site = Site::start(&env);
    let seeds = [
        json!({ "url": site.url("a.test", "/start"), "label": "page" }),
        json!({ "url": site.url("b.test", "/start"), "label": "page" }),
    ];
    let mut engine = site.engine(&env);

    let mut handled = Vec::new();
    let report = crawl(&mut engine, "ordered", &seeds, &mut |inside, handle| {
        let url = handle["crawl_request"]["url"].as_str().unwrap().to_owned();
        if url == site.url("a.test", "/start") {
            for (enqueued, priority, forefront) in [
                (site.url("a.test", "/low"), 0, false),
                (site.url("a.test", "/high"), 5, false),
                (site.url("a.test", "/front"), -5, true),
                (site.url("b.test", "/urgent"), 100, false),
            ] {
                let reply = inside.call(
                    "crawl_request.enqueue",
                    &json!({
                        "run_id": handle["run_id"],
                        "crawl_request_id": handle["crawl_request"]["id"],
                        "lease_epoch": handle["lease_epoch"],
                        "url": enqueued,
                        "label": "page",
                        "priority": priority,
                        "forefront": forefront,
                    }),
                );
                assert_eq!(reply["result"]["kind"], "added", "{reply}");
            }
        }
        handled.push(url);
        json!({ "result": { "records": [] } })
    });
    assert!(engine.finished());

    assert_eq!(
        (&report["result"]["lifecycle"], handled),
        (
            &json!("finished"),
            [
                site.url("a.test", "/start"),
                site.url("b.test", "/urgent"),
                site.url("a.test", "/front"),
                site.url("b.test", "/start"),
                site.url("a.test", "/high"),
                site.url("a.test", "/low"),
            ]
            .to_vec()
        )
    );
}
