//! The Rust SDK (#1223): a program connects to the linked engine on the
//! production runtime, starts a Run, finishes it and closes, in async, in
//! a scope and through `blocking`, each test with its own Goodfetch home
//! given as the connection's environment map.

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_collie::{Collie, ConnectOptions, Error, RunId, StartedAs, blocking};
use goodfetch_protocol::run::Lifecycle;
use goodfetch_test_support::TestEnv;

mod public_items;

/// What a connection opens with: the test's environment, which names its
/// Goodfetch home.
fn options(env: &TestEnv) -> ConnectOptions {
    let environment: BTreeMap<OsString, OsString> = env
        .env()
        .iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect();
    ConnectOptions::default().environment(environment)
}

/// A single-threaded runtime, as a program's own may be.
fn current_thread() -> tokio::runtime::Runtime {
    goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// Covers: story:11-3, story:11-11, story:11-59
#[test]
fn a_program_connects_starts_a_run_finishes_it_and_closes_on_a_single_threaded_runtime() {
    let env = TestEnv::new();
    let first = RunId::parse("first").unwrap();
    current_thread().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();

        let run = collie.start().run_id(&first).await.unwrap();
        let report = run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(run.id(), &first);
        assert_eq!(run.started_as(), StartedAs::New);
        assert_eq!(report.run_id, first);
        assert_eq!(report.lifecycle, Lifecycle::Finished);
        let after = collie.start().await;
        assert!(matches!(after, Err(Error::Closed)), "{after:?}");
    });
}

/// Covers: story:11-3, story:11-59
#[test]
fn a_scope_lends_its_connection_to_the_body_and_closes_it_after() {
    let env = TestEnv::new();
    let (report, kept) = current_thread().block_on(async {
        Collie::scope(options(&env), async |collie| {
            let report = collie.start().await?.finish().await?;
            Ok((report, collie.clone()))
        })
        .await
        .unwrap()
    });

    assert_eq!(report.lifecycle, Lifecycle::Finished);
    let after = current_thread().block_on(async { kept.start().await });
    assert!(matches!(after, Err(Error::Closed)), "{after:?}");
}

/// Covers: story:11-3
#[test]
fn a_program_without_async_connects_starts_a_run_finishes_it_and_closes() {
    let env = TestEnv::new();
    let first = RunId::parse("first").unwrap();
    let collie = blocking::Collie::connect(options(&env)).unwrap();

    let run = collie.start().run_id(&first).send().unwrap();
    let report = run.finish().send().unwrap();
    collie.clone().close().unwrap();

    assert_eq!(run.id(), &first);
    assert_eq!(report.lifecycle, Lifecycle::Finished);
    let after = collie.start().send();
    assert!(matches!(after, Err(Error::Closed)), "{after:?}");
}

/// `blocking` inside an async runtime hands each call to the connection's
/// own runtime and waits on a channel: it never enters a runtime, which
/// would panic here, and blocks only the thread that calls it.
/// Covers: story:11-59
#[test]
fn blocking_inside_an_async_runtime_never_nests_a_runtime() {
    let env = TestEnv::new();
    current_thread().block_on(async {
        let collie = blocking::Collie::connect(options(&env)).unwrap();

        let report = collie.start().send().unwrap().finish().send().unwrap();
        collie.close().unwrap();

        assert_eq!(report.lifecycle, Lifecycle::Finished);
    });
}
