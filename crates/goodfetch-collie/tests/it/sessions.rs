//! A Collie program's Sessions over stdio (spec #577, ticket #697), through
//! `goodfetch-protocol`'s types: `run.new_session` opens a Session of its
//! own, its own browser context in the Run's one browser process, whose
//! cookies and `localStorage` no other Session of the Run shares, as the
//! Proving ground's record of what each request sent shows (ADRs 0021 and
//! 0092; ADR 0037 / SA-09, SA-10).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

#[cfg(unix)]
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::page::{PageOpened, SessionId};
use goodfetch_test_support::{Mode, TestEnv};
#[cfg(unix)]
use goodfetch_test_support::{ProvingGround, browser_cache};
use rstest::rstest;
use serde_json::{Value, json};

#[cfg(unix)]
use crate::COLLIE;
use crate::page::{opened, started, started_with};
#[cfg(unix)]
use crate::page_events::Exchange;
use goodfetch_test_client::Attached;

/// The Proving ground's own seed for these tests.
#[cfg(unix)]
const SEED: u64 = 697;

/// The Session `run.new_session` answered.
#[cfg(unix)]
fn session_of(answer: &Value) -> SessionId {
    serde_json::from_value(answer["result"]["session_id"].clone())
        .unwrap_or_else(|error| panic!("{error}: {answer}"))
}

/// Opens a page in the Session `session`.
#[cfg(unix)]
fn page_in(engine: &mut Attached, id: i64, session: &SessionId) -> PageOpened {
    opened(&engine.ask(id, "session.new_page", &json!({ "session_id": session })))
}

/// Loads the storage page on `page` with `mark`, and answers its status.
#[cfg(unix)]
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

/// The requests the Proving ground received for targets on `scope.test`
/// that start with `prefix`, in arrival order: each one's target and the
/// cookies it sent.
#[cfg(unix)]
fn storage_requests(proving_ground: &ProvingGround, prefix: &str) -> Vec<(String, Value)> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == "scope.test" && request.target.starts_with(prefix))
        .map(|request| (request.target.clone(), json!(request.cookies)))
        .collect()
}

/// Covers: story:5-60, operation:run.new_session, operation:session.new_page
#[cfg(unix)]
#[test]
fn two_sessions_of_one_run_keep_their_own_cookies_and_storage_in_one_browser_process() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "isolated");

    let first = session_of(&engine.ask(2, "run.new_session", &json!({ "run_id": "isolated" })));
    let second = session_of(&engine.ask(3, "run.new_session", &json!({ "run_id": "isolated" })));
    let first_page = page_in(&mut engine, 4, &first);
    let second_page = page_in(&mut engine, 5, &second);
    let statuses = [
        store(&mut engine, 6, &proving_ground, &first_page, "first"),
        store(&mut engine, 7, &proving_ground, &second_page, "second"),
        store(&mut engine, 8, &proving_ground, &first_page, "first"),
        store(&mut engine, 9, &proving_ground, &second_page, "second"),
    ];
    // A browser's own helpers (renderers, its GPU and network processes,
    // Linux's zygote) each name the `--type` they are.
    let processes = crate::page::browser_command_lines(&env)
        .iter()
        .filter(|line| !line.contains("--type="))
        .count();
    let finished = engine.ask(10, "run.finish", &json!({ "run_id": "isolated" }));
    let _ = engine.finish();

    assert_eq!(
        (
            first != second,
            (
                first_page.session_id == first,
                second_page.session_id == second
            ),
            statuses.to_vec(),
            storage_requests(&proving_ground, "/storage?"),
            storage_requests(&proving_ground, "/storage/found")
                .into_iter()
                .map(|(target, _)| target)
                .collect::<Vec<_>>(),
            processes,
            &finished["result"]["lifecycle"],
        ),
        (
            true,
            (true, true),
            vec![json!(200); 4],
            vec![
                ("/storage?mark=first".to_owned(), json!([])),
                ("/storage?mark=second".to_owned(), json!([])),
                ("/storage?mark=first".to_owned(), json!([["mark", "first"]])),
                (
                    "/storage?mark=second".to_owned(),
                    json!([["mark", "second"]])
                ),
            ],
            vec![
                "/storage/found?mark=none".to_owned(),
                "/storage/found?mark=none".to_owned(),
                "/storage/found?mark=first".to_owned(),
                "/storage/found?mark=second".to_owned(),
            ],
            1,
            &json!("finished"),
        )
    );
}

