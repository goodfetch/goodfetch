//! A Run's browser pool on Collie's executable over stdio (spec #213's Seam
//! 1, ADR 0040 and ADR 0037's SA-09), through `goodfetch-protocol`'s types:
//! a Run's Sessions share a browser process as browser contexts, up to
//! `governor.browser.contexts_per_process`, never with another Run's; a
//! browser is recycled after its pages, past its memory cap or once idle,
//! each Session's cookies and `localStorage` carried to its replacement, as
//! the Proving ground's record of what each request sent shows; and a
//! browser the pool launched writes its crash dumps where the test's
//! environment names (ADR 0074).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the test lists processes and the crash-dump folder on its own thread, never an async worker"
)]

use std::time::{Duration, Instant};

use goodfetch_protocol::page::{PageOpened, SessionId};
use goodfetch_test_client::{HomeLayout, robots_txt_read, robots_txt_readings};
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use rstest::rstest;
use serde_json::{Map, Value, json};

use crate::page::{opened, started_with};
use goodfetch_test_client::Attached;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 235;

/// How long a browser has to start or to exit before the count is taken.
const SETTLE: Duration = Duration::from_secs(30);

/// The Run settings that put `pool` under `governor.browser`.
fn pool(pool: Value) -> Map<String, Value> {
    let mut settings = Map::new();
    let mut governor = Map::new();
    governor.insert("browser".to_owned(), pool);
    settings.insert("governor".to_owned(), Value::Object(governor));
    settings
}

/// The browser processes the test's engine runs: a browser's own helpers
/// (renderers, its GPU and network processes, Linux's zygote) each name the
/// `--type` they are.
fn browsers(env: &TestEnv) -> usize {
    crate::page::browser_command_lines(env)
        .iter()
        .filter(|line| !line.contains("--type="))
        .count()
}

/// The number of browser processes once it is `expected`, or after
/// [`SETTLE`] whatever it is then: browsers start and exit on their own
/// time.
fn browsers_once(env: &TestEnv, expected: usize) -> usize {
    let deadline = Instant::now() + SETTLE;
    loop {
        let found = browsers(env);
        if found == expected || Instant::now() > deadline {
            return found;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The files in the test's crash-dump folder and its subfolders, but
/// Crashpad's own settings, once a dump (`.dmp`) is among them, or after
/// [`SETTLE`].
fn crash_dumps(env: &TestEnv) -> Vec<std::path::PathBuf> {
    let deadline = Instant::now() + SETTLE;
    loop {
        let mut found = Vec::new();
        let mut folders = vec![env.crash_dump_folder().to_owned()];
        while let Some(folder) = folders.pop() {
            let Ok(entries) = std::fs::read_dir(&folder) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    folders.push(path);
                } else if path.file_name().is_some_and(|name| name != "settings.dat") {
                    found.push(path);
                }
            }
        }
        if found.iter().any(|file| is_dump(file)) || Instant::now() > deadline {
            return found;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Whether `file` is a minidump, as Crashpad names one.
fn is_dump(file: &std::path::Path) -> bool {
    file.extension().is_some_and(|extension| extension == "dmp")
}

/// A renderer of a browser the pool launched, crashed on purpose through
/// Chromium's own `chrome://crash`, writes its dump into the folder the
/// test's environment names, where teardown would fail the test (#373),
/// and the navigation it crashed under raises `page_crashed` (spec #594's
/// P10), not the `navigation_failed` of the load the browser abandoned.
/// Covers: operation:page.goto, story:5-27
#[cfg(unix)]
#[test]
fn a_crashed_renderer_of_a_pooled_browser_writes_its_dump_in_the_tests_crash_dump_folder() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(&env, &cache, &proving_ground, "crash", &Map::new(), |_| {});
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "crash" })));

    let crashed = engine.ask(
        3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": "chrome://crash" }),
    );
    let dumps = crash_dumps(&env);
    let _ = engine.ask(4, "run.finish", &json!({ "run_id": "crash" }));
    let _ = engine.finish();
    // The dump was the point: it would fail teardown.
    for dump in &dumps {
        std::fs::remove_file(dump).unwrap();
    }

    assert_eq!(
        (
            dumps.iter().filter(|file| is_dump(file)).count(),
            crashed["error"]["data"]["code"].clone()
        ),
        (1, json!("page_crashed")),
        "{dumps:?} {crashed}"
    );
}

