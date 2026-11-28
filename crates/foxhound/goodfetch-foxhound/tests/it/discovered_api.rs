//! The discovered-API tier (spec #579; ADR 0020; pitfall B09), on
//! Foxhound's executable against the Proving ground's infinite lists and
//! Chrome for Testing: under `fetch.tier: auto`, a sample's browser run
//! captures the JSON calls its page makes, and a run on `discovered_api`
//! replays them for the same request, its handler's
//! `page.wait_for_response` and `page.wait_for_request_finished` answered
//! from the replayed calls; a Label whose
//! samples matched there is learned onto `discovered_api`, whose later
//! requests replay the plan for their own lists, by page number or by
//! cursor, at a fraction of the browser's requests. A list whose call
//! carries a token the site takes once stays on the browser.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;

use goodfetch_test_client::HomeLayout;
use goodfetch_test_support::ObservedRequest;
use goodfetch_test_support::{ProvingGround, TestEnv, skipped_without_gui_session};
use rusqlite::Connection;
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, provenance, settings, tiers};
use crate::frontier::{Inside, crawl_declaring};

/// The Proving ground's seed.
const SEED: u64 = 770;

const HOST: &str = "tiers.test";

/// The lists of each crawl: the first three are its Label's samples.
const LISTS: u64 = 6;

/// The waits a handler arms before it reloads its page, by list kind: the
/// first two calls of its list, as URL patterns.
fn waits(kind: &str) -> [String; 2] {
    match kind {
        "scroll" | "guarded" => [
            format!(r"/api/{kind}/\d+\?page=1$"),
            format!(r"/api/{kind}/\d+\?page=2$"),
        ],
        _ => [
            format!(r"/api/{kind}/\d+(\?token=\d+)?$"),
            format!(r"/api/{kind}/\d+\?after="),
        ],
    }
}

/// A handler that reads its list's first two pages from the calls its
/// page makes: it arms a `page.wait_for_request_finished` for each, whose
/// bodies the engine keeps, and a `page.wait_for_response` for the first,
/// reloads the page, reads each finished call's body and pushes one Record
/// of its items and the first response's URL and status. A call its tier
/// cannot serve leaves it with nothing to push; a call answered without
/// items adds none.
fn read_the_list(kind: &'static str) -> impl FnMut(&mut Inside<'_>, &Value) -> Value {
    move |inside, handle| {
        let page_id = handle["page_id"].clone();
        let engine = inside.engine();
        let patterns = if kind == "signed" {
            vec![waits(kind)[0].clone()]
        } else {
            waits(kind).to_vec()
        };
        let mut armed = vec![(
            "response".to_owned(),
            "page.wait_for_response",
            patterns[0].clone(),
        )];
        for (index, pattern) in patterns.iter().enumerate() {
            armed.push((
                format!("finished-{index}"),
                "page.wait_for_request_finished",
                pattern.clone(),
            ));
        }
        for (id, method, pattern) in &armed {
            engine.send(&json!({
                "jsonrpc": "2.0", "id": id, "method": method,
                "params": {
                    "page_id": page_id,
                    "url": { "kind": "regex", "pattern": pattern },
                    "timeout": 15_000,
                },
            }));
        }
        engine.send(&json!({
            "jsonrpc": "2.0", "id": "reload", "method": "page.reload",
            "params": { "page_id": page_id },
        }));
        let mut replies = std::collections::BTreeMap::new();
        while replies.len() < armed.len() + 1 {
            let reply = engine.next().unwrap();
            let id = reply["id"].as_str().unwrap_or_default().to_owned();
            replies.insert(id, reply);
        }
        let Some(first) = replies["response"]
            .get("result")
            .map(|result| result["response"].clone())
        else {
            return json!({ "result": { "records": [] } });
        };
        let mut items = Vec::new();
        for (id, _, _) in armed.iter().skip(1) {
            let Some(response) = replies[id].get("result").map(|result| &result["response"]) else {
                return json!({ "result": { "records": [] } });
            };
            let read = inside.call(
                "response.body",
                &json!({ "page_id": page_id, "request_id": response["request_id"] }),
            );
            let path = read["result"]["path"]
                .as_str()
                .unwrap_or_else(|| panic!("{} {read}", handle["tier"]));
            let body: Value = serde_json::from_slice(&body_of(path)).unwrap();
            items.extend(body["items"].as_array().into_iter().flatten().cloned());
        }
        let reply = inside.call(
            "crawl_request.push",
            &json!({
                "run_id": handle["run_id"],
                "crawl_request_id": handle["crawl_request"]["id"],
                "lease_epoch": handle["lease_epoch"],
                "dataset": "pages",
                "records": [{
                    "list": handle["crawl_request"]["url"],
                    "items": items,
                    "first": [first["url"], first["status"]],
                }],
            }),
        );
        assert!(reply.get("result").is_some(), "{reply}");
        json!({ "result": { "records": [] } })
    }
}

