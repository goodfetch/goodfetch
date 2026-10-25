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

use super::{PROVENANCE, RequestLog, provenance};
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
        .block_on(provenance(
            &owner,
            env.folder(),
            &joined(redaction),
            &json!({ "action": "filled", "value": "pa ss\"w/rd>?9~" }),
        ))
        .unwrap();

    assert_eq!(
        std::fs::read_to_string(env.folder().join(PROVENANCE)).unwrap(),
        written
    );
}