/// A navigation the browser abandons on a live renderer, as it does for an
/// answer with no document, raises `navigation_failed` with the browser's
/// `net::ERR_ABORTED`, not the `page_crashed` of a renderer that crashed
/// under it, and the page answers its next navigation.
/// Covers: operation:page.goto, story:5-27
#[test]
fn a_navigation_abandoned_on_a_live_renderer_raises_navigation_failed_and_the_page_goes_on() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "abandoned",
        &Map::new(),
        |_| {},
    );
    let page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "abandoned" })));

    let abandoned = engine.ask(3,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("fetch-edge.test", "/no-content") }),
    );
    let next = engine.ask(4,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url("fetch-edge.test", "/target") }),
    );
    let _ = engine.ask(5, "run.finish", &json!({ "run_id": "abandoned" }));
    let _ = engine.finish();

    assert_eq!(
        (
            abandoned["error"]["data"]["code"].clone(),
            abandoned["error"]["data"]["error"].clone(),
            next["result"]["response"]["status"].clone(),
        ),
        (
            json!("navigation_failed"),
            json!("net::ERR_ABORTED"),
            json!(200)
        ),
        "{abandoned} {next}"
    );
}

/// The Session `run.new_session` answered for `run_id`.
fn new_session(engine: &mut Attached, id: i64, run_id: &str) -> SessionId {
    let answer = engine.ask(id, "run.new_session", &json!({ "run_id": run_id }));
    serde_json::from_value(answer["result"]["session_id"].clone())
        .unwrap_or_else(|error| panic!("{error}: {answer}"))
}

/// Opens a page in the Session `session`.
fn page_in(engine: &mut Attached, id: i64, session: &SessionId) -> PageOpened {
    opened(&engine.ask(id, "session.new_page", &json!({ "session_id": session })))
}

/// Loads the storage page on `page` with `mark`, which keeps it in a
/// cookie and in `localStorage` and reports the mark `localStorage` held.
fn store(
    engine: &mut Attached,
    id: i64,
    proving_ground: &ProvingGround,
    page: &PageOpened,
    mark: &str,
) -> Value {
    let url = proving_ground.url("scope.test", &format!("/storage?mark={mark}"));
    engine.ask(
        id,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    )["result"]["response"]["status"]
        .clone()
}

/// What the Proving ground received on `scope.test` for targets starting
/// with `prefix`, in arrival order: each one's target and the cookies it
/// sent.
fn storage_requests(proving_ground: &ProvingGround, prefix: &str) -> Vec<(String, Value)> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == "scope.test" && request.target.starts_with(prefix))
        .map(|request| (request.target.clone(), json!(request.cookies)))
        .collect()
}

/// `run.new_session` answers once the Session's browser context is made, in
/// a browser launched for it when none of its Run had room, so the browsers
/// are counted as each Run's last answer leaves them, with no wait (#2115).
/// Covers: story:3-61, setting:governor.browser.contexts_per_process, operation:run.new_session
#[cfg(unix)]
#[test]
fn sessions_of_one_run_share_a_browser_process_up_to_its_contexts_and_never_another_runs() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "shared",
        &pool(json!({ "contexts_per_process": 2 })),
        |_| {},
    );

    for id in 2..5 {
        let _ = new_session(&mut engine, id, "shared");
    }
    let one_run = browsers(&env);
    let other = engine.ask(5, "run.start", &json!({ "run_id": "other" }));
    let _ = new_session(&mut engine, 6, "other");
    let two_runs = browsers(&env);
    let _ = engine.ask(7, "run.finish", &json!({ "run_id": "other" }));
    let _ = engine.ask(8, "run.finish", &json!({ "run_id": "shared" }));
    let _ = engine.finish();

    assert_eq!(
        (one_run, other.get("result").is_some(), two_runs),
        (2, true, 3)
    );
}