/// The bytes of the body file at `path`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run's body file from outside the engine, on no async worker"
)]
fn body_of(path: &str) -> Vec<u8> {
    std::fs::read(path).unwrap()
}

/// The Records the Run `run_id` committed, sorted.
fn committed(env: &TestEnv, run_id: &str) -> Vec<Value> {
    let connection = Connection::open(HomeLayout::of(env.home()).run_file(run_id)).unwrap();
    let mut statement = connection.prepare("SELECT data FROM records").unwrap();
    let mut records: Vec<Value> = statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|data| serde_json::from_str(&data.unwrap()).unwrap())
        .collect();
    records.sort_by_key(ToString::to_string);
    records
}

/// Each `fetch_tier` Provenance entry of `run_id`, as `<tier> <chosen>`.
fn chosen(env: &TestEnv, run_id: &str) -> Vec<String> {
    tiers(env, run_id)
        .iter()
        .map(|entry| {
            format!(
                "{} {}",
                entry[1].as_str().unwrap(),
                entry[2].as_str().unwrap()
            )
        })
        .collect()
}

/// How many requests the Proving ground received for the lists after the
/// samples, `/<kind>/<n>` and `/api/<kind>/<n>` for `n` past 3, since the
/// `skip` first it received.
fn later_requests(proving_ground: &ProvingGround, kind: &str, skip: usize) -> (usize, usize) {
    let received = proving_ground.received_requests().unwrap();
    let all: Vec<&ObservedRequest> = received.every().collect();
    let count = all[skip..]
        .iter()
        .filter(|request| {
            let target = request.target.as_str();
            (4..=LISTS).any(|n| {
                target == format!("/{kind}/{n}")
                    || target.starts_with(&format!("/api/{kind}/{n}?"))
                    || target == format!("/api/{kind}/{n}")
            })
        })
        .count();
    (count, all.len())
}

