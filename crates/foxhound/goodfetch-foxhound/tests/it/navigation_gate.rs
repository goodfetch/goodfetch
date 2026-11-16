//! The navigation gate Foxhound sets on a handler's pages (spec #579's
//! `gate`; ADRs 0086, 0087, 0093, 0118 and 0121), on Foxhound's executable
//! against the Proving ground and Chrome for Testing: a page the handler
//! opens in its crawl's Run, and every page opened from it, takes a permit
//! of its domain's Politeness for each navigation, each redirect hop and
//! each `page.request`, and is refused with `site_preferences_refused`
//! where the site's robots.txt, read through the copy the user's runs
//! share, disallows it for `goodfetch-foxhound` or asks for a Crawl-delay
//! above `site_preferences.max_crawl_delay`; a refusal the handler lets
//! escape ends its request `skipped` with the refusal's reason, and eight
//! handlers at once press their one domain a delay apart. Its lease check
//! is the work item's lease: past it, the handler's page sends nothing more.
#![expect(
    clippy::disallowed_methods,
    reason = "the handler, the test's own code, waits in real time for the popup the browser navigates; its SQL statements are its own, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "the test's proxy thread, outside the engine, says it held a request (ADR 0060)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use goodfetch_test_client::Attached;
use goodfetch_test_support::{
    BROWSERS_PATH_VARIABLE, DISPATCH_TARGET, ProvingGround, TestEnv, browser_cache, proxy_in_front,
};
use rusqlite::{Connection, OpenFlags};
use serde_json::{Value, json};

use crate::frontier::{Inside, crawl_with};
use crate::{attached_with, before_their_turn};

/// The Proving ground's seed.
const SEED: u64 = 760;

/// The `access-policy` family's robots.txt host: its `*` group disallows
/// `/private/` and asks for a Crawl-delay of 2 s.
const ROBOTS_HOST: &str = "1.access-policy.test";

/// The governor's floors in each case's home, above what a case holds at
/// once: its attempts and its pages are admitted whatever the host's load.
/// A governor above normal admits none past a kind's floor (ADR 0040), so
/// on a loaded host a handler's `run.new_page` would wait for its admission
/// while its lease, which nothing renews then, runs out.
const ADMITTED_WHATEVER_THE_LOAD: &str =
    "governor.http.floor = 16\ngovernor.browser_page.floor = 16\n";

/// An engine whose pages launch Chrome for Testing from the test's cache,
/// its governor admitting each case's attempts and pages at once.
fn engine(env: &TestEnv) -> Attached {
    env.user_configuration(ADMITTED_WHATEVER_THE_LOAD);
    let cache = browser_cache(env).expect("Playwright's Chrome for Testing is installed");
    // A navigation may wait out its domain's delay with nothing to say.
    attached_with(env, Some(Duration::from_secs(60)), |command| {
        command.env(BROWSERS_PATH_VARIABLE, &cache);
    })
}

/// The settings of a crawl under `fetch.tier: http` that reaches every
/// host of `proving_ground`, with `more` merged in.
fn settings(proving_ground: &ProvingGround, more: &Value) -> Value {
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    let mut settings = json!({
        "fetch": { "tier": "http" },
        "network": { "resolve": resolve },
        // The handler's pages reach the Proving ground through the egress
        // relay, which judges a mapping's answers as any other.
        "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
    });
    if let (Some(settings), Some(more)) = (settings.as_object_mut(), more.as_object()) {
        settings.extend(more.clone());
    }
    settings
}

/// Opens a page in the handler's Run, answering its ID.
fn new_page(inside: &mut Inside<'_>, handle: &Value) -> String {
    let opened = inside.call("run.new_page", &json!({ "run_id": handle["run_id"] }));
    opened["result"]["page_id"]
        .as_str()
        .unwrap_or_else(|| panic!("no page opened: {opened}"))
        .to_owned()
}

/// The Error code, the reason and the URL of a call's refusal.
fn refusal(answer: &Value) -> (Value, Value, Value) {
    let data = &answer["error"]["data"];
    (
        data["code"].clone(),
        data["reason"].clone(),
        data["url"].clone(),
    )
}

/// How many requests for `path` the Proving ground received on `host`.
fn requested(proving_ground: &ProvingGround, host: &str, path: &str) -> usize {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == host && request.target == path)
        .count()
}

/// A handler's `goto` to a page the site's robots.txt disallows for
/// `goodfetch-foxhound` raises `site_preferences_refused` with `robots_txt`
/// and never reaches the site, while one it allows loads.
///
/// Covers: story:5-134, error:site_preferences_refused, operation:page.goto
#[test]
fn a_handlers_goto_robots_txt_disallows_is_refused_before_it_leaves() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let disallowed = proving_ground.url(ROBOTS_HOST, "/private/catalog/2");
    let allowed = proving_ground.url(ROBOTS_HOST, "/catalog/3");

    let mut answers = Vec::new();
    let report = crawl_with(
        &mut engine,
        "robots",
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &json!({})),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            for url in [&disallowed, &allowed] {
                answers.push(inside.call("page.goto", &json!({ "page_id": page_id, "url": url })));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            refusal(&answers[0]),
            answers[1]["result"]["response"]["status"].clone(),
            requested(&proving_ground, ROBOTS_HOST, "/private/catalog/2"),
        ),
        (
            &json!("finished"),
            (
                json!("site_preferences_refused"),
                json!("robots_txt"),
                json!(disallowed)
            ),
            json!(200),
            0,
        ),
        "{answers:?}"
    );
}

/// A `page.request` robots.txt disallows answers no response and raises
/// `site_preferences_refused` with `robots_txt`; a click on a link it
/// disallows leaves the page on its document, the refusal raising from the
/// wait armed for that navigation; neither reaches the site.
///
/// Covers: story:5-134, error:site_preferences_refused, operation:page.request, operation:locator.click, operation:page.evaluate, operation:page.wait_for_url
#[test]
fn a_disallowed_request_answers_nothing_and_a_disallowed_click_leaves_the_page() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let requested_url = proving_ground.url(ROBOTS_HOST, "/private/catalog/3");

    let mut answers = Vec::new();
    let report = crawl_with(
        &mut engine,
        "refusals",
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/2"), "label": "page" })],
        &settings(&proving_ground, &json!({})),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            let page = proving_ground.url(ROBOTS_HOST, "/catalog/1");
            inside.call("page.goto", &json!({ "page_id": page_id, "url": page }));
            answers.push(inside.call(
                "page.request",
                &json!({ "page_id": page_id, "url": requested_url }),
            ));
            // A wait armed before the click hears the gate refuse the
            // navigation the click starts, so the page is read once the
            // refusal came, not after a guessed while.
            let engine = inside.engine();
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "refused", "method": "page.wait_for_url",
                "params": {
                    "page_id": page_id,
                    "url": { "kind": "glob", "pattern": "**/private/catalog/2" },
                },
            }));
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "click", "method": "locator.click",
                "params": { "locator": { "page_id": page_id, "selector": "a.next" } },
            }));
            let mut others = Vec::new();
            let clicked = answer_to(engine, "click", &mut others);
            let refused = match others.iter().position(|other| other["id"] == "refused") {
                Some(at) => others.remove(at),
                None => answer_to(engine, "refused", &mut others),
            };
            assert!(others.is_empty(), "{others:?}");
            answers.push(clicked);
            answers.push(refused);
            answers.push(inside.call(
                "page.evaluate",
                &json!({ "page_id": page_id, "function": "() => location.pathname" }),
            ));
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            refusal(&answers[0]),
            answers[0].get("result").is_none(),
            answers[1].get("result").is_some(),
            refusal(&answers[2]).1,
            &answers[3]["result"]["value"],
            requested(&proving_ground, ROBOTS_HOST, "/private/catalog/3"),
            requested(&proving_ground, ROBOTS_HOST, "/private/catalog/2"),
        ),
        (
            &json!("finished"),
            (
                json!("site_preferences_refused"),
                json!("robots_txt"),
                json!(requested_url)
            ),
            true,
            true,
            json!("robots_txt"),
            &json!("/catalog/1"),
            0,
            0,
        ),
        "{answers:?}"
    );
}

/// The navigation test whose page links to `/b` with `#next`.
const HISTORY: &str = "2.navigation.test";

/// The fetch edge pages: redirects and a script's redirect.
const FETCH_EDGE: &str = "fetch-edge.test";

/// The access family's host, whose popup page opens a popup with `#open`.
const POPUP_HOST: &str = "1.access.test";

/// The delay between starts the gate's cases set: long enough that two
/// requests of one domain sent without it would arrive closer.
const DELAY_MILLISECONDS: u64 = 1_000;

/// Each request `host` received for one of `targets`, its target and
/// arrival time in microseconds, in arrival order.
fn arrivals(proving_ground: &ProvingGround, host: &str, targets: &[&str]) -> Vec<(String, u64)> {
    let mut arrivals: Vec<(String, u64)> = proving_ground
        .received_requests()
        .unwrap()
        .every()
        .filter(|request| request.host == host)
        .filter_map(|request| {
            let target = request.target.as_str();
            targets
                .contains(&target)
                .then(|| (target.to_owned(), request.timing.arrived_microseconds))
        })
        .collect();
    arrivals.sort_by_key(|(_, at)| *at);
    arrivals
}

