//! The Rust SDK (#1223): a program connects to the linked engine on the
//! production runtime, starts a Run, finishes it and closes, in async, in
//! a scope and through `blocking`, each test with its own Goodfetch home
//! given as the connection's environment map; and the members generated
//! from the catalog (ADR 0099), the `catalog-mapping` client conformance
//! case in `linked` mode.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::time::Duration;

use goodfetch_collie::{Collie, ConnectOptions, ErrorCode, RunId, StartedAs, blocking};
use goodfetch_protocol::run::{Lifecycle, ProgressMessage};
use goodfetch_protocol::settings::SettingInvalidDetails;
use goodfetch_test_support::{TestEnv, real_time_runtime};
use serde_json::json;

mod closing;
mod engine_choice;
mod events;
mod limits;
mod logging;
mod page;
mod public_items;
mod start;

/// What a connection opens with: the test's environment, which names its
/// Goodfetch home.
fn options(env: &TestEnv) -> ConnectOptions {
    ConnectOptions::default().environment(env.env_os())
}

/// Covers: story:11-3, story:11-11, story:11-59
#[test]
fn a_program_connects_starts_a_run_finishes_it_and_closes_on_a_single_threaded_runtime() {
    let env = TestEnv::new();
    let first = RunId::parse("first").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();

        let run = collie.start().run_id(&first).await.unwrap();
        let report = run.finish().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(run.id(), &first);
        assert_eq!(run.started_as(), StartedAs::New);
        assert_eq!(report.run_id, first);
        assert_eq!(report.lifecycle, Lifecycle::Finished);
        let after = collie.start().await;
        assert!(
            matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
            "{after:?}"
        );
    });
}

/// Covers: story:11-3, story:11-59
#[test]
fn a_scope_lends_its_connection_to_the_body_and_closes_it_after() {
    let env = TestEnv::new();
    let (report, kept) = real_time_runtime().block_on(async {
        Collie::scope(options(&env), async |collie| {
            let report = collie.start().await?.finish().await?;
            Ok((report, collie.clone()))
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
    assert!(
        matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
        "{after:?}"
    );
}

/// `blocking` inside an async runtime hands each call to the connection's
/// own runtime and waits on a channel: it never enters a runtime, which
/// would panic here, and blocks only the thread that calls it.
/// Covers: story:11-59
#[test]
fn blocking_inside_an_async_runtime_never_nests_a_runtime() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let collie = blocking::Collie::connect(options(&env)).unwrap();

        let report = collie.start().send().unwrap().finish().send().unwrap();
        collie.close().unwrap();

        assert_eq!(report.lifecycle, Lifecycle::Finished);
    });
}

/// The `catalog-mapping` client conformance case (ADR 0099), linked: each
/// member a Run takes from the catalog reaches its operation, and once the
/// connection is closed every member fails with `closed` before it is
/// sent.
/// Covers: story:11-178
#[test]
fn the_members_generated_from_the_catalog_reach_the_engine() {
    let env = TestEnv::new();
    let message = ProgressMessage::parse("three pages left").unwrap();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();

        let progress_set = run.set_progress_message(message.clone()).await.unwrap();
        let progress_read = run.progress().await.unwrap();
        let finished = run.finish().await.unwrap();
        let report = run.report().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(
            (&progress_set.run_id, progress_set.lifecycle),
            (run.id(), Lifecycle::Running)
        );
        assert_eq!(progress_read.message, Some(message));
        assert_eq!(
            (&finished.run_id, finished.lifecycle),
            (run.id(), Lifecycle::Finished)
        );
        assert_eq!(
            (&report.run_id, report.lifecycle),
            (run.id(), Lifecycle::Finished)
        );
        let after = run.report().await;
        assert!(
            matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
            "{after:?}"
        );
    });
}

/// The `call-timeout-three-states` client conformance case, linked, with
/// the two tests below, which refuse `Duration::ZERO` and send no timeout:
/// the linked engine takes a call's `timeout` left out or given as a
/// duration (ADR 0080). What each state sends is
/// `goodfetch-collie-client`'s
/// `a_call_sends_its_timeout_left_out_null_or_in_milliseconds_rounded_up`.
/// A `Duration` is never negative nor infinite, so zero is the one value to
/// refuse.
/// Covers: story:11-40, setting:timeout.run, operation:run.finish
#[test]
fn a_run_finishes_with_its_timeout_left_out_or_given_as_a_duration() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let omitted = collie.start().await.unwrap().finish().await.unwrap();
        let given = collie
            .start()
            .await
            .unwrap()
            .finish()
            .timeout(Duration::from_secs(30))
            .await
            .unwrap();
        collie.close().await.unwrap();

        assert_eq!(
            (omitted.lifecycle, given.lifecycle),
            (Lifecycle::Finished, Lifecycle::Finished)
        );
    });
}

