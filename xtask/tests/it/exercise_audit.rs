//! `xtask features audit`, the exercise audit (ADR 0072): a test that
//! declares an `operation:`, `error:`, `event:`, `command:` or `exit:`
//! Feature must have reached it, as its exercise record shows. Each case
//! takes one record away from a fixture whose every declaring test reached
//! what it declares.

use rstest::rstest;

use crate::workspace::{Workspace, finding};

/// The fixture's exercise records, as `just test` leaves them.
const RECORDS: &str = "records";

/// Audits the fixture's records.
fn audit(workspace: &Workspace, more: &[&str]) -> crate::workspace::Outcome {
    let mut args = vec!["features", "audit", "--records", RECORDS];
    args.extend(more);
    workspace.xtask(&args)
}

#[test]
fn audit_passes_when_every_declaring_test_reached_what_it_declares() {
    let workspace = Workspace::from_fixture("exercise-audit");

    let outcome = audit(&workspace, &[]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), vec![]);
}

#[rstest]
#[case::operation_never_called(
    "records/sample-it-calls_echo_then_crawl.jsonl",
    r#""method":"sample.crawl""#,
    r#""method":"sample.echo""#,
    "operation:sample.crawl in sample::it calls_echo_then_crawl"
)]
#[case::error_code_never_received(
    "records/sample-it-gets_the_refusal.jsonl",
    r#""error_code":"sample_refused""#,
    r#""error_code":"sample_conflict""#,
    "error:sample_refused in sample::it gets_the_refusal"
)]
#[case::event_never_received(
    "records/sample-it-receives_the_echoed_event.jsonl",
    r#""event":"sample.echoed""#,
    r#""event":"sample.crawled""#,
    "event:sample.echoed in sample::it receives_the_echoed_event"
)]
#[case::command_never_run(
    "records/sample-it-installs_with_its_exit_code.jsonl",
    r#""command":"browsers install""#,
    r#""command":"browsers clean""#,
    "command:browsers install in sample::it installs_with_its_exit_code"
)]
#[case::exit_code_never_seen(
    "records/sample-it-installs_with_its_exit_code.jsonl",
    r#""exit_code":12"#,
    r#""exit_code":0"#,
    "exit:not_installed in sample::it installs_with_its_exit_code"
)]
fn audit_fails_a_declaring_test_that_never_reached_its_feature(
    #[case] record: &str,
    #[case] reached: &str,
    #[case] instead: &str,
    #[case] unreached: &str,
) {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.edit(record, reached, instead);

    let outcome = audit(&workspace, &[]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), vec![finding("unreached", unreached)]);
}

#[rstest]
#[case::integration_test(
    "records/sample-it-gets_the_refusal.jsonl",
    "sample::it gets_the_refusal"
)]
#[case::unit_test_declaring_an_operation(
    "records/sample-tests-echoes_in_process.jsonl",
    "sample tests::echoes_in_process"
)]
fn audit_reports_a_declaring_test_whose_record_is_missing(
    #[case] record: &str,
    #[case] test: &str,
) {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.remove(record);

    let outcome = audit(&workspace, &[]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), vec![finding("record-missing", test)]);
}

#[test]
fn audit_reports_a_record_that_is_no_json_lines() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.append(
        "records/sample-it-gets_the_refusal.jsonl",
        "request answered\n",
    );

    let outcome = audit(&workspace, &[]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![finding(
            "record-unreadable",
            "records/sample-it-gets_the_refusal.jsonl"
        )]
    );
}

#[test]
fn audit_over_the_tests_that_ran_leaves_out_a_test_that_did_not_run() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.remove("records/sample-it-installs_with_its_exit_code.jsonl");

    let outcome = audit(&workspace, &["--junit", "junit/nextest.xml"]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), vec![]);
}

#[test]
fn audit_over_the_tests_that_ran_still_reports_one_that_ran_without_a_record() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.remove("records/sample-it-receives_the_echoed_event.jsonl");

    let outcome = audit(&workspace, &["--junit", "junit/nextest.xml"]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![finding(
            "record-missing",
            "sample::it receives_the_echoed_event"
        )]
    );
}