/// The lines of the engine's log of the test's home, across its files, in
/// the order of their times.
fn engine_log(env: &TestEnv) -> Vec<Value> {
    let mut lines: Vec<Value> = std::fs::read_dir(env.home().join("logs"))
        .unwrap()
        .flat_map(|file| {
            std::fs::read_to_string(file.unwrap().path())
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str(line).unwrap())
                .collect::<Vec<Value>>()
        })
        .collect();
    lines.sort_by_cached_key(|line| {
        line["timestamp"]
            .as_str()
            .and_then(|at| at.parse::<jiff::Timestamp>().ok())
    });
    lines
}

/// When the navigation gate admitted each request of `domain_key`, by the
/// engine's log.
fn admitted(env: &TestEnv, domain_key: &str) -> Vec<jiff::Timestamp> {
    engine_log(env)
        .iter()
        .filter(|line| {
            line["target"] == json!("goodfetch::navigation_gate")
                && line["fields"]["domain_key"] == json!(domain_key)
        })
        .map(|line| line["timestamp"].as_str().unwrap().parse().unwrap())
        .collect()
}

/// How many of `admitted` the gate admitted before their turn after `from`,
/// a moment before the first of them: the n-th no sooner than n - 1 delays
/// after it, to the millisecond its starts are kept in. Two neighbours prove
/// nothing: the gate takes each start ahead of its wait, so the first's late
/// wake can leave the next closer than the delay by as much.
fn admitted_too_soon(
    admitted: &[jiff::Timestamp],
    from: jiff::Timestamp,
    delay: Duration,
) -> usize {
    let delay = i64::try_from(delay.as_millis()).unwrap();
    admitted
        .iter()
        .zip(0..)
        .filter(|(at, places)| at.as_millisecond() < from.as_millisecond() + places * delay)
        .count()
}

/// What the site `host` and the navigation gate saw of its domain `domain_key`
/// under the delay, for a site that was asked for `targets`, its robots.txt
/// first: how many of them arrived, each once; whether robots.txt did not come
/// first or a later one reached the site before its turn after it
/// ([`before_their_turn`]); and how many requests the gate admitted.
fn permits_of(
    env: &TestEnv,
    proving_ground: &ProvingGround,
    (host, domain_key): (&str, &str),
    targets: &[&str],
) -> (usize, bool, usize) {
    let arrived = arrivals(proving_ground, host, targets);
    let arrived_once = targets
        .iter()
        .filter(|target| {
            arrived
                .iter()
                .filter(|(arrived, _)| arrived == *target)
                .count()
                == 1
        })
        .count();
    let arrived_too_soon = arrived
        .first()
        .is_some_and(|(first, _)| first != "/robots.txt")
        || !before_their_turn(&arrived, DELAY_MILLISECONDS * 1_000).is_empty();
    (
        arrived_once,
        arrived_too_soon,
        admitted(env, domain_key).len(),
    )
}

/// A URL matcher for a page's waits: the glob `pattern`.
fn glob(pattern: &str) -> Value {
    json!({ "kind": "glob", "pattern": pattern })
}

/// A handler's `goto`, a click that navigates, a `page.request`, each
/// redirect hop, a script's redirect and a popup's navigation each take a
/// permit of their domain's Politeness: under `politeness.domain.delay`, the
/// gate admits every one of them at least the delay after the one before it
/// there, each reaches the site once, and the first after the robots.txt read
/// for their host reaches it at least the delay after that read.
///
/// Covers: story:5-134, story:5-112, story:5-113, setting:politeness.domain.delay, operation:page.goto, operation:locator.click, operation:page.request, operation:page.wait_for_url
#[test]
fn every_navigation_of_a_handlers_pages_takes_a_permit_of_its_domain() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let more = json!({ "politeness": { "domain": { "delay": DELAY_MILLISECONDS } } });

    let mut failed = Vec::new();
    let report = crawl_with(
        &mut engine,
        "permits",
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            let url = |host: &str, path: &str| json!({ "url": proving_ground.url(host, path) });
            let click =
                |selector: &str| json!({ "locator": { "page_id": page_id, "selector": selector } });
            let steps = [
                ("page.goto", url(HISTORY, "/")),
                ("locator.click", click("#next")),
                ("page.wait_for_url", json!({ "url": glob("**/b") })),
                ("page.request", url(HISTORY, "/c")),
                ("page.goto", url(FETCH_EDGE, "/redirect/hops/2")),
                ("page.goto", url(FETCH_EDGE, "/refresh/script")),
                ("page.wait_for_url", json!({ "url": glob("**/target") })),
                ("page.goto", url(POPUP_HOST, "/popup")),
                ("locator.click", click("#open")),
            ];
            failed = steps_on_the_page(inside, &page_id, steps);
            // The popup navigates on its own, once its permit comes: the
            // handler answers once it did, while its lease still holds.
            let until = Instant::now() + Duration::from_secs(10);
            while requested(&proving_ground, POPUP_HOST, "/popup/opened") == 0
                && Instant::now() < until
            {
                std::thread::sleep(Duration::from_millis(100));
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let history = permits_of(
        &env,
        &proving_ground,
        (HISTORY, "navigation.test"),
        &["/robots.txt", "/", "/b", "/c"],
    );
    let edge = permits_of(
        &env,
        &proving_ground,
        (FETCH_EDGE, "fetch-edge.test"),
        &[
            "/robots.txt",
            "/redirect/hops/2",
            "/redirect/hops/1",
            "/redirect/hops/0",
            "/refresh/script",
            "/target",
        ],
    );
    let popup = permits_of(
        &env,
        &proving_ground,
        (POPUP_HOST, "access.test"),
        &["/robots.txt", "/popup", "/popup/opened"],
    );
    assert_eq!(
        (&report["result"]["lifecycle"], failed, history, edge, popup),
        (
            &json!("finished"),
            Vec::<Value>::new(),
            (4, false, 3),
            (6, false, 5),
            (3, false, 2),
        )
    );
}

/// Calls each of `steps` on the page `page_id`, the page named in each but a
/// click's, whose locator names it: answers the calls that failed.
fn steps_on_the_page<const STEPS: usize>(
    inside: &mut Inside<'_>,
    page_id: &str,
    steps: [(&str, Value); STEPS],
) -> Vec<Value> {
    let mut failed = Vec::new();
    for (method, mut params) in steps {
        if method != "locator.click" {
            params["page_id"] = json!(page_id);
        }
        let answer = inside.call(method, &params);
        if answer.get("result").is_none() {
            failed.push(answer);
        }
    }
    failed
}

/// How long [`a_navigation_slow_to_reach_its_site_takes_the_next_ones_permit_no_sooner`]
/// holds the first navigation on its way: short of the delay, so the next
/// one's permit could come early only if the hold moved it.
const SLOW_ON_ITS_WAY: Duration = Duration::from_millis(300);

/// A click's request stays on its way to the site, here in the Session's
/// proxy, long after its send, and that moves no permit: the gate holds the
/// delay between sends (ADR 0086), so it admits the next request the delay
/// after the one before, and each still reaches the site no sooner than its
/// turn. The test names the hold it forces. How close the two arrive is not
/// judged: it also holds how soon the handler asked, which no figure bounds on
/// a loaded host.
///
/// Covers: story:5-134, setting:politeness.domain.delay, operation:locator.click, operation:page.request
#[test]
fn a_navigation_slow_to_reach_its_site_takes_the_next_ones_permit_no_sooner() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let held = Arc::new(AtomicBool::new(false));
    let holding = Arc::clone(&held);
    let front = proxy_in_front(
        proving_ground.description().fixtures.proxies["open"].clone(),
        move |line| {
            if line.contains("/b HTTP/") {
                std::thread::sleep(SLOW_ON_ITS_WAY);
                holding.store(true, Ordering::SeqCst);
            }
        },
    );
    let more = json!({
        "politeness": { "domain": { "delay": DELAY_MILLISECONDS } },
        "proxy": { "server": format!("http://{front}") },
    });

    let mut failed = Vec::new();
    let report = crawl_with(
        &mut engine,
        "slow-way",
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            let steps = [
                (
                    "page.goto",
                    json!({ "url": proving_ground.url(HISTORY, "/") }),
                ),
                (
                    "locator.click",
                    json!({ "locator": { "page_id": page_id, "selector": "#next" } }),
                ),
                ("page.wait_for_url", json!({ "url": glob("**/b") })),
                (
                    "page.request",
                    json!({ "url": proving_ground.url(HISTORY, "/c") }),
                ),
            ];
            failed = steps_on_the_page(inside, &page_id, steps);
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let history = permits_of(
        &env,
        &proving_ground,
        (HISTORY, "navigation.test"),
        &["/robots.txt", "/", "/b", "/c"],
    );
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            failed,
            held.load(Ordering::SeqCst),
            history
        ),
        (&json!("finished"), Vec::<Value>::new(), true, (4, false, 3))
    );
}

