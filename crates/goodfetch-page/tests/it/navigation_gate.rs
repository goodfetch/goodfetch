//! The page's navigation gate and lease check at each of their call sites
//! (spec #577's `page_value`; ADRs 0087, 0093, 0118 and 0121), through a
//! Recording gate on every page of an in-process engine (ADR 0125), against
//! real Chrome for Testing and the Proving ground. Story 112 and 113 are
//! made tested by Foxhound's cases (A05-66), which drive these interfaces;
//! these tests declare the error they raise.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_engine::lease::LeaseEnded;
use goodfetch_page::page_value::{Kept, NavigationKind};
use goodfetch_page::url_pattern::UrlPattern;
use goodfetch_protocol::page::SitePreference;
use goodfetch_protocol::settings::TIMEOUT_NAVIGATION;
use goodfetch_recording_gate::RecordingGate;
use serde_json::{Map, Value, json};

use crate::handler_page::{Program, RUN, settled, settled_or, with_handler_page};

/// The navigation test whose page links to `/b`.
const HISTORY: &str = "2.navigation.test";

/// The access family's host, whose popup page opens a popup.
const POPUP_HOST: &str = "1.access.test";

/// The fetch edge pages: redirects and refresh stubs.
const FETCH_EDGE: &str = "fetch-edge.test";

/// The held `page.goto`'s own `timeout`, in milliseconds. A call's own
/// timeout bounds that navigation alone; the Run's `timeout.navigation`
/// also bounds each command of the browser's launch and of each page's
/// opening, which a loaded host stretches past a few seconds (fix-queue
/// 121), so the Run keeps its default.
const GOTO_TIMEOUT_MS: u64 = 3_000;

/// How long the gate holds the goto's permit: past `GOTO_TIMEOUT_MS`, so a
/// wait that counted against the timeout would fail the call.
const PAST_THE_GOTO_TIMEOUT: Duration = Duration::from_millis(GOTO_TIMEOUT_MS + 1_000);

/// How long the gate holds the permit of a call that takes no timeout of
/// its own (`page.request`, `page.reload`): past `timeout.navigation`'s
/// default, which bounds it. The Run keeps that default rather than a
/// shorter one, which would bound its browser's launch and page openings
/// below what every other browser test gives them.
fn past_the_navigation_timeout() -> Duration {
    let default = (TIMEOUT_NAVIGATION.default)()
        .as_u64()
        .expect("invariant: `timeout.navigation`'s default is milliseconds");
    Duration::from_millis(default + 1_000)
}

/// Each request the gate admitted from the `from`th on, as its path and
/// kind.
fn admitted_since(gate: &RecordingGate, from: usize) -> Vec<(String, NavigationKind)> {
    gate.admitted()
        .into_iter()
        .skip(from)
        .map(|navigation| {
            let path = url::Url::parse(&navigation.url)
                .map(|url| url.path().to_owned())
                .unwrap_or(navigation.url);
            (path, navigation.kind)
        })
        .collect()
}

/// Runs `step` on `program` and answers what the gate admitted for it once
/// `count` requests were.
async fn admitted_by(
    gate: &RecordingGate,
    program: &mut Program,
    count: usize,
    step: impl AsyncFnOnce(&mut Program),
) -> Vec<(String, NavigationKind)> {
    let from = gate.admitted().len();
    step(program).await;
    settled_or(
        || gate.admitted().len() >= from + count,
        || format!("admitted {:?}", gate.admitted()),
    )
    .await;
    admitted_since(gate, from)
}

