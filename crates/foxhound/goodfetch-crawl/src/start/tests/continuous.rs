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