/// A crawl of the lists of `kind` under `fetch.tier`, `tier`, its HTTP
/// sent from the browser (`fetch.http_from: browser`), as `run_id`, read
/// by [`read_the_list`].
fn crawl(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    (run_id, kind, tier): (&str, &'static str, &str),
) {
    let mut engine = engine(env);
    let seeds: Vec<Value> = (1..=LISTS)
        .map(|n| json!({ "url": proving_ground.url(HOST, &format!("/{kind}/{n}")), "label": kind }))
        .collect();
    let finished = crawl_declaring(
        &mut engine,
        run_id,
        &seeds,
        &json!({ kind: {} }),
        &settings(
            proving_ground,
            &json!({ "fetch": { "tier": tier, "http_from": "browser" } }),
        ),
        &mut read_the_list(kind),
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
}

/// A list a script loads page by page, by page number or by cursor, is
/// learned onto `discovered_api` from its samples: each sample's run there
/// replays its browser's calls, its handler's waits answered from them,
/// and stages what the browser's run staged; the later lists replay the
/// plan for their own pages. Every list commits the Records a crawl on
/// the browser commits, and the later ones cost the site a fraction of
/// the browser's requests: their document and their two calls, twice the
/// document with the reload, where the browser loads every page of the
/// list each time.
///
/// Covers: story:5-119, story:5-114, setting:fetch.tier, operation:page.wait_for_response, operation:page.wait_for_request_finished, operation:page.reload, operation:response.body, operation:crawl_request.push
#[rstest::rstest]
#[case::page_number("scroll")]
#[case::cursor("cursor")]
fn an_infinite_list_compared_against_the_browser_is_served_by_its_replayed_calls(
    #[case] kind: &'static str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    let auto = format!("api-{kind}");
    crawl(&env, &proving_ground, (&auto, kind, "auto"));
    let (replayed, seen) = later_requests(&proving_ground, kind, 0);
    let on_browser = format!("browser-{kind}");
    crawl(&env, &proving_ground, (&on_browser, kind, "browser"));
    let (browsed, _) = later_requests(&proving_ground, kind, seen);

    let sample = [
        "browser_fetch sample",
        "browser sample",
        "discovered_api sample",
    ];
    let expected_chosen: Vec<String> = sample
        .iter()
        .chain(&sample)
        .chain(&sample)
        .map(|entry| (*entry).to_owned())
        .chain((4..=LISTS).map(|_| "discovered_api compared".to_owned()))
        .collect();
    assert_eq!(chosen(&env, &auto), expected_chosen);
    let records = committed(&env, &auto);
    assert_eq!(records.len(), 6, "{records:?}");
    assert!(
        records
            .iter()
            .all(|record| record["items"].as_array().unwrap().len() == 4),
        "{records:?}"
    );
    assert_eq!(records, committed(&env, &on_browser));
    // Three lists, each its document twice and two calls.
    assert_eq!(replayed, 3 * (2 + 2));
    assert!(browsed >= 3 * (2 + 2 * 3), "{browsed}");
}

/// A handler that waits for its list's second call and pushes what it was
/// told: its status, or the code and setting of the refusal.
fn read_the_second_call(inside: &mut Inside<'_>, handle: &Value) -> Value {
    let page_id = handle["page_id"].clone();
    let engine = inside.engine();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "second", "method": "page.wait_for_response",
        "params": {
            "page_id": page_id,
            "url": { "kind": "regex", "pattern": r"/api/deep/\d+\?page=2$" },
            "timeout": 15_000,
        },
    }));
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "reload", "method": "page.reload",
        "params": { "page_id": page_id },
    }));
    let mut second = Value::Null;
    for _ in 0..2 {
        let reply = engine.next().unwrap();
        if reply["id"] == "second" {
            second = reply;
        }
    }
    let told = match second.get("result") {
        Some(result) => json!({ "status": result["response"]["status"] }),
        None => json!({
            "code": second["error"]["data"]["code"],
            "setting": second["error"]["data"]["setting"],
        }),
    };
    let reply = inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "dataset": "pages",
            "records": [{ "list": handle["crawl_request"]["url"], "told": told }],
        }),
    );
    assert!(reply.get("result").is_some(), "{reply}");
    json!({ "result": { "records": [] } })
}

/// A replayed call whose answer nests past `network.max_json_depth` answers
/// the wait for it `response_limit_exceeded` naming the setting: a list
/// learned onto `discovered_api` from samples that nest within it meets the
/// depth on a later list, and its handler is told so.
///
/// Covers: story:5-119, row:SC-05, error:response_limit_exceeded, setting:network.max_json_depth, operation:page.wait_for_response
#[test]
fn a_replayed_call_past_the_json_depth_answers_its_wait_response_limit_exceeded() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let seeds: Vec<Value> = (1..=LISTS)
        .map(|n| json!({ "url": proving_ground.url(HOST, &format!("/deep/{n}")), "label": "deep" }))
        .collect();

    let finished = crawl_declaring(
        &mut engine,
        "api-deep",
        &seeds,
        &json!({ "deep": {} }),
        &settings(&proving_ground, &json!({ "fetch": { "tier": "auto" } })),
        &mut read_the_second_call,
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");

    let refused = json!({
        "code": "response_limit_exceeded",
        "setting": "network.max_json_depth",
    });
    let mut told: Vec<(String, Value)> = committed(&env, "api-deep")
        .into_iter()
        .map(|record| {
            let list = record["list"].as_str().unwrap().to_owned();
            (list, record["told"].clone())
        })
        .collect();
    told.sort_by(|left, right| left.0.cmp(&right.0));
    let expected: Vec<(String, Value)> = (1..=LISTS)
        .map(|n| {
            let told = if n < 4 {
                json!({ "status": 200 })
            } else {
                refused.clone()
            };
            (proving_ground.url(HOST, &format!("/deep/{n}")), told)
        })
        .collect();
    assert_eq!(told, expected);
}