/// The gate is asked once for each main-frame navigation, whatever starts
/// it, and once for each redirect or refresh hop and each `page.request`,
/// on every page of its Session, one opened later included.
/// Covers: operation:page.goto, operation:page.reload, operation:page.go_back, operation:locator.click, operation:page.request, operation:run.new_page
#[test]
fn the_gate_admits_each_main_frame_navigation_and_each_hop_once() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        let document = |path: &str| (path.to_owned(), NavigationKind::Document);
        let redirect = |path: &str| (path.to_owned(), NavigationKind::Redirect);

        let goto = admitted_by(&gate, program, 1, async |program| {
            program.goto(HISTORY, "/").await;
        })
        .await;
        let click = admitted_by(&gate, program, 1, async |program| {
            let clicked = program
                .call(
                    "locator.click",
                    &json!({ "locator": { "page_id": program.page, "selector": "#next" } }),
                )
                .await;
            assert!(clicked.get("result").is_some(), "{clicked}");
        })
        .await;
        let back = admitted_by(&gate, program, 1, async |program| {
            program.on_page("page.go_back", json!({})).await;
        })
        .await;
        let reload = admitted_by(&gate, program, 1, async |program| {
            program.on_page("page.reload", json!({})).await;
        })
        .await;
        let hops = admitted_by(&gate, program, 3, async |program| {
            program.goto(FETCH_EDGE, "/redirect/hops/2").await;
        })
        .await;
        let refresh = admitted_by(&gate, program, 2, async |program| {
            program.goto(FETCH_EDGE, "/refresh/meta/0").await;
        })
        .await;
        let script = admitted_by(&gate, program, 2, async |program| {
            program.goto(FETCH_EDGE, "/refresh/script").await;
        })
        .await;
        let request = admitted_by(&gate, program, 1, async |program| {
            let url = program.proving_ground.url(HISTORY, "/b");
            let sent = program.on_page("page.request", json!({ "url": url })).await;
            assert!(sent.get("result").is_some(), "{sent}");
        })
        .await;
        let another_page = admitted_by(&gate, program, 1, async |program| {
            let opened = program
                .call("run.new_page", &json!({ "run_id": RUN }))
                .await;
            program.page = opened["result"]["page_id"]
                .as_str()
                .unwrap_or_else(|| panic!("no page opened: {opened}"))
                .to_owned();
            program.goto(HISTORY, "/b").await;
        })
        .await;
        // Nothing more was asked once each settled.
        let total = gate.admitted().len();
        // A page whose Session registered no response observer keeps none.
        assert_eq!(gate.observed(), Vec::new());

        assert_eq!(
            (
                goto,
                click,
                back,
                reload,
                hops,
                refresh,
                script,
                request,
                another_page,
                total
            ),
            (
                vec![document("/")],
                vec![document("/b")],
                vec![document("/")],
                vec![document("/")],
                vec![
                    document("/redirect/hops/2"),
                    redirect("/redirect/hops/1"),
                    redirect("/redirect/hops/0"),
                ],
                vec![document("/refresh/meta/0"), document("/target")],
                vec![document("/refresh/script"), document("/target")],
                vec![("/b".to_owned(), NavigationKind::Request)],
                vec![document("/b")],
                13,
            )
        );
    });
}

/// The path of each send the gate heard from the `from`th on.
fn sent_since(gate: &RecordingGate, from: usize) -> Vec<String> {
    gate.sent()
        .iter()
        .skip(from)
        .filter_map(|url| path_of(Some(url)))
        .collect()
}

/// The gate hears each request it admitted as it leaves the browser, so
/// a delay counts from the send rather than the admission (ADR 0086): a
/// navigation and a `page.request` once the browser took the command that
/// sends them on, and the Session's first request, which its egress relay
/// answers `407`, again once the browser took the relay's credentials,
/// since it leaves only then.
/// Covers: operation:page.goto, operation:page.request
#[test]
fn the_gate_hears_each_request_it_admitted_as_it_leaves_the_browser() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        let first = sent_since(&gate, 0);
        let from = gate.sent().len();
        program.goto(HISTORY, "/b").await;
        let second = sent_since(&gate, from);
        let from = gate.sent().len();
        let url = program.proving_ground.url(HISTORY, "/c");
        let sent = program.on_page("page.request", json!({ "url": url })).await;
        assert!(sent.get("result").is_some(), "{sent}");
        let request = sent_since(&gate, from);

        assert_eq!(
            (first, second, request),
            (
                vec!["/".to_owned(), "/".to_owned()],
                vec!["/b".to_owned()],
                vec!["/c".to_owned()],
            ),
            "{:?}",
            gate.sent()
        );
    });
}

/// The path of `url`, when it is a URL.
fn path_of(url: Option<&str>) -> Option<String> {
    url.and_then(|url| url::Url::parse(url).ok())
        .map(|url| url.path().to_owned())
}

/// The Error code, the reason and the URL's path of a call's refusal.
fn refusal(answer: &Value) -> (Value, Value, Option<String>) {
    let data = &answer["error"]["data"];
    (
        data["code"].clone(),
        data["reason"].clone(),
        path_of(data["url"].as_str()),
    )
}

