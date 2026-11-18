//! Foxhound's Rust SDK (#2806): a program connects to the linked Foxhound
//! engine on the production runtime, starts a Run, finishes it and closes,
//! in async, in a scope and through `blocking`, each test with its own
//! Goodfetch home given as the connection's environment map; the values
//! are the Collie client crate's own, which a helper written for Collie
//! takes; and a Foxhound connection that meets a Collie engine fails with
//! `not_in_product`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_collie::linked::{Composed, LinkedConnection};
use goodfetch_foxhound::{ConnectOptions, Connection, Error, Foxhound, RunId, RunReport, blocking};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::NotInProductDetails;
use goodfetch_protocol::run::{Lifecycle, RUN_REPORT, RunParams};
use goodfetch_protocol::settings::{COLLIE_ENGINE_PATH_VARIABLE, FOXHOUND_ENGINE_PATH_VARIABLE};
use goodfetch_test_support::TestEnv;

mod public_items;

/// What a connection opens with: the test's environment, which names its
/// Goodfetch home.
fn options(env: &TestEnv) -> ConnectOptions {
    ConnectOptions::default().environment(environment(env))
}

/// The test's environment as a connection's map.
fn environment(env: &TestEnv) -> BTreeMap<OsString, OsString> {
    env.env()
        .iter()
        .map(|(name, value)| (name.into(), value.into()))
        .collect()
}

/// A single-threaded runtime, as a program's own may be.
fn current_thread() -> tokio::runtime::Runtime {
    goodfetch_runtime::current_thread()
        .enable_all()
        .build()
        .unwrap()
}

/// A helper written for Collie: it takes the client crate's Connection,
/// starts a Run on it and finishes it.
async fn start_and_finish(connection: &Connection) -> Result<RunReport, Error> {
    connection.start().await?.finish().await
}

/// Covers: story:11-1002
#[test]
fn a_program_connects_starts_a_run_finishes_it_and_closes_on_a_single_threaded_runtime() {
    let env = TestEnv::new();
    let first = RunId::parse("first").unwrap();
    current_thread().block_on(async {
        let foxhound = Foxhound::connect(options(&env)).await.unwrap();

        let run = foxhound.start().run_id(&first).await.unwrap();
        let report = run.finish().await.unwrap();
        foxhound.close().await.unwrap();

        assert_eq!(report.run_id, first);
        assert_eq!(report.lifecycle, Lifecycle::Finished);
        let after = foxhound.start().await;
        assert!(matches!(after, Err(Error::Closed)), "{after:?}");
    });
}

/// Covers: story:11-1002
#[test]
fn a_scope_lends_its_connection_to_the_body_and_closes_it_after() {
    let env = TestEnv::new();
    let (report, kept) = current_thread().block_on(async {
        Foxhound::scope(options(&env), async |foxhound| {
            let report = foxhound.start().await?.finish().await?;
            Ok((report, foxhound.clone()))
        })
        .await
        .unwrap()
    });

    assert_eq!(report.lifecycle, Lifecycle::Finished);
    let after = current_thread().block_on(async { kept.start().await });
    assert!(matches!(after, Err(Error::Closed)), "{after:?}");
}