/// The Error code of `answer`'s error, if it failed.
fn code_of(answer: &Value) -> &Value {
    &answer["error"]["data"]["code"]
}

/// Covers: story:5-60, operation:session.close, operation:run.new_page, error:closed
#[cfg(unix)]
#[test]
fn a_closed_session_closes_its_pages_gives_their_admissions_back_and_closes_again_as_a_no_op() {
    let env = TestEnv::new();
    // One browser page at a time: a page of a closed Session that kept its
    // admission would hold the next page's opening for good.
    env.user_configuration("[settings.governor.browser_page]\nfloor = 1\nceiling = 1\n");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "closing");

    let first = session_of(&engine.ask(2, "run.new_session", &json!({ "run_id": "closing" })));
    let first_page = page_in(&mut engine, 3, &first);
    let stored = store(&mut engine, 4, &proving_ground, &first_page, "first");
    let closed = engine.ask(5, "session.close", &json!({ "session_id": first }));
    let closed_again = engine.ask(6, "session.close", &json!({ "session_id": first }));
    let gone_page = engine.ask(
        7,
        "page.goto",
        &json!({
            "page_id": first_page.page_id,
            "url": proving_ground.url("scope.test", "/about"),
        }),
    );
    let no_new_page = engine.ask(8, "session.new_page", &json!({ "session_id": first }));
    let default_page = opened(&engine.ask(9, "run.new_page", &json!({ "run_id": "closing" })));
    let fresh = store(&mut engine, 10, &proving_ground, &default_page, "default");
    let default_closed = engine.ask(
        11,
        "session.close",
        &json!({ "session_id": default_page.session_id }),
    );
    let no_default_page = engine.ask(12, "run.new_page", &json!({ "run_id": "closing" }));
    let finished = engine.ask(13, "run.finish", &json!({ "run_id": "closing" }));
    let _ = engine.finish();

    assert_eq!(
        (
            (stored, fresh),
            (&closed["result"], &closed_again["result"]),
            (code_of(&gone_page), code_of(&no_new_page)),
            (&default_closed["result"], code_of(&no_default_page)),
            storage_requests(&proving_ground, "/storage?"),
            &finished["result"]["lifecycle"],
        ),
        (
            (json!(200), json!(200)),
            (&json!({}), &json!({})),
            (&json!("closed"), &json!("closed")),
            (&json!({}), &json!("closed")),
            vec![
                ("/storage?mark=first".to_owned(), json!([])),
                ("/storage?mark=default".to_owned(), json!([])),
            ],
            &json!("finished"),
        )
    );
}

/// A page a Session asked for while the governor held it back, and the
/// Session closed meanwhile, never seats the closed Session again: the call
/// raises `closed`, and the
/// browser keeps no context for it, so the Run's next Session takes the
/// context it gave back in the same browser process (#697's criterion; the
/// deep review's 577-2).
///
/// Covers: story:5-60, operation:session.close, operation:session.new_page, setting:governor.browser.contexts_per_process
#[cfg(unix)]
#[test]
fn a_page_asked_for_across_its_sessions_close_gives_its_browser_context_back() {
    let env = TestEnv::new();
    // One browser page at a time, so the second Session's page waits for
    // the first's to close.
    env.user_configuration("[settings.governor.browser_page]\nfloor = 1\nceiling = 1\n");
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let settings = json!({ "governor": { "browser": { "contexts_per_process": 2 } } });
    let engine = started_with(
        &env,
        &cache,
        &proving_ground,
        "reseated",
        settings.as_object().unwrap(),
        |_| {},
    );
    let mut exchange = Exchange::new(engine);
    let new_session = json!({ "run_id": "reseated" });
    let first = exchange.call("run.new_session", &new_session)["result"]["session_id"].clone();
    let second = exchange.call("run.new_session", &new_session)["result"]["session_id"].clone();
    let first_page = opened(&exchange.call("session.new_page", &json!({ "session_id": first })));

    let held = exchange.request("session.new_page", &json!({ "session_id": second }));
    // A call answered after it shows the engine took the held one in.
    let _ = exchange.call("session.pages", &json!({ "session_id": first }));
    let closed = exchange.call("session.close", &json!({ "session_id": second }));
    let _ = exchange.call("page.close", &json!({ "page_id": first_page.page_id }));
    let held = exchange.answer(held);
    let third = exchange.call("run.new_session", &new_session);
    let browsers = crate::page::browser_command_lines(&env)
        .iter()
        .filter(|line| !line.contains("--type="))
        .count();
    let _ = exchange.call("run.finish", &json!({ "run_id": "reseated" }));
    let _ = exchange.engine.finish();

    assert_eq!(
        (
            &closed["result"],
            &held["error"]["data"]["code"],
            third["result"]["session_id"].is_string(),
            browsers,
        ),
        (&json!({}), &json!("closed"), true, 1),
        "{held} {third}"
    );
}