/// The first of `lines` whose `url` at `pointer` ends with `suffix`.
fn ending_with(lines: Vec<Value>, pointer: &str, suffix: &str) -> Value {
    lines
        .into_iter()
        .find(|line| {
            line.pointer(pointer)
                .and_then(Value::as_str)
                .is_some_and(|url| url.ends_with(suffix))
        })
        .unwrap_or_default()
}

/// A refusal raises `site_preferences_refused` with its reason from the
/// call that asked: a navigation never leaves the browser, a refused
/// `page.request` answers no response, and a document refused once its
/// response arrived leaves the page on a blank document.
/// Covers: error:site_preferences_refused, operation:page.goto, operation:page.request, operation:page.formats
#[test]
fn a_refusal_raises_site_preferences_refused_from_the_call_that_asked() {
    let gate = RecordingGate::new();
    gate.refuse("/private-goto", SitePreference::RobotsTxt);
    gate.refuse("/private-request", SitePreference::CrawlDelay);
    gate.refuse_documents("/target", SitePreference::UseSignals);
    with_handler_page(&gate, Map::new(), async |program| {
        let goto = program.proving_ground.url(HISTORY, "/private-goto");
        let refused_goto = program.on_page("page.goto", json!({ "url": goto })).await;
        let request = program.proving_ground.url(HISTORY, "/private-request");
        let refused_request = program
            .on_page("page.request", json!({ "url": request }))
            .await;
        let document = program.proving_ground.url(FETCH_EDGE, "/target");
        let refused_document = program
            .on_page("page.goto", json!({ "url": document }))
            .await;
        let left = program
            .on_page("page.formats", json!({ "formats": { "markdown": true } }))
            .await;

        assert_eq!(
            (
                refusal(&refused_goto),
                program.requested(HISTORY, "/private-goto"),
                refusal(&refused_request),
                refused_request.get("result").is_none(),
                program.requested(HISTORY, "/private-request"),
                refusal(&refused_document),
                left["result"]["markdown"].as_str().map(str::trim),
            ),
            (
                (
                    json!("site_preferences_refused"),
                    json!("robots_txt"),
                    Some("/private-goto".to_owned())
                ),
                false,
                (
                    json!("site_preferences_refused"),
                    json!("crawl_delay"),
                    Some("/private-request".to_owned())
                ),
                true,
                false,
                (
                    json!("site_preferences_refused"),
                    json!("use_signals"),
                    Some("/target".to_owned())
                ),
                Some(""),
            )
        );
    });
}

/// A navigation no call waits on, such as a click's, is aborted as a
/// `page.request_failed` naming the refusal, which the Run trace records,
/// and the page keeps its document.
/// Covers: error:site_preferences_refused, operation:locator.click, event:page.request_failed
#[test]
fn a_refused_navigation_no_call_waits_on_is_aborted_and_the_page_keeps_its_document() {
    let gate = RecordingGate::new();
    gate.refuse("/b", SitePreference::RobotsTxt);
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        program.listen("request_failed").await;
        let clicked = program
            .call(
                "locator.click",
                &json!({ "locator": { "page_id": program.page, "selector": "#next" } }),
            )
            .await;
        program
            .until_event("page.request_failed", |request| {
                path_of(request["url"].as_str()).as_deref() == Some("/b")
            })
            .await;
        let aborted = ending_with(program.events("page.request_failed"), "/url", "/b");
        let kept = program
            .on_page("page.formats", json!({ "formats": {} }))
            .await;
        settled(|| ending_with(program.request_log(), "/data/url", "/b") != Value::Null).await;
        let recorded = ending_with(program.request_log(), "/data/url", "/b");

        assert_eq!(
            (
                clicked.get("result").is_some(),
                aborted["failure"]["error"]["code"].clone(),
                aborted["failure"]["error"]["reason"].clone(),
                path_of(kept["result"]["url"].as_str()),
                program.requested(HISTORY, "/b"),
                recorded["data"]["failure"]["error"]["code"].clone(),
            ),
            (
                true,
                json!("site_preferences_refused"),
                json!("robots_txt"),
                Some("/".to_owned()),
                false,
                json!("site_preferences_refused"),
            )
        );
    });
}

