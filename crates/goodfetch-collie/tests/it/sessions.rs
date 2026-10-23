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