/// A Session's browser recycled after its pages, or past its memory cap,
/// opens no new page: the Session's next one opens in its replacement,
/// which sends the cookie and finds the `localStorage` the Session kept,
/// while the page it left behind stays open there until the Session
/// closes, when the recycled browser closes too.
/// Covers: story:3-62, setting:governor.browser.recycle_pages, setting:governor.browser.memory_cap, operation:session.new_page, operation:session.close
#[cfg(unix)]
#[rstest]
#[case::after_its_pages(json!({ "recycle_pages": 1 }))]
#[case::past_its_memory_cap(json!({ "memory_cap": 1 }))]
fn a_recycled_browser_hands_its_session_over_with_its_cookies_and_storage(#[case] limit: Value) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "recycled",
        &pool(limit),
        |_| {},
    );
    let session = new_session(&mut engine, 2, "recycled");

    let first = page_in(&mut engine, 3, &session);
    let stored = store(&mut engine, 4, &proving_ground, &first, "first");
    let second = page_in(&mut engine, 5, &session);
    let carried = store(&mut engine, 6, &proving_ground, &second, "second");
    let while_left_open = browsers_once(&env, 2);
    let closed = engine.ask(7, "session.close", &json!({ "session_id": session }));
    let once_closed = browsers_once(&env, 1);
    let _ = engine.ask(8, "run.finish", &json!({ "run_id": "recycled" }));
    let _ = engine.finish();

    assert_eq!(
        (
            (stored, carried),
            storage_requests(&proving_ground, "/storage?"),
            storage_requests(&proving_ground, "/storage/found")
                .into_iter()
                .map(|(target, _)| target)
                .collect::<Vec<_>>(),
            (while_left_open, &closed["result"], once_closed),
        ),
        (
            (json!(200), json!(200)),
            vec![
                ("/storage?mark=first".to_owned(), json!([])),
                (
                    "/storage?mark=second".to_owned(),
                    json!([["mark", "first"]])
                ),
            ],
            vec![
                "/storage/found?mark=none".to_owned(),
                "/storage/found?mark=first".to_owned(),
            ],
            (2, &json!({}), 1),
        )
    );
}

/// The page the engine opens in a Session to read a document's robots.txt
/// moves no Session, whatever its browser reached: the Session stays with
/// the page that is still loading, so what that page writes once the
/// reading reached the site is written before the Session's next page moves
/// it, and is carried.
/// Covers: story:3-62, setting:governor.browser.recycle_pages, setting:governor.browser.memory_cap, operation:session.new_page
#[cfg(unix)]
#[rstest]
#[case::after_its_pages(json!({ "recycle_pages": 1 }))]
#[case::past_its_memory_cap(json!({ "memory_cap": 1 }))]
fn storage_written_after_the_engines_own_page_read_robots_txt_is_carried(#[case] limit: Value) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "reading",
        &pool(limit),
        |_| {},
    );
    let session = new_session(&mut engine, 2, "reading");
    let first = page_in(&mut engine, 3, &session);
    let stored = store(&mut engine, 4, &proving_ground, &first, "first");
    robots_txt_read(&env, "reading");

    let written = engine.ask(
        5,
        "page.evaluate",
        &json!({
            "page_id": first.page_id,
            "function": "() => { localStorage.setItem(\"mark\", \"after-reading\"); }",
        }),
    );
    let second = page_in(&mut engine, 6, &session);
    let carried = store(&mut engine, 7, &proving_ground, &second, "second");
    let _ = engine.ask(8, "run.finish", &json!({ "run_id": "reading" }));
    let _ = engine.finish();

    assert_eq!(
        (
            (stored, written.get("result").is_some(), carried),
            storage_requests(&proving_ground, "/storage/found")
                .into_iter()
                .map(|(target, _)| target)
                .collect::<Vec<_>>(),
        ),
        (
            (json!(200), true, json!(200)),
            vec![
                "/storage/found?mark=none".to_owned(),
                "/storage/found?mark=after-reading".to_owned(),
            ],
        ),
        "{written}"
    );
}

