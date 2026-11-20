//! Foxhound's Rust SDK (#2806): a program connects to the linked Foxhound
//! engine on the production runtime, starts a Run, finishes it and closes,
//! in async, in a scope and through `blocking`, each test with its own
//! Goodfetch home given as the connection's environment map; the values
//! are the Collie client crate's own, which a helper written for Collie
//! takes. A Foxhound connection that meets a Collie engine is tested on
//! Collie's crate's linked connection, which both open through.

use connection_library::finish_one_run;
use goodfetch_foxhound::{
    Answer, ConnectOptions, Connection, Error, ErrorCode, Foxhound, RunId, RunReport, blocking,
};
use goodfetch_protocol::run::{Lifecycle, RUN_REPORT, RunParams};
use goodfetch_protocol::settings::{COLLIE_ENGINE_PATH_VARIABLE, FOXHOUND_ENGINE_PATH_VARIABLE};
use goodfetch_test_support::{TestEnv, real_time_runtime};

mod limits;
mod public_items;

/// What a connection opens with: the test's environment, which names its
/// Goodfetch home.
fn options(env: &TestEnv) -> ConnectOptions {
    ConnectOptions::default().environment(env.env_os())
}

/// A helper written for Collie: it takes the client crate's Connection,
/// which both Products' connections implement, starts a Run on it and
/// finishes it.
async fn start_and_finish(connection: &impl Connection) -> Result<RunReport, Error> {
    connection
        .start()
        .await?
        .finish()
        .await
        .map(Answer::into_inner)
}

/// The same helper written for Collie without async: it takes the client
/// crate's blocking Connection.
fn start_and_finish_blocking(connection: &impl blocking::Connection) -> Result<RunReport, Error> {
    connection
        .start()
        .send()?
        .finish()
        .send()
        .map(Answer::into_inner)
}

/// Covers: story:11-1002
#[test]
fn a_program_connects_starts_a_run_finishes_it_and_closes_on_a_single_threaded_runtime() {
    let env = TestEnv::new();
    let first = RunId::parse("first").unwrap();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env)).await.unwrap();

        let run = foxhound.start().run_id(&first).await.unwrap();
        let report = run.finish().await.unwrap();
        foxhound.close().await.unwrap();

        assert_eq!(report.run_id, first);
        assert_eq!(report.lifecycle, Lifecycle::Finished);
        let after = foxhound.start().await;
        assert!(
            matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
            "{after:?}"
        );
    });
}

/// Covers: story:11-1002
#[test]
fn a_scope_lends_its_connection_to_the_body_and_closes_it_after() {
    let env = TestEnv::new();
    let (report, kept) = real_time_runtime().block_on(async {
        Foxhound::scope(options(&env), async |foxhound| {
            let report = foxhound.start().await?.finish().await?;
            Ok((report, foxhound.clone()))
        })
        .await
        .unwrap()
    });

    assert_eq!(report.lifecycle, Lifecycle::Finished);
    let after = real_time_runtime().block_on(async { kept.start().await });
    assert!(
        matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
        "{after:?}"
    );
}

/// Two connections at once in one process, each with its own linked
/// engine, and a third after both closed.
/// Covers: story:11-1002
#[test]
fn a_process_holds_many_connections_and_connects_again_after_a_close() {
    let left = TestEnv::new();
    let right = TestEnv::new();
    real_time_runtime().block_on(async {
        let first = Foxhound::connect(options(&left)).await.unwrap();
        let second = Foxhound::connect(options(&right)).await.unwrap();

        let reports = (
            start_and_finish(&first).await.unwrap(),
            start_and_finish(&second).await.unwrap(),
        );
        first.close().await.unwrap();
        second.close().await.unwrap();
        let again = Foxhound::connect(options(&left)).await.unwrap();
        let third = start_and_finish(&again).await.unwrap();
        again.close().await.unwrap();

        assert_eq!(
            [reports.0.lifecycle, reports.1.lifecycle, third.lifecycle],
            [Lifecycle::Finished; 3]
        );
    });
}