/// The answer to the call `id` `engine` sends next, the requests and
/// replies before it kept in `others`.
fn answer_to(engine: &mut Attached, id: &str, others: &mut Vec<Value>) -> Value {
    loop {
        let message = engine.next().expect("the engine answers");
        if message["id"] == id {
            return message;
        }
        others.push(message);
    }
}

/// How long the permit-wait case waits between two reads of the Run's live
/// status.
const PROGRESS_POLL: Duration = Duration::from_millis(50);

/// A handler whose `goto` waits for its domain's delay between starts is
/// not timed out meanwhile: the wait lasts longer than the attempt's lease
/// was granted for, the lease is renewed through it, the Run's live status
/// names it, and the attempt then commits, never sent again. The handler
/// opens its page within the lease, which nothing renews before its `goto`
/// waits, so a first seed's handler starts the Run's browser before it
/// ([`warm_up_the_browser`]).
///
/// Covers: story:5-134, setting:timeout.handler, setting:politeness.domain.delay, operation:run.progress
#[test]
fn a_handler_waiting_for_a_permit_is_not_timed_out_and_its_wait_is_named() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    // A lease of 1 s plus the 10 s margin, shorter than the delay.
    let more = json!({
        "timeout": { "handler": 1_000 },
        "politeness": { "domain": { "delay": 12_000 } },
    });
    let warm_up_seed = proving_ground.url(HISTORY, "/");

    let (mut phases, mut went, mut others) = (Vec::new(), None, Vec::new());
    let (mut handled, mut warmed) = (0, false);
    let report = crawl_with(
        &mut engine,
        "waits",
        &[
            json!({ "url": warm_up_seed, "label": "page" }),
            json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" }),
        ],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            if handle["crawl_request"]["url"] == warm_up_seed.as_str() {
                if !warmed {
                    warmed = true;
                    warm_up_the_browser(inside, handle);
                }
                return json!({ "result": { "records": [] } });
            }
            handled += 1;
            if handled > 1 {
                return json!({ "result": { "records": [] } });
            }
            let page_id = new_page(inside, handle);
            let url = proving_ground.url(ROBOTS_HOST, "/catalog/3");
            let engine = inside.engine();
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "goto", "method": "page.goto",
                "params": { "page_id": page_id, "url": url },
            }));
            // The live status is read until it names the wait, within a
            // deadline shorter than the delay the `goto` waits out.
            let deadline = Instant::now() + Duration::from_secs(10);
            for poll in 0.. {
                let id = format!("progress-{poll}");
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": id, "method": "run.progress",
                    "params": { "run_id": handle["run_id"] },
                }));
                let progress = answer_to(engine, &id, &mut others);
                phases = progress["result"]["in_flight"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|request| request["phase"].clone())
                    .collect();
                if phases.contains(&json!("waiting_for_permit")) || Instant::now() >= deadline {
                    break;
                }
                std::thread::sleep(PROGRESS_POLL);
            }
            went = Some(answer_to(engine, "goto", &mut others));
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (
            &report["result"]["lifecycle"],
            handled,
            phases,
            went.map(|went| went["result"]["response"]["status"].clone()),
            others,
        ),
        (
            &json!("finished"),
            1,
            vec![json!("waiting_for_permit")],
            Some(json!(200)),
            Vec::<Value>::new(),
        )
    );
}

/// The host of the service worker page, a secure context over plain HTTP.
const SERVICE_WORKER_HOST: &str = "access.localhost";

/// The domain key of [`SERVICE_WORKER_HOST`]'s requests: not the crawl
/// request's, so each navigation of its page takes a Permit of its own.
const SERVICE_WORKER_KEY: &str = SERVICE_WORKER_HOST;

/// The delay between starts the service worker cases set.
const SERVICE_WORKER_DELAY: Duration = Duration::from_secs(3);

/// What the navigation gate took of a domain key in the Politeness file
/// (ADR 0086).
#[derive(Debug, PartialEq, Eq)]
struct Taken {
    /// How many Permits of the key were taken: those held, and those given
    /// back, which the file counts as it deletes each.
    permits: i64,
    /// The key's last start, in milliseconds since the epoch: every
    /// request of the key the gate admits takes or records one.
    last_start: Option<i64>,
}

/// What the gate took of `key` so far in the Politeness file of `env`'s
/// home, read in one statement. A give-back deletes its Permit and counts
/// it in one transaction, so a Permit counts once whether it is still held
/// or was given back.
fn taken(env: &TestEnv, key: &str) -> Taken {
    let file = Connection::open_with_flags(
        env.home().join("politeness.sqlite"),
        OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    file.busy_timeout(Duration::from_secs(10)).unwrap();
    file.query_row(
        "SELECT (SELECT count(*) FROM permits WHERE key = ?1)
             + coalesce((SELECT generation FROM returns WHERE key = ?1), 0),
             (SELECT last_start FROM domain_limits WHERE key = ?1)",
        [key],
        |row| {
            Ok(Taken {
                permits: row.get(0)?,
                last_start: row.get(1)?,
            })
        },
    )
    .unwrap()
}

/// Crawls one page, its handler going to the service worker page whose
/// worker treats navigations as `navigations` says, waiting until the
/// worker controls it, then reloading it. Answers what the gate had taken
/// of [`SERVICE_WORKER_KEY`] right before the reload and once it answered,
/// how many requests for the page arrived, how many the gate admitted of the
/// key's, and how many of those it admitted before their turn after the
/// crawl's start ([`admitted_too_soon`]).
fn reloaded_under_a_service_worker(navigations: &str) -> ((Taken, Taken), usize, usize, usize) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let page = format!("/service-worker/?navigations={navigations}");
    let more = json!({
        "politeness": { "domain": { "delay": SERVICE_WORKER_DELAY.as_millis() } },
    });

    let mut going = None;
    let mut reload = None;
    let report = crawl_with(
        &mut engine,
        navigations,
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            let url = proving_ground.url(SERVICE_WORKER_HOST, &page);
            going = Some(jiff::Timestamp::now());
            let went = inside.call("page.goto", &json!({ "page_id": page_id, "url": url }));
            assert_eq!(went["result"]["response"]["status"], 200, "{went}");
            let title = json!({ "page_id": page_id, "function": "() => document.title" });
            let until = Instant::now() + Duration::from_secs(20);
            while inside.call("page.evaluate", &title)["result"]["value"] != "done" {
                assert!(
                    Instant::now() < until,
                    "the worker never controlled the page"
                );
                std::thread::sleep(Duration::from_millis(100));
            }
            // The gate takes a navigation's Permit and start before its
            // request leaves, and the reload answers once its document
            // arrived: what the reload took is in the file by then.
            let before = taken(&env, SERVICE_WORKER_KEY);
            let reloaded = inside.call("page.reload", &json!({ "page_id": page_id }));
            reload = Some((before, taken(&env, SERVICE_WORKER_KEY)));
            assert_eq!(reloaded["result"]["response"]["status"], 200, "{reloaded}");
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let arrived = arrivals(&proving_ground, SERVICE_WORKER_HOST, &[page.as_str()]).len();
    let admitted = admitted(&env, SERVICE_WORKER_KEY);
    (
        reload.unwrap(),
        arrived,
        admitted.len(),
        admitted_too_soon(&admitted, going.unwrap(), SERVICE_WORKER_DELAY),
    )
}

/// A service worker's request carrying a navigation of a handler's page,
/// its `navigate`-mode request or its `cors` refetch of the pending
/// navigation's URL, takes that navigation's permit, once: the reload takes
/// one Permit of its domain, and the gate admits it no sooner than the delay
/// between starts after the page's first load.
///
/// Covers: story:5-134, story:5-112, operation:page.reload
#[rstest::rstest]
#[case::its_navigate_mode_request("pass")]
#[case::its_refetch_of_a_pending_navigation("refetch")]
fn a_service_workers_navigation_request_takes_the_permit(#[case] navigations: &str) {
    let ((before, after), arrived, admitted, too_soon) =
        reloaded_under_a_service_worker(navigations);

    assert_eq!(
        (after.permits - before.permits, arrived, admitted, too_soon),
        (1, 2, 2, 0),
        "{before:?} {after:?}"
    );
}

/// A navigation a service worker answers from its own cache sends nothing
/// to the site and takes no permit: the reload neither reaches the site
/// nor takes a Permit or a start of its domain, whose first load took one.
///
/// Covers: story:5-134, operation:page.reload
#[test]
fn a_navigation_a_service_worker_answers_from_its_cache_takes_no_permit() {
    let ((before, after), arrived, _, _) = reloaded_under_a_service_worker("cache");

    assert_eq!(
        (arrived, before.permits, &after),
        (1, 1, &before),
        "{before:?} {after:?}"
    );
}

/// The host whose robots.txt has a group for `Goodfetch-Foxhound`
/// (`/foxhound/`), for `operator-bot` (`/operator/`), for a bare `Foxhound`
/// (`/bare/`) and for `*` (`/star/`).
const TOKENS_HOST: &str = "tokens.access-policy.test";

/// The host whose robots.txt has a group for a bare `Foxhound` (`/bare/`)
/// and for `*` (`/star/`).
const STAR_HOST: &str = "star.access-policy.test";

/// The pages of the two token hosts a handler's `goto` reaches.
const TOKEN_PAGES: [(&str, &str); 6] = [
    (TOKENS_HOST, "/foxhound/page"),
    (TOKENS_HOST, "/operator/page"),
    (TOKENS_HOST, "/bare/page"),
    (TOKENS_HOST, "/star/page"),
    (STAR_HOST, "/bare/page"),
    (STAR_HOST, "/star/page"),
];

/// Crawls one page as `run_id` under `more`, its handler going to each of
/// [`TOKEN_PAGES`]; answers, for each, whether robots.txt refused it.
fn token_refusals(run_id: &str, more: &Value) -> Vec<bool> {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let mut refused = Vec::new();
    let report = crawl_with(
        &mut engine,
        run_id,
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, more),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            for (host, path) in TOKEN_PAGES {
                let url = proving_ground.url(host, path);
                let went = inside.call("page.goto", &json!({ "page_id": page_id, "url": url }));
                refused.push(match refusal(&went) {
                    (code, reason, _) if code == "site_preferences_refused" => {
                        assert_eq!(reason, "robots_txt", "{went}");
                        true
                    }
                    _ => {
                        assert_eq!(went["result"]["response"]["status"], 200, "{went}");
                        false
                    }
                });
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    refused
}

/// A handler's navigation obeys the robots.txt group for
/// `goodfetch-foxhound`, written in any case, merged with the group for
/// `site_preferences.robots_txt_user_agent`; a group naming only a bare
/// `Foxhound` applies nothing, and the `*` group applies only where
/// neither of the first two has a group.
///
/// Covers: story:5-131, story:5-134, setting:site_preferences.robots_txt_user_agent, error:site_preferences_refused
#[test]
fn a_handlers_navigation_obeys_foxhounds_group_merged_with_the_operators_else_stars() {
    let with_operator = token_refusals(
        "operator",
        &json!({ "site_preferences": { "robots_txt_user_agent": "operator-bot" } }),
    );
    let without = token_refusals("foxhound", &json!({}));

    assert_eq!(
        (with_operator, without),
        (
            vec![true, true, false, false, false, true],
            vec![true, false, false, false, false, true],
        )
    );
}

/// The host whose robots.txt asks every crawler for a Crawl-delay of 5 s.
const CRAWL_DELAY_HOST: &str = "crawl-delay.access-policy.test";

/// A handler's `goto` that the site's preferences refuse raises
/// `site_preferences_refused` with its reason and never reaches the site:
/// a page robots.txt disallows, with `robots_txt`, and a host asking for a
/// Crawl-delay above `site_preferences.max_crawl_delay`, with
/// `crawl_delay`. The handler lets the refusal escape, and its request
/// ends `skipped` with that reason, never retried.
///
/// Covers: story:6-244, error:site_preferences_refused, operation:page.goto, operation:run.outcomes
#[rstest::rstest]
/// Covers: setting:site_preferences.respect_robots_txt
#[case::robots_txt(ROBOTS_HOST, "/private/catalog/2", "robots_txt")]
/// Covers: setting:site_preferences.max_crawl_delay
#[case::crawl_delay(CRAWL_DELAY_HOST, "/catalog/1", "crawl_delay")]
fn a_handlers_refused_goto_escaping_it_ends_the_request_skipped_with_its_reason(
    #[case] host: &str,
    #[case] path: &str,
    #[case] reason: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let refused = proving_ground.url(host, path);
    let more = json!({ "site_preferences": { "max_crawl_delay": 3_000 } });

    let (mut went, mut handled) = (Value::Null, 0);
    let report = crawl_with(
        &mut engine,
        "escapes",
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            handled += 1;
            let page_id = new_page(inside, handle);
            went = inside.call("page.goto", &json!({ "page_id": page_id, "url": refused }));
            json!({ "error": went["error"] })
        },
    );
    let outcomes = engine.call(
        "outcomes",
        "run.outcomes",
        &json!({ "run_id": "escapes" }),
        &mut |_| None,
    );
    assert!(engine.finished());

    let outcome = &outcomes["result"]["outcomes"][0];
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            refusal(&went),
            handled,
            (&outcome["kind"], &outcome["reason"], &outcome["attempts"]),
            requested(&proving_ground, host, path),
        ),
        (
            &json!("finished"),
            (
                json!("site_preferences_refused"),
                json!(reason),
                json!(refused)
            ),
            1,
            (&json!("skipped"), &json!(reason), &json!(1)),
            0,
        ),
        "{went} {outcomes}"
    );
}

