//! The Run trace's writers through their interface: each line passes
//! through the Run's `Mask` as a kept output (spec #564), which a Run whose
//! `security.redaction` is off lifts.
//! Covers: story:4-56, story:4-76
#![expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run trace's files back"
)]

use goodfetch_protocol::page::events::PAGE_REQUEST_FAILED;
use goodfetch_test_support::TestEnv;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use goodfetch_counted_work::{CountedWork, Row};

use super::{PROVENANCE, Provenance, RequestLog};
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
    let log = RequestLog::new(
        TaskOwner::root(BlockingPolicy::Inline),
        env.folder(),
        joined(redaction),
        goodfetch_counted_work::CountedWork::default(),
    );

    goodfetch_test_support::runtime()
        .block_on(log.record(
            &PAGE_REQUEST_FAILED,
            &json!({ "url": "https://example.test/?p=pa%20ss%22w%2Frd%3E%3F9~" }),
        ))
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(env.folder().join(super::FILE)).unwrap(),
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
    let owner = TaskOwner::root(BlockingPolicy::Inline);

    goodfetch_test_support::runtime()
        .block_on(Provenance::new(env.folder()).record(
            &owner,
            &joined(redaction),
            &CountedWork::default(),
            json!({ "action": "filled", "value": "pa ss\"w/rd>?9~" }),
        ))
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(env.folder().join(PROVENANCE)).unwrap(),
        written
    );
}

/// The Run holds its Provenance log's file (ADR 0140's E5): its folder is
/// created and its file opened once, at the first entry, on `E5.file_opens`
/// and never on a page's `F1.file_opens`, and each entry is on file as soon
/// as it is recorded, so a read straight after the Call that wrote it sees
/// it and a process killed then has written it: nothing waits in memory.
#[test]
fn the_provenance_log_opens_its_file_once_and_each_entry_is_on_file_once_recorded() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Threads);
    let mask = joined(Redaction::On);
    let work = CountedWork::default();
    let log = Provenance::new(env.folder());
    let runtime = goodfetch_test_support::runtime();

    let read = (1..=3)
        .map(|step| {
            runtime
                .block_on(log.record(
                    &owner,
                    &mask,
                    &work,
                    json!({ "action": "step", "step": step }),
                ))
                .unwrap();
            std::fs::read_to_string(env.folder().join(PROVENANCE)).unwrap()
        })
        .collect::<Vec<_>>();

    assert_eq!(
        (
            read,
            work.read(Row::RunTraceFileOpens),
            work.read(Row::PageFileOpens),
            work.read(Row::RunTraceSpawns),
        ),
        (
            vec![
                "{\"action\":\"step\",\"step\":1}\n".to_owned(),
                "{\"action\":\"step\",\"step\":1}\n{\"action\":\"step\",\"step\":2}\n".to_owned(),
                "{\"action\":\"step\",\"step\":1}\n{\"action\":\"step\",\"step\":2}\n{\"action\":\"step\",\"step\":3}\n"
                    .to_owned(),
            ],
            2,
            0,
            3,
        )
    );
}
