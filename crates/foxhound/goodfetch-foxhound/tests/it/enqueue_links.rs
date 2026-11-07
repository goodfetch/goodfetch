//! A handler's `enqueueLinks` on Foxhound's executable over stdio (spec
//! #581's seam: the protocol against the Proving ground; ADRs 0093 and
//! 0120), under `fetch.tier: http`, whose engine fetches each page and
//! serves it: `crawl_request.links` reads each element a locator matches
//! as a Payload function reads it, and `crawl_request.enqueue_links`
//! answers each one added, `no_link`, `skipped_by_payload` or not added
//! with its reason, each link taking its own Payload; a `Shift_JIS` page's
//! link is fetched with its query in `Shift_JIS`, the bytes a browser sends;
//! and a page's links resolve against its `<base href>`, or the URL it was
//! redirected to (pitfall U08).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::attached;
use crate::frontier::{Inside, crawl_with};

/// The Proving ground's seed.
const SEED: u64 = 863;

/// The settings of a crawl under `fetch.tier: http` whose hosts `hosts`
/// resolve to the Proving ground, on loopback, which its private ranges
/// list them for.
fn served(proving_ground: &ProvingGround, hosts: &[&str]) -> Value {
    let ip = proving_ground.address().ip().to_string();
    let resolve: serde_json::Map<String, Value> = hosts
        .iter()
        .map(|host| ((*host).to_owned(), json!([ip])))
        .collect();
    json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": resolve },
        "egress": { "private_ranges": hosts },
    })
}

/// The attempt's fields every `crawl_request.*` call names, beside the
/// locator `selector` on its page.
fn on_page(handle: &Value, selector: &str) -> Value {
    json!({
        "run_id": handle["run_id"],
        "crawl_request_id": handle["crawl_request"]["id"],
        "lease_epoch": handle["lease_epoch"],
        "locator": { "page_id": handle["page_id"], "selector": selector },
    })
}

/// `enqueueLinks(locator, label, payload?)` from the attempt `handle`
/// gave, with `fields` beside it; answers its result or its error's code.
fn enqueue_links(inside: &mut Inside<'_>, handle: &Value, selector: &str, fields: &Value) -> Value {
    let mut params = on_page(handle, selector);
    for (name, value) in fields.as_object().unwrap() {
        params[name] = value.clone();
    }
    let reply = inside.call("crawl_request.enqueue_links", &params);
    reply
        .get("result")
        .unwrap_or_else(|| &reply["error"]["data"]["code"])
        .clone()
}

/// The Records of the Dataset `pages` in the Run `run_id`, in order.
fn pages(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let connection = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    let mut statement = connection
        .prepare("SELECT data FROM records WHERE dataset = 'pages' ORDER BY rowid")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|data| serde_json::from_str(&data.unwrap()).unwrap())
        .collect()
}

/// The targets the Proving ground received on `host`, robots.txt aside,
/// in order.
fn targets(observations: &Value, host: &str) -> Vec<String> {
    ReceivedRequests::of(observations)
        .of_the_case()
        .filter(|request| request["host"] == host)
        .map(|request| request["target"].as_str().unwrap().to_owned())
        .collect()
}