/// With `site_preferences.respect_robots_txt` off, a handler's `goto` to a
/// page robots.txt disallows, or to a host asking for a Crawl-delay above
/// `site_preferences.max_crawl_delay`, loads: the gate reads no robots.txt.
///
/// Covers: story:6-244, setting:site_preferences.respect_robots_txt, operation:page.goto
#[test]
fn with_robots_txt_not_respected_a_handlers_goto_is_not_refused() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let more = json!({
        "site_preferences": { "respect_robots_txt": false, "max_crawl_delay": 3_000 },
    });
    let urls = [
        proving_ground.url(ROBOTS_HOST, "/private/catalog/2"),
        proving_ground.url(CRAWL_DELAY_HOST, "/catalog/1"),
    ];

    let mut statuses = Vec::new();
    let report = crawl_with(
        &mut engine,
        "unrespected",
        &[json!({ "url": proving_ground.url(ROBOTS_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            for url in &urls {
                let went = inside.call("page.goto", &json!({ "page_id": page_id, "url": url }));
                statuses.push(went["result"]["response"]["status"].clone());
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    assert_eq!(
        (&report["result"]["lifecycle"], statuses),
        (&json!("finished"), vec![json!(200), json!(200)])
    );
}

/// The `access-policy` family's TDMRep-header task's host: its second page
/// answers with `tdm-reservation: 1`.
const TDMREP_HOST: &str = "5.access-policy.test";

/// The `access-policy` family's host whose pages declare their signals in
/// markup: none on page 1, a `TDMRep` `<meta>` on page 2, an RSL `Link`
/// header on page 3 and an RSL `<link>` on page 4, the license prohibiting
/// every AI use.
const MARKUP_HOST: &str = "markup.access-policy.test";

/// A handler's `goto` to a document whose `TDMRep` header reserves its
/// rights, under `site_preferences`: answers the call and whether the page
/// then holds any text.
fn goto_reserved_document(site_preferences: &Value) -> (Value, Value) {
    goto_declaring(TDMREP_HOST, "/catalog/2", site_preferences)
}

/// A handler's `goto` to `path` on `host`, from a page of the crawl of its
/// first page, under `site_preferences`: answers the call and whether the
/// page then holds any text.
fn goto_declaring(host: &str, path: &str, site_preferences: &Value) -> (Value, Value) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let reserved = proving_ground.url(host, path);
    let more = json!({ "site_preferences": site_preferences });

    let (mut went, mut text) = (Value::Null, Value::Null);
    let report = crawl_with(
        &mut engine,
        "signals",
        &[json!({ "url": proving_ground.url(host, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            went = inside.call("page.goto", &json!({ "page_id": page_id, "url": reserved }));
            text = inside.call(
                "page.evaluate",
                &json!({
                    "page_id": page_id,
                    "function": "() => document.documentElement.textContent.trim().length > 0",
                }),
            )["result"]["value"]
                .clone();
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    (went, text)
}

/// With `site_preferences.respect_use_signals` on and a Purpose the
/// document's `TDMRep` header refuses, a handler's `goto` to it raises
/// `site_preferences_refused` with `use_signals` and leaves the page on a
/// blank document.
///
/// Covers: story:5-134, error:site_preferences_refused, setting:site_preferences.respect_use_signals, setting:site_preferences.purpose, operation:page.goto, operation:page.evaluate
#[test]
fn a_handlers_goto_to_a_document_whose_use_signals_refuse_the_purpose_is_refused_blank() {
    let (went, text) =
        goto_reserved_document(&json!({ "respect_use_signals": true, "purpose": "ai-input" }));

    assert_eq!(
        (
            &went["error"]["data"]["code"],
            &went["error"]["data"]["reason"],
            &text
        ),
        (
            &json!("site_preferences_refused"),
            &json!("use_signals"),
            &json!(false)
        ),
        "{went}"
    );
}

/// The same holds for a document that declares its signals in its
/// response's markup or names its license: a `TDMRep` `<meta>`, an RSL
/// `Link` header or an RSL `<link>`, the license fetched once for the Run
/// and prohibiting every AI use; a page of the same host declaring none
/// loads.
///
/// Covers: story:5-134, setting:site_preferences.respect_use_signals, setting:site_preferences.purpose, operation:page.goto, operation:page.evaluate
#[rstest::rstest]
/// Covers: error:site_preferences_refused
#[case::a_tdmrep_meta("/catalog/2", false)]
/// Covers: error:site_preferences_refused
#[case::an_rsl_link_header("/catalog/3", false)]
/// Covers: error:site_preferences_refused
#[case::an_rsl_link_element("/catalog/4", false)]
#[case::no_signal("/catalog/1", true)]
fn a_handlers_goto_to_a_document_whose_markup_or_license_refuses_the_purpose_is_refused_blank(
    #[case] path: &str,
    #[case] loads: bool,
) {
    let (went, text) = goto_declaring(
        MARKUP_HOST,
        path,
        &json!({ "respect_use_signals": true, "purpose": "ai-input" }),
    );

    let answered = if loads {
        went["result"]["response"]["status"].clone()
    } else {
        json!([
            went["error"]["data"]["code"],
            went["error"]["data"]["reason"]
        ])
    };
    let expected = if loads {
        json!(200)
    } else {
        json!(["site_preferences_refused", "use_signals"])
    };
    assert_eq!((answered, text), (expected, json!(loads)), "{went}");
}

/// With `site_preferences.respect_use_signals` off, or a Purpose of
/// `search`, the same `goto` loads the document.
///
/// Covers: story:5-134, setting:site_preferences.respect_use_signals, setting:site_preferences.purpose, operation:page.goto, operation:page.evaluate
#[rstest::rstest]
#[case::not_honoured(json!({ "respect_use_signals": false, "purpose": "ai-input" }))]
#[case::for_search(json!({ "respect_use_signals": true, "purpose": "search" }))]
fn a_handlers_goto_to_a_reserved_document_loads_unless_the_signals_are_honoured_for_a_purpose(
    #[case] site_preferences: Value,
) {
    let (went, text) = goto_reserved_document(&site_preferences);

    assert_eq!(
        (&went["result"]["response"]["status"], &text),
        (&json!(200), &json!(true)),
        "{went}"
    );
}

/// The `access-policy` family's Content-Signal task's host, whose
/// robots.txt says `Content-Signal: … ai-train=no` to every crawler.
const CONTENT_SIGNAL_HOST: &str = "3.access-policy.test";

/// The `access-policy` family's TDMRep-file task's host, whose
/// `/.well-known/tdmrep.json` reserves `/catalog*`.
const TDMREP_FILE_HOST: &str = "4.access-policy.test";

/// What a handler's calls on the page its tier serves, the request's own
/// on `fetch.tier: http`, answered, under `site_preferences`: each of the
/// `calls` made for the Proving ground, a method and its params beside the
/// page's ID, then whether the
/// page then holds no element at all, a blank document, as
/// `locator.wait_for_count` answers it, and the paths of `host` the site
/// received that are no robots.txt or signal file.
fn on_the_served_page(
    seed: (&str, &str),
    calls: &dyn Fn(&ProvingGround) -> Vec<(&'static str, Value)>,
    site_preferences: &Value,
    host: &str,
) -> (Vec<Value>, Value, Vec<String>) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let more = json!({ "site_preferences": site_preferences });
    let calls = calls(&proving_ground);

    let (mut answers, mut blank) = (Vec::new(), Value::Null);
    let report = crawl_with(
        &mut engine,
        "served",
        &[json!({ "url": proving_ground.url(seed.0, seed.1), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = &handle["page_id"];
            for (method, params) in &calls {
                let mut params = params.clone();
                params["page_id"] = page_id.clone();
                answers.push(inside.call(method, &params));
            }
            let counted = inside.call(
                "locator.wait_for_count",
                &json!({
                    "locator": { "page_id": page_id, "selector": "body *" },
                    "count": { "max": 0 },
                }),
            );
            blank = json!(counted["result"]["count"] == json!(0));
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
    let mut sent: Vec<String> = proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host.split(':').next() == Some(host))
        .map(|request| request.target.clone())
        .filter(|target| target != "/robots.txt" && !target.starts_with("/.well-known/"))
        .collect();
    sent.sort();
    (answers, blank, sent)
}

/// The code and the reason of a call's refusal, or its response's status.
fn refused_or_status(answer: &Value) -> Value {
    match answer.get("error") {
        Some(error) => json!([error["data"]["code"], error["data"]["reason"]]),
        None => answer["result"]["response"]["status"].clone(),
    }
}

/// On a page a document tier serves, the request's own on `http`, the
/// document a handler's `goto` reaches is judged as a browser page's is
/// (ADR 0093): one whose response's `TDMRep` header refuses the Purpose
/// raises `site_preferences_refused` with `use_signals` and leaves the page
/// blank, and a `page.request` its response refuses raises the same.
///
/// Covers: story:5-134, error:site_preferences_refused, setting:site_preferences.respect_use_signals, setting:site_preferences.purpose, operation:page.goto, operation:page.request, operation:locator.wait_for_count
#[test]
fn a_served_pages_goto_to_a_document_whose_use_signals_refuse_the_purpose_is_refused_blank() {
    let reserved = |method: &'static str| {
        move |proving_ground: &ProvingGround| {
            vec![(
                method,
                json!({ "url": proving_ground.url(TDMREP_HOST, "/catalog/2") }),
            )]
        }
    };
    let honoured = json!({ "respect_use_signals": true, "purpose": "ai-input" });
    let (requested, _, _) = on_the_served_page(
        (TDMREP_HOST, "/catalog/1"),
        &reserved("page.request"),
        &honoured,
        TDMREP_HOST,
    );
    let (went, blank, _) = on_the_served_page(
        (TDMREP_HOST, "/catalog/1"),
        &reserved("page.goto"),
        &honoured,
        TDMREP_HOST,
    );

    let refused = json!(["site_preferences_refused", "use_signals"]);
    assert_eq!(
        (
            refused_or_status(&went[0]),
            blank,
            refused_or_status(&requested[0])
        ),
        (refused.clone(), json!(true), refused),
        "{went:?} {requested:?}"
    );
}

/// A handler's `goto` to a host that declared, before any request, that
/// the Purpose is refused, its robots.txt's `Content-Signal` line or
/// `TDMRep`'s well-known file, is refused with `use_signals` before it is
/// sent, as a crawl request is (ADR 0093), on a served page as on a
/// browser page.
///
/// Covers: story:5-134, error:site_preferences_refused, setting:site_preferences.respect_use_signals, setting:site_preferences.purpose, operation:page.goto
#[rstest::rstest]
#[case::a_content_signal_on_a_served_page(CONTENT_SIGNAL_HOST, "ai-train", false)]
#[case::a_tdmrep_file_on_a_served_page(TDMREP_FILE_HOST, "collect", false)]
/// Covers: operation:run.new_page
#[case::a_content_signal_on_a_browser_page(CONTENT_SIGNAL_HOST, "ai-train", true)]
fn a_handlers_goto_to_a_host_whose_declared_signals_refuse_the_purpose_is_never_sent(
    #[case] host: &str,
    #[case] purpose: &str,
    #[case] browser: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let more = json!({ "site_preferences": { "respect_use_signals": true, "purpose": purpose } });
    let target = proving_ground.url(host, "/catalog/1");

    let mut went = Value::Null;
    let report = crawl_with(
        &mut engine,
        "declared",
        &[json!({ "url": proving_ground.url(TDMREP_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = if browser {
                json!(new_page(inside, handle))
            } else {
                handle["page_id"].clone()
            };
            went = inside.call("page.goto", &json!({ "page_id": page_id, "url": target }));
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");

    assert_eq!(
        (
            refused_or_status(&went),
            requested(&proving_ground, host, "/catalog/1")
        ),
        (json!(["site_preferences_refused", "use_signals"]), 0),
        "{went}"
    );
}

/// With `site_preferences.respect_use_signals` off, the default, a
/// handler's `goto` to a host whose `TDMRep` file reserves the page loads
/// and the gate reads no signal file; with robots.txt not respected either,
/// the page's own request is all the host receives, counted once the Run
/// finished, so a later robots.txt read could not be missed.
///
/// Covers: story:5-134, setting:site_preferences.respect_use_signals, setting:site_preferences.respect_robots_txt, operation:page.goto
#[rstest::rstest]
#[case::a_served_page_respecting_robots_txt(true, false, 1)]
#[case::a_served_page(false, false, 0)]
/// Covers: operation:run.new_page
#[case::a_browser_page(false, true, 0)]
fn with_use_signals_not_honoured_a_handlers_goto_reads_no_signal_file(
    #[case] respect_robots_txt: bool,
    #[case] browser: bool,
    #[case] robots_txt_reads: usize,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let more = json!({ "site_preferences": { "respect_robots_txt": respect_robots_txt } });
    let target = proving_ground.url(TDMREP_FILE_HOST, "/catalog/1");

    let mut went = Value::Null;
    let report = crawl_with(
        &mut engine,
        "unhonoured",
        &[json!({ "url": proving_ground.url(TDMREP_HOST, "/catalog/1"), "label": "page" })],
        &settings(&proving_ground, &more),
        &mut |inside, handle| {
            let page_id = if browser {
                json!(new_page(inside, handle))
            } else {
                handle["page_id"].clone()
            };
            went = inside.call("page.goto", &json!({ "page_id": page_id, "url": target }));
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");

    assert_eq!(
        (
            refused_or_status(&went),
            requested(&proving_ground, TDMREP_FILE_HOST, "/catalog/1"),
            proving_ground
                .received_requests()
                .unwrap()
                .every()
                .filter(
                    |request| request.host == TDMREP_FILE_HOST && request.target == "/robots.txt"
                )
                .count(),
            requested(
                &proving_ground,
                TDMREP_FILE_HOST,
                "/.well-known/tdmrep.json"
            ),
        ),
        (json!(200), 1, robots_txt_reads, 0),
        "{went}"
    );
}

/// A handler's `goto` on a served page to a refresh stub is given the
/// stub, its tier running no refresh (ADR 0118): only the browser runs one,
/// and the stub's target is never requested.
///
/// Covers: story:5-125, operation:page.goto
#[test]
fn a_served_pages_goto_to_a_refresh_stub_is_given_the_stub() {
    let (went, _, sent) = on_the_served_page(
        (FETCH_EDGE, "/redirect/hops/0"),
        &|proving_ground| {
            vec![(
                "page.goto",
                json!({ "url": proving_ground.url(FETCH_EDGE, "/refresh/meta/0") }),
            )]
        },
        &json!({}),
        FETCH_EDGE,
    );

    let url = went[0]["result"]["response"]["url"]
        .as_str()
        .unwrap_or_default();
    assert_eq!(
        (
            refused_or_status(&went[0]),
            url.ends_with("/refresh/meta/0"),
            sent
        ),
        (
            json!(200),
            true,
            vec!["/redirect/hops/0".to_owned(), "/refresh/meta/0".to_owned()]
        ),
        "{went:?}"
    );
}

/// How many handlers run at once in the eight-handler case.
const HANDLERS: usize = 8;

/// The catalog host, whose robots.txt sets no rules: the eight handlers'
/// one domain.
const CATALOG_HOST: &str = "catalog.test";

/// The delay between starts the eight-handler case sets: eight of them
/// outlast its lease.
const EIGHT_DELAY_MILLISECONDS: u64 = 3_500;

/// Eight handlers running at once, each going to its own page of one
/// domain at the same moment, press that domain one request at a time: the
/// gate admits each at least `politeness.domain.delay` after the one before,
/// each reaches the site no sooner than its turn after robots.txt, on the
/// Proving ground's arrival times, the site counts one request per page and
/// sees no misstep, and no handler is
/// timed out while its `goto` waits for its permit, though the last waits
/// longer than its lease was granted for.
///
/// Each seed is a page of a plain host of its own, so the eight are
/// handled at once, and the engine fetches it on the Session's own
/// transport under the forced `http` tier: nothing of it reaches the
/// catalog's domain or the browser. An earlier crawl of the same user read
/// each seed host's robots.txt ([`read_the_seed_hosts_robots_txt`]), so no
/// seed's fetch waits one delay after that file's.
/// Each is added once the handler before it opened its page in the Run, so
/// each page takes the gate and lease of its own attempt, the one handed
/// over last. Nothing renews a lease while its handler only holds its page,
/// so the eight openings must fit in the first one's lease: a first seed's
/// handler starts the Run's browser before them, and its page's navigation
/// passes the Session's egress relay once, so that no `goto` of the eight
/// waits for the browser's start or the relay's credentials, and the
/// governor admits the eight attempts and their pages at once whatever the
/// host's load ([`ADMITTED_WHATEVER_THE_LOAD`]). A request delivered again
/// once its lease ended, before or after the eighth, opens no page.
///
/// Covers: story:6-245, story:5-134, setting:politeness.domain.delay, setting:crawl.handler.concurrency, setting:timeout.handler, setting:crawl.keep_alive, operation:page.goto, operation:run.new_page, operation:run.add_seed
#[test]
fn eight_handlers_at_once_press_their_domain_one_delay_apart() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let pages: Vec<String> = (1..=HANDLERS).map(|n| format!("/catalog/{n}")).collect();
    start_eight(&mut engine, &proving_ground);

    let (mut handles, mut opened, mut answers) = (Vec::new(), Vec::new(), Vec::new());
    // The requests delivered so far, the warm-up's first, and those
    // delivered again.
    let (mut requests, mut again) = (
        vec![eight_seed(&proving_ground, 0)["url"].clone()],
        Vec::new(),
    );
    let report = loop {
        let message = engine.next().expect("the engine answers");
        let id = message["id"].as_str().unwrap_or_default().to_owned();
        if message["method"] == "crawl_request.handle" {
            let request = message["params"]["crawl_request"]["url"].clone();
            if requests.contains(&request) {
                // Delivered again, its lease lost, whenever it comes: it
                // opens no page, and the assertion names it.
                again.push(request);
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": message["id"], "result": { "records": [] },
                }));
                continue;
            }
            requests.push(request);
            handles.push(message["id"].clone());
            engine.send(&json!({
                "jsonrpc": "2.0", "id": format!("new-{}", handles.len() - 1),
                "method": "run.new_page", "params": { "run_id": "eight" },
            }));
        } else if id == "finish" {
            break message;
        } else if id.starts_with("new-") {
            opened.push(message["result"]["page_id"].clone());
            if opened.len() < HANDLERS {
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": "add", "method": "run.add_seed",
                    "params": { "run_id": "eight", "seed": eight_seed(&proving_ground, opened.len() + 1) },
                }));
                continue;
            }
            // Every handler holds its page: all go at once.
            for (handler, page_id) in opened.iter().enumerate() {
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": format!("goto-{handler}"), "method": "page.goto",
                    "params": {
                        "page_id": page_id,
                        "url": proving_ground.url(CATALOG_HOST, &pages[handler]),
                    },
                }));
            }
            engine.send(&json!({
                "jsonrpc": "2.0", "id": "finish", "method": "run.finish",
                "params": { "run_id": "eight" },
            }));
        } else if let Some(number) = id.strip_prefix("goto-") {
            let number: usize = number.parse().unwrap();
            answers.push(status_or_code(&message));
            engine.send(&json!({
                "jsonrpc": "2.0", "id": handles[number], "result": { "records": [] },
            }));
        }
    };
    assert!(engine.finished());

    let mut targets: Vec<&str> = pages.iter().map(String::as_str).collect();
    targets.push("/robots.txt");
    let arrived = arrivals(&proving_ground, CATALOG_HOST, &targets);
    let early = before_their_turn(&arrived, EIGHT_DELAY_MILLISECONDS * 1_000);
    let admitted = admitted(&env, CATALOG_HOST).len();
    let observations = proving_ground.observations().unwrap();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            handles.len(),
            again,
            answers,
            arrived.first().map(|(target, _)| target.as_str()),
            arrived.len(),
            early,
            admitted,
            &observations["findings"],
        ),
        (
            &json!("finished"),
            HANDLERS,
            Vec::<Value>::new(),
            vec![json!(200); HANDLERS],
            Some("/robots.txt"),
            HANDLERS + 1,
            Vec::<String>::new(),
            HANDLERS,
            &json!([]),
        ),
        "{arrived:?}"
    );
}

/// The hosts of the seeds of the crawls whose handlers open their own
/// pages: plain hosts of the Proving ground, each a domain of its own and
/// none the catalog's, so a seed's fetch, on the Session's own transport
/// under the forced `http` tier, never presses the domain the handlers'
/// navigations are paced on, nor passes the browser's egress relay.
const SEED_HOSTS: [&str; 10] = [
    "shop.test",
    "heavy.test",
    "a.test",
    "b.test",
    "api.test",
    "other.test",
    "many.test",
    "capped.test",
    "alice.github.io",
    "bob.github.io",
];

/// The URL of the seed `n` of such a crawl.
fn seed_url(proving_ground: &ProvingGround, n: usize) -> String {
    proving_ground.url(SEED_HOSTS[n], "/seed")
}

/// Crawls one page of each seed host with no delay, as the Run `primed`:
/// the user's Runs share each host's robots.txt copy from then on (ADR
/// 0086), so the eight-handler crawl, whose delay would keep each seed's
/// fetch one delay after its host's robots.txt, sends each seed at once
/// and its eight openings fit in the first one's lease.
fn read_the_seed_hosts_robots_txt(engine: &mut Attached, proving_ground: &ProvingGround) {
    let seeds: Vec<Value> = SEED_HOSTS
        .iter()
        .map(|host| json!({ "url": proving_ground.url(host, "/primed"), "label": "page" }))
        .collect();
    let report = crawl_with(
        engine,
        "primed",
        &seeds,
        &settings(
            proving_ground,
            &json!({ "politeness": { "domain": { "delay": 0 } } }),
        ),
        &mut |_, _| json!({ "result": { "records": [] } }),
    );
    assert_eq!(report["result"]["lifecycle"], "finished", "{report}");
}

/// The eight-handler crawl's seed `n`, each on a domain of its own.
fn eight_seed(proving_ground: &ProvingGround, n: usize) -> Value {
    json!({ "url": seed_url(proving_ground, n), "label": "page" })
}

/// Starts the eight-handler crawl from its first seed, which [`warm_up`]
/// handles, then adds the first of the eight.
fn start_eight(engine: &mut Attached, proving_ground: &ProvingGround) {
    read_the_seed_hosts_robots_txt(engine, proving_ground);
    let settings = settings(
        proving_ground,
        &json!({
            // A lease of 15 s plus the 10 s margin, shorter than the last
            // handler's wait for its permit.
            "timeout": { "handler": 15_000 },
            "politeness": { "domain": { "delay": EIGHT_DELAY_MILLISECONDS } },
            // Kept alive until `run.finish`: it waits for each seed.
            "crawl": { "handler": { "concurrency": HANDLERS }, "keep_alive": true },
        }),
    );
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "start", "method": "crawl.start",
        "params": { "run_id": "eight", "seeds": [eight_seed(proving_ground, 0)], "settings": settings },
    }));
    warm_up(engine, proving_ground);
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "add", "method": "run.add_seed",
        "params": { "run_id": "eight", "seed": eight_seed(proving_ground, 1) },
    }));
}