/// A Session that opens no page past its first and only sends `page.request`
/// is held to its browser's limits all the same, the pages the engine opens
/// for it counting: past them its requests go out of a replacement, with the
/// cookie it kept, while the program's page stays open in the recycled browser.
/// Covers: story:3-62, setting:governor.browser.recycle_pages, setting:governor.browser.memory_cap, operation:page.request
#[cfg(unix)]
#[rstest]
#[case::after_its_pages(json!({ "recycle_pages": 1 }))]
#[case::past_its_memory_cap(json!({ "memory_cap": 1 }))]
fn a_session_that_only_sends_requests_moves_off_its_recycled_browser(#[case] limit: Value) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "requests",
        &pool(limit),
        |_| {},
    );
    let session = new_session(&mut engine, 2, "requests");
    let page = page_in(&mut engine, 3, &session);
    let stored = store(&mut engine, 4, &proving_ground, &page, "first");
    let url = proving_ground.url("scope.test", "/storage/found?mark=requested");
    let statuses: Vec<Value> = (5..8)
        .map(|id| {
            engine.ask(
                id,
                "page.request",
                &json!({ "page_id": page.page_id, "url": url }),
            )["result"]["status"]
                .clone()
        })
        .collect();
    let replaced = browsers_once(&env, 2);
    let _ = engine.ask(8, "run.finish", &json!({ "run_id": "requests" }));
    let _ = engine.finish();

    assert_eq!(
        (
            stored,
            statuses,
            replaced,
            storage_requests(&proving_ground, "/storage/found?mark=requested").pop(),
        ),
        (
            json!(200),
            vec![json!(200), json!(200), json!(200)],
            2,
            Some((
                "/storage/found?mark=requested".to_owned(),
                json!([["mark", "first"]])
            )),
        )
    );
}

/// A `page.request` sent while a page of its Session is still navigating
/// leaves the Session where it is, though its browser is past its limits:
/// a move then would capture the Session's storage before that navigation's
/// document wrote it. The stalled-robots host never sends the rest of its
/// robots.txt, so its navigation is still in flight when the request answers.
/// Covers: story:3-62, setting:governor.browser.recycle_pages, operation:page.request
#[cfg(unix)]
#[test]
fn a_request_while_a_navigation_is_in_flight_moves_no_session() {
    const STALLED: &str = "stalled-robots.access-policy.test";
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "racing",
        &pool(json!({ "recycle_pages": 1 })),
        |_| {},
    );
    let session = new_session(&mut engine, 2, "racing");
    let page = page_in(&mut engine, 3, &session);
    engine.send(&json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "page.goto",
        "params": {
            "page_id": page.page_id,
            "url": proving_ground.url(STALLED, "/robots.txt"),
        },
    }));
    // The engine's page took the robots.txt's body as a stream, which never
    // ends: the document it arrived for has its head, and no `load` comes.
    let reading = traced_once(&env, "racing", "Fetch.takeResponseBodyAsStream");

    let requested = engine.ask(
        5,
        "page.request",
        &json!({
            "page_id": page.page_id,
            "url": proving_ground.url("scope.test", "/storage/found?mark=racing"),
        }),
    );
    let while_navigating = browsers(&env);
    let _ = engine.ask(6, "run.finish", &json!({ "run_id": "racing" }));
    let _ = engine.finish();

    assert_eq!(
        (reading, &requested["result"]["status"], while_navigating),
        (true, &json!(200), 1),
        "{requested}"
    );
}

