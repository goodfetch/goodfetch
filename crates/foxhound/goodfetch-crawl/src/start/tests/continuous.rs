//! A Continuous crawl through the engine's public interface over the
//! in-memory network (spec #1836's Seam 1; ADR 0037's RT-12 and RT-46;
//! ADR 0139), on the paused clock: `crawl.continuous` implies
//! `crawl.keep_alive`, revisits each URL by its Label's `revisit.policy`
//! keeping its ID, reports `idle` and `next_due_at`, preserves coverage
//! across failed revisits, persists in the `revisits` table (and works at
//! `run_state.memory.budget: 0`), ends cleanly on `run.finish`, counts idle
//! time toward `timeout.run`, and leaves batch crawls untouched.

#![expect(
    clippy::disallowed_types,
    reason = "in-memory test transport records headers under a plain lock, on no loom path"
)]

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use super::{Client, Reply, answer_all, crawl, crawl_connected, dependencies, seeds};

fn seed(url: &str) -> Value {
    json!({ "url": url, "label": "page" })
}

fn every_label(interval_ms: u64) -> Value {
    json!({
        "page": {
            "settings": {
                "revisit": {
                    "policy": { "kind": "every", "interval": interval_ms },
                    "change_check": "none",
                }
            }
        }
    })
}

/// Advances the paused clock so the engine processes due revisits.
async fn advance(duration: Duration) {
    tokio::time::sleep(duration).await;
}

/// Polls `run.progress` until `predicate` holds, answering handler requests
/// along the way.
async fn progress_matching(
    client: &mut Client,
    run_id: &str,
    predicate: impl Fn(&Value) -> bool,
    handler: &mut impl FnMut(&Value) -> Reply,
) -> Value {
    let mut after: Option<Value> = None;
    loop {
        let params = match &after {
            None => json!({ "run_id": run_id }),
            Some(pos) => json!({ "run_id": run_id, "after": pos }),
        };
        let answer = client.call("run.progress", params, handler).await;
        assert!(
            answer["error"].is_null(),
            "run.progress failed while waiting for progress: {answer}"
        );
        let result = answer["result"].clone();
        if predicate(&result) {
            return result;
        }
        after = Some(result["position"].clone());
    }
}

/// A Continuous crawl stays running when its Frontier empties, reports
/// `idle: true` with `next_due_at`, re-admits due URLs keeping the same ID
/// and incrementing attempt numbers (never a `duplicate`), and runs a full
/// month (30 daily revisits) in seconds on the paused clock.
///
/// Covers: row:RT-12, row:RT-46, setting:crawl.continuous, setting:revisit.policy, operation:run.progress, operation:run.finish
#[test]
fn a_continuous_crawl_revisits_for_a_month_keeping_id_and_reporting_next_due_at() {
    let env = TestEnv::new();
    let seen_ids = std::cell::RefCell::new(Vec::<(String, u64)>::new());
    let mut handler = |params: &Value| {
        let id = params["crawl_request"]["id"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        let attempt = params["attempt"].as_u64().unwrap_or(0);
        seen_ids.borrow_mut().push((id, attempt));
        Reply::Records(Vec::new())
    };

    let (idle_progress, outcomes, report) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let mut labels = every_label(86_400_000);
            labels["article"] =
                json!({ "settings": { "revisit": { "policy": { "kind": "never" } } } });
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "month",
                        "seeds": [seed("https://shop.test/daily"), seed("https://shop.test/once")],
                        "settings": { "crawl": { "continuous": true } },
                        "labels": labels,
                    }),
                    &mut handler,
                )
                .await;
            client
                .call(
                    "run.add_seed",
                    json!({
                        "run_id": "month",
                        "seed": { "url": "https://shop.test/article", "label": "article" },
                    }),
                    &mut handler,
                )
                .await;
            let is_idle = |p: &Value| p["idle"] == true && p.get("next_due_at").is_some();
            let idle_progress = progress_matching(client, "month", is_idle, &mut handler).await;
            for _ in 0..30 {
                advance(Duration::from_hours(24)).await;
                let _ = progress_matching(client, "month", is_idle, &mut handler).await;
            }
            let report = client
                .call("run.finish", json!({ "run_id": "month" }), &mut handler)
                .await;
            let outcomes = client
                .call("run.outcomes", json!({ "run_id": "month" }), &mut handler)
                .await;
            (idle_progress, outcomes, report)
        },
    );

    assert_eq!(idle_progress["idle"], true);
    assert!(
        idle_progress["next_due_at"]
            .as_str()
            .is_some_and(|s| s.ends_with('Z')),
        "expected RFC 3339 next_due_at, got: {idle_progress}"
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    assert_eq!(report["result"]["completeness"], "complete", "{report}");
    assert_eq!(report["result"]["requests"]["succeeded"], 3, "{report}");
    assert_eq!(report["result"]["requests"]["skipped"], 0, "{report}");

    let recorded = seen_ids.borrow();
    let page_1_attempts: Vec<u64> = recorded
        .iter()
        .filter(|(id, _)| id == "1")
        .map(|(_, a)| *a)
        .collect();
    assert!(page_1_attempts.len() >= 30, "got {page_1_attempts:?}");
    assert_eq!((page_1_attempts[0], page_1_attempts[1]), (1, 2));
    let article_attempts: Vec<u64> = recorded
        .iter()
        .filter(|(id, _)| id == "3")
        .map(|(_, a)| *a)
        .collect();
    assert_eq!(article_attempts, vec![1]);
    assert!(
        outcomes["result"]["outcomes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|o| o["kind"] != "skipped"),
        "a revisit is never a duplicate"
    );
}

/// `run.finish` schedules no more revisits, finishes first visits still
/// waiting, and ends the Run `finished`; idle time counts toward `timeout.run`.
///
/// Covers: row:RT-12, row:RT-46, setting:crawl.continuous, setting:timeout.run
#[test]
fn a_continuous_crawls_idle_time_counts_toward_run_timeout() {
    let env = TestEnv::new();

    let report = crawl_connected(
        &json!({ "timeout": { "run": 60_000 } }),
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "timed",
                        "seeds": seeds(1),
                        "settings": { "crawl": { "continuous": true } },
                        "labels": {
                            "page": {
                                "settings": {
                                    "revisit": { "policy": { "kind": "every", "interval": 3_600_000 } }
                                }
                            }
                        },
                    }),
                    &mut answer_all,
                )
                .await;
            client
                .call("crawl.serve", json!({ "run_id": "timed" }), &mut answer_all)
                .await
        },
    );

    assert_eq!(report["result"]["lifecycle"], "stopped", "{report}");
    assert_eq!(
        report["result"]["stop_reason"],
        json!({ "kind": "timeout" }),
        "{report}"
    );
    assert_eq!(
        report["result"]["ended_at"],
        json!("2026-01-05T09:01:00Z"),
        "{report}"
    );
}

/// Completeness reads whether each URL has ever been visited: a URL that
/// succeeded on its first visit and failed on a later revisit stays `complete`.
///
/// Covers: row:RT-46, setting:crawl.continuous, setting:revisit.policy
#[test]
fn a_later_failed_revisit_does_not_take_back_coverage() {
    let env = TestEnv::new();
    let mut calls = 0usize;
    let mut handler = |_: &Value| {
        let n = calls;
        calls += 1;
        if n == 0 {
            Reply::Records(Vec::new())
        } else {
            Reply::Fails("handler_failed", false)
        }
    };

    let report = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "coverage",
                        "seeds": [seed("https://shop.test/item")],
                        "settings": { "crawl": { "continuous": true } },
                        "labels": {
                            "page": {
                                "settings": {
                                    "revisit": {
                                        "policy": { "kind": "every", "interval": 10_000 },
                                        "change_check": "none",
                                    }
                                }
                            }
                        },
                    }),
                    &mut handler,
                )
                .await;

            // Wait for first visit to settle and go idle.
            let _ =
                progress_matching(client, "coverage", |p| p["idle"] == true, &mut handler).await;

            // Advance past the 10 s revisit interval so the revisit runs and fails.
            advance(Duration::from_secs(10)).await;
            let _ = progress_matching(
                client,
                "coverage",
                |p| p["requests"]["failed"] == 1,
                &mut handler,
            )
            .await;

            client
                .call("run.finish", json!({ "run_id": "coverage" }), &mut handler)
                .await
        },
    );

    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    assert_eq!(
        report["result"]["completeness"], "complete",
        "a URL already visited once stays covered when a later revisit fails: {report}"
    );
    assert_eq!(report["result"]["requests"]["failed"], 1, "{report}");
}