/// Handles the eight-handler crawl's first seed: its handler opens a page,
/// which starts the Run's browser, and goes to a page of another domain
/// through the Session's egress relay, then answers.
fn warm_up(engine: &mut Attached, proving_ground: &ProvingGround) {
    let (mut started, mut handle, mut others) = (false, None, Vec::new());
    while !started || handle.is_none() {
        let message = engine.next().expect("the engine answers");
        if message["method"] == "crawl_request.handle" {
            handle = Some(message["id"].clone());
        } else {
            assert_eq!(message["id"], "start", "{message}");
            assert!(message.get("result").is_some(), "{message}");
            started = true;
        }
    }
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "warm-page", "method": "run.new_page",
        "params": { "run_id": "eight" },
    }));
    let page_id = answer_to(engine, "warm-page", &mut others)["result"]["page_id"].clone();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "warm-goto", "method": "page.goto",
        "params": { "page_id": page_id, "url": proving_ground.url("1.links.test", "/guide/") },
    }));
    let went = answer_to(engine, "warm-goto", &mut others);
    engine.send(&json!({
        "jsonrpc": "2.0", "id": handle, "result": { "records": [] },
    }));
    assert_eq!(
        (status_or_code(&went), others),
        (json!(200), Vec::<Value>::new())
    );
}