/// The case's refusal: a call's `Duration::ZERO` is refused with
/// `setting_invalid` naming the setting it would set, given as an argument,
/// and the null form (ADR 0080).
/// Covers: story:11-42, error:setting_invalid, setting:timeout.close, operation:run.stop
#[test]
fn a_zero_duration_is_refused_naming_the_setting_and_the_null_form() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();

        let refused = run.stop().timeout(Duration::ZERO).await;
        collie.close().await.unwrap();

        let Err(failure) = &refused else {
            panic!("{refused:?}");
        };
        let details: SettingInvalidDetails = failure.details().unwrap();
        assert_eq!(
            (
                failure.code(),
                serde_json::to_value(&details.limit).unwrap(),
                failure
                    .fix()
                    .is_some_and(|fix| fix.contains("`timeout: null`")),
            ),
            (
                ErrorCode::SETTING_INVALID,
                json!({ "key": "timeout.close", "value": 0, "layer": "run", "origin": "argument" }),
                true,
            )
        );
    });
}

/// The case's null form: `.no_timeout()` sends `null`, so the call waits
/// with no deadline of its own, in place of the Run's `timeout.close` (ADR
/// 0080).
/// Covers: story:11-40, setting:timeout.close, operation:run.stop
#[test]
fn no_timeout_waits_for_the_stop_with_no_deadline() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        let collie = Collie::connect(options(&env)).await.unwrap();
        let run = collie.start().await.unwrap();

        let stopped = run.stop().no_timeout().await.unwrap();
        collie.close().await.unwrap();

        assert_eq!(stopped.lifecycle, Lifecycle::Stopped);
    });
}

/// The `connect-timeout-three-states` client conformance case, linked:
/// `connect()` opens with its `timeout` left out, given as a duration or
/// none, and refuses `Duration::ZERO` with `setting_invalid` before the
/// engine starts (ADRs 0080 and 0083). A linked engine always answers
/// `hello`, so the deadline passing is `goodfetch-collie-client`'s
/// `an_unanswered_hello_fails_the_opening_past_its_timeout_in_its_three_states`.
/// Covers: story:11-41, story:11-42, setting:timeout.connect
#[test]
fn connect_takes_its_timeout_left_out_given_or_none_and_refuses_zero() {
    let env = TestEnv::new();
    real_time_runtime().block_on(async {
        for options in [
            options(&env),
            options(&env).timeout(Duration::from_secs(30)),
            options(&env).no_timeout(),
        ] {
            Collie::connect(options).await.unwrap().close().await.unwrap();
        }

        let refused = Collie::connect(options(&env).timeout(Duration::ZERO)).await;
        let Err(failure) = refused else {
            panic!("{refused:?}");
        };
        assert_eq!(
            (
                failure.code(),
                &serde_json::to_value(failure.limit()).unwrap(),
                failure
                    .fix()
                    .is_some_and(|fix| fix.contains("`.no_timeout()`")),
            ),
            (
                ErrorCode::SETTING_INVALID,
                &json!({ "key": "timeout.connect", "value": 0, "layer": "run", "origin": "argument" }),
                true,
            )
        );
    });
}

/// The same members, through `blocking`, under the same names.
/// Covers: story:11-178
#[test]
fn blocking_repeats_the_members_generated_from_the_catalog() {
    let env = TestEnv::new();
    let message = ProgressMessage::parse("three pages left").unwrap();
    let collie = blocking::Collie::connect(options(&env)).unwrap();
    let run: blocking::Run = collie.start().send().unwrap();

    let progress_set = run.set_progress_message(message.clone()).send().unwrap();
    let finished = run.finish().send().unwrap();
    collie.clone().close().unwrap();

    assert_eq!(progress_set.message, Some(message));
    assert_eq!(finished.lifecycle, Lifecycle::Finished);
    let after = run.report().send();
    assert!(
        matches!(&after, Err(error) if error.code() == ErrorCode::CLOSED),
        "{after:?}"
    );
}