/// A refusal no navigation call waits on raises from the waiters armed for
/// it: a `page.wait_for_url` and a `page.wait_for_response` armed for the
/// URL a click navigates to raise `site_preferences_refused` at once,
/// never `deadline_exceeded` at `timeout.wait`.
/// Covers: error:site_preferences_refused, operation:page.wait_for_url, operation:page.wait_for_response
#[test]
fn a_refused_navigation_raises_from_the_waiters_armed_for_it() {
    let gate = RecordingGate::new();
    gate.refuse("/b", SitePreference::RobotsTxt);
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(HISTORY, "/").await;
        let page = program.page.clone();
        let matcher = json!({ "kind": "glob", "pattern": "**/b" });
        let url = program
            .request(
                "page.wait_for_url",
                &json!({ "page_id": page, "url": matcher }),
            )
            .await;
        let response = program
            .request(
                "page.wait_for_response",
                &json!({ "page_id": page, "url": matcher }),
            )
            .await;
        let clicked = program
            .call(
                "locator.click",
                &json!({ "locator": { "page_id": page, "selector": "#next" } }),
            )
            .await;
        let waited_url = program.answer(url).await;
        let waited_response = program.answer(response).await;

        assert_eq!(
            (
                clicked.get("result").is_some(),
                refusal(&waited_url),
                refusal(&waited_response),
            ),
            (
                true,
                (
                    json!("site_preferences_refused"),
                    json!("robots_txt"),
                    Some("/b".to_owned())
                ),
                (
                    json!("site_preferences_refused"),
                    json!("robots_txt"),
                    Some("/b".to_owned())
                ),
            )
        );
    });
}

/// The lease check is asked before each command the page sends, a
/// `page.request`'s included: once the lease ended, a navigation, a
/// `page.request` or a click raises the error the lease check gives (the
/// Recording gate's `call_cancelled`, Foxhound's `attempt_ended`) and
/// nothing reaches the site; that moves the page to `Closing` (spec #594's
/// P31), so its next call raises `closed`.
#[rstest::rstest]
/// Covers: operation:page.goto
#[case::a_navigation("page.goto", "/after-goto")]
/// Covers: operation:page.request
#[case::a_page_request("page.request", "/after-request")]
/// Covers: operation:locator.click
#[case::a_click("locator.click", "/b")]
fn a_page_whose_lease_ended_sends_nothing_more(#[case] method: &str, #[case] path: &str) {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        let before = gate.lease_checks();
        program.goto(HISTORY, "/").await;
        let navigation_checks = gate.lease_checks() - before;
        let before = gate.lease_checks();
        let url = program.proving_ground.url(HISTORY, "/c");
        let sent = program.on_page("page.request", json!({ "url": url })).await;
        assert!(sent.get("result").is_some(), "{sent}");
        let request_checks = gate.lease_checks() - before;

        gate.end_lease(LeaseEnded::Lost);
        let refused = if method == "locator.click" {
            program
                .call(
                    method,
                    &json!({ "locator": { "page_id": program.page, "selector": "#next" } }),
                )
                .await
        } else {
            let url = program.proving_ground.url(HISTORY, path);
            program.on_page(method, json!({ "url": url })).await
        };
        let later = program
            .on_page(
                "page.goto",
                json!({ "url": program.proving_ground.url(HISTORY, "/after-close") }),
            )
            .await;
        let code = |answer: &Value| answer["error"]["data"]["code"].clone();

        assert_eq!(
            (
                navigation_checks > 0,
                request_checks > 0,
                code(&refused),
                code(&later),
                program.requested(HISTORY, path),
                program.requested(HISTORY, "/after-close"),
                program.requested(HISTORY, "/c"),
            ),
            (
                true,
                true,
                json!("call_cancelled"),
                json!("closed"),
                false,
                false,
                true,
            )
        );
    });
}

