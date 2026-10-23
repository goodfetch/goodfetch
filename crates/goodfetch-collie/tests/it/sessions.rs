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

use goodfetch_protocol::page::{PageOpened, SessionId};
use goodfetch_test_support::{ProvingGround, TestEnv, browser_cache};
use serde_json::{Value, json};

use crate::attached::Attached;
use crate::page::{opened, started};
use crate::run::call;

/// The Proving ground's own seed for these tests.
const SEED: u64 = 697;

/// The Session `run.new_session` answered.
fn session_of(answer: &Value) -> SessionId {
    serde_json::from_value(answer["result"]["session_id"].clone())
        .unwrap_or_else(|error| panic!("{error}: {answer}"))
}

/// Opens a page in the Session `session`.
fn page_in(engine: &mut Attached, id: i64, session: &SessionId) -> PageOpened {
    opened(&call(
        engine,
        id,
        "session.new_page",
        &json!({ "session_id": session }),
    ))
}

/// Loads the storage page on `page` with `mark`, and answers its status.
fn store(
    engine: &mut Attached,
    id: i64,
    proving_ground: &ProvingGround,
    page: &PageOpened,
    mark: &str,
) -> Value {
    let url = proving_ground.url("scope.test", &format!("/storage?mark={mark}"));
    call(
        engine,
        id,
        "page.goto",
        &json!({ "page_id": page.page_id, "url": url }),
    )["result"]["response"]["status"]
        .clone()
}

/// The requests the Proving ground received for targets on `scope.test`
/// that start with `prefix`, in arrival order: each one's target and the
/// cookies it sent.
fn storage_requests(proving_ground: &ProvingGround, prefix: &str) -> Vec<(String, Value)> {
    proving_ground.observations().unwrap()["requests"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|request| {
            request["host"] == json!("scope.test")
                && request["target"]
                    .as_str()
                    .is_some_and(|target| target.starts_with(prefix))
        })
        .map(|request| {
            (
                request["target"].as_str().unwrap().to_owned(),
                request["cookies"].clone(),
            )
        })
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

    let first = session_of(&call(
        &mut engine,
        2,
        "run.new_session",
        &json!({ "run_id": "isolated" }),
    ));
    let second = session_of(&call(
        &mut engine,
        3,
        "run.new_session",
        &json!({ "run_id": "isolated" }),
    ));
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
    let finished = call(
        &mut engine,
        10,
        "run.finish",
        &json!({ "run_id": "isolated" }),
    );
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

    let first = session_of(&call(
        &mut engine,
        2,
        "run.new_session",
        &json!({ "run_id": "closing" }),
    ));
    let first_page = page_in(&mut engine, 3, &first);
    let stored = store(&mut engine, 4, &proving_ground, &first_page, "first");
    let closed = call(
        &mut engine,
        5,
        "session.close",
        &json!({ "session_id": first }),
    );
    let closed_again = call(
        &mut engine,
        6,
        "session.close",
        &json!({ "session_id": first }),
    );
    let gone_page = call(
        &mut engine,
        7,
        "page.goto",
        &json!({
            "page_id": first_page.page_id,
            "url": proving_ground.url("scope.test", "/about"),
        }),
    );
    let no_new_page = call(
        &mut engine,
        8,
        "session.new_page",
        &json!({ "session_id": first }),
    );
    let default_page = opened(&call(
        &mut engine,
        9,
        "run.new_page",
        &json!({ "run_id": "closing" }),
    ));
    let fresh = store(&mut engine, 10, &proving_ground, &default_page, "default");
    let default_closed = call(
        &mut engine,
        11,
        "session.close",
        &json!({ "session_id": default_page.session_id }),
    );
    let no_default_page = call(
        &mut engine,
        12,
        "run.new_page",
        &json!({ "run_id": "closing" }),
    );
    let finished = call(
        &mut engine,
        13,
        "run.finish",
        &json!({ "run_id": "closing" }),
    );
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

/// The Error code of `answer`'s error and the details that name the key,
/// its layer and the Session source's kind.
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

    let in_settings = call(
        &mut engine,
        2,
        "run.new_session",
        &json!({
            "run_id": "sourced",
            "settings": { "session": { "source": { "kind": "saved_identity", "name": "other" } } },
        }),
    );
    let other_setting = call(
        &mut engine,
        3,
        "run.new_session",
        &json!({ "run_id": "sourced", "settings": { "browser": { "headless": true } } }),
    );
    let inherited = call(
        &mut engine,
        4,
        "run.new_session",
        &json!({ "run_id": "sourced" }),
    );
    let default_inherited = call(
        &mut engine,
        5,
        "run.new_page",
        &json!({ "run_id": "sourced" }),
    );
    let own = call(
        &mut engine,
        6,
        "run.new_session",
        &json!({
            "run_id": "sourced",
            "source": { "kind": "cookie_header", "header": "mark=own", "domain": "scope.test" },
        }),
    );
    let fresh = session_of(&call(
        &mut engine,
        7,
        "run.new_session",
        &json!({ "run_id": "sourced", "source": null }),
    ));
    let page = page_in(&mut engine, 8, &fresh);
    let stored = store(&mut engine, 9, &proving_ground, &page, "fresh");
    let _ = call(
        &mut engine,
        10,
        "run.finish",
        &json!({ "run_id": "sourced" }),
    );
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
    let mut engine = Attached::start(&env);
    engine.send(&crate::attached::hello("collie", "0.1"));
    let _ = engine.next();
    let _ = call(&mut engine, 1, "run.start", &json!({ "run_id": "ended" }));
    let _ = call(&mut engine, 2, "run.finish", &json!({ "run_id": "ended" }));

    let session = call(
        &mut engine,
        3,
        "run.new_session",
        &json!({ "run_id": "ended" }),
    );
    let page = call(
        &mut engine,
        4,
        "run.new_page",
        &json!({ "run_id": "ended" }),
    );
    let never_started = call(
        &mut engine,
        5,
        "run.new_session",
        &json!({ "run_id": "never-started" }),
    );
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