/// A list whose call carries a token its document issued, which the site
/// takes once, cannot be replayed: each sample's run on `discovered_api`
/// is refused the call, its wait escalates, and the Label stays on the
/// browser (ADR 0020).
///
/// Covers: story:5-119, setting:fetch.tier, operation:page.wait_for_response, operation:page.wait_for_request_finished
#[test]
fn a_list_whose_call_carries_a_token_taken_once_stays_on_the_browser() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();

    crawl(&env, &proving_ground, ("api-signed", "signed", "auto"));

    let sample = [
        "browser_fetch sample",
        "browser sample",
        "discovered_api sample",
    ];
    let expected_chosen: Vec<String> = sample
        .iter()
        .chain(&sample)
        .chain(&sample)
        .map(|entry| (*entry).to_owned())
        .chain((4..=LISTS).map(|_| "browser compared".to_owned()))
        .collect();
    assert_eq!(chosen(&env, "api-signed"), expected_chosen);
    let records = committed(&env, "api-signed");
    assert_eq!(records.len(), 6, "{records:?}");
}

/// The lists of `kinds` on `origin`, each a Label of its kind's name.
fn lists_of(origin: &str, kinds: &[&str]) -> Vec<Value> {
    kinds
        .iter()
        .flat_map(|kind| {
            (1..=LISTS)
                .map(move |n| json!({ "url": format!("{origin}/{kind}/{n}"), "label": kind }))
        })
        .collect()
}

/// A crawl as `run_id` of the scroll and guarded lists on `origin`, under
/// the `fetch` settings `fetch`, each read by [`read_the_list`].
fn crawl_scroll_and_guarded(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    (run_id, origin): (&str, &str),
    fetch: &Value,
) {
    let mut engine = engine(env);
    let mut scroll = read_the_list("scroll");
    let mut guarded = read_the_list("guarded");
    let finished = crawl_declaring(
        &mut engine,
        run_id,
        &lists_of(origin, &["scroll", "guarded"]),
        &json!({ "scroll": {}, "guarded": {} }),
        &settings(proving_ground, &json!({ "fetch": fetch })),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let url = handle["crawl_request"]["url"].as_str().unwrap_or_default();
            if url.contains("/guarded/") {
                guarded(inside, handle)
            } else {
                scroll(inside, handle)
            }
        },
    );
    assert!(engine.finished());
    assert_eq!(finished["result"]["lifecycle"], "finished", "{finished}");
}

/// The TLS fingerprints, without what a resumed handshake adds, of the
/// requests for the lists of `kind` past the samples, their documents and
/// their calls apart, among `requests`.
fn later_fingerprints(
    requests: &[&ObservedRequest],
    kind: &str,
) -> (BTreeSet<String>, BTreeSet<String>) {
    let ja4 = |request: &&&ObservedRequest| {
        request
            .tls
            .as_ref()
            .and_then(|hello| hello["ja4_base"].as_str())
            .unwrap_or("none")
            .to_owned()
    };
    let documents = requests
        .iter()
        .filter(|request| (4..=LISTS).any(|n| request.target == format!("/{kind}/{n}")))
        .map(|request| ja4(&request))
        .collect();
    let calls = requests
        .iter()
        .filter(|request| {
            (4..=LISTS).any(|n| request.target.starts_with(&format!("/api/{kind}/{n}?")))
        })
        .map(|request| ja4(&request))
        .collect();
    (documents, calls)
}