/// A browser with no open page past `governor.browser.recycle_idle` closes,
/// and the Run's next Session gets a browser launched again.
/// Covers: story:3-62, setting:governor.browser.recycle_idle, operation:session.close
#[cfg(unix)]
#[test]
fn an_idle_browser_closes_and_the_next_session_gets_a_new_one() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "idle",
        &pool(json!({ "recycle_idle": 500 })),
        |_| {},
    );

    let session = new_session(&mut engine, 2, "idle");
    let page = page_in(&mut engine, 3, &session);
    let stored = store(&mut engine, 4, &proving_ground, &page, "idle");
    let while_open = browsers_once(&env, 1);
    let _ = engine.ask(5, "session.close", &json!({ "session_id": session }));
    let once_idle = browsers_once(&env, 0);
    let next = new_session(&mut engine, 6, "idle");
    let next_page = page_in(&mut engine, 7, &next);
    let stored_again = store(&mut engine, 8, &proving_ground, &next_page, "again");
    let relaunched = browsers_once(&env, 1);
    let _ = engine.ask(9, "run.finish", &json!({ "run_id": "idle" }));
    let _ = engine.finish();

    assert_eq!(
        ((stored, stored_again), (while_open, once_idle, relaunched)),
        ((json!(200), json!(200)), (1, 0, 1))
    );
}

/// Whether the Run `run_id`'s trace recorded `method`, once it did, or
/// after [`SETTLE`] whatever it recorded then.
#[cfg(unix)]
fn traced_once(env: &TestEnv, run_id: &str, method: &str) -> bool {
    let deadline = Instant::now() + SETTLE;
    loop {
        let traced = crate::page::traced_methods(env, run_id).contains(method);
        if traced || Instant::now() > deadline {
            return traced;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// A Session that closes while the engine's own page still reads its
/// document's robots.txt closes that page with its others, so the browser
/// it held is idle past `governor.browser.recycle_idle` and closes: the
/// stalled-robots host never sends its robots.txt's body, so the read is
/// still waiting on it when the Session closes, far short of its
/// `timeout.navigation`.
/// Covers: story:3-62, setting:governor.browser.recycle_idle, operation:session.close
#[cfg(unix)]
#[test]
fn a_session_closed_while_its_robots_txt_read_waits_leaves_its_browser_idle() {
    const STALLED: &str = "stalled-robots.access-policy.test";
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut settings = pool(json!({ "recycle_idle": 500 }));
    settings.insert("timeout".to_owned(), json!({ "navigation": 120_000 }));
    let mut engine = started_with(&env, &cache, &proving_ground, "stalled", &settings, |_| {});

    let session = new_session(&mut engine, 2, "stalled");
    let page = page_in(&mut engine, 3, &session);
    let went = engine.ask(
        4,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url(STALLED, "/catalog/1") }),
    )["result"]["response"]["status"]
        .clone();
    // The engine's page took the robots.txt's body as a stream, which never
    // ends.
    let reading = traced_once(&env, "stalled", "Fetch.takeResponseBodyAsStream");
    let _ = engine.ask(5, "session.close", &json!({ "session_id": session }));
    let once_idle = browsers_once(&env, 0);
    let _ = engine.ask(6, "run.finish", &json!({ "run_id": "stalled" }));
    let _ = engine.finish();

    assert_eq!((went, reading, once_idle), (json!(200), true, 0));
}

/// A robots.txt read whose body stalls past the Run's
/// `timeout.navigation` broke off: Provenance records the file
/// unreachable, never the start that arrived as the whole file. The
/// stalled-robots host sends its robots.txt's head and the start of its
/// body, then nothing.
/// Covers: setting:timeout.navigation
#[cfg(unix)]
#[test]
fn a_robots_txt_read_that_stalls_past_the_navigation_timeout_is_unreachable() {
    const STALLED: &str = "stalled-robots.access-policy.test";
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut settings = Map::new();
    settings.insert("timeout".to_owned(), json!({ "navigation": 3_000 }));
    let mut engine = started_with(&env, &cache, &proving_ground, "stalls", &settings, |_| {});

    let session = new_session(&mut engine, 2, "stalls");
    let page = page_in(&mut engine, 3, &session);
    let went = engine.ask(
        4,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": proving_ground.url(STALLED, "/catalog/1") }),
    )["result"]["response"]["status"]
        .clone();
    let deadline = Instant::now() + SETTLE;
    let mut files: Vec<Value> = Vec::new();
    while files.is_empty() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        files = robots_txt_readings(&env, "stalls")
            .iter()
            .map(|entry| entry["file"].clone())
            .collect();
    }
    let _ = engine.ask(5, "run.finish", &json!({ "run_id": "stalls" }));
    let _ = engine.finish();

    assert_eq!((went, files), (json!(200), vec![json!("unreachable")]));
}