/// On `links.test/answers/`, `crawl_request.links` reads each element in
/// document order, and a Payload function's answers give each link its
/// own Payload: the ISBN of the link carrying one, `null` for `2`, which
/// is skipped. Each element is answered: the first page added, the same
/// page again a `duplicate`, the anchor `no_link`, the mail address
/// `unsupported_scheme`, the off-site page `out_of_scope`, the URL that
/// does not parse `invalid_url`, `2` `skipped_by_payload` and `3` added.
/// Each added page's handler reads its Payload, and the Proving ground
/// receives no request for a link not added.
///
/// Covers: story:6-232, operation:crawl_request.links, operation:crawl_request.enqueue_links
#[test]
fn enqueue_links_reports_each_matched_element_and_gives_each_link_its_payload() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url("links.test", "/answers/"), "label": "index" })];
    let mut engine = attached(&env);

    let mut read = Value::Null;
    let mut answered = Value::Null;
    let report = crawl_with(
        &mut engine,
        "answers",
        &seeds,
        &served(&proving_ground, &["links.test"]),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            if handle["crawl_request"]["label"] == "page" {
                let page = json!({ "url": handle["crawl_request"]["url"], "payload": handle["crawl_request"]["payload"] });
                return json!({ "result": { "records": [{ "dataset": "pages", "data": page }] } });
            }
            let links = inside.call("crawl_request.links", &on_page(handle, "ul.links a"));
            read = links["result"].clone();
            // The program's Payload function.
            let payloads: Vec<Value> = read["links"]
                .as_array()
                .unwrap()
                .iter()
                .map(
                    |link| match (&link["href"], &link["attributes"]["data-isbn"]) {
                        (_, Value::String(isbn)) => json!({ "isbn": isbn }),
                        (Value::String(href), _) if href == "2" => Value::Null,
                        _ => json!({ "from": link["text"] }),
                    },
                )
                .collect();
            answered = enqueue_links(
                inside,
                handle,
                "ul.links a",
                &json!({ "label": "page", "payloads": payloads }),
            );
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let url = |path: &str| proving_ground.url("links.test", path);
    let link = |href: Option<&str>, url: Option<String>, text: &str, attributes: Value| {
        let mut link = json!({ "text": text, "attributes": attributes });
        if let Some(href) = href {
            link["href"] = json!(href);
        }
        if let Some(url) = url {
            link["url"] = json!(url);
        }
        link
    };
    assert_eq!(
        read,
        json!({ "links": [
            link(Some("/answers/1"), Some(url("/answers/1")), "One", json!({ "href": "/answers/1" })),
            link(Some("/answers/1#top"), Some(url("/answers/1#top")), "One again", json!({ "href": "/answers/1#top" })),
            link(None, None, "No link", json!({ "name": "here" })),
            link(Some("mailto:shop@links.test"), Some("mailto:shop@links.test".to_owned()), "Mail", json!({ "href": "mailto:shop@links.test" })),
            link(Some("http://elsewhere.test/"), Some("http://elsewhere.test/".to_owned()), "Elsewhere", json!({ "href": "http://elsewhere.test/" })),
            link(Some("http://[::1/"), None, "Broken", json!({ "href": "http://[::1/" })),
            link(Some("2"), Some(url("/answers/2")), "Two", json!({ "href": "2" })),
            link(Some("3"), Some(url("/answers/3")), "Three", json!({ "href": "3", "data-isbn": "978-3" })),
        ] })
    );
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            answered,
            pages(&env, "answers"),
            targets(&observations, "links.test"),
            targets(&observations, "elsewhere.test"),
        ),
        (
            &json!("finished"),
            json!({ "links": [
                { "kind": "added", "id": "2", "url": url("/answers/1") },
                { "kind": "not_added", "reason": "duplicate", "url": "/answers/1#top" },
                { "kind": "no_link" },
                { "kind": "not_added", "reason": "unsupported_scheme", "url": "mailto:shop@links.test" },
                { "kind": "not_added", "reason": "out_of_scope", "url": "http://elsewhere.test/" },
                { "kind": "not_added", "reason": "invalid_url", "url": "http://[::1/" },
                { "kind": "skipped_by_payload", "url": "2" },
                { "kind": "added", "id": "3", "url": url("/answers/3") },
            ] }),
            vec![
                json!({ "url": url("/answers/1"), "payload": { "from": "One" } }),
                json!({ "url": url("/answers/3"), "payload": { "isbn": "978-3" } }),
            ],
            vec![
                "/answers/".to_owned(),
                "/answers/1".to_owned(),
                "/answers/3".to_owned()
            ],
            Vec::<String>::new(),
        )
    );
}