/// Time spent waiting for a permit counts against no timeout: a navigation
/// held at the gate past its own `timeout` and a `page.request` held past
/// `timeout.navigation` arrive, the lease is held through each wait, and
/// each permit is released once its navigation loaded or its response
/// arrived.
/// Covers: operation:page.goto, operation:page.request
#[test]
fn a_permit_wait_counts_against_no_timeout_and_holds_the_lease() {
    let gate = RecordingGate::new();
    gate.hold("/held-goto", PAST_THE_GOTO_TIMEOUT);
    gate.hold("/held-request", past_the_navigation_timeout());
    with_handler_page(&gate, Map::new(), async |program| {
        let goto = program
            .on_page(
                "page.goto",
                json!({
                    "url": program.proving_ground.url(HISTORY, "/held-goto"),
                    "timeout": GOTO_TIMEOUT_MS,
                }),
            )
            .await;
        let request = program
            .on_page(
                "page.request",
                json!({ "url": program.proving_ground.url(HISTORY, "/held-request") }),
            )
            .await;
        settled_or(
            || gate.released().len() >= 2,
            || format!("goto {goto}, request {request}"),
        )
        .await;
        let waits: Vec<(Option<String>, bool)> = gate
            .waits()
            .into_iter()
            .map(|wait| (path_of(Some(&wait.url)), wait.ended))
            .collect();
        let released: Vec<Option<String>> = gate
            .released()
            .iter()
            .map(|url| path_of(Some(url)))
            .collect();

        assert_eq!(
            (
                goto["result"]["response"]["status"].clone(),
                request["result"]["status"].clone(),
                waits,
                released,
            ),
            (
                json!(404),
                json!(404),
                vec![
                    (Some("/held-goto".to_owned()), true),
                    (Some("/held-request".to_owned()), true),
                ],
                vec![
                    Some("/held-goto".to_owned()),
                    Some("/held-request".to_owned())
                ],
            )
        );
    });
}

/// The response observer Foxhound registers receives each response the
/// engine observes on the page whose URL its patterns match, with its body
/// cut at its byte cap, its whole length and its page named, those a
/// service worker answered included and marked so: one from its Cache
/// Storage, and one it fetched itself. One that matches nothing is never
/// passed to it.
/// Covers: operation:page.goto
#[test]
fn the_response_observer_receives_each_kept_response_with_its_body_a_service_workers_included() {
    let gate = RecordingGate::new();
    gate.observe(Kept {
        urls: vec![
            UrlPattern::glob("**/effects.js").unwrap(),
            UrlPattern::glob("**/cached.json*").unwrap(),
            UrlPattern::glob("**/network.json*").unwrap(),
        ],
        byte_cap: 8,
    });
    with_handler_page(&gate, Map::new(), async |program| {
        let went = program.goto(HISTORY, "/").await;
        program
            .goto("access.localhost", "/service-worker/?case=observed")
            .await;
        let observed_both = |gate: &RecordingGate| {
            let observed = gate.observed();
            ["/cached.json", "/network.json"]
                .iter()
                .all(|path| observed.iter().any(|response| response.url.contains(path)))
        };
        settled(|| observed_both(&gate)).await;
        let mut observed: Vec<(Option<String>, u16, bool, usize)> = gate
            .observed()
            .into_iter()
            .map(|observed| {
                (
                    path_of(Some(&observed.url)),
                    observed.status,
                    observed.from_service_worker,
                    observed.body.as_ref().map_or(0, Vec::len),
                )
            })
            .collect();
        observed.sort();
        let on_its_page = gate
            .observed()
            .iter()
            .all(|observed| observed.page_id.0 == program.page);
        let cached_length = gate
            .observed()
            .iter()
            .find(|observed| observed.url.contains("/cached.json"))
            .map(|observed| observed.body_length);

        assert_eq!(
            (
                went["response"]["status"].clone(),
                observed,
                on_its_page,
                cached_length
            ),
            (
                json!(200),
                vec![
                    (Some("/effects.js".to_owned()), 200, false, 8),
                    (Some("/service-worker/cached.json".to_owned()), 200, true, 8),
                    (
                        Some("/service-worker/network.json".to_owned()),
                        200,
                        true,
                        8
                    ),
                ],
                true,
                // `{"cached":true}`, whole, though its body arrives cut.
                Some(15),
            )
        );
    });
}