/// A batch crawl accepts `revisit.policy` while `config.explain` marks it
/// inactive (`yields_to` `crawl.continuous = false`), writes 0 rows to
/// `revisits`, and a Continuous crawl at `run_state.memory.budget: 0` reads
/// due windows from `revisits` and `run.discard` removes the run file.
///
/// Covers: row:RT-46, setting:crawl.continuous, setting:revisit.policy, operation:config.explain, operation:run.discard
#[test]
fn batch_accepts_revisit_policy_marked_inactive_and_spilled_continuous_uses_revisits_table() {
    let env = TestEnv::new();

    let (explained, spilled_visits, discarded) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let explained = client
                .call(
                    "config.explain",
                    json!({
                        "keys": ["revisit.policy"],
                        "settings": { "revisit": { "policy": { "kind": "every", "interval": 60_000 } } }
                    }),
                    &mut answer_all,
                )
                .await;
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "batch-revisit",
                        "seeds": [seed("https://shop.test/a")],
                        "labels": every_label(1000),
                    }),
                    &mut answer_all,
                )
                .await;
            let batch_report = client
                .call(
                    "run.finish",
                    json!({ "run_id": "batch-revisit" }),
                    &mut answer_all,
                )
                .await;
            assert_eq!(batch_report["result"]["lifecycle"], "finished");

            let count = Rc::new(Cell::new(0usize));
            let count_h = Rc::clone(&count);
            let mut count_handler = move |_: &Value| {
                count_h.set(count_h.get() + 1);
                Reply::Records(Vec::new())
            };
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "spilled-cont",
                        "seeds": [seed("https://shop.test/b")],
                        "settings": {
                            "crawl": { "continuous": true },
                            "run_state": { "memory": { "budget": 0 } },
                        },
                        "labels": every_label(5_000),
                    }),
                    &mut count_handler,
                )
                .await;
            let _ = progress_matching(
                client,
                "spilled-cont",
                |p| p["idle"] == true && p.get("next_due_at").is_some(),
                &mut count_handler,
            )
            .await;
            advance(Duration::from_secs(5)).await;
            let _ = progress_matching(
                client,
                "spilled-cont",
                |p| p["idle"] == true && count.get() >= 2,
                &mut count_handler,
            )
            .await;
            let id = json!({ "run_id": "spilled-cont" });
            let finish_spilled = client
                .call("run.finish", id.clone(), &mut count_handler)
                .await;
            assert_eq!(finish_spilled["result"]["lifecycle"], "finished");
            let discarded = client.call("run.discard", id, &mut count_handler).await;
            (explained, count.get(), discarded)
        },
    );

    let setting = &explained["result"]["settings"][0];
    assert_eq!(setting["key"], "revisit.policy");
    assert_eq!(
        setting["yields_to"]["key"], "crawl.continuous",
        "expected revisit.policy to yield to crawl.continuous on batch crawl: {setting}"
    );
    assert_eq!(setting["yields_to"]["value"], false);
    assert!(
        spilled_visits >= 2,
        "expected spilled revisit to run, got {spilled_visits}"
    );
    assert!(
        discarded.get("error").is_none(),
        "expected discard ok: {discarded}"
    );
}

type RecordedRequest = (String, Vec<(String, String)>);
type SiteResponse = (u16, Vec<(String, String)>, String);
type Responder = Box<dyn Fn(&goodfetch_fetch_tier::Hop) -> SiteResponse + Send + Sync>;

/// Custom in-memory HTTP transport that records request headers and serves
/// scripted responses for conditional GET / 304 / signature tests (#1841).
struct ScriptedSite {
    recorded: std::sync::Mutex<Vec<RecordedRequest>>,
    respond: Responder,
}

impl goodfetch_fetch_tier::Transport for ScriptedSite {
    fn send<'a>(
        &'a self,
        hop: &'a goodfetch_fetch_tier::Hop,
        _reuse: goodfetch_fetch_tier::Reuse,
    ) -> goodfetch_fetch_tier::Sending<'a, goodfetch_fetch_tier::Sent> {
        if hop.url.path() == "/robots.txt" {
            let sent = goodfetch_fetch_tier::Sent {
                status: 404,
                headers: vec![("content-type".to_owned(), "text/plain".to_owned())],
                body: Box::new(goodfetch_memory_site::Whole::new("not found")),
                reused: false,
                decoded: false,
                certificate_exception: None,
            };
            return Box::pin(std::future::ready(Ok(sent)));
        }
        self.recorded
            .lock()
            .unwrap()
            .push((hop.url.to_string(), hop.headers.clone()));
        let (status, headers, body) = (self.respond)(hop);
        let sent = goodfetch_fetch_tier::Sent {
            status,
            headers,
            body: Box::new(goodfetch_memory_site::Whole::new(body)),
            reused: false,
            decoded: false,
            certificate_exception: None,
        };
        Box::pin(std::future::ready(Ok(sent)))
    }
}

fn engine_with_site(site: std::sync::Arc<ScriptedSite>) -> goodfetch_engine::Engine {
    let identity = goodfetch_engine::Identity {
        product: goodfetch_protocol::catalog::Product::Foxhound,
        product_version: goodfetch_protocol::connection::ProductVersion::parse("1.4.0").unwrap(),
        channel: goodfetch_engine::Channel::CratesIo,
    };
    let composition = goodfetch_engine::Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_frontier::contribution())
        .with(goodfetch_session_pool::contribution())
        .with(goodfetch_dataset::contribution(crate::url_of))
        .with(crate::contribution(
            goodfetch_fetch_tier::FetchTiers::serving(site),
        ));
    goodfetch_engine::Engine::new(identity, &composition).unwrap()
}

/// A revisit sends stored `If-None-Match` and `If-Modified-Since` validators
/// (independent of the Response cache, even after 40 days); a 304 ends
/// `unchanged` with 0 HTML parses and no handler run, leaving
/// `run.failed_requests` and `run.retry_failed` unchanged; a changed 200
/// runs the handler with 1 single Page analysis parse (ADR 0118, ADR 0139, ADR 0140).
///
/// Covers: row:CO-31, setting:revisit.change_check, setting:revisit.on_unchanged, operation:run.failed_requests, operation:run.retry_failed
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end Continuous crawl 304/change test scenario"
)]
#[test]
fn conditional_get_304_ends_unchanged_with_zero_parses_and_change_runs_handler() {
    let env = TestEnv::new();
    let version = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(1));
    let v_resp = std::sync::Arc::clone(&version);
    let site = std::sync::Arc::new(ScriptedSite {
        recorded: std::sync::Mutex::new(Vec::new()),
        respond: Box::new(move |hop| {
            let current = v_resp.load(std::sync::atomic::Ordering::SeqCst);
            let etag = format!("\"v{current}\"");
            let lm = "Mon, 05 Jan 2026 08:00:00 GMT".to_owned();
            let inm = hop
                .headers
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case("if-none-match"))
                .map(|(_, v)| v.as_str());
            if inm == Some(etag.as_str()) {
                return (
                    304,
                    vec![("etag".to_owned(), etag), ("last-modified".to_owned(), lm)],
                    String::new(),
                );
            }
            (
                200,
                vec![
                    (
                        "content-type".to_owned(),
                        "text/html; charset=utf-8".to_owned(),
                    ),
                    ("etag".to_owned(), etag),
                    ("last-modified".to_owned(), lm),
                ],
                format!(
                    "<!doctype html><html><body><h1>Version {current}</h1><p>Body {current}</p></body></html>"
                ),
            )
        }),
    });
    let metrics = goodfetch_engine::metrics::Metrics::none();
    let work = metrics.counted_work().clone();
    let engine = engine_with_site(std::sync::Arc::clone(&site)).with_metrics(metrics);
    let visit_count = Rc::new(Cell::new(0usize));
    let visit_count_h = Rc::clone(&visit_count);
    let mut handler = move |_: &Value| {
        visit_count_h.set(visit_count_h.get() + 1);
        Reply::Records(vec![json!({ "seen": visit_count_h.get() })])
    };

    let (
        (parses_after_first, parses_after_304, parses_after_change),
        (failed_reqs, retried, outcomes, report),
    ) = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            // 40-day interval exceeds Response cache's 30-day ceiling.
            let interval_ms = 40 * 86_400_000u64;
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "cond-304",
                        "seeds": [seed("https://shop.test/cond")],
                        "settings": {
                            "crawl": { "continuous": true },
                            "fetch": { "tier": "http" },
                        },
                        "labels": {
                            "page": {
                                "settings": {
                                    "revisit": {
                                        "policy": { "kind": "every", "interval": interval_ms },
                                        "change_check": "conditional+signature",
                                        "on_unchanged": "skip",
                                    }
                                }
                            }
                        },
                    }),
                    &mut handler,
                )
                .await;
            let _ = progress_matching(
                client,
                "cond-304",
                |p| p["idle"] == true && visit_count.get() == 1,
                &mut handler,
            )
            .await;
            let p1 = work.read(goodfetch_counted_work::Row::PageParses);

            // Advance 40 days -> conditional GET -> 304 -> unchanged, 0 parses, no handler.
            advance(Duration::from_hours(40 * 24)).await;
            let _ = progress_matching(
                client,
                "cond-304",
                |p| p["idle"] == true && p["requests"]["unchanged"] == 1,
                &mut handler,
            )
            .await;
            let p2 = work.read(goodfetch_counted_work::Row::PageParses);
            assert_eq!(visit_count.get(), 1, "304 must not invoke handler");

            let failed_reqs = client
                .call(
                    "run.failed_requests",
                    json!({ "run_id": "cond-304" }),
                    &mut handler,
                )
                .await;
            let retried = client
                .call(
                    "run.retry_failed",
                    json!({ "run_id": "cond-304" }),
                    &mut handler,
                )
                .await;

            // Now site changes to v2 -> next revisit gets 200 -> handler runs.
            version.store(2, std::sync::atomic::Ordering::SeqCst);
            advance(Duration::from_hours(40 * 24)).await;
            let _ = progress_matching(
                client,
                "cond-304",
                |p| p["idle"] == true && visit_count.get() == 2,
                &mut handler,
            )
            .await;
            let p3 = work.read(goodfetch_counted_work::Row::PageParses);

            let report = client
                .call("run.finish", json!({ "run_id": "cond-304" }), &mut handler)
                .await;
            let outcomes = client
                .call(
                    "run.outcomes",
                    json!({ "run_id": "cond-304" }),
                    &mut handler,
                )
                .await;
            ((p1, p2, p3), (failed_reqs, retried, outcomes, report))
        },
    )
    .0;

    assert_eq!(
        parses_after_304, parses_after_first,
        "a 304 revisit must add 0 Page analysis parses"
    );
    assert_eq!(
        parses_after_change,
        parses_after_first + 1,
        "a changed 200 revisit must compute its Page signature in the single Page analysis with no extra parse"
    );
    assert_eq!(
        failed_reqs["result"]["requests"]
            .as_array()
            .map_or(99, Vec::len),
        0,
        "unchanged must not appear in run.failed_requests: {failed_reqs}"
    );
    assert_eq!(
        retried["result"]["readmitted"], 0,
        "run.retry_failed must not retry unchanged: {retried}"
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    assert_eq!(report["result"]["requests"]["failed"], 0, "{report}");

    let reqs = site.recorded.lock().unwrap();
    assert_eq!(reqs.len(), 3, "expected 3 HTTP requests, got {reqs:?}");
    assert!(
        reqs[1]
            .1
            .iter()
            .any(|(k, v)| k.eq_ignore_ascii_case("if-none-match") && v == "\"v1\""),
        "revisit must send stored ETag in If-None-Match: {:?}",
        reqs[1].1
    );
    assert!(
        reqs[1].1.iter().any(|(k, v)| {
            k.eq_ignore_ascii_case("if-modified-since") && v == "Mon, 05 Jan 2026 08:00:00 GMT"
        }),
        "revisit must send stored Last-Modified in If-Modified-Since: {:?}",
        reqs[1].1
    );
    let kinds: Vec<&str> = outcomes["result"]["outcomes"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|o| o["kind"].as_str())
        .collect();
    assert_eq!(kinds, vec!["succeeded", "unchanged", "succeeded"]);
}