/// One Payload for every link, the call's `include` narrowing the scope,
/// and the refusals of a call that adds nothing: `payload` with
/// `payloads`, `payloads` of another length than the matches and a
/// selector that is no CSS are `invalid_params`, a pattern the engine
/// cannot match `pattern_unsupported`, a page no tier serves `closed`.
///
/// Covers: story:6-232, story:6-225, operation:crawl_request.enqueue_links, error:pattern_unsupported
#[test]
fn one_payload_takes_every_link_include_narrows_and_a_bad_call_adds_nothing() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url("links.test", "/answers/"), "label": "index" })];
    let mut engine = attached(&env);

    let mut refused = Vec::new();
    let mut answered = Value::Null;
    let report = crawl_with(
        &mut engine,
        "one-payload",
        &seeds,
        &served(&proving_ground, &["links.test"]),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            if handle["crawl_request"]["label"] == "page" {
                let page = json!({ "url": handle["crawl_request"]["url"], "payload": handle["crawl_request"]["payload"] });
                return json!({ "result": { "records": [{ "dataset": "pages", "data": page }] } });
            }
            for (selector, fields) in [
                ("ul.links a", json!({ "payload": 1, "payloads": [] })),
                ("ul.links a", json!({ "payloads": [1, 2] })),
                ("ul.links >", json!({})),
                ("ul.links a", json!({ "include": ["**/{a,{b}}"] })),
            ] {
                let mut fields = fields;
                fields["label"] = json!("page");
                refused.push(enqueue_links(inside, handle, selector, &fields));
            }
            let mut elsewhere = on_page(handle, "a");
            elsewhere["locator"]["page_id"] = json!("document-0");
            elsewhere["label"] = json!("page");
            let closed = inside.call("crawl_request.enqueue_links", &elsewhere);
            refused.push(closed["error"]["data"]["code"].clone());
            answered = enqueue_links(
                inside,
                handle,
                "ul.links a[href]",
                &json!({ "label": "page", "payload": { "shelf": "answers" }, "include": ["**/answers/3"] }),
            );
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let url = |path: &str| proving_ground.url("links.test", path);
    let kinds: Vec<Value> = answered["links"]
        .as_array()
        .unwrap()
        .iter()
        .map(|link| link.get("reason").unwrap_or(&link["kind"]).clone())
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            refused,
            kinds,
            pages(&env, "one-payload")
        ),
        (
            &json!("finished"),
            vec![
                json!("invalid_params"),
                json!("invalid_params"),
                json!("invalid_params"),
                json!("pattern_unsupported"),
                json!("closed"),
            ],
            vec![
                json!("out_of_scope"),
                json!("out_of_scope"),
                json!("unsupported_scheme"),
                json!("out_of_scope"),
                json!("invalid_url"),
                json!("out_of_scope"),
                json!("added"),
            ],
            vec![json!({ "url": url("/answers/3"), "payload": { "shelf": "answers" } })],
        )
    );
}

/// `limit: 1` adds at most one link: once `/answers/1` is added, each
/// later element whose link the Frontier would have judged is not added,
/// with `link_limit`, and the Proving ground receives no request for it;
/// the anchor without a link stays `no_link` and the link its Payload
/// function skipped `skipped_by_payload`, the program's own answers.
/// `limit: 0` adds none.
///
/// Covers: story:6-232, operation:crawl_request.enqueue_links
#[test]
fn limit_adds_at_most_that_many_links_and_answers_each_link_past_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url("links.test", "/answers/"), "label": "index" })];
    let mut engine = attached(&env);

    let mut answered = Vec::new();
    let report = crawl_with(
        &mut engine,
        "limit",
        &seeds,
        &served(&proving_ground, &["links.test"]),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            if handle["crawl_request"]["label"] == "index" {
                let mut payloads = vec![json!({}); 8];
                payloads[6] = Value::Null;
                for limit in [0, 1] {
                    answered.push(enqueue_links(
                        inside,
                        handle,
                        "ul.links a",
                        &json!({ "label": "page", "payloads": payloads, "limit": limit }),
                    ));
                }
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let past = |url: &str| json!({ "kind": "not_added", "reason": "link_limit", "url": url });
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            answered,
            targets(&observations, "links.test"),
        ),
        (
            &json!("finished"),
            vec![
                json!({ "links": [
                    past("/answers/1"),
                    past("/answers/1#top"),
                    { "kind": "no_link" },
                    past("mailto:shop@links.test"),
                    past("http://elsewhere.test/"),
                    past("http://[::1/"),
                    { "kind": "skipped_by_payload", "url": "2" },
                    past("3"),
                ] }),
                json!({ "links": [
                    { "kind": "added", "id": "2", "url": proving_ground.url("links.test", "/answers/1") },
                    past("/answers/1#top"),
                    { "kind": "no_link" },
                    past("mailto:shop@links.test"),
                    past("http://elsewhere.test/"),
                    past("http://[::1/"),
                    { "kind": "skipped_by_payload", "url": "2" },
                    past("3"),
                ] }),
            ],
            vec!["/answers/".to_owned(), "/answers/1".to_owned()],
        )
    );
}

