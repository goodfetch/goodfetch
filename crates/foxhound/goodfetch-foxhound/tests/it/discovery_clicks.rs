//! Discovery clicks (`crawl_request.enqueue_links_by_clicking_elements`,
//! issue 1087, CO-06, pitfall U14): clicking matched elements through the
//! guard records script-only links (`onclick`, popups, `history.pushState`)
//! without navigating away, aborts any unsafe request (`method` not
//! `GET`/`HEAD`/`OPTIONS`) unsent and reports it as `unsafe_request_aborted`,
//! reports guard refusals, and escalates from `http` under `fetch.tier: auto`.
#![expect(
    clippy::disallowed_methods,
    reason = "loopback test HTTP server for unsafe POST and guard-refusal verification"
)]
#![expect(
    clippy::disallowed_types,
    reason = "test server request log read after the crawl finishes"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use goodfetch_test_support::{ProvingGround, TestEnv, ToolStatus};
use serde_json::{Value, json};

use crate::fetch_tiers::{engine, settings};
use crate::frontier::{Inside, crawl_from};
use crate::provenance::records as read_records;

/// Proving ground seed for `links/3` discovery clicks.
const SEED: u64 = 1087;

/// Local loopback HTTP server recording every `(method, target)` it receives.
struct LocalSite {
    origin: String,
    received: Arc<Mutex<Vec<(String, String)>>>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}

impl LocalSite {
    fn start(index_html: String) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind loopback");
        let port = listener.local_addr().expect("local addr").port();
        listener
            .set_nonblocking(true)
            .expect("nonblocking loopback listener");
        let received = Arc::new(Mutex::new(Vec::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let received_bg = Arc::clone(&received);
        let stop_bg = Arc::clone(&stop);
        let handle = thread::spawn(move || {
            while !stop_bg.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                        let mut buf = [0_u8; 4096];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let req = String::from_utf8_lossy(&buf[..n]);
                        let mut parts = req.lines().next().unwrap_or_default().split_whitespace();
                        let method = parts.next().unwrap_or_default().to_owned();
                        let target = parts.next().unwrap_or_default().to_owned();
                        if !method.is_empty() {
                            received_bg.lock().unwrap().push((method, target.clone()));
                        }
                        let body = if target == "/" {
                            index_html.as_str()
                        } else {
                            "<!doctype html><html><body><p>ok</p></body></html>"
                        };
                        let resp = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                            body.len(),
                            body
                        );
                        let _ = stream.write_all(resp.as_bytes());
                    }
                    Err(err) if err.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            origin: format!("http://127.0.0.1:{port}"),
            received,
            stop,
            handle: Some(handle),
        }
    }

    fn snapshot(&self) -> Vec<(String, String)> {
        self.received.lock().unwrap().clone()
    }
}

impl Drop for LocalSite {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

fn click_call(inside: &mut Inside<'_>, handle: &Value, selector: &str, label: &str) -> Value {
    inside.call(
        "crawl_request.enqueue_links_by_clicking_elements",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "locator": { "page_id": handle["page_id"], "selector": selector },
            "label": label,
        }),
    )
}

fn push_room(inside: &mut Inside<'_>, handle: &Value) {
    let read = inside.call(
        "page.read",
        &json!({
            "page_id": handle["page_id"],
            "fields": {
                "id": { "locator": { "kind": "css", "selector": "article.entry" }, "attribute": "data-id" },
                "title": { "locator": { "kind": "css", "selector": "h1.title" } },
            },
        }),
    );
    let values = &read["result"]["values"];
    let _ = inside.call(
        "crawl_request.push",
        &json!({
            "run_id": handle["run_id"],
            "crawl_request_id": handle["crawl_request"]["id"],
            "lease_epoch": handle["lease_epoch"],
            "dataset": "rooms",
            "records": [{ "id": values["id"], "title": values["title"] }],
        }),
    );
}