/// `revisit.change_check` modes (`conditional+signature`, `conditional`,
/// `signature`, `none`), `revisit.on_unchanged` (`skip`, `handler`), and
/// `config.explain` yielding on a batch crawl (ADR 0139).
///
/// Covers: setting:revisit.change_check, setting:revisit.on_unchanged, operation:config.explain
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end test covering all revisit.change_check and revisit.on_unchanged modes"
)]
#[test]
fn revisit_change_check_and_on_unchanged_modes() {
    let env = TestEnv::new();
    let site =
        std::sync::Arc::new(ScriptedSite {
            recorded: std::sync::Mutex::new(Vec::new()),
            respond: Box::new(move |_| {
                // Always returns 200 with identical body & ETag (ignores conditional headers).
                (
                200,
                vec![
                    ("content-type".to_owned(), "text/html; charset=utf-8".to_owned()),
                    ("etag".to_owned(), "\"same\"".to_owned()),
                ],
                "<!doctype html><html><body><h1>Stable</h1><p>Unchanged content</p></body></html>"
                    .to_owned(),
            )
            }),
        });
    let engine = engine_with_site(std::sync::Arc::clone(&site));
    let calls_by_label =
        std::cell::RefCell::new(std::collections::BTreeMap::<String, usize>::new());
    let mut handler = |params: &Value| {
        let label = params["crawl_request"]["label"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        *calls_by_label.borrow_mut().entry(label).or_default() += 1;
        Reply::Records(Vec::new())
    };

    let (explained, report) = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let explained = client
                .call(
                    "config.explain",
                    json!({
                        "keys": ["revisit.change_check", "revisit.on_unchanged"],
                        "settings": {
                            "revisit": { "change_check": "signature", "on_unchanged": "handler" }
                        }
                    }),
                    &mut handler,
                )
                .await;
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "modes",
                        "seeds": [
                            { "url": "https://shop.test/cs", "label": "cs" },
                            { "url": "https://shop.test/cond", "label": "cond" },
                            { "url": "https://shop.test/sig", "label": "sig" },
                            { "url": "https://shop.test/none", "label": "none" },
                            { "url": "https://shop.test/run-handler", "label": "run_handler" },
                        ],
                        "settings": {
                            "crawl": { "continuous": true },
                            "fetch": { "tier": "http" },
                        },
                        "labels": {
                            "cs": { "settings": { "revisit": { "policy": { "kind": "every", "interval": 5_000 }, "change_check": "conditional+signature" } } },
                            "cond": { "settings": { "revisit": { "policy": { "kind": "every", "interval": 5_000 }, "change_check": "conditional" } } },
                            "sig": { "settings": { "revisit": { "policy": { "kind": "every", "interval": 5_000 }, "change_check": "signature" } } },
                            "none": { "settings": { "revisit": { "policy": { "kind": "every", "interval": 5_000 }, "change_check": "none" } } },
                            "run_handler": { "settings": { "revisit": { "policy": { "kind": "every", "interval": 5_000 }, "change_check": "signature", "on_unchanged": "handler" } } },
                        },
                    }),
                    &mut handler,
                )
                .await;
            let _ = progress_matching(
                client,
                "modes",
                |p| p["idle"] == true && p["requests"]["succeeded"] == 5,
                &mut handler,
            )
            .await;
            // Advance 5s for one revisit of all 5 URLs.
            advance(Duration::from_secs(5)).await;
            let _ = progress_matching(
                client,
                "modes",
                |p| p["idle"] == true && p["requests"]["unchanged"] == 2 && site.recorded.lock().unwrap().len() >= 10,
                &mut handler,
            )
            .await;
            let report = client
                .call("run.finish", json!({ "run_id": "modes" }), &mut handler)
                .await;
            (explained, report)
        },
    )
    .0;

    // config.explain marks both inactive on batch crawl.
    for setting in explained["result"]["settings"].as_array().unwrap() {
        assert_eq!(setting["yields_to"]["key"], "crawl.continuous", "{setting}");
        assert_eq!(setting["yields_to"]["value"], false, "{setting}");
    }

    let counts = calls_by_label.borrow();
    // `cs` and `sig` saw matching signature -> skipped handler on revisit (1 call each).
    assert_eq!(counts.get("cs").copied(), Some(1), "{counts:?}");
    assert_eq!(counts.get("sig").copied(), Some(1), "{counts:?}");
    // `cond` treats every 200 as changed; `none` skips checks; `run_handler` runs handler on unchanged -> 2 calls each.
    assert_eq!(counts.get("cond").copied(), Some(2), "{counts:?}");
    assert_eq!(counts.get("none").copied(), Some(2), "{counts:?}");
    assert_eq!(counts.get("run_handler").copied(), Some(2), "{counts:?}");
    assert_eq!(report["result"]["requests"]["unchanged"], 2, "{report}");

    // Verify `sig` and `none` sent NO If-None-Match header on revisit, while `cs` and `cond` did.
    let reqs = site.recorded.lock().unwrap();
    let revisit_has_inm = |path: &str| {
        reqs.iter()
            .filter(|(u, _)| u.ends_with(path))
            .nth(1)
            .is_some_and(|(_, h)| {
                h.iter()
                    .any(|(k, _)| k.eq_ignore_ascii_case("if-none-match"))
            })
    };
    assert!(
        revisit_has_inm("/cs"),
        "conditional+signature must send If-None-Match"
    );
    assert!(
        revisit_has_inm("/cond"),
        "conditional must send If-None-Match"
    );
    assert!(
        !revisit_has_inm("/sig"),
        "signature must send no validators"
    );
    assert!(!revisit_has_inm("/none"), "none must send no validators");
}