/// The Error code of `answer`'s error and the details that name the key,
/// its layer and the Session source's kind.
#[cfg(unix)]
fn refusal_of(answer: &Value) -> (Value, Value, Value, Value) {
    let data = &answer["error"]["data"];
    (
        data["code"].clone(),
        data["key"].clone(),
        data["layer"].clone(),
        data["kind"].clone(),
    )
}

/// Covers: story:5-62, operation:run.new_session, error:setting_not_applicable, error:capability_unavailable
#[cfg(unix)]
#[test]
fn session_source_is_refused_in_a_sessions_own_settings_and_the_runs_is_the_default_beneath() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut settings = serde_json::Map::new();
    settings.insert(
        "session".to_owned(),
        json!({ "source": { "kind": "saved_identity", "name": "portal" } }),
    );
    let mut engine =
        crate::page::started_with(&env, &cache, &proving_ground, "sourced", &settings, |_| {});

    let in_settings = engine.ask(
        2,
        "run.new_session",
        &json!({
            "run_id": "sourced",
            "settings": { "session": { "source": { "kind": "saved_identity", "name": "other" } } },
        }),
    );
    let other_setting = engine.ask(
        3,
        "run.new_session",
        &json!({ "run_id": "sourced", "settings": { "browser": { "headless": true } } }),
    );
    let inherited = engine.ask(4, "run.new_session", &json!({ "run_id": "sourced" }));
    let default_inherited = engine.ask(5, "run.new_page", &json!({ "run_id": "sourced" }));
    let own = engine.ask(
        6,
        "run.new_session",
        &json!({
            "run_id": "sourced",
            "source": { "kind": "cookie_header", "header": "mark=own", "domain": "scope.test" },
        }),
    );
    let fresh = session_of(&engine.ask(
        7,
        "run.new_session",
        &json!({ "run_id": "sourced", "source": null }),
    ));
    let page = page_in(&mut engine, 8, &fresh);
    let stored = store(&mut engine, 9, &proving_ground, &page, "fresh");
    let _ = engine.ask(10, "run.finish", &json!({ "run_id": "sourced" }));
    let _ = engine.finish();

    let refused =
        |code: &str, key: &str, kind: Value| (json!(code), json!(key), json!("run"), kind);
    assert_eq!(
        (
            refusal_of(&in_settings),
            refusal_of(&other_setting),
            refusal_of(&inherited),
            refusal_of(&default_inherited),
            refusal_of(&own),
            stored,
        ),
        (
            refused("setting_not_applicable", "session.source", Value::Null),
            refused("setting_not_applicable", "browser.headless", Value::Null),
            refused(
                "capability_unavailable",
                "session.source",
                json!("saved_identity")
            ),
            refused(
                "capability_unavailable",
                "session.source",
                json!("saved_identity")
            ),
            refused(
                "capability_unavailable",
                "session.source",
                json!("cookie_header")
            ),
            json!(200),
        )
    );
}