/// Pitfall U14: Proving ground `links/3` has 12 rooms linked via plain `<a>`,
/// `onclick="location.assign(...)"`, `window.open(...)`, and
/// `history.pushState(...)`. Starting on `auto` (initial attempt on `http`),
/// `crawl_request.enqueue_links_by_clicking_elements` escalates to `browser`,
/// discovers every script-only room without navigating away, and crawls all 12
/// rooms whole so the Proving ground oracle accepts them.
///
/// Covers: story:6-234, operation:crawl_request.enqueue_links_by_clicking_elements
#[test]
fn pitfall_u14_crawls_script_only_links_whole_escalating_from_http_under_auto() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut attached = engine(&env);
    let start = proving_ground.start_url("links/3").unwrap();
    let crawl_settings = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "auto", "http_from": "browser", "sample": { "count": 1 } },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
        }),
    );

    let mut home_tiers = Vec::new();
    let mut click_answer = Value::Null;
    let finished = crawl_from(
        &mut attached,
        &json!({
            "run_id": "u14-links",
            "seeds": [{ "url": start, "label": "home" }],
            "labels": {
                "home": {},
                "room": { "settings": { "fetch": { "tier": "http" } } },
            },
            "datasets": [{ "name": "rooms", "key": ["id"] }],
            "settings": crawl_settings,
        }),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let label = handle["crawl_request"]["label"]
                .as_str()
                .unwrap_or_default();
            if label == "home" {
                home_tiers.push(handle["tier"].clone());
                let _ = inside.call(
                    "crawl_request.enqueue_links",
                    &json!({
                        "run_id": handle["run_id"],
                        "crawl_request_id": handle["crawl_request"]["id"],
                        "lease_epoch": handle["lease_epoch"],
                        "locator": { "page_id": handle["page_id"], "selector": "a[href]" },
                        "label": "room",
                    }),
                );
                let answer = click_call(inside, handle, "button", "room");
                if answer.get("result").is_some() {
                    click_answer = answer["result"].clone();
                }
            } else {
                push_room(inside, handle);
            }
            json!({ "result": { "records": [] } })
        },
    );

    let committed = read_records(&mut attached, "u14-links", "rooms");
    assert!(attached.finished());
    let records: Vec<Value> = committed
        .iter()
        .map(|entry| entry["value"].clone())
        .collect();
    let verdict = proving_ground
        .judge_records("links/3", ToolStatus::Success, &records)
        .unwrap();

    // Verify each room URL was requested at most once (clicks never sent
    // navigations or popups over the network).
    let received = proving_ground.received_requests().unwrap();
    let mut room_hits = std::collections::BTreeMap::<String, usize>::new();
    for req in received.of_the_case() {
        if req.target.starts_with("/rooms/") {
            *room_hits.entry(req.target.clone()).or_default() += 1;
        }
    }

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            &home_tiers,
            click_answer["links"].as_array().map(Vec::len),
            click_answer.get("aborted"),
            click_answer.get("refused"),
            records.len(),
            &verdict["outcome"],
            room_hits.values().all(|&count| count == 1),
        ),
        (
            &json!("finished"),
            &vec![json!("http"), json!("browser")],
            Some(8),
            None,
            None,
            12,
            &json!("verified"),
            true,
        ),
        "click_answer={click_answer}, verdict={verdict:?}, room_hits={room_hits:?}"
    );
}