/// The status of a `goto`'s response, or the Error code that refused it.
fn status_or_code(answer: &Value) -> Value {
    let status = &answer["result"]["response"]["status"];
    if status.is_null() {
        code(answer)
    } else {
        status.clone()
    }
}

/// The Error code of a call's refusal.
fn code(answer: &Value) -> Value {
    answer["error"]["data"]["code"].clone()
}

/// A handler's page past its work item's lease sends nothing more (ADRs
/// 0003 and 0087; spec #594's P31): once the lease ended, a click, a
/// navigation and a `page.request`, each the first call on a page of the
/// handler's, are refused with `attempt_ended` and the site gets no request
/// after the lease's end; each page then moves to `Closing`, so a later call
/// on it raises `closed`. A page a program opened in a Run of its own
/// carries no lease and still navigates long after.
///
/// The lease is the 1 s handler timeout plus its 10 s margin, in real time:
/// a test of the binary sets short durations through settings (TESTING.md,
/// "Time"). Nothing renews it while the handler opens its pages, so a
/// first seed's handler starts the Run's browser before the handler under
/// test is leased ([`warm_up_the_browser`]).
///
/// Covers: story:5-135, error:attempt_ended, setting:timeout.handler, setting:crawl.retry.max, operation:page.goto, operation:page.request, operation:locator.click, operation:run.new_page
#[test]
fn a_handlers_page_past_its_work_items_lease_sends_nothing_more() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let settings = settings(
        &proving_ground,
        &json!({
            // A lease of 1 s plus the 10 s margin.
            "timeout": { "handler": 1_000 },
            "crawl": { "retry": { "max": 0 } },
        }),
    );
    let mut others = Vec::new();
    let program_page = program_page(
        &mut engine,
        &json!({ "network": settings["network"], "egress": settings["egress"] }),
        &mut others,
    );
    let url = |path: &str| proving_ground.url(CATALOG_HOST, path);
    // The first seed's handler starts the Run's browser: a crawl leases
    // its seeds in order, one at a time (`crawl_with`'s handler
    // concurrency of 1), so it opens the Run's first page before the
    // request under test is leased.
    let (warm_up_seed, leased_seed) = (seed_url(&proving_ground, 0), seed_url(&proving_ground, 1));

    let (mut before, mut refused, mut later, mut requests) = (None, Vec::new(), Vec::new(), None);
    let mut warmed = false;
    let report = crawl_with(
        &mut engine,
        "zombie",
        // The first seed starts the Run's browser; the third keeps the Run
        // going past the second's lease.
        &[
            json!({ "url": warm_up_seed, "label": "page" }),
            json!({ "url": leased_seed, "label": "page" }),
            json!({ "url": seed_url(&proving_ground, 2), "label": "page" }),
        ],
        &settings,
        &mut |inside, handle| {
            if handle["crawl_request"]["url"] == warm_up_seed && !warmed {
                warmed = true;
                warm_up_the_browser(inside, handle);
            }
            if handle["crawl_request"]["url"] != leased_seed {
                return json!({ "result": { "records": [] } });
            }
            let pages: Vec<String> = (0..3).map(|_| new_page(inside, handle)).collect();
            before = Some(inside.call(
                "page.goto",
                &json!({ "page_id": pages[0], "url": url("/catalog/1") }),
            ));
            let engine = inside.engine();
            next_seed_delivered(engine, &mut others);
            let lease_ended = requested_in_all(&proving_ground);
            (refused, later) = past_the_lease(engine, &pages, &url, &mut others);
            requests = Some((lease_ended, requested_in_all(&proving_ground)));
            // The second seed's attempt, delivered meanwhile, ends at once.
            for delivered in others
                .iter()
                .filter(|other| other["method"] == "crawl_request.handle")
            {
                engine.send(&json!({
                    "jsonrpc": "2.0", "id": delivered["id"], "result": { "records": [] },
                }));
            }
            json!({ "result": { "records": [] } })
        },
    );
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "program-goto", "method": "page.goto",
        "params": { "page_id": program_page, "url": url("/catalog/5") },
    }));
    let program_goto = answer_to(&mut engine, "program-goto", &mut others);
    assert!(engine.finished());

    let (lease_ended, after) = requests.expect("the handler ran");
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            before.map(|before| before["result"]["response"]["status"].clone()),
            refused,
            later,
            after - lease_ended,
            requested(&proving_ground, CATALOG_HOST, "/catalog/4"),
            program_goto["result"]["response"]["status"].clone(),
        ),
        (
            &json!("finished"),
            Some(json!(200)),
            vec![json!("attempt_ended"); 3],
            vec![json!("closed"); 3],
            0,
            0,
            json!(200),
        ),
        "{others:?}"
    );
}