/// A URL last served by a browser tier uses `signature` on revisit whatever
/// `revisit.change_check` says (sending no validators), and the Run report
/// notes it with `revisit_browser_signature` (ADR 0139).
///
/// Covers: setting:revisit.change_check, setting:revisit.on_unchanged
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end resume test for browser-tier signature fallback and report warning"
)]
#[test]
fn browser_tier_last_served_revisit_uses_signature_and_warns() {
    let env = TestEnv::new();
    let site = std::sync::Arc::new(ScriptedSite {
        recorded: std::sync::Mutex::new(Vec::new()),
        respond: Box::new(move |_| {
            (
                200,
                vec![
                    (
                        "content-type".to_owned(),
                        "text/html; charset=utf-8".to_owned(),
                    ),
                    ("etag".to_owned(), "\"browser-etag\"".to_owned()),
                ],
                "<!doctype html><html><body><h1>Browser Page</h1><p>Stable</p></body></html>"
                    .to_owned(),
            )
        }),
    });
    let engine = engine_with_site(std::sync::Arc::clone(&site));
    let calls = Rc::new(Cell::new(0usize));
    let calls_h = Rc::clone(&calls);
    let mut handler = move |_: &Value| {
        calls_h.set(calls_h.get() + 1);
        Reply::Records(Vec::new())
    };

    // Pass 1: initial visit records signature + etag, then stop cleanly.
    let _ = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "browser-sig",
                        "seeds": [seed("https://shop.test/browser")],
                        "settings": {
                            "crawl": { "continuous": true },
                            "fetch": { "tier": "http" },
                        },
                        "labels": {
                            "page": {
                                "settings": {
                                    "revisit": {
                                        "policy": { "kind": "every", "interval": 5_000 },
                                        "change_check": "conditional+signature",
                                    }
                                }
                            }
                        },
                    }),
                    &mut handler,
                )
                .await;
            let _ = progress_matching(
                client,
                "browser-sig",
                |p| p["idle"] == true && calls.get() == 1,
                &mut handler,
            )
            .await;
            client
                .call("run.stop", json!({ "run_id": "browser-sig" }), &mut handler)
                .await
        },
    )
    .0;

    // Mark the persisted revisit row as last served by the `browser` tier.
    let conn = super::run_file(&env, "browser-sig");
    conn.execute("UPDATE revisits SET last_tier = 'browser'", [])
        .unwrap();
    drop(conn);

    // Pass 2: resume — revisit must skip conditional headers, use signature,
    // end unchanged with no handler call, and emit `revisit_browser_signature`.
    let report = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "browser-sig",
                        "seeds": [seed("https://shop.test/browser")],
                        "settings": {
                            "crawl": { "continuous": true },
                            "fetch": { "tier": "http" },
                        },
                        "labels": {
                            "page": {
                                "settings": {
                                    "revisit": {
                                        "policy": { "kind": "every", "interval": 5_000 },
                                        "change_check": "conditional+signature",
                                    }
                                }
                            }
                        },
                    }),
                    &mut handler,
                )
                .await;
            advance(Duration::from_secs(6)).await;
            let _ = progress_matching(
                client,
                "browser-sig",
                |p| p["idle"] == true && p["requests"]["unchanged"] == 1,
                &mut handler,
            )
            .await;
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "browser-sig" }),
                    &mut handler,
                )
                .await
        },
    )
    .0;

    assert_eq!(
        calls.get(),
        1,
        "unchanged browser-tier revisit must not call handler"
    );
    let reqs = site.recorded.lock().unwrap();
    assert_eq!(reqs.len(), 2, "expected 2 HTTP requests, got {reqs:?}");
    assert!(
        !reqs[1]
            .1
            .iter()
            .any(|(k, _)| k.eq_ignore_ascii_case("if-none-match")),
        "browser-tier revisit must not send If-None-Match: {:?}",
        reqs[1].1
    );
    let warnings = report["result"]["warnings"]
        .as_array()
        .expect("report warnings");
    assert!(
        warnings
            .iter()
            .any(|w| w["code"] == "revisit_browser_signature"),
        "expected revisit_browser_signature warning in report: {warnings:?}"
    );
}

/// A domain whose Politeness (ADR 0086: 1s delay, 1 in flight) cannot keep up
/// with its revisit interval falls behind: no request is sent sooner than
/// Politeness allows, and the Run report carries `revisit_domain_behind`
/// (ADR 0004, ADR 0139).
#[test]
fn domain_behind_under_politeness_reports_warning_and_respects_delay() {
    let env = TestEnv::new();
    let timestamps = std::sync::Arc::new(std::sync::Mutex::new(Vec::<tokio::time::Instant>::new()));
    let recorded_ts = std::sync::Arc::clone(&timestamps);
    let site = std::sync::Arc::new(ScriptedSite {
        recorded: std::sync::Mutex::new(Vec::new()),
        respond: Box::new(move |_| {
            recorded_ts
                .lock()
                .unwrap()
                .push(tokio::time::Instant::now());
            (
                200,
                vec![(
                    "content-type".to_owned(),
                    "text/html; charset=utf-8".to_owned(),
                )],
                "<!doctype html><html><body><h1>Slow</h1><p>Body</p></body></html>".to_owned(),
            )
        }),
    });
    let engine = engine_with_site(std::sync::Arc::clone(&site));
    let mut handler = |_: &Value| Reply::Records(Vec::new());

    let report = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "behind-politeness",
                        "seeds": [seed("https://slow.test/a")],
                        "settings": {
                            "crawl": { "continuous": true },
                            "fetch": { "tier": "http" },
                            "politeness": { "domain": { "delay": 1_000, "concurrency": 1 } },
                        },
                        "labels": every_label(400),
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["error"].is_null(), "crawl.start failed: {started}");
            // First visit finishes at t=0; revisit is due at t=400ms, while
            // Politeness holds `slow.test` until t=1000ms.
            let _ = progress_matching(
                client,
                "behind-politeness",
                |p| p["idle"] == true && p["requests"]["succeeded"] == 1,
                &mut handler,
            )
            .await;
            advance(Duration::from_millis(500)).await;
            advance(Duration::from_millis(700)).await;
            let _ = progress_matching(
                client,
                "behind-politeness",
                |p| p["idle"] == true && timestamps.lock().unwrap().len() >= 2,
                &mut handler,
            )
            .await;
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "behind-politeness" }),
                    &mut handler,
                )
                .await
        },
    )
    .0;

    let times = timestamps.lock().unwrap();
    assert!(
        times.len() >= 2,
        "expected at least 2 requests, got {}",
        times.len()
    );
    for pair in times.windows(2) {
        assert!(
            pair[1].saturating_duration_since(pair[0]) >= Duration::from_millis(1_000),
            "politeness 1s delay violated: {:?}",
            pair[1].saturating_duration_since(pair[0])
        );
    }
    let warnings = report["result"]["warnings"]
        .as_array()
        .expect("report warnings");
    assert!(
        warnings.iter().any(|w| w["code"] == "revisit_domain_behind"
            && w["message"]
                .as_str()
                .is_some_and(|m| m.contains("slow.test"))),
        "expected revisit_domain_behind warning for slow.test: {warnings:?}"
    );
}

/// Only first visits are evidence for Learned rules and trap paths: a run of
/// 105 identical revisits under `/items/{number}` never closes the path as
/// `trap_path`, and a newly added `/items/2` is still admitted and handled
/// (ADR 0005, ADR 0139).
#[test]
fn revisits_never_teach_learned_rules_or_trap_paths() {
    let env = TestEnv::new();
    let seen = Rc::new(Cell::new(0_u32));
    let seen_h = Rc::clone(&seen);
    let site = std::sync::Arc::new(ScriptedSite {
        recorded: std::sync::Mutex::new(Vec::new()),
        respond: Box::new(move |_| {
            (
                200,
                vec![(
                    "content-type".to_owned(),
                    "text/html; charset=utf-8".to_owned(),
                )],
                "<!doctype html><html><head><title>Item</title></head><body><main><p>Same body</p></main></body></html>"
                    .to_owned(),
            )
        }),
    });
    let engine = engine_with_site(std::sync::Arc::clone(&site));
    let mut handler = move |_: &Value| {
        seen_h.set(seen_h.get() + 1);
        Reply::Records(Vec::new())
    };

    let report = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "no-revisit-trap",
                        "seeds": [seed("https://shop.test/items/1")],
                        "settings": {
                            "crawl": { "continuous": true },
                            "fetch": { "tier": "http" },
                            "politeness": { "domain": { "delay": 0 } },
                        },
                        "labels": every_label(1_000),
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["error"].is_null(), "crawl.start failed: {started}");
            // Drive 105 visits (1 first visit + 104 revisits), past ADR 0005's
            // 100-page trap_path threshold.
            for target in 1..=105 {
                let _ = progress_matching(
                    client,
                    "no-revisit-trap",
                    |p| p["idle"] == true && seen.get() >= target,
                    &mut handler,
                )
                .await;
                if target < 105 {
                    advance(Duration::from_millis(1_100)).await;
                }
            }
            // Add a second URL under the same `/items/{number}` pattern: it
            // must be admitted (not refused as `trap_path`) and handled.
            let added = client
                .call(
                    "run.add_seed",
                    json!({
                        "run_id": "no-revisit-trap",
                        "seed": seed("https://shop.test/items/2"),
                    }),
                    &mut handler,
                )
                .await;
            assert_eq!(
                added["result"]["kind"], "added",
                "second item under /items/{{number}} must not be refused as trap_path: {added}"
            );
            let _ = progress_matching(
                client,
                "no-revisit-trap",
                |p| p["idle"] == true && seen.get() >= 106,
                &mut handler,
            )
            .await;
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "no-revisit-trap" }),
                    &mut handler,
                )
                .await
        },
    )
    .0;

    let learned = report["result"]["learned_rules"]
        .as_array()
        .expect("learned_rules array");
    assert!(
        learned.is_empty(),
        "revisits must not teach any learned rules or trap paths: {learned:?}"
    );
}