/// A browser is not idle while a Session is seated in it or opens a page
/// there: with `governor.browser.recycle_idle` shorter than any launch,
/// each Session still gets its page instead of a browser recycled before
/// the page opens, again and again.
/// Covers: story:3-62, setting:governor.browser.recycle_idle, operation:session.new_page
#[cfg(unix)]
#[test]
fn a_browser_launching_for_a_session_is_not_idle_before_its_page_opens() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "launching",
        &pool(json!({ "recycle_idle": 1 })),
        |_| {},
    );

    let session = new_session(&mut engine, 2, "launching");
    let page = page_in(&mut engine, 3, &session);
    let stored = store(&mut engine, 4, &proving_ground, &page, "launching");
    let _ = engine.ask(5, "run.finish", &json!({ "run_id": "launching" }));
    let _ = engine.finish();

    assert_eq!(stored, json!(200));
}

/// A browser that does not come up within the call's deadline is closed
/// before the call answers its failure, so no launch the call gave up on
/// stays running beside the next one on the same data folder: with
/// `timeout.navigation` far shorter than any browser takes to answer, each
/// `run.new_page` raises `deadline_exceeded`, and no browser is left.
/// Covers: operation:run.new_page, error:deadline_exceeded, setting:timeout.navigation
#[cfg(unix)]
#[test]
fn a_browser_that_does_not_come_up_in_time_is_closed_before_the_next_launch() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut settings = Map::new();
    settings.insert(
        "timeout".to_owned(),
        json!({ "navigation": 1, "close": 1_000 }),
    );
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "slow-start",
        &settings,
        |_| {},
    );

    let first = engine.ask(2, "run.new_page", &json!({ "run_id": "slow-start" }));
    let after_first = browsers_once(&env, 0);
    let second = engine.ask(3, "run.new_page", &json!({ "run_id": "slow-start" }));
    let after_second = browsers_once(&env, 0);
    let _ = engine.ask(4, "run.finish", &json!({ "run_id": "slow-start" }));
    let _ = engine.finish();

    assert_eq!(
        (
            &first["error"]["data"]["code"],
            &second["error"]["data"]["code"],
            after_first,
            after_second,
        ),
        (
            &json!("deadline_exceeded"),
            &json!("deadline_exceeded"),
            0,
            0
        ),
        "{first} {second}"
    );
}

/// The identity seed of each Session the Run's run state keeps, by its
/// position.
fn identity_seeds(env: &TestEnv, run_id: &str) -> Vec<(u32, i64)> {
    let file = HomeLayout::of(env.home()).run_file(run_id);
    let connection = rusqlite::Connection::open(file).unwrap();
    let mut statement = connection
        .prepare("SELECT position, identity_seed FROM page_sessions ORDER BY position")
        .unwrap();
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .unwrap()
        .collect::<rusqlite::Result<_>>()
        .unwrap()
}

/// Each Session draws its own identity seed when it opens, kept in run
/// state, and a recycle leaves it as it was (ADR 0059).
/// Covers: story:3-62, operation:run.new_session
#[cfg(unix)]
#[test]
fn each_session_keeps_its_own_identity_seed_in_run_state_across_a_recycle() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "seeded",
        &pool(json!({ "recycle_pages": 1 })),
        |_| {},
    );
    let first = new_session(&mut engine, 2, "seeded");
    let _ = new_session(&mut engine, 3, "seeded");
    let before = identity_seeds(&env, "seeded");

    let _ = page_in(&mut engine, 4, &first);
    let _ = page_in(&mut engine, 5, &first);
    let after = identity_seeds(&env, "seeded");
    let _ = engine.ask(6, "run.finish", &json!({ "run_id": "seeded" }));
    let _ = engine.finish();

    assert_eq!(
        (
            before
                .iter()
                .map(|(position, _)| *position)
                .collect::<Vec<_>>(),
            before[0].1 != before[1].1,
            after == before,
        ),
        (vec![1, 2], true, true)
    );
}