/// Discovery clicks record `onclick`, popup and `pushState` URLs without
/// navigating, abort unsafe `POST` requests unsent and report them as
/// `unsafe_request_aborted` with method and URL, and report guard refusals
/// for unclickable (disabled) elements.
///
/// Covers: story:6-234, story:6-235, operation:crawl_request.enqueue_links_by_clicking_elements
#[test]
fn discovery_clicks_abort_unsafe_post_and_report_guard_refusal_without_navigating() {
    let env = TestEnv::new();
    let html = r#"<!doctype html>
<html>
<head><meta charset="utf-8"><title>Discovery</title></head>
<body>
  <button type="button" onclick="location.assign('/from-onclick')">Onclick</button>
  <button type="button" onclick="window.open('/from-popup')">Popup</button>
  <button type="button" onclick="history.pushState(null, '', '/from-pushstate')">PushState</button>
  <button type="button" onclick="fetch('/place-order', { method: 'POST', body: 'product=1' })">Order</button>
  <button type="button" disabled>Disabled</button>
</body>
</html>"#
        .to_owned();
    let site = LocalSite::start(html);
    let mut attached = engine(&env);
    let start = format!("{}/", site.origin);

    let mut click_answer = Value::Null;
    let mut received_during_clicks = Vec::new();
    let finished = crawl_from(
        &mut attached,
        &json!({
            "run_id": "discovery-safety",
            "seeds": [{ "url": start, "label": "home" }],
            "labels": {
                "home": {},
                "found": {},
            },
            "settings": {
                "fetch": { "tier": "browser" },
                "timeout": { "action": 500 },
                "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
                "egress": { "private_ranges": ["127.0.0.1"] },
            },
        }),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let label = handle["crawl_request"]["label"]
                .as_str()
                .unwrap_or_default();
            if label == "home" {
                let answer = click_call(inside, handle, "button", "found");
                click_answer = answer["result"].clone();
                received_during_clicks = site.snapshot();
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(attached.finished());
    let received_after = site.snapshot();

    let added_urls: Vec<String> = click_answer["links"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|entry| entry["kind"] == "added")
        .filter_map(|entry| entry["url"].as_str().map(str::to_owned))
        .collect();
    let expected_urls = vec![
        format!("{}/from-onclick", site.origin),
        format!("{}/from-popup", site.origin),
        format!("{}/from-pushstate", site.origin),
    ];
    let expected_aborted = json!([{
        "kind": "unsafe_request_aborted",
        "method": "POST",
        "url": format!("{}/place-order", site.origin),
    }]);
    let refused = click_answer["refused"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let any_post_received = received_after
        .iter()
        .any(|(method, target)| method == "POST" || target == "/place-order");
    let any_nav_during_clicks = received_during_clicks
        .iter()
        .any(|(_, target)| target != "/" && target != "/robots.txt" && target != "/favicon.ico");

    assert_eq!(
        (
            &finished["result"]["lifecycle"],
            added_urls,
            &click_answer["aborted"],
            refused.len(),
            refused.first().map(|r| &r["index"]),
            refused.first().map(|r| &r["code"]),
            any_post_received,
            any_nav_during_clicks,
        ),
        (
            &json!("finished"),
            expected_urls,
            &expected_aborted,
            1,
            Some(&json!(4)),
            Some(&json!("deadline_exceeded")),
            false,
            false,
        ),
        "click_answer={click_answer}, during={received_during_clicks:?}, after={received_after:?}"
    );
}

/// Under a forced non-browser tier (`fetch.tier: http`), discovery clicks
/// fail cleanly with `browser_required` instead of escalating.
///
/// Covers: story:6-234, operation:crawl_request.enqueue_links_by_clicking_elements
#[test]
fn forced_http_tier_refuses_discovery_clicks_with_browser_required() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut attached = engine(&env);
    let start = proving_ground.start_url("links/3").unwrap();
    let crawl_settings = settings(
        &proving_ground,
        &json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "concurrency": 1, "delay": 0 } },
        }),
    );

    let mut error_code = Value::Null;
    let finished = crawl_from(
        &mut attached,
        &json!({
            "run_id": "forced-http-click",
            "seeds": [{ "url": start, "label": "home" }],
            "labels": { "home": {}, "room": {} },
            "settings": crawl_settings,
        }),
        &mut |inside: &mut Inside<'_>, handle: &Value| {
            let answer = click_call(inside, handle, "button", "room");
            error_code = answer["error"]["data"]["code"].clone();
            json!({ "result": { "records": [] } })
        },
    );
    assert!(attached.finished());
    assert_eq!(
        (&finished["result"]["lifecycle"], error_code),
        (&json!("finished"), json!("browser_required"))
    );
}