/// Covers: error:run_ended, operation:run.new_session, operation:run.new_page
#[test]
fn new_work_on_a_finished_run_raises_run_ended() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&goodfetch_test_client::hello(Product::Collie));
    let _ = engine.next();
    let _ = engine.ask(1, "run.start", &json!({ "run_id": "ended" }));
    let _ = engine.ask(2, "run.finish", &json!({ "run_id": "ended" }));

    let session = engine.ask(3, "run.new_session", &json!({ "run_id": "ended" }));
    let page = engine.ask(4, "run.new_page", &json!({ "run_id": "ended" }));
    let never_started = engine.ask(5, "run.new_session", &json!({ "run_id": "never-started" }));
    let _ = engine.finish();

    assert_eq!(
        (
            code_of(&session),
            &session["error"]["data"]["run_id"],
            code_of(&page),
            code_of(&never_started),
        ),
        (
            &json!("run_ended"),
            &json!("ended"),
            &json!("run_ended"),
            &json!("run_not_found"),
        )
    );
}

/// Waits up to 30 s for every browser process the test's engines started to
/// exit, and answers how many are left.
#[cfg(unix)]
#[expect(
    clippy::disallowed_methods,
    reason = "the test polls the process list on its own thread, never an async worker"
)]
fn browsers_left(env: &TestEnv) -> usize {
    for _ in 0..300 {
        if crate::page::browser_command_lines(env).is_empty() {
            return 0;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    crate::page::browser_command_lines(env).len()
}

/// Covers: story:5-60, operation:run.resume, operation:session.new_page, operation:session.close
#[cfg(unix)]
#[test]
fn a_run_resumed_after_its_engine_was_killed_reopens_its_sessions_from_run_state() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "resumed");
    let default_page = opened(&engine.ask(2, "run.new_page", &json!({ "run_id": "resumed" })));
    let own = session_of(&engine.ask(3, "run.new_session", &json!({ "run_id": "resumed" })));
    let closed = session_of(&engine.ask(4, "run.new_session", &json!({ "run_id": "resumed" })));
    let _ = engine.ask(5, "session.close", &json!({ "session_id": closed }));
    engine.kill();
    let left = browsers_left(&env);

    let mut resumed =
        Attached::start_with(&env, COLLIE, crate::page::BROWSER_DEADLINE, |command| {
            command.env(goodfetch_test_support::BROWSERS_PATH_VARIABLE, &cache);
        });
    resumed.send(&goodfetch_test_client::hello(Product::Collie));
    let _ = resumed.next();
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    let resume = resumed.ask(
        1,
        "run.resume",
        &json!({
            "run_id": "resumed",
            "settings": {
                "network": { "resolve": resolve },
                "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
            },
        }),
    );
    let own_page = page_in(&mut resumed, 2, &own);
    let stored = store(&mut resumed, 3, &proving_ground, &own_page, "own");
    let default_again = opened(&resumed.ask(4, "run.new_page", &json!({ "run_id": "resumed" })));
    let still_closed = resumed.ask(5, "session.new_page", &json!({ "session_id": closed }));
    let closed_again = resumed.ask(6, "session.close", &json!({ "session_id": closed }));
    let finished = resumed.ask(7, "run.finish", &json!({ "run_id": "resumed" }));
    let _ = resumed.finish();

    assert_eq!(
        (
            left,
            &resume["result"]["started_as"],
            (own_page.session_id == own, stored),
            default_again.session_id == default_page.session_id,
            (code_of(&still_closed), &closed_again["result"]),
            &finished["result"]["lifecycle"],
        ),
        (
            0,
            &json!("resumed"),
            (true, json!(200)),
            true,
            (&json!("closed"), &json!({})),
            &json!("finished"),
        )
    );
}

/// The ad of `clean-up.test`'s `/ads` page, which EasyList blocks.
#[cfg(unix)]
const AD: &str = "/img/top-ad-sidebar.gif";

/// Loads `clean-up.test`'s `/ads` page on `page`, and answers how many
/// requests for its ad the Proving ground has received since it started.
#[cfg(unix)]
fn ads_loaded(
    engine: &mut Attached,
    id: i64,
    proving_ground: &ProvingGround,
    page: &PageOpened,
) -> usize {
    let url = proving_ground.url("clean-up.test", "/ads");
    let loaded = engine.ask(
        id,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    );
    assert_eq!(
        loaded["result"]["response"]["status"],
        json!(200),
        "{loaded}"
    );
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| request.host == "clean-up.test" && request.target == AD)
        .count()
}