/// Opens a page in the Run of `handle`'s request and leaves it open, which
/// launches the Run's browser: under load a launch takes seconds of a lease
/// nothing renews (ADR 0093 renews it only through a permit's wait), so a
/// case whose handler must open its pages, then navigate, within a short
/// lease runs once the browser is up. Its own lease may end meanwhile:
/// what the engine sends until the page opened is left aside.
fn warm_up_the_browser(inside: &mut Inside<'_>, handle: &Value) {
    let engine = inside.engine();
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "warm-up-page", "method": "run.new_page",
        "params": { "run_id": handle["run_id"] },
    }));
    let opened = answer_to(engine, "warm-up-page", &mut Vec::new());
    assert!(opened.get("result").is_some(), "{opened}");
}

/// Opens a page in a Run of the program's own, `program`, whose settings
/// are `settings`, answering its ID; the replies and requests before it are
/// kept in `others`.
fn program_page(engine: &mut Attached, settings: &Value, others: &mut Vec<Value>) -> Value {
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "program", "method": "run.start",
        "params": { "run_id": "program", "settings": settings },
    }));
    let started = answer_to(engine, "program", others);
    assert!(started.get("result").is_some(), "{started}");
    engine.send(&json!({
        "jsonrpc": "2.0", "id": "program-page", "method": "run.new_page",
        "params": { "run_id": "program" },
    }));
    answer_to(engine, "program-page", others)["result"]["page_id"].clone()
}

/// Waits until the crawl delivers its next seed, keeping what comes in
/// `others`: at a handler concurrency of 1, it does once the attempt in
/// flight ended, which a handler that only holds its pages does when its
/// lease ends. The wait fails past the engine's message deadline.
fn next_seed_delivered(engine: &mut Attached, others: &mut Vec<Value>) {
    loop {
        let message = engine.next().expect("the engine answers");
        let delivered = message["method"] == "crawl_request.handle";
        others.push(message);
        if delivered {
            return;
        }
    }
}

/// What a handler past its lease gets: the codes of a click on its first
/// page, a navigation of its second and a `page.request` of its third, each
/// the first call on its page, then of a navigation of each page.
fn past_the_lease(
    engine: &mut Attached,
    pages: &[String],
    url: &impl Fn(&str) -> String,
    others: &mut Vec<Value>,
) -> (Vec<Value>, Vec<Value>) {
    let calls = [
        (
            "locator.click",
            json!({ "locator": { "page_id": pages[0], "selector": "a" } }),
        ),
        (
            "page.goto",
            json!({ "page_id": pages[1], "url": url("/catalog/2") }),
        ),
        (
            "page.request",
            json!({ "page_id": pages[2], "url": url("/catalog/3") }),
        ),
    ];
    let mut refused = Vec::new();
    for (n, (method, params)) in calls.iter().enumerate() {
        let id = format!("zombie-{n}");
        engine.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        refused.push(code(&answer_to(engine, &id, others)));
    }
    let mut later = Vec::new();
    for (n, page) in pages.iter().enumerate() {
        let id = format!("later-{n}");
        engine.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "page.goto",
            "params": { "page_id": page, "url": url("/catalog/4") },
        }));
        later.push(code(&answer_to(engine, &id, others)));
    }
    (refused, later)
}

