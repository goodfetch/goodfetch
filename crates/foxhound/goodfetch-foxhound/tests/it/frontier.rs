//! A crawl's Frontier on Foxhound's executable over stdio (spec #581's
//! seam: the protocol against the Proving ground; ADR 0094): each seed is
//! answered as an enqueue is, added with its ID or not added with its
//! reason, `run.seeds` lists the answers, so a crawl of nothing says why,
//! and a handler's `crawl_request.enqueue` resolves a relative URL against
//! its document's base URL, after a `<base href>` and after a redirect, on
//! the Proving ground's `links` family.
#![expect(
    clippy::disallowed_types,
    reason = "the test's handler fetches the Proving ground's pages from outside the engine (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{Read as _, Write as _};

use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::crawl::Attached;

/// The Proving ground's seed.
const SEED: u64 = 828;

/// A page of the Proving ground as a handler reads it.
struct Fetched {
    /// The URL it was reached at, after its redirects.
    url: String,
    status: u16,
    body: String,
}

/// Fetches `url`, a page of the Proving ground, following its redirects,
/// as the crawl's handler does.
fn fetch(proving_ground: &ProvingGround, url: &str) -> Fetched {
    let authority_and_path = url.strip_prefix("http://").unwrap();
    let (authority, path) = authority_and_path.split_at(authority_and_path.find('/').unwrap());
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
fn attributes(html: &str, name: &str) -> Vec<String> {
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
fn crawl(
    engine: &mut Attached,
    run_id: &str,
    seeds: &[Value],
    handler: &mut impl FnMut(&mut Inside<'_>, &Value) -> Value,
) -> Value {
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": run_id, "seeds": seeds, "settings": { "crawl": { "handler": { "concurrency": 1 } } } },
    }));
    let mut calls = 0;
    loop {
        let message = engine
            .next()
            .expect("the engine closed before the Run ended");
        if message["method"] == "crawl_request.handle" {
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
struct Inside<'a> {
    engine: &'a mut Attached,
    calls: &'a mut u32,
}

impl Inside<'_> {
    /// Calls `method` and answers its reply; with one request in flight,
    /// nothing else comes meanwhile.
    fn call(&mut self, method: &str, params: &Value) -> Value {
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
    fn enqueue(
        &mut self,
        handle: &Value,
        url: &str,
        label: &str,
        document: Option<Value>,
    ) -> Value {
        let mut params = json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "url": url,
            "label": label,
        });
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

fn added(id: &str, url: &str) -> Value {
    json!({ "kind": "added", "id": id, "url": url })
}

fn not_added(reason: &str, url: &str) -> Value {
    json!({ "kind": "not_added", "reason": reason, "url": url })
}

/// Covers: story:6-201, story:6-202, story:6-211, operation:run.seeds, operation:crawl.start, operation:crawl_request.enqueue
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
    ]
    .into_iter()
    .map(|url| json!({ "url": url, "label": "shelf" }))
    .collect();
    let mut engine = Attached::start(&env);

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
    assert!(engine.finish());

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
    let mut engine = Attached::start(&env);

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
    let requested = proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .len();
    assert!(engine.finish());

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
    let mut engine = Attached::start(&env);

    let mut enqueued = Vec::new();
    let report = crawl(&mut engine, "links", &seeds, &mut |inside, handle| {
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
    });
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finish());

    let books = Connection::open(
        env.home()
            .join("0.0")
            .join("runs")
            .join("links")
            .join("run.sqlite"),
    )
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
    let seeds = [json!({ "url": "https://shop.test/a", "label": "page" })];
    let mut engine = Attached::start(&env);

    let mut handles = Vec::new();
    let mut answers = Vec::new();
    crawl(&mut engine, "ended", &seeds, &mut |inside, handle| {
        let mut stale = handle.clone();
        stale["lease_epoch"] = json!(handle["lease_epoch"].as_u64().unwrap() + 1);
        answers.push(inside.enqueue(&stale, "https://shop.test/b", "page", None));
        handles.push(handle.clone());
        json!({ "result": { "records": [] } })
    });
    let after = engine.call(
        "late",
        "crawl_request.enqueue",
        &json!({
            "run_id": "ended", "crawl_request_id": handles[0]["crawl_request"]["id"],
            "lease_epoch": handles[0]["lease_epoch"], "url": "https://shop.test/c", "label": "page",
        }),
        &mut |_| None,
    );
    assert!(engine.finish());

    assert_eq!(
        (answers, &after["error"]["data"]["code"]),
        (vec![json!("attempt_ended")], &json!("attempt_ended"))
    );
}
