//! An attempt's watch ends the attempt on a panic in it or in a task it
//! started while it runs, and nothing on a panic in the reading of a late
//! answer.

use goodfetch_engine::CallError;
use goodfetch_engine::metrics::Metrics;
use goodfetch_test_support::runtime;
use serde_json::{Value, json};

use super::{Watch, after};

/// What a watch's error says: its code and details.
fn said(error: &CallError) -> (String, Option<Value>) {
    (
        error.code().to_owned(),
        error
            .details()
            .map(|details| Value::Object(details.clone())),
    )
}

/// What the watch says of a panic that ended an attempt.
fn panicked() -> (String, Option<Value>) {
    (
        "engine_panicked".to_owned(),
        Some(json!({ "ended": "attempt" })),
    )
}

/// An attempt that panics as it is polled.
fn panics() -> impl Future<Output = u8> {
    std::future::poll_fn(|_| -> std::task::Poll<u8> { panic!("the attempt panics") })
}

/// An attempt that panics ends with `engine_panicked`, `ended: attempt`;
/// one that does not ends with its own output.
#[test]
fn an_attempt_that_panics_ends_with_engine_panicked() {
    let (ended, answered) = runtime().block_on(async {
        let watch = Watch::new(&Metrics::none());
        (watch.ended(panics()).await, watch.ended(async { 7 }).await)
    });

    assert_eq!(
        (ended.map_err(|error| said(&error)), answered.ok()),
        (Err(panicked()), Some(7))
    );
}

/// A task the attempt started that panics while the attempt runs ends it,
/// though the attempt itself would wait on; one that ends without
/// panicking ends nothing.
#[test]
fn a_task_the_attempt_started_that_panics_ends_it() {
    let (ended, answered) = runtime().block_on(async {
        let (watch, other) = (Watch::new(&Metrics::none()), Watch::new(&Metrics::none()));
        let (ended, ()) = tokio::join!(
            watch.ended(std::future::pending::<()>()),
            watch.over(async { panic!("the attempt's task panics") }),
        );
        let (answered, ()) = tokio::join!(
            other.ended(async {
                tokio::task::yield_now().await;
                7
            }),
            other.over(async {}),
        );
        (ended, answered)
    });

    assert_eq!(
        (ended.map_err(|error| said(&error)), answered.ok()),
        (Err(panicked()), Some(7))
    );
}

/// The reading of a late answer that panics ends nothing: the panic stays
/// in its task, and the answer is discarded.
#[test]
fn a_late_answers_reading_that_panics_ends_nothing() {
    runtime().block_on(after(&Metrics::none(), async {
        panic!("the late answer's reading panics")
    }));
}