/// Under `fetch.http_from: learned` (#2420), a sample's replay of its
/// browser's calls leaves from a Session's own transport first, and from
/// the browser where that replay differed: the infinite-scroll list, whose
/// API answers any client alike, is learned onto `discovered_api` with its
/// calls sent from the transport, its later lists' documents and calls
/// arriving with the transport's TLS fingerprint; the guarded list, whose
/// API answers only the pinned browser, keeps `discovered_api` with its
/// calls sent from the browser, arriving with the browser's. Provenance's
/// tier entry names where each replay's calls left from, and every list
/// commits the Records a crawl on the browser commits.
///
/// Covers: story:5-119, setting:fetch.http_from, setting:fetch.tier
#[test]
fn under_learned_a_list_replays_its_calls_from_the_cheapest_sender_that_matches_the_browser() {
    if skipped_without_gui_session(
        "its transport replays the full build's identity, which differs from the headless shell's, so no call leaves the browser",
    ) {
        return;
    }
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    env.user_configuration(&format!(
        "tls.trusted_cas.\"{HOST}\" = ['{}']\n",
        proving_ground.description().ca.display()
    ));
    let origin = format!("https://{HOST}:{}", proving_ground.address().port());

    crawl_scroll_and_guarded(
        &env,
        &proving_ground,
        ("learned", &origin),
        &json!({ "tier": "auto", "http_from": "learned" }),
    );
    let received = proving_ground.received_requests().unwrap();
    let requests: Vec<&ObservedRequest> = received.every().collect();
    crawl_scroll_and_guarded(
        &env,
        &proving_ground,
        ("on-browser", &origin),
        &json!({ "tier": "browser" }),
    );

    let mut learned: Vec<Value> = provenance(&env, "learned", "fetch_tier")
        .iter()
        .filter(|entry| entry["chosen"] != "sample")
        .map(|entry| {
            json!([
                entry["label"],
                entry["tier"],
                entry["chosen"],
                entry["calls_sent_on"]
            ])
        })
        .collect();
    learned.sort_by_key(ToString::to_string);
    let mut expected: Vec<Value> = [("guarded", "browser_fetch"), ("scroll", "http")]
        .into_iter()
        .flat_map(|(label, sent_on)| {
            (4..=LISTS).map(move |_| json!([label, "discovered_api", "compared", sent_on]))
        })
        .collect();
    expected.sort_by_key(ToString::to_string);
    assert_eq!(learned, expected);

    let (scroll_documents, scroll_calls) = later_fingerprints(&requests, "scroll");
    let (guarded_documents, guarded_calls) = later_fingerprints(&requests, "guarded");
    assert_eq!(scroll_documents.len(), 1, "{scroll_documents:?}");
    assert_eq!(scroll_calls, scroll_documents);
    assert_eq!(guarded_documents.len(), 1, "{guarded_documents:?}");
    assert_eq!(guarded_calls, guarded_documents);
    assert_ne!(scroll_calls, guarded_calls);

    let records = committed(&env, "learned");
    assert_eq!(records.len(), 2 * 6, "{records:?}");
    assert!(
        records
            .iter()
            .all(|record| record["items"].as_array().unwrap().len() == 4),
        "{records:?}"
    );
    assert_eq!(records, committed(&env, "on-browser"));
}

/// End-to-end discovered-API refusal on Linux (spec #565; #2368): a crawl
/// learns `discovered_api` from its three sample lists while the API host
/// resolves to a listed stand-in address (`127.0.0.2`), then the API host
/// rebinds to `127.0.0.1` (unlisted) before the fourth list replays its
/// calls. The replayed call is never sent (zero connections on `127.0.0.1`),
/// neither armed wait (`page.wait_for_response`,
/// `page.wait_for_request_finished`) is answered (both return
/// `egress_refused` with `rule: "private_address"` instead of escalating),
/// and the Run report and outcomes record the refusal with its reason.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_types,
    reason = "the test binds local HTTP and UDP DNS listeners outside the engine to control rebinding and observe connections directly (ADR 0060)"
)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs stand-in HTTP/DNS threads and reads child test env vars outside any async worker"
)]
mod linux {
    use std::io::{ErrorKind, Read as _, Write as _};
    use std::net::{Ipv4Addr, SocketAddr, TcpListener, UdpSocket};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    use goodfetch_test_support::{BROWSERS_PATH_VARIABLE, DnsNamespace, TestEnv, browser_cache};
    use serde_json::{Value, json};

    use super::{body_of, chosen};
    use crate::attached_with;
    use crate::frontier::{Inside, crawl_declaring};

    const CHILD: &str = "discovered_api::linux::in_the_namespace_a_refused_replayed_call_answers_no_wait_and_reports_the_refusal";
    const REBIND_FLAG_VAR: &str = "GOODFETCH_TEST_DISCOVERED_API_REBIND_FLAG";

