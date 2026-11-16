//! The Run trace's writers through their interface: each line passes
//! through the Run's `Mask` as a kept output (spec #564), which a Run whose
//! `security.redaction` is off lifts.
//! Covers: story:4-56, story:4-76
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run trace's files back"
)]

use std::sync::Arc;

use goodfetch_protocol::home::{RUN_TRACE_CDP_COMMANDS, RUN_TRACE_PROVENANCE, RUN_TRACE_REQUESTS};
use goodfetch_protocol::page::events::PAGE_REQUEST_FAILED;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use goodfetch_counted_work::{CountedWork, Row};

use super::{RequestLog, RunTrace};
use crate::mask::{Mask, Masks, Reach, Redaction};
use crate::tasks::{BlockingPolicy, TaskOwner};

fn joined(redaction: Redaction) -> Mask {
    let mask = Masks::new().for_run(redaction);
    mask.join(
        "proxy.password",
        &SecretString::from("pa ss\"w/rd>?9~"),
        Reach::Value,
    );
    mask
}

/// The Run trace of a Run whose folder is `env`'s, under `owner`, masked
/// with `redaction`, counting on `work`, its Provenance log on.
fn trace(
    owner: &Arc<TaskOwner>,
    env: &TestEnv,
    redaction: Redaction,
    work: &CountedWork,
) -> RunTrace {
    RunTrace::new(
        Arc::clone(owner),
        env.folder(),
        joined(redaction),
        work.clone(),
        true,
    )
}

#[rstest]
#[case::redaction_on(
    Redaction::On,
    "{\"data\":{\"url\":\"https://example.test/?p=%proxy.password%\"},\"event\":\"page.request_failed\"}\n"
)]
#[case::redaction_off(
    Redaction::Off,
    "{\"data\":{\"url\":\"https://example.test/?p=pa%20ss%22w%2Frd%3E%3F9~\"},\"event\":\"page.request_failed\"}\n"
)]
fn the_request_log_writes_a_resolved_value_only_when_its_runs_redaction_is_off(
    #[case] redaction: Redaction,
    #[case] written: &str,
) {
    let env = TestEnv::new();
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let trace = trace(&owner, &env, redaction, &CountedWork::default());

    goodfetch_test_support::runtime()
        .block_on(RequestLog::new(&trace).record(
            &PAGE_REQUEST_FAILED,
            &json!({ "url": "https://example.test/?p=pa%20ss%22w%2Frd%3E%3F9~" }),
        ))
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(env.folder().join(RUN_TRACE_REQUESTS)).unwrap(),
        written
    );
}

#[rstest]
#[case::redaction_on(
    Redaction::On,
    "{\"action\":\"filled\",\"value\":\"%proxy.password%\"}\n"
)]
#[case::redaction_off(
    Redaction::Off,
    "{\"action\":\"filled\",\"value\":\"pa ss\\\"w/rd>?9~\"}\n"
)]
fn the_provenance_log_writes_a_resolved_value_only_when_its_runs_redaction_is_off(
    #[case] redaction: Redaction,
    #[case] written: &str,
) {
    let env = TestEnv::new();
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let trace = trace(&owner, &env, redaction, &CountedWork::default());

    goodfetch_test_support::runtime()
        .block_on(trace.provenance(json!({ "action": "filled", "value": "pa ss\"w/rd>?9~" })))
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(env.folder().join(RUN_TRACE_PROVENANCE)).unwrap(),
        written
    );
}

/// The Run's one writer holds its Provenance log's file (ADR 0140's E5):
/// its folder is created and its file opened once, at the first entry, the
/// Run's work on `E5.run_file_opens`, never an entry's `E5.file_opens` or a
/// page's `F1.file_opens`, its one task is started then, on `E5.run_spawns`,
/// and each entry is on file once it is recorded, so a read straight after
/// the Call that wrote it sees it and a process killed then has written it.
#[test]
fn the_provenance_log_opens_its_file_once_and_each_entry_is_on_file_once_recorded() {
    let env = TestEnv::new();
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Threads));
    let work = CountedWork::default();
    let trace = trace(&owner, &env, Redaction::On, &work);
    let runtime = goodfetch_test_support::runtime();

    let read = (1..=3)
        .map(|step| {
            runtime
                .block_on(trace.provenance(json!({ "action": "step", "step": step })))
                .unwrap();
            std::fs::read_to_string(env.folder().join(RUN_TRACE_PROVENANCE)).unwrap()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        (
            read,
            [
                Row::RunTraceRunFileOpens,
                Row::RunTraceFileOpens,
                Row::PageFileOpens,
                Row::RunTraceRunSpawns,
                Row::RunTraceSpawns,
            ]
            .map(|row| work.read(row)),
        ),
        (
            vec![
                "{\"action\":\"step\",\"step\":1}\n".to_owned(),
                "{\"action\":\"step\",\"step\":1}\n{\"action\":\"step\",\"step\":2}\n".to_owned(),
                "{\"action\":\"step\",\"step\":1}\n{\"action\":\"step\",\"step\":2}\n{\"action\":\"step\",\"step\":3}\n"
                    .to_owned(),
            ],
            [2, 0, 0, 1, 0],
        )
    );
}

