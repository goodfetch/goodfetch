//! A start reaching the Run its Run ID names through the Rust SDK (ADRs
//! 0078 and 0102), the `start-reaches-its-run` client conformance case in
//! `linked` mode: a fresh Run ID begins the Run, a start of it while it
//! runs fails with `run_in_use`, a start after it was closed unfinished
//! resumes it, logged at info, and one after it finished returns it ended,
//! logged at warning, where a call that would give it work fails with
//! `run_ended`. Tags travel in the start, through `blocking` too.

use goodfetch_collie::{Collie, Error, RunId, StartedAs, Tags, blocking};
use goodfetch_test_support::{TestEnv, logged, real_time_runtime};

use super::options;

/// The Error code `result` fails with.
fn code<T: std::fmt::Debug>(result: Result<T, Error>) -> String {
    let Err(Error::Failed(failure)) = result else {
        panic!("{result:?}");
    };
    failure.data.code
}

/// Covers: story:11-126, story:11-127
#[test]
fn a_start_begins_refuses_a_live_run_resumes_it_and_returns_it_ended_once_finished() {
    let env = TestEnv::new();
    let held = RunId::parse("held").unwrap();
    let (outcome, log) = logged(|| {
        real_time_runtime().block_on(async {
            let collie = Collie::connect(options(&env)).await.unwrap();

            let begun = collie
                .start()
                .run_id(&held)
                .tags(Tags::parse(["shop"]).unwrap())
                .await
                .unwrap();
            let in_use = code(collie.start().run_id(&held).await);
            begun.close().await.unwrap();
            let resumed = collie.start().run_id(&held).await.unwrap();
            resumed.finish().await.unwrap();
            let ended = collie.start().run_id(&held).await.unwrap();
            let no_work = code(ended.new_page().await);
            collie.close().await.unwrap();
            (
                [begun.started_as(), resumed.started_as(), ended.started_as()],
                in_use,
                no_work,
            )
        })
    });
    let log = String::from_utf8(log).unwrap();
    let start_lines: Vec<&str> = log
        .lines()
        .filter(|line| line.contains("goodfetch::client::run"))
        .collect();

    assert_eq!(
        outcome,
        (
            [StartedAs::New, StartedAs::Resumed, StartedAs::Ended],
            "run_in_use".to_owned(),
            "run_ended".to_owned(),
        )
    );
    assert_eq!(start_lines.len(), 2, "{log}");
    assert!(
        start_lines[0].contains("INFO")
            && start_lines[0]
                .contains("The start resumed the Run `held` from its committed state."),
        "{log}"
    );
    assert!(
        start_lines[1].contains("WARN")
            && start_lines[1].contains("The start found the Run `held` ended"),
        "{log}"
    );
}

/// A program without async tags its Run the same way.
/// Covers: story:11-126
#[test]
fn a_program_without_async_tags_its_run() {
    let env = TestEnv::new();
    let collie = blocking::Collie::connect(options(&env)).unwrap();

    let run = collie
        .start()
        .tags(Tags::parse(["shop", "eu"]).unwrap())
        .send()
        .unwrap();
    let report = run.finish().send();
    collie.close().unwrap();

    assert!(report.is_ok(), "{report:?}");
}