/// A response the engine observes is a body it reads itself (ticket #644):
/// past the Run's `network.max_body_size` its body is never read and the
/// observer receives it as unavailable, while one under the limit arrives
/// cut at the byte cap as ever.
/// Covers: setting:network.max_body_size
#[test]
fn an_observed_response_past_network_max_body_size_is_passed_as_unavailable() {
    let gate = RecordingGate::new();
    gate.observe(Kept {
        urls: vec![
            UrlPattern::glob("**/page-events/large.json").unwrap(),
            UrlPattern::glob("**/page-events/data.json").unwrap(),
        ],
        byte_cap: 8,
    });
    let mut settings = Map::new();
    settings.insert("network".to_owned(), json!({ "max_body_size": 16_384 }));
    with_handler_page(&gate, settings, async |program| {
        program.goto("access.test", "/page-events/large.json").await;
        program.goto("access.test", "/page-events").await;
        let observed_both = |gate: &RecordingGate| {
            let observed = gate.observed();
            ["/large.json", "/data.json"]
                .iter()
                .all(|path| observed.iter().any(|response| response.url.contains(path)))
        };
        settled(|| observed_both(&gate)).await;

        let mut observed: Vec<(Option<String>, bool, Option<usize>)> = gate
            .observed()
            .into_iter()
            .map(|observed| {
                (
                    path_of(Some(&observed.url)),
                    observed.body_length > 8,
                    observed.body.as_ref().map(Vec::len),
                )
            })
            .collect();
        observed.sort();
        assert_eq!(
            observed,
            vec![
                (Some("/page-events/data.json".to_owned()), true, Some(8)),
                (Some("/page-events/large.json".to_owned()), false, None),
            ]
        );
    });
}

/// A service worker's request carrying a navigation of a page it controls
/// takes that navigation's permit, the gate asked once, as a service
/// worker's: its `navigate`-mode request (`pass`) and its `cors` refetch of
/// the pending navigation's URL (`refetch`) alike; the page's own
/// interception never sees the navigation the worker answers.
/// Covers: operation:page.reload
#[rstest::rstest]
#[case::its_navigate_mode_request("pass")]
#[case::its_refetch_of_a_pending_navigation("refetch")]
fn a_service_workers_request_carrying_a_navigation_is_admitted_once(#[case] navigations: &str) {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        let page = format!("/service-worker/?navigations={navigations}");
        program.goto("access.localhost", &page).await;
        settled_or(
            || program.reported(&page, "controlled").as_deref() == Some("true"),
            || format!("{:?}", program.reported(&page, "register")),
        )
        .await;
        let reloaded = admitted_by(&gate, program, 1, async |program| {
            let answer = program.on_page("page.reload", json!({})).await;
            assert!(answer.get("result").is_some(), "{answer}");
        })
        .await;
        settled(|| gate.released().iter().any(|url| url.contains(&page))).await;

        assert_eq!(
            reloaded,
            vec![("/service-worker/".to_owned(), NavigationKind::ServiceWorker)]
        );
    });
}

/// A navigation a service worker carries is its page's: its wait for the
/// gate's permit counts against no `timeout.navigation` of the page, and
/// the gate's refusal raises `site_preferences_refused` from the page's
/// `page.reload`, never a retryable `navigation_failed`.
/// Covers: operation:page.reload, error:site_preferences_refused
#[test]
fn a_service_workers_navigation_waits_and_is_refused_as_its_pages() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        let page = "/service-worker/?navigations=pass";
        program.goto("access.localhost", page).await;
        settled_or(
            || program.reported(page, "controlled").as_deref() == Some("true"),
            || format!("{:?}", program.reported(page, "register")),
        )
        .await;
        gate.hold("navigations=pass", past_the_navigation_timeout());
        let held = program.on_page("page.reload", json!({})).await;
        gate.stop_holding("navigations=pass");
        gate.refuse("navigations=pass", SitePreference::RobotsTxt);
        let refused = program.on_page("page.reload", json!({})).await;

        assert_eq!(
            (
                held["result"]["response"]["status"].clone(),
                refusal(&refused),
            ),
            (
                json!(200),
                (
                    json!("site_preferences_refused"),
                    json!("robots_txt"),
                    Some("/service-worker/".to_owned())
                ),
            )
        );
    });
}

/// The gate judges the document a service worker's request carrying a
/// page's navigation brought back, as it judges a page's own: its refusal
/// raises from the page's `page.reload`.
/// Covers: operation:page.reload, error:site_preferences_refused
#[test]
fn a_document_a_service_worker_brought_is_judged_when_it_arrives() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        let page = "/service-worker/?navigations=pass";
        program.goto("access.localhost", page).await;
        settled_or(
            || program.reported(page, "controlled").as_deref() == Some("true"),
            || format!("{:?}", program.reported(page, "register")),
        )
        .await;
        gate.refuse_documents("navigations=pass", SitePreference::UseSignals);
        let refused = program.on_page("page.reload", json!({})).await;

        assert_eq!(
            refusal(&refused),
            (
                json!("site_preferences_refused"),
                json!("use_signals"),
                Some("/service-worker/".to_owned())
            )
        );
    });
}