/// How many requests the Proving ground received, on every host.
fn requested_in_all(proving_ground: &ProvingGround) -> usize {
    proving_ground.received_requests().unwrap().every().count()
}

/// The host whose robots.txt asks every crawler for a Crawl-delay of 2 s
/// and answers 1.5 s after the request arrived.
const SLOW_ROBOTS_HOST: &str = "slow-robots.access-policy.test";

/// The domain key of [`SLOW_ROBOTS_HOST`]'s requests.
const SLOW_ROBOTS_KEY: &str = "access-policy.test";

/// [`SLOW_ROBOTS_HOST`]'s Crawl-delay, in milliseconds.
const SLOW_ROBOTS_CRAWL_DELAY_MILLISECONDS: u64 = 2_000;

/// How long after its request arrives [`SLOW_ROBOTS_HOST`]'s robots.txt
/// answers, in microseconds.
const SLOW_ROBOTS_ANSWER_MICROSECONDS: u64 = 1_500_000;

/// The answers to the calls `ids` `engine` sends next, in the order of
/// `ids`, whatever order they come in; the requests and replies between
/// them are kept in `others`.
fn answers_to(engine: &mut Attached, ids: &[String], others: &mut Vec<Value>) -> Vec<Value> {
    let mut answers = vec![Value::Null; ids.len()];
    while answers.iter().any(Value::is_null) {
        let message = engine.next().expect("the engine answers");
        match ids.iter().position(|id| message["id"] == id.as_str()) {
            Some(at) => answers[at] = message,
            None => others.push(message),
        }
    }
    answers
}

/// Sends a `page.goto` of each of `pages` to its URL of `urls` at once,
/// answering each call's response status, in order.
fn goto_at_once(inside: &mut Inside<'_>, pages: &[String], urls: &[String]) -> Vec<Value> {
    let engine = inside.engine();
    let ids: Vec<String> = (0..pages.len()).map(|n| format!("at-once-{n}")).collect();
    for ((id, page_id), url) in ids.iter().zip(pages).zip(urls) {
        engine.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "page.goto",
            "params": { "page_id": page_id, "url": url },
        }));
    }
    answers_to(engine, &ids, &mut Vec::new())
        .iter()
        .map(|answer| answer["result"]["response"]["status"].clone())
        .collect()
}

/// Two pages of a handler going at once to a host whose robots.txt answers
/// slowly, under its Crawl-delay, reach it no sooner than the Crawl-delay
/// after the robots.txt request, on the Proving ground's arrival times,
/// and the gate admits them the Crawl-delay apart (ADR 0086): the page whose
/// navigation fetched robots.txt counts the delay from its response's arrival,
/// never from its send, and the page that read the copy meanwhile counts it
/// from that copy's fetch, never going at once because it fetched nothing
/// itself.
///
/// Covers: story:5-134, operation:page.goto, operation:run.new_page
#[test]
fn a_handlers_navigations_after_a_slow_robots_txt_wait_its_crawl_delay_from_its_arrival() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let urls: Vec<String> = (1..=2)
        .map(|n| proving_ground.url(SLOW_ROBOTS_HOST, &format!("/catalog/{n}")))
        .collect();

    let mut statuses = Vec::new();
    let report = crawl_with(
        &mut engine,
        "slow-robots",
        &[json!({ "url": proving_ground.url(HISTORY, "/"), "label": "page" })],
        &settings(&proving_ground, &json!({})),
        &mut |inside, handle| {
            let pages = [new_page(inside, handle), new_page(inside, handle)];
            statuses = goto_at_once(inside, &pages, &urls);
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let arrived = arrivals(
        &proving_ground,
        SLOW_ROBOTS_HOST,
        &["/robots.txt", "/catalog/1", "/catalog/2"],
    );
    // Each navigation's turn counts from robots.txt's answer, not its request.
    let mut from_its_answer = arrived.clone();
    if let Some((_, read)) = from_its_answer.first_mut() {
        *read += SLOW_ROBOTS_ANSWER_MICROSECONDS;
    }
    let early = before_their_turn(
        &from_its_answer,
        SLOW_ROBOTS_CRAWL_DELAY_MILLISECONDS * 1_000,
    );
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            statuses,
            arrived.first().map(|(target, _)| target.as_str()),
            arrived.len(),
            early,
            admitted(&env, SLOW_ROBOTS_KEY).len(),
        ),
        (
            &json!("finished"),
            vec![json!(200); 2],
            Some("/robots.txt"),
            3,
            Vec::<String>::new(),
            2,
        ),
        "{arrived:?}"
    );
}

/// The delay between starts of the case whose first navigation is its
/// Session's first request through the egress relay, in milliseconds.
const RELAY_DELAY_MILLISECONDS: u64 = 1_000;

/// When the engine sent each `page.goto`'s request on for its last time, by
/// its log, each with whether that was with the Session's proxy credentials
/// after the relay's `407`: its `Fetch.continueWithAuth`, else the last
/// `Fetch.continueRequest` before the call answered. The page's icon, which
/// the first document's load may ask for, leaves before the next document,
/// which its gate holds.
fn navigations_sent(env: &TestEnv) -> Vec<(jiff::Timestamp, bool)> {
    let mut sent = Vec::new();
    let mut latest: Option<(jiff::Timestamp, bool)> = None;
    for line in engine_log(env) {
        let at = || line["timestamp"].as_str().unwrap().parse().unwrap();
        let credentials = latest.is_some_and(|(_, credentials)| credentials);
        match (
            line["target"].as_str().unwrap_or_default(),
            line["fields"]["method"].as_str().unwrap_or_default(),
        ) {
            ("goodfetch::cdp::command", "Fetch.continueWithAuth") if !credentials => {
                latest = Some((at(), true));
            }
            ("goodfetch::cdp::command", "Fetch.continueRequest") if !credentials => {
                latest = Some((at(), false));
            }
            (DISPATCH_TARGET, "page.goto") => sent.extend(latest.take()),
            _ => {}
        }
    }
    sent
}

/// A handler's navigations of one domain leave the delay between starts
/// apart, its Session's first request through its egress relay included: the
/// browser sends it only once the relay answered `407` and got the Session's
/// credentials, so its delay counts from that send, never from its admission,
/// nor from the command that first sent it on (ADR 0086). The test judges the
/// sends themselves, by when the engine's log says it sent each command: an
/// arrival comes later than its send by a way to the site the first request
/// alone has to open (the relay dials the site, and the TLS handshake follows),
/// which no figure bounds on a loaded host. Each send's turn counts from the
/// first's: the first is recorded after its command leaves, so the delays from
/// it hold to the microsecond, while a later send is recorded just before its
/// command and its neighbour's gap can come short by that moment.
///
/// Covers: story:5-134, setting:politeness.domain.delay, operation:page.goto, operation:run.new_page
#[test]
fn a_handlers_navigations_through_a_fresh_egress_relay_leave_the_delay_apart() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mut engine = engine(&env);
    let settings = settings(
        &proving_ground,
        &json!({ "politeness": { "domain": { "delay": RELAY_DELAY_MILLISECONDS } } }),
    );
    let pages = ["/catalog/1", "/catalog/2", "/catalog/3"];

    let mut statuses = Vec::new();
    let report = crawl_with(
        &mut engine,
        "fresh-relay",
        &[json!({ "url": seed_url(&proving_ground, 1), "label": "page" })],
        &settings,
        &mut |inside, handle| {
            let page_id = new_page(inside, handle);
            for path in pages {
                let url = proving_ground.url(CATALOG_HOST, path);
                let went = inside.call("page.goto", &json!({ "page_id": page_id, "url": url }));
                statuses.push(went["result"]["response"]["status"].clone());
            }
            json!({ "result": { "records": [] } })
        },
    );
    assert!(engine.finished());

    let sent = navigations_sent(&env);
    let arrived = arrivals(&proving_ground, CATALOG_HOST, &pages);
    let sends: Vec<(String, u64)> = sent
        .iter()
        .map(|(at, _)| ("sent".to_owned(), at.as_microsecond().try_into().unwrap()))
        .collect();
    assert_eq!(
        (
            &report["result"]["lifecycle"],
            statuses,
            arrived.len(),
            sent.iter()
                .map(|(_, credentials)| *credentials)
                .collect::<Vec<_>>(),
            before_their_turn(&sends, RELAY_DELAY_MILLISECONDS * 1_000),
        ),
        (
            &json!("finished"),
            vec![json!(200); 3],
            3,
            vec![true, false, false],
            Vec::<String>::new(),
        ),
        "{sent:?} {arrived:?}"
    );
}