/// Covers: story:11-1002
#[test]
fn a_program_without_async_connects_starts_a_run_finishes_it_and_closes() {
    let env = TestEnv::new();
    let foxhound = blocking::Foxhound::connect(options(&env)).unwrap();

    let report = foxhound.start().send().unwrap().finish().send().unwrap();
    foxhound.clone().close().unwrap();

    assert_eq!(report.lifecycle, Lifecycle::Finished);
    let after = foxhound.start().send();
    assert!(
        matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
        "{after:?}"
    );
}

/// `blocking` inside an async runtime never enters a runtime, which would
/// panic here.
/// Covers: story:11-1002
#[test]
fn blocking_inside_an_async_runtime_never_nests_a_runtime() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let foxhound = blocking::Foxhound::connect(options(&env)).unwrap();

        let report = foxhound.start().send().unwrap().finish().send().unwrap();
        foxhound.close().unwrap();

        assert_eq!(report.lifecycle, Lifecycle::Finished);
    });
}

/// One helper typed for the client crate's Connection takes both
/// Products' connections and closes them, and a `match` on the client
/// crate's `Error` catches what either fails with. The helper is a
/// library's, `tests/libraries/connection-library`, which takes the client
/// crate by a semver range: Cargo builds the program's one copy for both.
/// Covers: story:11-1003, story:11-1004, story:11-179, story:11-180
#[test]
fn a_helper_typed_for_connection_takes_collies_and_foxhounds() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options(&env)).await.unwrap();
        let collie = goodfetch_collie::Collie::connect(options(&env))
            .await
            .unwrap();

        let reports = [
            finish_one_run(&foxhound).await.unwrap(),
            finish_one_run(&collie).await.unwrap(),
        ];
        Connection::close(&foxhound).await.unwrap();
        Connection::close(&collie).await.unwrap();
        let closed = [
            finish_one_run(&foxhound).await.unwrap(),
            finish_one_run(&collie).await.unwrap(),
        ];

        assert_eq!(
            reports.map(|report| report.map(|report| report.lifecycle)),
            [Some(Lifecycle::Finished); 2]
        );
        assert!(closed.iter().all(Option::is_none), "{closed:?}");
    });
}

/// One helper typed for the client crate's blocking Connection takes both
/// Products' blocking connections, which carry Collie's members under the
/// same names, and closes them; a call through the client crate's own
/// `call` reads the Run's report.
/// Covers: story:11-1003, story:11-1004, story:11-179
#[test]
fn a_helper_typed_for_the_blocking_connection_takes_collies_and_foxhounds() {
    let env = TestEnv::new();
    let foxhound = blocking::Foxhound::connect(options(&env)).unwrap();
    let collie = goodfetch_collie::blocking::Collie::connect(options(&env)).unwrap();

    let reports = [
        start_and_finish_blocking(&foxhound).unwrap(),
        start_and_finish_blocking(&collie).unwrap(),
    ];
    let connection: &goodfetch_collie_client::blocking::ClientConnection = foxhound.as_ref();
    let read: RunReport = connection
        .call(
            RUN_REPORT.name,
            &RunParams {
                run_id: reports[0].run_id.clone(),
            },
        )
        .send()
        .unwrap();
    blocking::Connection::close(&foxhound).unwrap();
    blocking::Connection::close(&collie).unwrap();

    assert_eq!(
        (&read.run_id, read.lifecycle),
        (&reports[0].run_id, Lifecycle::Finished)
    );
    assert_eq!(
        reports.map(|report| report.lifecycle),
        [Lifecycle::Finished; 2]
    );
}

/// The linked engine is the one the crate composes: neither engine-path
/// variable, both known to the engine, moves it, even when each names no
/// file.
/// Covers: story:11-1007
#[test]
fn neither_engine_path_variable_moves_the_linked_engine() {
    let env = TestEnv::new();
    let mut environment = env.env_os();
    let missing = env.home().join("no-engine-here");
    for variable in [COLLIE_ENGINE_PATH_VARIABLE, FOXHOUND_ENGINE_PATH_VARIABLE] {
        environment.insert(variable.into(), missing.clone().into());
    }
    let options = ConnectOptions::default().environment(environment);

    real_time_runtime().block_on(async {
        let foxhound = Foxhound::connect(options).await.unwrap();
        let report = start_and_finish(&foxhound).await.unwrap();
        foxhound.close().await.unwrap();

        assert_eq!(report.lifecycle, Lifecycle::Finished);
    });
}