    /// Answers a single UDP DNS query: `A` gets `ip` (TTL 0), any other
    /// query type (such as `AAAA`) gets a `NOERROR` empty answer.
    fn dns_reply(query: &[u8], ip: Ipv4Addr) -> Option<Vec<u8>> {
        if query.len() < 12 {
            return None;
        }
        let mut cursor = 12;
        while cursor < query.len() {
            let len = usize::from(query[cursor]);
            cursor += 1;
            if len == 0 {
                break;
            }
            cursor = cursor.checked_add(len)?;
        }
        if cursor + 4 > query.len() {
            return None;
        }
        let qtype = u16::from_be_bytes([query[cursor], query[cursor + 1]]);
        let question_end = cursor + 4;
        let mut reply = Vec::with_capacity(question_end + 16);
        reply.extend_from_slice(&query[..2]);
        // Standard response, recursion available, NOERROR.
        reply.extend_from_slice(&[0x81, 0x80]);
        // QDCOUNT = 1
        reply.extend_from_slice(&[0x00, 0x01]);
        // ANCOUNT = 1 for A (qtype 1), else 0
        let ancount: u16 = u16::from(qtype == 1);
        reply.extend_from_slice(&ancount.to_be_bytes());
        // NSCOUNT = 0, ARCOUNT = 0
        reply.extend_from_slice(&[0x00, 0x00, 0x00, 0x00]);
        reply.extend_from_slice(&query[12..question_end]);
        if qtype == 1 {
            // Name pointer to offset 12, TYPE A, CLASS IN, TTL 0, RDLENGTH 4
            reply.extend_from_slice(&[
                0xc0, 0x0c, 0x00, 0x01, 0x00, 0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04,
            ]);
            reply.extend_from_slice(&ip.octets());
        }
        Some(reply)
    }

    /// Covers: story:4-98, story:4-102, story:4-103, error:egress_refused
    #[test]
    fn a_refused_replayed_call_on_discovered_api_answers_no_wait_and_reports_the_refusal() {
        let env = TestEnv::new();
        let cache = browser_cache(&env).expect("Playwright's Chrome for Testing is installed");
        let public = Ipv4Addr::new(127, 0, 0, 2);
        let rebind_flag = env.folder().join("rebind.flag");

        let dns_socket = UdpSocket::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).unwrap();
        let dns_addr = dns_socket.local_addr().unwrap();
        dns_socket
            .set_read_timeout(Some(Duration::from_millis(50)))
            .unwrap();
        let stop = Arc::new(AtomicBool::new(false));

        let dns_flag = rebind_flag.clone();
        let dns_stop = Arc::clone(&stop);
        let dns_thread = thread::spawn(move || {
            let mut buf = [0u8; 512];
            while !dns_stop.load(Ordering::Relaxed) {
                if let Ok((len, peer)) = dns_socket.recv_from(&mut buf) {
                    let ip = if dns_flag.exists() {
                        Ipv4Addr::LOCALHOST
                    } else {
                        public
                    };
                    if let Some(reply) = dns_reply(&buf[..len], ip) {
                        let _ = dns_socket.send_to(&reply, peer);
                    }
                }
            }
        });