/// Every kind of entry goes through the Run's one writer (ADR 0140's E5): the
/// Run trace's folder is created once and each file opened once a Run, at its
/// first entry, its one task started then, all the Run's work, and no entry
/// opens, creates or spawns anything more; a CDP command's line, which no one
/// waits for, is on file once the Run ends.
#[test]
fn every_kind_of_entry_goes_through_one_writer_that_opens_each_file_once() {
    let env = TestEnv::new();
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Threads));
    let work = CountedWork::default();
    let trace = trace(&owner, &env, Redaction::On, &work);
    let requests = RequestLog::new(&trace);

    goodfetch_test_support::runtime().block_on(async {
        for step in 1..=3 {
            trace
                .provenance(json!({ "action": "step", "step": step }))
                .await
                .unwrap();
            requests
                .record(&PAGE_REQUEST_FAILED, &json!({ "step": step }))
                .await
                .unwrap();
            trace.cdp_command("Page.navigate").await;
        }
        owner.stop().await;
    });

    let lines = |file: &str| {
        std::fs::read_to_string(env.folder().join(file))
            .unwrap()
            .lines()
            .count()
    };
    assert_eq!(
        (
            [
                RUN_TRACE_PROVENANCE,
                RUN_TRACE_REQUESTS,
                RUN_TRACE_CDP_COMMANDS
            ]
            .map(lines),
            [
                Row::RunTraceRunFileOpens,
                Row::RunTraceRunSpawns,
                Row::RunTraceFileOpens,
                Row::RunTraceSpawns,
                Row::InterceptedFileOpens,
                Row::CdpFileOpens,
                Row::PageFileOpens,
                Row::InterceptedEntries,
            ]
            .map(|row| work.read(row)),
        ),
        ([3, 3, 3], [4, 1, 0, 0, 0, 0, 0, 3])
    );
}

/// A Run whose `provenance.log` is off writes no Provenance entry and starts
/// no writer for it: nothing is opened, spawned or written (ADR 0138).
#[test]
fn a_run_with_its_provenance_log_off_writes_no_provenance_entry() {
    let env = TestEnv::new();
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Threads));
    let work = CountedWork::default();
    let trace = RunTrace::new(
        Arc::clone(&owner),
        env.folder(),
        joined(Redaction::On),
        work.clone(),
        false,
    );

    goodfetch_test_support::runtime().block_on(async {
        trace.provenance(json!({ "action": "step" })).await.unwrap();
        owner.stop().await;
    });

    assert_eq!(
        (
            env.folder().join(RUN_TRACE_PROVENANCE).exists(),
            [
                Row::RunTraceRunFileOpens,
                Row::RunTraceFileOpens,
                Row::RunTraceRunSpawns,
                Row::RunTraceSpawns,
            ]
            .map(|row| work.read(row)),
        ),
        (false, [0, 0, 0, 0])
    );
}

/// A file the writer cannot write is closed: the entry that failed reports
/// it, and the next entry opens the file again and is on file. That open is
/// the entry's work, on `E5.file_opens`, beside the Run's first one.
#[cfg(target_os = "linux")]
#[test]
fn a_file_that_failed_a_write_is_opened_again_by_the_next_entry() {
    let env = TestEnv::new();
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Threads));
    let work = CountedWork::default();
    let trace = trace(&owner, &env, Redaction::On, &work);
    let runtime = goodfetch_test_support::runtime();
    let path = env.folder().join(RUN_TRACE_PROVENANCE);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink("/dev/full", &path).unwrap();

    let failed = runtime
        .block_on(trace.provenance(json!({ "action": "step", "step": 1 })))
        .is_err();
    std::fs::remove_file(&path).unwrap();
    runtime
        .block_on(trace.provenance(json!({ "action": "step", "step": 2 })))
        .unwrap();

    assert_eq!(
        (
            failed,
            std::fs::read_to_string(&path).unwrap(),
            work.read(Row::RunTraceRunFileOpens),
            work.read(Row::RunTraceFileOpens),
        ),
        (true, "{\"action\":\"step\",\"step\":2}\n".to_owned(), 2, 1)
    );
}
