//! A start reaching the Run its Run ID names through the client crate
//! (ADRs 0078 and 0102), with the Scripted engine as the Daemon playing the
//! corpus's script of the `start-reaches-its-run` client conformance case,
//! which the TypeScript SDK plays too: a fresh Run ID begins the Run, an
//! interrupted Run's resumes it, logged at info, and a finished Run's
//! returns it ended, logged at warning, where a call that would give it
//! work fails with `run_ended`; each start the engine refuses fails with
//! its code and details. The script checks every message the crate sends,
//! so `Run::id` sends none.
//! Covers: story:11-126, story:11-127

use goodfetch_collie_client::transport::{Endpoint, Network};
use goodfetch_collie_client::{ClientConnection, ConnectionRuntime, Error, ErrorCode};
use goodfetch_protocol::catalog::ErrorGroup;
use goodfetch_protocol::connection::DaemonEndpoint;
use goodfetch_protocol::run::{
    ReachedLimit, RunConflictDetails, RunDetails, RunId, StartInput, StartedAs, Tags,
};
use goodfetch_test_support::{TestEnv, logged};
use serde_json::json;

use crate::scripted_daemon::{assert_start_lines, scripted_daemon_hearing, traced_daemon_hello};

/// The corpus's script of the case.
const SCRIPT: &str = "run/start-reaches-its-run.json";

/// The failure `result` holds.
pub(crate) fn failure<T: std::fmt::Debug>(result: Result<T, Error>) -> Error {
    let Err(failure) = result else {
        panic!("{result:?}");
    };
    failure
}

/// `error`'s code, group and whether it is retryable.
pub(crate) fn code_group_retryable(error: &Error) -> (ErrorCode, ErrorGroup, bool) {
    (error.code(), error.group(), error.retryable())
}

/// What each step of the script answered.
struct Played {
    started_as: [StartedAs; 3],
    named: RunId,
    no_work: Error,
    in_use: Error,
    conflict: Error,
    reached: Error,
}

/// Plays the script's steps on a Connection to the Daemon at `endpoint`.
async fn play(endpoint: DaemonEndpoint) -> Played {
    let id = |text: &str| RunId::parse(text).unwrap();
    let (connection, _said) = ClientConnection::open(
        ConnectionRuntime::driven(tokio::runtime::Handle::current()),
        &Network::Os,
        &Endpoint::Daemon(endpoint),
        traced_daemon_hello(),
    )
    .await
    .unwrap();

    let begun = connection
        .start()
        .run_id(&id("nightly"))
        .tags(Tags::parse(["shop", "eu"]).unwrap())
        .await
        .unwrap();
    let named = begun.id().clone();
    begun.close().await.unwrap();
    let resumed = connection.start().run_id(&id("nightly")).await.unwrap();
    resumed.finish().await.unwrap();
    let ended = connection.start().run_id(&id("nightly")).await.unwrap();
    let no_work = failure(ended.new_page().await);
    let in_use = failure(connection.start().run_id(&id("busy")).await);
    let conflict = failure(connection.start().run_id(&id("crawled")).await);
    let reached = failure(connection.start().run_id(&id("capped")).await);
    connection.close().await.unwrap();
    // `run.started_as` is a read (issue 2798): it answers once the
    // Connection is closed, which it could not if it sent a message.
    Played {
        started_as: [begun.started_as(), resumed.started_as(), ended.started_as()],
        named,
        no_work,
        in_use,
        conflict,
        reached,
    }
}

#[test]
fn a_start_reaches_the_run_its_run_id_names_logs_a_resume_and_an_end_and_fails_each_refusal() {
    let env = TestEnv::new();
    let (process, endpoint) = scripted_daemon_hearing(&env, SCRIPT, &traced_daemon_hello());
    let id = |text: &str| RunId::parse(text).unwrap();
    let runtime = goodfetch_test_support::real_time_runtime();
    let (played, log) = logged(|| runtime.block_on(play(endpoint)));
    let Played {
        started_as,
        named,
        no_work,
        in_use,
        conflict,
        reached,
    } = played;

    assert_eq!(
        (started_as, named.as_str()),
        (
            [StartedAs::New, StartedAs::Resumed, StartedAs::Ended],
            "nightly"
        )
    );
    assert_start_lines(&String::from_utf8(log).unwrap(), "nightly", "nightly");
    assert_eq!(
        (
            no_work.code(),
            no_work.retryable(),
            no_work.details::<RunDetails>().unwrap().run_id
        ),
        (ErrorCode::RUN_ENDED, false, id("nightly"))
    );
    assert_eq!(
        (
            in_use.code(),
            in_use.retryable(),
            in_use.details::<RunDetails>().unwrap().run_id
        ),
        (ErrorCode::RUN_IN_USE, true, id("busy"))
    );
    assert_eq!(
        (
            conflict.code(),
            conflict.details::<RunConflictDetails>().unwrap()
        ),
        (
            ErrorCode::RUN_CONFLICT,
            RunConflictDetails {
                run_id: id("crawled"),
                differences: vec![
                    StartInput::Product,
                    StartInput::Kind,
                    StartInput::Caller,
                    StartInput::Seeds
                ],
            }
        )
    );
    let reached_limit: ReachedLimit = reached.details().unwrap();
    assert_eq!(
        (
            reached.code(),
            serde_json::to_value(&reached_limit.scope).unwrap(),
            reached_limit.usage_kind.as_str(),
            serde_json::to_value(&reached_limit).unwrap()["resets_at"].clone(),
        ),
        (
            ErrorCode::LIMIT_REACHED,
            json!({ "kind": "run", "id": "capped" }),
            "requests",
            json!("2026-02-01T00:00:00Z"),
        )
    );
    let status = process.child().wait().unwrap();
    assert!(
        status.success(),
        "the Scripted engine played its script whole"
    );
}