/// Covers: story:5-94, operation:run.new_session, setting:filter_lists.ads
#[cfg(unix)]
#[test]
fn a_sessions_own_filter_list_settings_choose_its_pages_clean_up_over_the_runs_and_survive_a_resume()
 {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = browser_cache(&env).unwrap();
    let mut engine = started(&env, &cache, &proving_ground, "cleaned");
    let own = session_of(&engine.ask(
        2,
        "run.new_session",
        &json!({ "run_id": "cleaned", "settings": { "filter_lists": { "ads": false } } }),
    ));
    let default_page = opened(&engine.ask(3, "run.new_page", &json!({ "run_id": "cleaned" })));
    let by_the_run = ads_loaded(&mut engine, 4, &proving_ground, &default_page);
    let own_page = page_in(&mut engine, 5, &own);
    let by_the_session = ads_loaded(&mut engine, 6, &proving_ground, &own_page);
    engine.kill();

    let mut resumed =
        Attached::start_with(&env, COLLIE, crate::page::BROWSER_DEADLINE, |command| {
            command.env(goodfetch_test_support::BROWSERS_PATH_VARIABLE, &cache);
        });
    resumed.send(&goodfetch_test_client::hello(Product::Collie));
    let _ = resumed.next();
    let resolve: serde_json::Map<String, Value> = proving_ground
        .description()
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    let _ = resumed.ask(
        1,
        "run.resume",
        &json!({
            "run_id": "cleaned",
            "settings": {
                "network": { "resolve": resolve },
                "egress": { "private_ranges": resolve.keys().collect::<Vec<_>>() },
            },
        }),
    );
    let own_page = page_in(&mut resumed, 2, &own);
    let after_the_resume = ads_loaded(&mut resumed, 3, &proving_ground, &own_page);
    let _ = resumed.ask(4, "run.finish", &json!({ "run_id": "cleaned" }));
    let _ = resumed.finish();

    assert_eq!((by_the_run, by_the_session, after_the_resume), (0, 1, 2));
}

/// Runs Collie's command `args` with `--json` in `mode`, which fails, and
/// returns its exit status, Error code and operation.
fn failed_command(env: &TestEnv, args: &[&str], mode: Mode) -> (Option<i32>, Value, Value) {
    let checked = goodfetch_test_support::cli(env, env!("CARGO_BIN_EXE_goodfetch-collie"))
        .args(args)
        .arg("--json")
        .mode(mode)
        .json("CommandFailed")
        .unwrap();
    (
        checked.output.exit,
        checked.documents[0]["error"]["code"].clone(),
        checked.documents[0]["error"]["operation"].clone(),
    )
}

/// `run new-session` through the command generated from the catalog, on a
/// Run the engine of an earlier command stopped as it exited: no live Run
/// has its ID, so the call fails with `run_not_found`.
/// Covers: command:run.new-session
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn run_new_session_on_a_run_an_earlier_command_stopped_fails_with_run_not_found(
    #[case] mode: Mode,
) {
    let env = TestEnv::new();
    let started = goodfetch_test_support::cli(&env, env!("CARGO_BIN_EXE_goodfetch-collie"))
        .args(["run", "start", "--run-id", "first", "--attached"])
        .run()
        .unwrap();
    assert_eq!(started.exit, Some(0), "{started}");

    assert_eq!(
        failed_command(&env, &["run", "new-session", "first"], mode),
        (Some(3), json!("run_not_found"), json!("run.new_session"))
    );
}

/// `session new-page`, `session close` and `session pages` (#719) through
/// the commands generated from the catalog: no engine holds the Session
/// they name, so each fails with `closed`.
/// Covers: command:session.new-page, command:session.close, command:session.pages
#[rstest]
#[case::attached(Mode::Attached)]
#[case::daemon(Mode::Daemon)]
fn session_commands_on_a_session_no_engine_holds_fail_with_closed(#[case] mode: Mode) {
    let env = TestEnv::new();

    assert_eq!(
        (
            failed_command(&env, &["session", "new-page", "gone.session-1"], mode),
            failed_command(&env, &["session", "close", "gone.session-1"], mode),
            failed_command(&env, &["session", "pages", "gone.session-1"], mode),
        ),
        (
            (Some(3), json!("closed"), json!("session.new_page")),
            (Some(3), json!("closed"), json!("session.close")),
            (Some(3), json!("closed"), json!("session.pages")),
        )
    );
}