/// Test double for `CleanUpVersions` that starts on `v1`, then exposes `v2`
/// once `published` flips to true, recording version/date/switches into run
/// state just as `goodfetch-page::Versions` does.
struct SwitchableRules {
    v1_digest: String,
    v1_published: std::time::SystemTime,
    v2_digest: String,
    v2_published: std::time::SystemTime,
    v2_available: std::sync::atomic::AtomicBool,
    switched: std::sync::atomic::AtomicBool,
    refresh_calls: std::sync::atomic::AtomicUsize,
}

impl SwitchableRules {
    fn new() -> std::sync::Arc<Self> {
        let base = std::time::UNIX_EPOCH + Duration::from_hours(490_000);
        std::sync::Arc::new(Self {
            v1_digest: format!("sha256:{:064}", 1),
            v1_published: base,
            v2_digest: format!("sha256:{:064}", 2),
            v2_published: base + Duration::from_hours(24),
            v2_available: std::sync::atomic::AtomicBool::new(false),
            switched: std::sync::atomic::AtomicBool::new(false),
            refresh_calls: std::sync::atomic::AtomicUsize::new(0),
        })
    }

    fn publish_v2(&self) {
        self.v2_available
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }
}

impl goodfetch_engine::runs::CleanUpVersions for SwitchableRules {
    fn choose<'a>(
        &'a self,
        _context: goodfetch_engine::runs::RulesContext<'a>,
    ) -> goodfetch_engine::runs::Answering<
        'a,
        Result<goodfetch_engine::runs::Chosen, goodfetch_engine::CallError>,
    > {
        let digest = self.v1_digest.clone();
        Box::pin(async move {
            Ok(goodfetch_engine::runs::Chosen {
                digest,
                fallback: None,
            })
        })
    }

    fn held<'a>(
        &'a self,
        run: &'a goodfetch_engine::runs::RunParts,
        _holding: goodfetch_engine::runs::Holding,
        _context: goodfetch_engine::runs::RulesContext<'a>,
    ) -> goodfetch_engine::runs::Answering<'a, ()> {
        let digest = self.v1_digest.clone();
        let published_at = self.v1_published;
        Box::pin(async move {
            run.state
                .write(move |connection| {
                    goodfetch_engine::run_state::run_row::set_clean_up_version(
                        connection,
                        &digest,
                        published_at,
                    )
                })
                .await
                .unwrap();
        })
    }

    fn refresh<'a>(
        &'a self,
        run: &'a goodfetch_engine::runs::RunParts,
        context: goodfetch_engine::runs::RulesContext<'a>,
    ) -> goodfetch_engine::runs::Answering<'a, Option<goodfetch_engine::runs::Switched>> {
        self.refresh_calls
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let should_switch = self.v2_available.load(std::sync::atomic::Ordering::SeqCst)
            && !self
                .switched
                .swap(true, std::sync::atomic::Ordering::SeqCst);
        if !should_switch {
            return Box::pin(async { None });
        }
        let from_version = self.v1_digest.clone();
        let to_version = self.v2_digest.clone();
        let published_at = self.v2_published;
        let switched_at = std::time::SystemTime::from(context.clock.timestamp());
        Box::pin(async move {
            let switch = goodfetch_protocol::clean_up_rules::CleanUpRulesSwitch {
                from_version: from_version.clone(),
                to_version: to_version.clone(),
                published_at,
                switched_at,
            };
            run.state
                .write(move |connection| {
                    goodfetch_engine::run_state::run_row::set_clean_up_version(
                        connection,
                        &switch.to_version,
                        switch.published_at,
                    )?;
                    goodfetch_engine::run_state::run_row::record_clean_up_switch(
                        connection, &switch,
                    )
                })
                .await
                .unwrap();
            Some(goodfetch_engine::runs::Switched {
                from_version,
                to_version,
                published_at,
                switched_at,
            })
        })
    }
}

fn engine_with_rules(rules: std::sync::Arc<SwitchableRules>) -> goodfetch_engine::Engine {
    let identity = goodfetch_engine::Identity {
        product: goodfetch_protocol::catalog::Product::Foxhound,
        product_version: goodfetch_protocol::connection::ProductVersion::parse("1.4.0").unwrap(),
        channel: goodfetch_engine::Channel::CratesIo,
    };
    let composition = goodfetch_engine::Composition::default()
        .with(goodfetch_engine::contribution())
        .with(goodfetch_frontier::contribution())
        .with(goodfetch_session_pool::contribution())
        .with(goodfetch_dataset::contribution(crate::url_of))
        .with(crate::contribution(super::tiers()))
        .with(goodfetch_engine::Contribution::new("test-clean-up").with_clean_up_versions(rules));
    goodfetch_engine::Engine::new(identity, &composition).unwrap()
}

/// Under `clean_up.follow_latest` (on by default in a Continuous crawl),
/// the Run adopts a newly published Clean-up rules version at its next Safe
/// point, stamps subsequent Records' Provenance with the new version so
/// change tracking reports the crossing, and records the version, date and
/// switch in its Run report (ADR 0047, ADR 0139, #1873).
///
/// Covers: setting:clean_up.follow_latest, setting:crawl.continuous
#[test]
fn a_continuous_crawl_adopts_new_clean_up_rules_at_its_next_safe_point_and_reports_version_date_and_crossing()
 {
    let env = TestEnv::new();
    let rules = SwitchableRules::new();
    let engine = engine_with_rules(std::sync::Arc::clone(&rules));
    let seen = Rc::new(Cell::new(0_u64));
    let seen_h = Rc::clone(&seen);
    let mut handler = move |_params: &Value| {
        let visit = seen_h.get() + 1;
        seen_h.set(visit);
        Reply::Records(vec![json!({ "visit": visit })])
    };

    let report = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "follow-latest",
                        "labels": every_label(1_000),
                        "datasets": [{ "name": "pages" }],
                        "seeds": [seed("https://news.test/front")],
                        "settings": { "crawl": { "continuous": true } },
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["error"].is_null(), "crawl.start failed: {started}");
            let _ = progress_matching(
                client,
                "follow-latest",
                |p| p["idle"] == true && seen.get() >= 1,
                &mut handler,
            )
            .await;

            // Publish v2 while idle, then advance clock past revisit interval.
            rules.publish_v2();
            advance(Duration::from_millis(1_100)).await;
            let _ = progress_matching(
                client,
                "follow-latest",
                |p| p["idle"] == true && seen.get() >= 2,
                &mut handler,
            )
            .await;

            client
                .call(
                    "run.finish",
                    json!({ "run_id": "follow-latest" }),
                    &mut handler,
                )
                .await
        },
    )
    .0;

    let connection = super::run_file(&env, "follow-latest");
    let items = goodfetch_dataset::read(
        &connection,
        goodfetch_dataset::Query {
            dataset: "pages",
            after: 0,
            through: None,
            wall: None,
        },
        10,
    )
    .unwrap();
    assert_eq!(items.len(), 2);
    let first = &items[0].provenance;
    let second = &items[1].provenance;
    assert_eq!(
        first.clean_up_rules.as_deref(),
        Some(rules.v1_digest.as_str())
    );
    assert_eq!(
        second.clean_up_rules.as_deref(),
        Some(rules.v2_digest.as_str())
    );
    assert_eq!(
        first.clean_up_rules_crossing(second),
        Some(
            goodfetch_protocol::foxhound::provenance::CleanUpRulesCrossing {
                from_version: Some(rules.v1_digest.clone()),
                to_version: Some(rules.v2_digest.clone()),
            }
        )
    );

    let report_rules = &report["result"]["clean_up_rules"];
    assert_eq!(report_rules["version"], rules.v2_digest);
    assert!(report_rules["published_at"].is_string());
    let switches = report_rules["switches"].as_array().expect("switches array");
    assert_eq!(switches.len(), 1);
    assert_eq!(switches[0]["from_version"], rules.v1_digest);
    assert_eq!(switches[0]["to_version"], rules.v2_digest);
    assert!(switches[0]["published_at"].is_string());
    assert!(switches[0]["switched_at"].is_string());
}