/// The gate judges a main-frame document a route fetched before the route
/// gets it: `route.fetch` and the navigation raise its refusal, and the
/// page is left on a blank document.
/// Covers: operation:page.goto, operation:route.fetch, error:site_preferences_refused
#[test]
fn a_document_a_route_fetched_is_judged_before_the_route_gets_it() {
    let gate = RecordingGate::new();
    gate.refuse_documents("/target", SitePreference::UseSignals);
    with_handler_page(&gate, Map::new(), async |program| {
        let page = program.page.clone();
        program
            .request(
                "page.route",
                &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/target" } }),
            )
            .await;
        let url = program.proving_ground.url(FETCH_EDGE, "/target");
        let navigating = program
            .request("page.goto", &json!({ "page_id": page, "url": url }))
            .await;
        let handle = program.until_request("route.handle").await;
        let fetched = program
            .call(
                "route.fetch",
                &json!({ "route_id": handle["params"]["route_id"] }),
            )
            .await;
        program.reply(&handle, json!({})).await;
        let navigated = program.answer(navigating).await;
        let left = program
            .on_page("page.formats", json!({ "formats": { "markdown": true } }))
            .await;
        let refused = (
            json!("site_preferences_refused"),
            json!("use_signals"),
            Some("/target".to_owned()),
        );

        assert_eq!(
            (
                refusal(&fetched),
                refusal(&navigated),
                left["result"]["markdown"].as_str().map(str::trim),
            ),
            (refused.clone(), refused, Some("")),
        );
    });
}

/// A popup a handler's page opens takes the page's value (A05-26): its
/// first navigation is admitted by the opener's gate before it leaves, and
/// once the lease ended a call on the popup raises its lease check's error and
/// nothing more reaches the site.
/// Covers: event:page.popup, operation:locator.click, operation:page.goto
#[test]
fn a_popup_takes_its_openers_gate_and_lease_check() {
    let gate = RecordingGate::new();
    with_handler_page(&gate, Map::new(), async |program| {
        program.goto(POPUP_HOST, "/popup").await;
        program.listen("popup").await;
        let opened = admitted_by(&gate, program, 1, async |program| {
            let clicked = program
                .call(
                    "locator.click",
                    &json!({ "locator": { "page_id": program.page, "selector": "#open" } }),
                )
                .await;
            assert!(clicked.get("result").is_some(), "{clicked}");
        })
        .await;
        let opener_page = program.page.clone();
        program
            .until_event("page.popup", |popup| popup["page_id"] == json!(opener_page))
            .await;
        let popup = program.events("page.popup")[0]["popup"]["page_id"].clone();

        gate.end_lease(LeaseEnded::Lost);
        let after = program
            .call(
                "page.goto",
                &json!({
                    "page_id": popup,
                    "url": program.proving_ground.url(POPUP_HOST, "/popup/after-lease"),
                }),
            )
            .await;

        assert_eq!(
            (
                opened,
                after["error"]["data"]["code"].clone(),
                program.requested(POPUP_HOST, "/popup/after-lease"),
            ),
            (
                vec![("/popup/opened".to_owned(), NavigationKind::Document)],
                json!("call_cancelled"),
                false,
            )
        );
    });
}

/// The gate is handed the body of the document it judges only when it
/// reads one: for a gate that reads none, the page takes no body from the
/// browser (the audit #1884's C5-4).
/// Covers: operation:page.goto
#[rstest::rstest]
fn a_document_is_judged_with_its_body_only_by_a_gate_that_reads_it(
    #[values(false, true)] reads: bool,
) {
    let gate = RecordingGate::new();
    if reads {
        gate.read_bodies();
    }
    with_handler_page(&gate, Map::new(), async |program| {
        let document = program.proving_ground.url(FETCH_EDGE, "/target");
        let went = program
            .on_page("page.goto", json!({ "url": document }))
            .await;
        assert!(went.get("result").is_some(), "{went}");
    });

    let judged: Vec<(bool, bool)> = gate
        .judged()
        .iter()
        .zip(gate.judged_bodies())
        .filter(|(url, _)| url.ends_with("/target"))
        .map(|(_, length)| (reads, length > 0))
        .collect();
    assert_eq!(judged, vec![(reads, reads)]);
}