/// A `Shift_JIS` page's link `search?q=日本&k=한` is fetched with its query
/// in `Shift_JIS`, 한 as its percent-encoded character reference: the bytes a
/// browser sends (ADR 0120).
///
/// Covers: story:6-233, operation:crawl_request.enqueue_links
#[test]
fn a_shift_jis_pages_link_is_fetched_with_its_query_in_shift_jis() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds =
        [json!({ "url": proving_ground.url("links.test", "/shift-jis/"), "label": "index" })];
    let mut engine = attached(&env);

    let mut answered = Value::Null;
    let report = crawl_with(
        &mut engine,
        "shift-jis",
        &seeds,
        &served(&proving_ground, &["links.test"]),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            if handle["crawl_request"]["label"] == "index" {
                answered = enqueue_links(inside, handle, "a", &json!({ "label": "page" }));
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let sent = "/shift-jis/search?q=%93%FA%96{&k=%26%2354620%3B";
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            answered,
            targets(&observations, "links.test")
        ),
        (
            &json!("finished"),
            json!({ "links": [{ "kind": "added", "id": "2", "url": proving_ground.url("links.test", sent) }] }),
            vec!["/shift-jis/".to_owned(), sent.to_owned()],
        )
    );
}

/// Pitfall U08: a shelf's relative links `book/<n>` resolve against its
/// document's base URL through `enqueueLinks` as through `enqueue`:
/// `1.links.test/guide/`'s `<base href="/library/shelves/">`, and the
/// final URL `/new/shelf/` that `2.links.test/old/shelf` moved (`301`) to.
/// The Proving ground records no request a wrong base made.
///
/// Covers: story:6-232, operation:crawl_request.enqueue_links
#[rstest::rstest]
/// Covers: story:6-231
#[case::base_href("1.links.test", "/guide/", "/library/shelves/book/")]
#[case::redirect("2.links.test", "/old/shelf", "/new/shelf/book/")]
fn enqueue_links_resolves_against_the_documents_base_url(
    #[case] host: &str,
    #[case] shelf: &str,
    #[case] books: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let seeds = [json!({ "url": proving_ground.url(host, shelf), "label": "shelf" })];
    let mut engine = attached(&env);

    let mut answered = Value::Null;
    let report = crawl_with(
        &mut engine,
        "base-url",
        &seeds,
        &served(&proving_ground, &[host]),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            if handle["crawl_request"]["label"] == "shelf" {
                answered = enqueue_links(inside, handle, "ul.books a", &json!({ "label": "book" }));
            }
            json!({ "result": { "records": [] } })
        },
    );
    let observations = proving_ground.observations().unwrap();
    assert!(engine.finished());

    let urls: Vec<Value> = answered["links"]
        .as_array()
        .unwrap_or_else(|| panic!("{answered}"))
        .iter()
        .map(|link| link["url"].clone())
        .collect();
    let expected: Vec<Value> = (1..=8)
        .map(|book| json!(proving_ground.url(host, &format!("{books}{book}"))))
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            urls,
            &observations["findings"]
        ),
        (&json!("finished"), expected, &json!([]))
    );
}