/// `clean_up.follow_latest: false` keeps the Clean-up rules pinned for the
/// Continuous crawl's whole life (ADR 0047, #1873).
///
/// Covers: setting:clean_up.follow_latest
#[test]
fn clean_up_follow_latest_false_keeps_rules_pinned_in_a_continuous_crawl() {
    let env = TestEnv::new();
    let rules = SwitchableRules::new();
    let engine = engine_with_rules(std::sync::Arc::clone(&rules));
    let seen = Rc::new(Cell::new(0_u64));
    let seen_h = Rc::clone(&seen);
    let mut handler = move |_params: &Value| {
        let visit = seen_h.get() + 1;
        seen_h.set(visit);
        Reply::Records(vec![json!({ "visit": visit })])
    };

    let report = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "pinned-rules",
                        "labels": every_label(1_000),
                        "datasets": [{ "name": "pages" }],
                        "seeds": [seed("https://news.test/front")],
                        "settings": {
                            "crawl": { "continuous": true },
                            "clean_up": { "follow_latest": false },
                        },
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["error"].is_null(), "crawl.start failed: {started}");
            let _ = progress_matching(
                client,
                "pinned-rules",
                |p| p["idle"] == true && seen.get() >= 1,
                &mut handler,
            )
            .await;

            rules.publish_v2();
            advance(Duration::from_millis(1_100)).await;
            let _ = progress_matching(
                client,
                "pinned-rules",
                |p| p["idle"] == true && seen.get() >= 2,
                &mut handler,
            )
            .await;

            client
                .call(
                    "run.finish",
                    json!({ "run_id": "pinned-rules" }),
                    &mut handler,
                )
                .await
        },
    )
    .0;

    assert_eq!(
        rules
            .refresh_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "refresh must not run when clean_up.follow_latest is false"
    );
    let connection = super::run_file(&env, "pinned-rules");
    let items = goodfetch_dataset::read(
        &connection,
        goodfetch_dataset::Query {
            dataset: "pages",
            after: 0,
            through: None,
            wall: None,
        },
        10,
    )
    .unwrap();
    assert_eq!(items.len(), 2);
    let first = &items[0].provenance;
    let second = &items[1].provenance;
    assert_eq!(
        first.clean_up_rules.as_deref(),
        Some(rules.v1_digest.as_str())
    );
    assert_eq!(
        second.clean_up_rules.as_deref(),
        Some(rules.v1_digest.as_str())
    );
    assert_eq!(first.clean_up_rules_crossing(second), None);
    assert_eq!(
        report["result"]["clean_up_rules"]["version"],
        rules.v1_digest
    );
    assert!(report["result"]["clean_up_rules"]["switches"].is_null());
}

/// `clean_up.follow_latest` is off by default in a batch crawl (#1873).
///
/// Covers: setting:clean_up.follow_latest
#[test]
fn clean_up_follow_latest_is_off_by_default_in_a_batch_crawl() {
    let env = TestEnv::new();
    let rules = SwitchableRules::new();
    rules.publish_v2();
    let engine = engine_with_rules(std::sync::Arc::clone(&rules));
    let mut handler = |_params: &Value| Reply::Records(vec![json!({ "ok": true })]);

    let report = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "batch-pinned",
                        "labels": { "page": {} },
                        "datasets": [{ "name": "pages" }],
                        "seeds": [seed("https://news.test/front")],
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["error"].is_null(), "crawl.start failed: {started}");
            client
                .call(
                    "run.finish",
                    json!({ "run_id": "batch-pinned" }),
                    &mut handler,
                )
                .await
        },
    )
    .0;

    assert_eq!(
        rules
            .refresh_calls
            .load(std::sync::atomic::Ordering::SeqCst),
        0,
        "batch crawl must not refresh Clean-up rules by default"
    );
    assert_eq!(
        report["result"]["clean_up_rules"]["version"],
        rules.v1_digest
    );
    assert!(report["result"]["clean_up_rules"]["switches"].is_null());
}

/// A revisit whose fetch fails retryably keeps the URL's coverage, shows in
/// `run.failed_requests` as its latest outcome, and is due again after
/// `revisit.on_error` (a Label setting, 2 h default; inactive on batch crawls).
///
/// Covers: setting:revisit.on_error, setting:revisit.gone_after, operation:run.failed_requests, operation:config.explain
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end revisit.on_error timeline test"
)]
fn revisit_on_error_reschedules_failed_revisit_and_keeps_coverage() {
    let env = TestEnv::new();
    let calls = Rc::new(Cell::new(0usize));
    let calls_h = Rc::clone(&calls);
    let mut handler = move |_: &Value| {
        let n = calls_h.get();
        calls_h.set(n + 1);
        // Visit 1 (n=0) succeeds; revisit 1 (n=1) fails retryably;
        // revisit 2 after revisit.on_error (n=2) succeeds.
        if n == 1 {
            Reply::Fails("response_unavailable", true)
        } else {
            Reply::Records(Vec::new())
        }
    };

    let (explained, failed_mid, failed_after, report) = crawl(
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let explained_defaults = client
                .call(
                    "config.explain",
                    json!({ "keys": ["revisit.on_error", "revisit.gone_after"] }),
                    &mut answer_all,
                )
                .await;
            let explained_yields = client
                .call(
                    "config.explain",
                    json!({
                        "keys": ["revisit.on_error", "revisit.gone_after"],
                        "settings": { "revisit": { "on_error": 60_000, "gone_after": 3 } },
                    }),
                    &mut answer_all,
                )
                .await;
            let explained = json!({ "defaults": explained_defaults, "yields": explained_yields });

            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "on-error",
                        "seeds": [seed("https://shop.test/flaky")],
                        "settings": {
                            "crawl": {
                                "continuous": true,
                                "retry": { "max": 0 },
                            },
                        },
                        "labels": {
                            "page": {
                                "settings": {
                                    "revisit": {
                                        "policy": { "kind": "every", "interval": 5_000 },
                                        "change_check": "none",
                                        "on_error": 30_000,
                                    }
                                }
                            }
                        },
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["error"].is_null(), "crawl.start failed: {started}");

            // First visit settles and goes idle.
            let _ =
                progress_matching(client, "on-error", |p| p["idle"] == true, &mut handler).await;
            assert_eq!(calls.get(), 1);

            // Advance 5 s: revisit 1 runs and ends failed retryably.
            advance(Duration::from_secs(5)).await;
            let p_failed = progress_matching(
                client,
                "on-error",
                |p| p["idle"] == true && p["requests"]["failed"] == 1,
                &mut handler,
            )
            .await;
            assert!(
                p_failed.get("next_due_at").is_some_and(|v| !v.is_null()),
                "failed retryable revisit must schedule next_due_at after revisit.on_error: {p_failed}"
            );
            assert_eq!(calls.get(), 2, "expected 1 initial + 1 failed revisit");

            let failed_mid = client
                .call(
                    "run.failed_requests",
                    json!({ "run_id": "on-error" }),
                    &mut handler,
                )
                .await;

            // Advance 15 s (< 30 s revisit.on_error): must NOT revisit yet.
            advance(Duration::from_secs(15)).await;
            tokio::task::yield_now().await;
            assert_eq!(
                calls.get(),
                2,
                "revisit must not run before revisit.on_error elapses"
            );

            // Advance remaining 15 s (total 30 s): revisit 2 runs and succeeds.
            advance(Duration::from_secs(15)).await;
            let _ = progress_matching(
                client,
                "on-error",
                |p| {
                    p["idle"] == true
                        && p["requests"]["succeeded"] == 1
                        && p["requests"]["failed"] == 0
                },
                &mut handler,
            )
            .await;
            assert_eq!(calls.get(), 3);

            let failed_after = client
                .call(
                    "run.failed_requests",
                    json!({ "run_id": "on-error" }),
                    &mut handler,
                )
                .await;

            let report = client
                .call("run.finish", json!({ "run_id": "on-error" }), &mut handler)
                .await;
            (explained, failed_mid, failed_after, report)
        },
    );

    let defaults = explained["defaults"]["result"]["settings"]
        .as_array()
        .unwrap();
    let on_error = defaults
        .iter()
        .find(|s| s["key"] == "revisit.on_error")
        .unwrap();
    assert_eq!(
        on_error["value"], 7_200_000,
        "default revisit.on_error must be 2 hours: {on_error}"
    );
    let gone_after = defaults
        .iter()
        .find(|s| s["key"] == "revisit.gone_after")
        .unwrap();
    assert_eq!(
        gone_after["value"], 1,
        "default revisit.gone_after must be 1: {gone_after}"
    );
    let yields = explained["yields"]["result"]["settings"]
        .as_array()
        .unwrap();
    let on_error_y = yields
        .iter()
        .find(|s| s["key"] == "revisit.on_error")
        .unwrap();
    assert_eq!(
        on_error_y["yields_to"]["key"], "crawl.continuous",
        "{on_error_y}"
    );
    let gone_after_y = yields
        .iter()
        .find(|s| s["key"] == "revisit.gone_after")
        .unwrap();
    assert_eq!(
        gone_after_y["yields_to"]["key"], "crawl.continuous",
        "{gone_after_y}"
    );

    let mid_reqs = failed_mid["result"]["requests"].as_array().unwrap();
    assert_eq!(
        mid_reqs.len(),
        1,
        "failed revisit must show in run.failed_requests: {failed_mid}"
    );
    assert_eq!(mid_reqs[0]["request"]["url"], "https://shop.test/flaky");
    assert_eq!(mid_reqs[0]["kind"], "failed", "{failed_mid}");

    let after_reqs = failed_after["result"]["requests"].as_array().unwrap();
    assert!(
        after_reqs.is_empty(),
        "recovered revisit must leave run.failed_requests: {failed_after}"
    );

    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    assert_eq!(report["result"]["completeness"], "complete", "{report}");
    assert_eq!(report["result"]["requests"]["succeeded"], 1, "{report}");
}