/// Two connections at once in one process, each with its own linked
/// engine, and a third after both closed.
/// Covers: story:11-1002
#[test]
fn a_process_holds_many_connections_and_connects_again_after_a_close() {
    let left = TestEnv::new();
    let right = TestEnv::new();
    current_thread().block_on(async {
        let first = Foxhound::connect(options(&left)).await.unwrap();
        let second = Foxhound::connect(options(&right)).await.unwrap();

        let reports = (
            start_and_finish(first.as_ref()).await.unwrap(),
            start_and_finish(second.as_ref()).await.unwrap(),
        );
        first.close().await.unwrap();
        second.close().await.unwrap();
        let again = Foxhound::connect(options(&left)).await.unwrap();
        let third = start_and_finish(again.as_ref()).await.unwrap();
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
    assert!(matches!(after, Err(Error::Closed)), "{after:?}");
}

/// `blocking` inside an async runtime never enters a runtime, which would
/// panic here.
/// Covers: story:11-1002
#[test]
fn blocking_inside_an_async_runtime_never_nests_a_runtime() {
    let env = TestEnv::new();
    current_thread().block_on(async {
        let foxhound = blocking::Foxhound::connect(options(&env)).unwrap();

        let report = foxhound.start().send().unwrap().finish().send().unwrap();
        foxhound.close().unwrap();

        assert_eq!(report.lifecycle, Lifecycle::Finished);
    });
}

/// One helper typed for the client crate's Connection takes both
/// Products' connections, and a `match` on the client crate's `Error`
/// catches what either fails with.
/// Covers: story:11-1003, story:11-1004
#[test]
fn a_helper_typed_for_connection_takes_collies_and_foxhounds() {
    let env = TestEnv::new();
    current_thread().block_on(async {
        let foxhound = Foxhound::connect(options(&env)).await.unwrap();
        let collie = goodfetch_collie::Collie::connect(options(&env))
            .await
            .unwrap();

        let reports = [
            start_and_finish(foxhound.as_ref()).await.unwrap(),
            start_and_finish(collie.as_ref()).await.unwrap(),
        ];
        foxhound.close().await.unwrap();
        collie.close().await.unwrap();
        let closed = [
            start_and_finish(foxhound.as_ref()).await,
            start_and_finish(collie.as_ref()).await,
        ];

        assert_eq!(
            reports.map(|report| report.lifecycle),
            [Lifecycle::Finished; 2]
        );
        assert!(
            closed
                .iter()
                .all(|after| matches!(after, Err(Error::Closed))),
            "{closed:?}"
        );
    });
}

/// The blocking connection hands out the client crate's blocking
/// Connection, which carries Collie's members under the same names.
/// Covers: story:11-1003, story:11-1004
#[test]
fn the_blocking_connection_is_the_client_crates() {
    let env = TestEnv::new();
    let foxhound = blocking::Foxhound::connect(options(&env)).unwrap();
    let run = foxhound.start().send().unwrap();
    run.finish().send().unwrap();

    let connection: &goodfetch_collie_client::blocking::Connection = foxhound.as_ref();
    let report: RunReport = connection
        .call(
            RUN_REPORT.name,
            &RunParams {
                run_id: run.id().clone(),
            },
        )
        .send()
        .unwrap();
    foxhound.close().unwrap();

    assert_eq!(
        (&report.run_id, report.lifecycle),
        (run.id(), Lifecycle::Finished)
    );
}

/// A Foxhound connection that meets a Collie engine fails at connect, on
/// `hello`, with the details and a fix that names the install command.
/// Covers: story:11-1005, error:not_in_product
#[test]
fn a_foxhound_connection_that_meets_a_collie_engine_fails_with_not_in_product() {
    let env = TestEnv::new();
    let failed = current_thread().block_on(LinkedConnection::connect(
        Composed::COLLIE,
        Product::Foxhound,
        options(&env),
    ));

    let Err(Error::Failed(error)) = failed else {
        panic!("connected: {failed:?}");
    };
    assert_eq!(error.data.code, "not_in_product");
    let details: NotInProductDetails =
        serde_json::from_value(serde_json::Value::Object(error.data.details.clone())).unwrap();
    assert_eq!(
        (details.product, details.needs, details.requested.as_deref()),
        (Product::Collie, Product::Foxhound, None)
    );
    let install = details.install.clone().unwrap();
    assert!(details.fix.contains(&install), "{details:?}");
}

/// The linked engine is the one the crate composes: neither engine-path
/// variable, both known to the engine, moves it, even when each names no
/// file.
/// Covers: story:11-1007
#[test]
fn neither_engine_path_variable_moves_the_linked_engine() {
    let env = TestEnv::new();
    let mut environment = environment(&env);
    let missing = env.home().join("no-engine-here");
    for variable in [COLLIE_ENGINE_PATH_VARIABLE, FOXHOUND_ENGINE_PATH_VARIABLE] {
        environment.insert(variable.into(), missing.clone().into());
    }
    let options = ConnectOptions::default().environment(environment);

    current_thread().block_on(async {
        let foxhound = Foxhound::connect(options).await.unwrap();
        let report = start_and_finish(foxhound.as_ref()).await.unwrap();
        foxhound.close().await.unwrap();

        assert_eq!(report.lifecycle, Lifecycle::Finished);
    });
}