        let env = env
            .with_variable(BROWSERS_PATH_VARIABLE, cache.to_str().unwrap())
            .with_variable(REBIND_FLAG_VAR, rebind_flag.to_str().unwrap());
        let namespace = DnsNamespace::enter(&env, dns_addr).unwrap();
        let output = namespace
            .output(
                &env,
                std::env::current_exe().unwrap(),
                [CHILD, "--include-ignored"],
            )
            .unwrap();
        stop.store(true, Ordering::Relaxed);
        let _ = dns_thread.join();

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);
        assert!(
            output.status.success() && !stdout.contains("running 0 tests"),
            "status={:?}\nstdout:\n{stdout}\nstderr:\n{stderr}",
            output.status
        );
    }

    /// Starts the local stand-in HTTP server for `/list/<n>` and
    /// `/api/list/<n>` on `public` and a watched listener on `127.0.0.1` at
    /// the same port.
    fn start_list_server(
        public: Ipv4Addr,
        stop: Arc<AtomicBool>,
    ) -> (u16, TcpListener, thread::JoinHandle<()>) {
        let http_listener = TcpListener::bind(SocketAddr::from((public, 0))).unwrap();
        let port = http_listener.local_addr().unwrap().port();
        http_listener.set_nonblocking(true).unwrap();
        let watched = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, port))).unwrap();
        watched.set_nonblocking(true).unwrap();

        let http_thread = thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match http_listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_nonblocking(false);
                        let mut buf = [0u8; 4096];
                        let len = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..len]);
                        let target = req
                            .lines()
                            .next()
                            .and_then(|line| line.split_whitespace().nth(1))
                            .unwrap_or("/");
                        let (content_type, body) = if let Some(n) = target.strip_prefix("/list/") {
                            let html = format!(
                                "<!doctype html><html><body><ul class=\"feed\"></ul><script>\
                                 const load = (u) => {{ const x = new XMLHttpRequest(); x.open('GET', u, false); x.send(); return JSON.parse(x.responseText); }};\
                                 for (let p = 1; p !== null; ) {{ p = load(`http://api.rebind.test:{port}/api/list/{n}?page=${{p}}`).next; }}\
                                 </script></body></html>"
                            );
                            ("text/html", html)
                        } else if let Some(rest) = target.strip_prefix("/api/list/") {
                            let (n, query) = rest.split_once('?').unwrap_or((rest, ""));
                            let page = query.strip_prefix("page=").unwrap_or("1");
                            let json_body = match page {
                                "1" => format!(r#"{{"items":["a-{n}","b-{n}"],"next":2}}"#),
                                "2" => format!(r#"{{"items":["c-{n}","d-{n}"],"next":null}}"#),
                                _ => r#"{"items":[],"next":null}"#.to_owned(),
                            };
                            ("application/json", json_body)
                        } else {
                            ("text/plain", "not found".to_owned())
                        };
                        let response = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: {content_type}\r\nAccess-Control-Allow-Origin: *\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                        let _ = stream.write_all(response.as_bytes());
                    }
                    Err(error) if error.kind() == ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        (port, watched, http_thread)
    }

    /// Handles one `/list/<n>` crawl request, flipping `rebind_flag` before
    /// `/list/4` reloads so its replayed call resolves to `127.0.0.1`.
    fn handle_list_page(
        inside: &mut Inside<'_>,
        handle: &Value,
        rebind_flag: &PathBuf,
        fourth_wait_errors: &mut Vec<(Value, Value)>,
    ) -> Value {
        let url = handle["crawl_request"]["url"].as_str().unwrap_or_default();
        let is_fourth = url.ends_with("/list/4");
        if is_fourth {
            std::fs::write(rebind_flag, b"1").unwrap();
        }
        let page_id = handle["page_id"].clone();
        let engine = inside.engine();
        let armed = [
            (
                "response",
                "page.wait_for_response",
                r"/api/list/\d+\?page=1$",
            ),
            (
                "finished-0",
                "page.wait_for_request_finished",
                r"/api/list/\d+\?page=1$",
            ),
            (
                "finished-1",
                "page.wait_for_request_finished",
                r"/api/list/\d+\?page=2$",
            ),
        ];
        for (id, method, pattern) in armed {
            engine.send(&json!({
                "jsonrpc": "2.0", "id": id, "method": method,
                "params": {
                    "page_id": page_id,
                    "url": { "kind": "regex", "pattern": pattern },
                    "timeout": 15_000,
                },
            }));
        }
        engine.send(&json!({
            "jsonrpc": "2.0", "id": "reload", "method": "page.reload",
            "params": { "page_id": page_id },
        }));
        let mut replies = std::collections::BTreeMap::new();
        while replies.len() < armed.len() + 1 {
            let reply = engine.next().unwrap();
            let id = reply["id"].as_str().unwrap_or_default().to_owned();
            replies.insert(id, reply);
        }
        if is_fourth {
            for (id, _, _) in armed {
                let err = &replies[id]["error"]["data"];
                fourth_wait_errors.push((err["code"].clone(), err["rule"].clone()));
            }
            return json!({ "error": replies["response"]["error"].clone() });
        }
        let Some(first) = replies["response"]
            .get("result")
            .map(|result| result["response"].clone())
        else {
            return json!({ "result": { "records": [] } });
        };
        let mut items = Vec::new();
        for id in ["finished-0", "finished-1"] {
            let Some(response) = replies[id].get("result").map(|result| &result["response"]) else {
                return json!({ "result": { "records": [] } });
            };
            let read = inside.call(
                "response.body",
                &json!({ "page_id": page_id, "request_id": response["request_id"] }),
            );
            let path = read["result"]["path"].as_str().unwrap();
            let body: Value = serde_json::from_slice(&body_of(path)).unwrap();
            items.extend(body["items"].as_array().into_iter().flatten().cloned());
        }
        let reply = inside.call(
            "crawl_request.push",
            &json!({
                "run_id": handle["run_id"],
                "crawl_request_id": handle["crawl_request"]["id"],
                "lease_epoch": handle["lease_epoch"],
                "dataset": "pages",
                "records": [{ "list": url, "items": items, "first": first["status"] }],
            }),
        );
        assert!(reply.get("result").is_some(), "{reply}");
        json!({ "result": { "records": [] } })
    }

    #[ignore = "run as a child by discovered_api::linux::a_refused_replayed_call_on_discovered_api_answers_no_wait_and_reports_the_refusal"]
    #[test]
    fn in_the_namespace_a_refused_replayed_call_answers_no_wait_and_reports_the_refusal() {
        let env = TestEnv::new();
        let cache = std::env::var(BROWSERS_PATH_VARIABLE).unwrap();
        let rebind_flag = PathBuf::from(std::env::var(REBIND_FLAG_VAR).unwrap());
        let stop = Arc::new(AtomicBool::new(false));
        let (port, watched, http_thread) =
            start_list_server(Ipv4Addr::new(127, 0, 0, 2), Arc::clone(&stop));

        let mut engine = attached_with(&env, Some(Duration::from_secs(90)), |command| {
            command.env(BROWSERS_PATH_VARIABLE, &cache);
        });
        let seeds: Vec<Value> = (1..=4)
            .map(|n| json!({ "url": format!("http://doc.test:{port}/list/{n}"), "label": "list" }))
            .collect();
        let settings = json!({
            "fetch": { "tier": "auto", "http_from": "transport" },
            "crawl": { "retry": { "max": 0 }, "scope": { "strategy": "all" } },
            "network": { "resolve": { "doc.test": ["127.0.0.2"] } },
            "egress": { "private_ranges": ["127.0.0.2"] },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
            "site_preferences": { "respect_robots_txt": false },
        });

        let mut fourth_wait_errors: Vec<(Value, Value)> = Vec::new();
        let finished = crawl_declaring(
            &mut engine,
            "api-refused",
            &seeds,
            &json!({ "list": {} }),
            &settings,
            &mut |inside, handle| {
                handle_list_page(inside, handle, &rebind_flag, &mut fourth_wait_errors)
            },
        );
        let outcomes_reply = engine.call(
            "outcomes",
            "run.outcomes",
            &json!({ "run_id": "api-refused" }),
            &mut |_| None,
        );
        assert!(engine.finished());
        stop.store(true, Ordering::Relaxed);
        let _ = http_thread.join();

        let fourth_outcome = outcomes_reply["result"]["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|outcome| {
                outcome["request"]["url"]
                    .as_str()
                    .is_some_and(|url| url.ends_with("/list/4"))
            })
            .cloned()
            .unwrap();
        let by_error = &finished["result"]["requests"]["by_error_code"];
        let tiers_chosen = chosen(&env, "api-refused");
        assert_eq!(
            (
                &finished["result"]["lifecycle"],
                &finished["result"]["requests"]["failed"],
                by_error
                    .as_array()
                    .unwrap_or_else(|| panic!("{finished}"))
                    .contains(&json!({ "name": "egress_refused", "count": 1 })),
                fourth_outcome["kind"].clone(),
                fourth_outcome["error"]["code"].clone(),
                fourth_outcome["error"]["details"]["rule"].clone(),
                fourth_wait_errors,
                tiers_chosen.last().map(String::as_str),
                watched.accept().map(|_| ()).unwrap_err().kind(),
            ),
            (
                &json!("finished"),
                &json!(1),
                true,
                json!("failed"),
                json!("egress_refused"),
                json!("private_address"),
                vec![
                    (json!("egress_refused"), json!("private_address")),
                    (json!("egress_refused"), json!("private_address")),
                    (json!("egress_refused"), json!("private_address")),
                ],
                Some("discovered_api compared"),
                ErrorKind::WouldBlock,
            ),
            "{finished}\n{outcomes_reply}\n{tiers_chosen:?}"
        );
    }
}