/// A URL answering 404 or 410 on `revisit.gone_after` revisits in a row ends
/// `gone` (`OutcomeKind::Gone`, `Ending::Gone`), runs no handler, is never due
/// again, and no longer counts as overdue (#1843).
///
/// Covers: setting:revisit.gone_after, setting:revisit.on_error, operation:run.outcomes, operation:run.failed_requests
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end revisit.gone_after timeline test"
)]
fn revisit_gone_after_ends_dead_url_gone_and_stops_revisiting() {
    let env = TestEnv::new();
    let once_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let twice_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let once_c = std::sync::Arc::clone(&once_hits);
    let twice_c = std::sync::Arc::clone(&twice_hits);

    let site = std::sync::Arc::new(ScriptedSite {
        recorded: std::sync::Mutex::new(Vec::new()),
        respond: Box::new(move |hop| {
            let html_headers = vec![("content-type".to_owned(), "text/html".to_owned())];
            match hop.url.path() {
                "/gone-once" => {
                    let n = once_c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if n == 0 {
                        (
                            200,
                            html_headers,
                            "<html><body><h1>Alive</h1></body></html>".to_owned(),
                        )
                    } else {
                        (
                            410,
                            html_headers,
                            "<html><body>Gone</body></html>".to_owned(),
                        )
                    }
                }
                "/gone-twice" => {
                    let n = twice_c.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    match n {
                        // 0: initial 200; 1: 404 (streak 1/2); 2: 200 (resets streak to 0);
                        // 3: 404 (streak 1/2); 4: 410 (streak 2/2 -> gone).
                        0 | 2 => (
                            200,
                            html_headers,
                            format!("<html><body><h1>Alive v{n}</h1></body></html>"),
                        ),
                        1 | 3 => (
                            404,
                            html_headers,
                            "<html><body>Not found</body></html>".to_owned(),
                        ),
                        _ => (
                            410,
                            html_headers,
                            "<html><body>Gone</body></html>".to_owned(),
                        ),
                    }
                }
                _ => (200, html_headers, "<html><body>ok</body></html>".to_owned()),
            }
        }),
    });
    let engine = engine_with_site(std::sync::Arc::clone(&site));
    let handler_calls = Rc::new(Cell::new(0usize));
    let handler_calls_h = Rc::clone(&handler_calls);
    let mut handler = move |_: &Value| {
        handler_calls_h.set(handler_calls_h.get() + 1);
        Reply::Records(Vec::new())
    };

    let (gone_outcomes, failed_end, idle_after, report) = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "gone-run",
                        "settings": { "crawl": { "continuous": true } },
                        "labels": {
                            "fast": {
                                "settings": {
                                    "revisit": {
                                        "policy": { "kind": "every", "interval": 5_000 },
                                        "change_check": "none",
                                    }
                                }
                            },
                            "patient": {
                                "settings": {
                                    "revisit": {
                                        "policy": { "kind": "every", "interval": 5_000 },
                                        "change_check": "none",
                                        "gone_after": 2,
                                        "on_error": 10_000,
                                    }
                                }
                            },
                        },
                        "seeds": [
                            { "url": "https://shop.test/gone-once", "label": "fast" },
                            { "url": "https://shop.test/gone-twice", "label": "patient" },
                        ],
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["error"].is_null(), "crawl.start failed: {started}");

            // First visits settle (2 handler calls).
            let _ =
                progress_matching(client, "gone-run", |p| p["idle"] == true, &mut handler).await;
            assert_eq!(handler_calls.get(), 2);

            // +5 s: /gone-once gets 410 -> gone immediately; /gone-twice gets 404 (1 of 2 -> failed).
            advance(Duration::from_secs(5)).await;
            let _ = progress_matching(
                client,
                "gone-run",
                |p| p["idle"] == true && p["requests"]["gone"] == 1 && p["requests"]["failed"] == 1,
                &mut handler,
            )
            .await;
            assert_eq!(
                handler_calls.get(),
                2,
                "404/410 revisits must not run handler"
            );

            // +10 s (revisit.on_error): /gone-twice gets 200 -> streak resets, handler runs (3).
            advance(Duration::from_secs(10)).await;
            let _ = progress_matching(
                client,
                "gone-run",
                |p| {
                    p["idle"] == true
                        && p["requests"]["gone"] == 1
                        && p["requests"]["succeeded"] == 1
                        && p["requests"]["failed"] == 0
                },
                &mut handler,
            )
            .await;
            assert_eq!(handler_calls.get(), 3);

            // +5 s: /gone-twice gets 404 (streak 1 of 2 -> failed).
            advance(Duration::from_secs(5)).await;
            let _ = progress_matching(
                client,
                "gone-run",
                |p| p["idle"] == true && p["requests"]["gone"] == 1 && p["requests"]["failed"] == 1,
                &mut handler,
            )
            .await;

            // +10 s: /gone-twice gets 410 (streak 2 of 2 -> gone!).
            advance(Duration::from_secs(10)).await;
            let idle_after = progress_matching(
                client,
                "gone-run",
                |p| p["idle"] == true && p["requests"]["gone"] == 2 && p["requests"]["failed"] == 0,
                &mut handler,
            )
            .await;
            assert_eq!(
                handler_calls.get(),
                3,
                "gone revisits must never run handler"
            );

            // Advance 1 hour: neither gone URL is ever due again.
            let hits_before = site.recorded.lock().unwrap().len();
            advance(Duration::from_secs(3_600)).await;
            tokio::task::yield_now().await;
            assert_eq!(
                site.recorded.lock().unwrap().len(),
                hits_before,
                "gone URLs must never be revisited"
            );

            let gone_outcomes = client
                .call(
                    "run.outcomes",
                    json!({ "run_id": "gone-run", "kinds": ["gone"] }),
                    &mut handler,
                )
                .await;
            let failed_end = client
                .call(
                    "run.failed_requests",
                    json!({ "run_id": "gone-run" }),
                    &mut handler,
                )
                .await;
            let report = client
                .call("run.finish", json!({ "run_id": "gone-run" }), &mut handler)
                .await;
            (gone_outcomes, failed_end, idle_after, report)
        },
    )
    .0;

    assert!(
        idle_after.get("next_due_at").is_none_or(Value::is_null),
        "when all URLs are gone, next_due_at must be absent/null: {idle_after}"
    );
    let outcomes = gone_outcomes["result"]["outcomes"].as_array().unwrap();
    assert_eq!(
        outcomes.len(),
        2,
        "expected 2 gone outcomes: {gone_outcomes}"
    );
    assert!(
        outcomes.iter().all(|o| o["kind"] == "gone"),
        "expected kind == gone: {gone_outcomes}"
    );
    assert!(
        failed_end["result"]["requests"]
            .as_array()
            .unwrap()
            .is_empty(),
        "gone URLs must not remain in run.failed_requests: {failed_end}"
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    assert_eq!(report["result"]["completeness"], "complete", "{report}");
    assert_eq!(report["result"]["requests"]["gone"], 2, "{report}");

    // Verify neither gone URL has a due timestamp in the run file (never overdue).
    let connection = super::run_file(&env, "gone-run");
    let rows = goodfetch_frontier::revisit::read_all(&connection).unwrap();
    assert_eq!(rows.len(), 2);
    assert!(
        rows.iter().all(|r| r.next_visit_at.is_none()),
        "gone URLs must have next_visit_at = None so they never count as overdue: {rows:?}"
    );
    assert_eq!(once_hits.load(std::sync::atomic::Ordering::SeqCst), 2);
    assert_eq!(twice_hits.load(std::sync::atomic::Ordering::SeqCst), 5);
}

/// `revisit.policy` defaults to `adaptive` (initial 1 day, min 1 hour, max
/// 30 days, both rates 0.5) and rejects invalid `min > max`, rates outside
/// `(0, 1]`, and `target_freshness` outside `(0, 1)` with `setting_invalid`
/// (ADR 0139; issue 1845).
///
/// Covers: setting:revisit.policy, setting:crawl.continuous
#[test]
fn revisit_policy_defaults_to_adaptive_and_rejects_invalid_settings() {
    let env = TestEnv::new();
    let (explained, bad_min_max, bad_rate, bad_target) = crawl_connected(
        &json!({}),
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let explained = client
                .call(
                    "config.explain",
                    json!({
                        "keys": ["revisit.policy"],
                        "settings": { "crawl": { "continuous": true } },
                    }),
                    &mut answer_all,
                )
                .await;
            let start_with = |run_id: &str, policy: Value| {
                json!({
                    "run_id": run_id,
                    "seeds": [seed("https://example.test/fast")],
                    "settings": { "crawl": { "continuous": true } },
                    "labels": { "page": { "settings": { "revisit": { "policy": policy } } } }
                })
            };
            let bad_min_max = client
                .call(
                    "crawl.start",
                    start_with(
                        "bad-min-max",
                        json!({ "kind": "adaptive", "min": 5_000, "max": 4_000 }),
                    ),
                    &mut answer_all,
                )
                .await;
            let bad_rate = client
                .call(
                    "crawl.start",
                    start_with(
                        "bad-rate",
                        json!({ "kind": "adaptive", "rate_changed": 0.0 }),
                    ),
                    &mut answer_all,
                )
                .await;
            let bad_target = client
                .call(
                    "crawl.start",
                    start_with(
                        "bad-target",
                        json!({ "kind": "estimated", "target_freshness": 1.0 }),
                    ),
                    &mut answer_all,
                )
                .await;
            (explained, bad_min_max, bad_rate, bad_target)
        },
    );

    assert_eq!(
        explained["result"]["settings"][0]["value"],
        json!({
            "kind": "adaptive",
            "initial": 86_400_000,
            "min": 3_600_000,
            "max": 2_592_000_000_u64,
            "rate_changed": 0.5,
            "rate_unchanged": 0.5,
        }),
        "expected default revisit.policy to be adaptive: {explained}"
    );
    assert_eq!(
        bad_min_max["error"]["data"]["code"], "setting_invalid",
        "{bad_min_max}"
    );
    assert_eq!(
        bad_rate["error"]["data"]["code"], "setting_invalid",
        "{bad_rate}"
    );
    assert_eq!(
        bad_target["error"]["data"]["code"], "setting_invalid",
        "{bad_target}"
    );
}

/// `adaptive` and `estimated` policies clamp URLs changing faster than `min`
/// to `min` and list them in the Run report's `revisits_at_min` (ADR 0139;
/// issue 1845).
///
/// Covers: setting:revisit.policy, setting:crawl.continuous
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end Continuous crawl test exercises adaptive and estimated policies and verifies Run report"
)]
#[test]
fn adaptive_and_estimated_policies_list_fast_urls_in_run_report() {
    let env = TestEnv::new();
    let fast_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let est_hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let fast_c = std::sync::Arc::clone(&fast_hits);
    let est_c = std::sync::Arc::clone(&est_hits);

    let site = std::sync::Arc::new(ScriptedSite {
        recorded: std::sync::Mutex::new(Vec::new()),
        respond: Box::new(move |hop| {
            let html_headers = vec![("content-type".to_owned(), "text/html".to_owned())];
            match hop.url.path() {
                "/fast" => {
                    let n = fast_c.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    (
                        200,
                        html_headers,
                        format!("<html><body>fast v{n}</body></html>"),
                    )
                }
                "/estimated" => {
                    let n = est_c.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
                    (
                        200,
                        html_headers,
                        format!("<html><body>est v{n}</body></html>"),
                    )
                }
                _ => (
                    200,
                    html_headers,
                    "<html><body>default</body></html>".to_owned(),
                ),
            }
        }),
    });
    let engine = engine_with_site(std::sync::Arc::clone(&site));
    let mut handler = |_: &Value| Reply::Records(Vec::new());

    let report = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "adaptive-est-run",
                        "seeds": [
                            { "url": "https://example.test/fast", "label": "fast" },
                            { "url": "https://example.test/estimated", "label": "est" },
                        ],
                        "settings": { "crawl": { "continuous": true } },
                        "labels": {
                            "fast": {
                                "settings": {
                                    "revisit": { "policy": {
                                        "kind": "adaptive", "initial": 10_000, "min": 5_000,
                                        "max": 60_000, "rate_changed": 0.5, "rate_unchanged": 0.5
                                    } }
                                }
                            },
                            "est": {
                                "settings": {
                                    "revisit": { "policy": {
                                        "kind": "estimated", "initial": 10_000, "min": 5_000,
                                        "max": 60_000, "target_freshness": 0.9
                                    } }
                                }
                            }
                        }
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["result"].is_object(), "start failed: {started}");

            for _ in 0..5 {
                let _ = progress_matching(
                    client,
                    "adaptive-est-run",
                    |p| p["idle"] == json!(true),
                    &mut handler,
                )
                .await;
                advance(Duration::from_secs(12)).await;
            }
            let _ = progress_matching(
                client,
                "adaptive-est-run",
                |p| p["idle"] == json!(true),
                &mut handler,
            )
            .await;

            client
                .call(
                    "run.finish",
                    json!({ "run_id": "adaptive-est-run" }),
                    &mut handler,
                )
                .await
        },
    )
    .0;

    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let at_min = report["result"]["revisits_at_min"]
        .as_array()
        .expect("revisits_at_min must be listed in Run report");
    let urls: Vec<&str> = at_min.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        urls,
        vec![
            "https://example.test/estimated",
            "https://example.test/fast",
        ],
        "expected both fast-changing URLs listed in Run report: {report}"
    );
}

/// `Cache-Control: max-age`, `Expires`, and `Retry-After` act as revisit floors
/// by default (winning over `revisit.policy`), and each can be switched off
/// individually under `revisit.site_hints.*` (ADR 0139; issue 1849).
///
/// Covers: setting:revisit.site_hints.cache_control, setting:revisit.site_hints.expires, setting:revisit.site_hints.retry_after
#[expect(
    clippy::too_many_lines,
    reason = "end-to-end Continuous crawl test exercises all three HTTP caching headers and per-header switches"
)]
#[test]
fn site_caching_headers_are_floors_by_default_and_switchable_individually() {
    let env = TestEnv::new();
    let hits = std::sync::Arc::new(std::sync::Mutex::new(std::collections::BTreeMap::<
        String,
        usize,
    >::new()));
    let hits_c = std::sync::Arc::clone(&hits);
    let site = std::sync::Arc::new(ScriptedSite {
        recorded: std::sync::Mutex::new(Vec::new()),
        respond: Box::new(move |hop| {
            let path = hop.url.path().to_owned();
            let n = {
                let mut map = hits_c.lock().unwrap();
                let entry = map.entry(path.clone()).or_insert(0);
                *entry += 1;
                *entry
            };
            let mut headers = vec![("content-type".to_owned(), "text/html".to_owned())];
            match path.as_str() {
                "/cc" | "/off" => {
                    headers.push(("cache-control".to_owned(), "public, max-age=120".to_owned()));
                }
                "/exp" => {
                    headers.push((
                        "date".to_owned(),
                        "Mon, 05 Jan 2026 09:00:00 GMT".to_owned(),
                    ));
                    headers.push((
                        "expires".to_owned(),
                        "Mon, 05 Jan 2026 09:02:00 GMT".to_owned(),
                    ));
                }
                "/ra" => headers.push(("retry-after".to_owned(), "120".to_owned())),
                _ => {}
            }
            (
                200,
                headers,
                format!("<html><body>{path} v{n}</body></html>"),
            )
        }),
    });
    let engine = engine_with_site(std::sync::Arc::clone(&site));
    let mut handler = |_: &Value| Reply::Records(Vec::new());

    let (explained_batch, report) = super::crawl_on(
        &engine,
        || dependencies(&env, RunStateOptions::default()),
        async |client| {
            let explained_batch = client
                .call(
                    "config.explain",
                    json!({
                        "keys": [
                            "revisit.site_hints.cache_control",
                            "revisit.site_hints.expires",
                            "revisit.site_hints.retry_after",
                        ],
                        "settings": {
                            "crawl": { "continuous": false },
                            "revisit": {
                                "site_hints": {
                                    "cache_control": false,
                                    "expires": false,
                                    "retry_after": false,
                                }
                            }
                        },
                    }),
                    &mut handler,
                )
                .await;
            let started = client
                .call(
                    "crawl.start",
                    json!({
                        "run_id": "site-hints-run",
                        "seeds": [
                            { "url": "https://cc.test/cc", "label": "on" },
                            { "url": "https://exp.test/exp", "label": "on" },
                            { "url": "https://ra.test/ra", "label": "on" },
                            { "url": "https://off.test/off", "label": "cc_off" },
                        ],
                        "settings": { "crawl": { "continuous": true } },
                        "labels": {
                            "on": { "settings": { "revisit": { "policy": { "kind": "every", "interval": 10_000 } } } },
                            "cc_off": { "settings": { "revisit": {
                                "policy": { "kind": "every", "interval": 10_000 },
                                "site_hints": { "cache_control": false }
                            } } }
                        }
                    }),
                    &mut handler,
                )
                .await;
            assert!(started["result"].is_object(), "start failed: {started}");
            let _ = progress_matching(client, "site-hints-run", |p| p["idle"] == json!(true), &mut handler).await;
            advance(Duration::from_secs(12)).await;
            let _ = progress_matching(client, "site-hints-run", |p| p["idle"] == json!(true), &mut handler).await;
            let report = client.call("run.finish", json!({ "run_id": "site-hints-run" }), &mut handler).await;
            (explained_batch, report)
        },
    )
    .0;

    for setting in explained_batch["result"]["settings"].as_array().unwrap() {
        assert_eq!(setting["yields_to"]["key"], "crawl.continuous", "{setting}");
    }
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let map = hits.lock().unwrap();
    assert_eq!(map.get("/cc").copied(), Some(1));
    assert_eq!(map.get("/exp").copied(), Some(1));
    assert_eq!(map.get("/ra").copied(), Some(1));
    assert!(
        map.get("/off").copied().unwrap_or(0) >= 2,
        "expected /off to revisit: {map:?}"
    );
}
