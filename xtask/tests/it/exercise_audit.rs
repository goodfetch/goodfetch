//! `xtask features audit`, the exercise audit (ADR 0072): a test that
//! declares an `operation:`, `error:`, `event:`, `command:` or `exit:`
//! Feature must have reached it, as its exercise record shows. Each case
//! takes one record away from a fixture whose every declaring test reached
//! what it declares.

use rstest::rstest;

use crate::workspace::{Outcome, Workspace, finding};

/// The fixture's exercise records, as `just test` leaves them.
const RECORDS: &str = "records";

/// Audits the fixture's records.
fn audit(workspace: &Workspace, more: &[&str]) -> Outcome {
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
    r#""command":"browsers.install""#,
    r#""command":"browsers.clean""#,
    "command:browsers.install in sample::it installs_with_its_exit_code"
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

#[test]
fn audit_reports_a_command_whose_arguments_match_no_command_feature() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.append(
        "records/sample-it-installs_with_its_exit_code.jsonl",
        "{\"fields\":{\"unmatched\":\"browsers frobnicate\",\"exit_code\":2},\"target\":\"goodfetch::test::command\"}\n",
    );

    let outcome = audit(&workspace, &[]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![finding(
            "command-unmatched",
            "`browsers frobnicate` in sample::it installs_with_its_exit_code"
        )]
    );
}

#[test]
fn audit_judges_the_error_code_and_event_of_a_test_once_it_reaches_rpc() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.append(
        "records/sample-it-refuses_a_text_the_stand_in_rejects.jsonl",
        "{\"fields\":{\"message\":\"request answered\",\"method\":\"hello\"},\"target\":\"goodfetch::rpc::dispatch\"}\n",
    );

    let outcome = audit(&workspace, &[]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![
            finding(
                "unreached",
                "error:sample_refused in sample::it refuses_a_text_the_stand_in_rejects"
            ),
            finding(
                "unreached",
                "event:sample.echoed in sample::it refuses_a_text_the_stand_in_rejects"
            ),
        ]
    );
}

#[test]
fn audit_judges_the_error_code_and_event_of_a_unit_test_that_reaches_rpc() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.edit(
        "crates/sample/src/lib.rs",
        "/// Covers: operation:sample.echo\n",
        "/// Covers: operation:sample.echo, error:sample_lost, event:sample.lost\n",
    );

    let outcome = audit(&workspace, &[]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![
            finding(
                "unreached",
                "error:sample_lost in sample tests::echoes_in_process"
            ),
            finding(
                "unreached",
                "event:sample.lost in sample tests::echoes_in_process"
            ),
        ]
    );
}

#[test]
fn audit_counts_an_error_code_the_run_recorded_as_reached() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.edit(
        "crates/sample/src/lib.rs",
        "/// Covers: operation:sample.echo\n",
        "/// Covers: operation:sample.echo, error:sample_lost\n",
    );
    workspace.append(
        "records/sample-tests-echoes_in_process.jsonl",
        "{\"fields\":{\"message\":\"the run recorded an error code\",\"error_code\":\"sample_lost\"},\"target\":\"goodfetch::run::outcome\"}\n",
    );

    let outcome = audit(&workspace, &[]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), vec![]);
}

#[test]
fn audit_over_the_tests_that_ran_fails_one_that_never_reached_its_feature() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.edit(
        "records/sample-it-calls_echo_then_crawl.jsonl",
        r#""method":"sample.crawl""#,
        r#""method":"sample.echo""#,
    );

    let outcome = audit(&workspace, &["--junit", "junit/nextest.xml"]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![finding(
            "unreached",
            "operation:sample.crawl in sample::it calls_echo_then_crawl"
        )]
    );
}

#[rstest]
#[case::integration_test(
    "records/sample-it-gets_the_refusal.jsonl",
    "sample::it gets_the_refusal"
)]
#[case::integration_test_that_never_reaches_rpc(
    "records/sample-it-refuses_a_text_the_stand_in_rejects.jsonl",
    "sample::it refuses_a_text_the_stand_in_rejects"
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

/// A declaring test whose name nextest never ran, while nextest ran tests
/// below it (an rstest form named apart from the scanner), is reported
/// rather than left out as a test that did not run.
#[test]
fn audit_over_the_tests_that_ran_reports_a_declaring_test_nextest_ran_under_other_names() {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.edit(
        "junit/nextest.xml",
        r#"<testcase name="gets_the_refusal" "#,
        r#"<testcase name="gets_the_refusal::case_1" "#,
    );

    let outcome = audit(&workspace, &["--junit", "junit/nextest.xml"]);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(
        outcome.findings(),
        vec![finding("name-unmatched", "sample::it gets_the_refusal")]
    );
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

/// The suites' audit step: the repository's `just features-audit`, with the
/// verdict its `exercise_audit` switch names, run in the fixture once its
/// records and JUnit file are where `just test pre-merge` leaves them. A
/// `cargo` first on the path runs this xtask for `cargo run --bin xtask`, and
/// answers `cargo metadata` with a target folder outside the workspace, as
/// CI's VM sets `CARGO_TARGET_DIR`: nextest still writes its JUnit file
/// under the workspace's own `target/nextest/`.
#[cfg(unix)]
fn suite_step(workspace: &Workspace, args: &[&str]) -> Outcome {
    suite_step_with(workspace, args, |_| {})
}

/// [`suite_step`], with `arrange` run on the workspace's root once the
/// records and the JUnit file are in place.
#[cfg(unix)]
fn suite_step_with(
    workspace: &Workspace,
    args: &[&str],
    arrange: impl FnOnce(&std::path::Path),
) -> Outcome {
    use std::os::unix::fs::PermissionsExt as _;

    let root = workspace.path();
    std::fs::create_dir_all(root.join("target/nextest/ci")).unwrap();
    std::fs::rename(root.join(RECORDS), root.join("target/test-records")).unwrap();
    std::fs::copy(
        root.join("junit/nextest.xml"),
        root.join("target/nextest/ci/junit.xml"),
    )
    .unwrap();
    arrange(root);
    let shims = root.join("shims");
    std::fs::create_dir_all(&shims).unwrap();
    let real_cargo = std::env::var_os("CARGO").expect("nextest runs the test under Cargo");
    let cargo = shims.join("cargo");
    std::fs::write(
        &cargo,
        format!(
            concat!(
                "#!/bin/sh\n",
                "case \"$1\" in\n",
                "  metadata) printf '{{\"target_directory\":\"%s/elsewhere/target\"}}\\n' \"$PWD\" ;;\n",
                "  run) while [ \"$1\" != -- ]; do shift; done; shift; CARGO='{cargo}' exec '{xtask}' \"$@\" ;;\n",
                "  *) exit 2 ;;\n",
                "esac\n",
            ),
            cargo = real_cargo.to_string_lossy(),
            xtask = env!("CARGO_BIN_EXE_xtask"),
        ),
    )
    .unwrap();
    std::fs::set_permissions(&cargo, std::fs::Permissions::from_mode(0o755)).unwrap();
    let path = std::env::join_paths(
        std::iter::once(shims).chain(std::env::split_paths(&std::env::var_os("PATH").unwrap())),
    )
    .unwrap();
    let justfile = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../justfile");
    let output = workspace
        .command("just")
        .arg("--justfile")
        .arg(justfile)
        .arg("--working-directory")
        .arg(root)
        .arg("features-audit")
        .args(args)
        .env("PATH", path)
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .unwrap();
    Outcome {
        code: output.status.code().unwrap(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

#[cfg(unix)]
#[rstest]
#[case::over_the_tests_that_ran(&[])]
#[case::over_every_declaring_test(&["all"])]
fn the_suites_audit_step_fails_a_declaring_test_that_never_reached_its_feature(
    #[case] args: &[&str],
) {
    let workspace = Workspace::from_fixture("exercise-audit");
    workspace.edit(
        "records/sample-it-calls_echo_then_crawl.jsonl",
        r#""method":"sample.crawl""#,
        r#""method":"sample.echo""#,
    );

    let outcome = suite_step(&workspace, args);

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome.findings().contains(&finding(
            "unreached",
            "operation:sample.crawl in sample::it calls_echo_then_crawl"
        )),
        "{}",
        outcome.stdout
    );
}

#[cfg(unix)]
#[test]
fn the_suites_audit_step_passes_when_every_test_that_ran_reached_what_it_declares() {
    let workspace = Workspace::from_fixture("exercise-audit");

    let outcome = suite_step(&workspace, &[]);

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert_eq!(outcome.findings(), vec![]);
}

#[cfg(unix)]
#[test]
fn the_suites_audit_step_fails_when_tests_left_records_but_no_junit_file() {
    let workspace = Workspace::from_fixture("exercise-audit");

    let outcome = suite_step_with(&workspace, &[], |root| {
        std::fs::remove_file(root.join("target/nextest/ci/junit.xml")).unwrap();
    });

    assert_eq!(outcome.code, 1, "{}{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome
            .stderr
            .contains("target/test-records holds exercise records"),
        "{}{}",
        outcome.stdout,
        outcome.stderr
    );
}

#[cfg(unix)]
#[test]
fn the_suites_audit_step_passes_when_no_test_ran() {
    let workspace = Workspace::from_fixture("exercise-audit");

    let outcome = suite_step_with(&workspace, &[], |root| {
        std::fs::remove_file(root.join("target/nextest/ci/junit.xml")).unwrap();
        std::fs::remove_dir_all(root.join("target/test-records")).unwrap();
    });

    assert_eq!(outcome.code, 0, "{}{}", outcome.stdout, outcome.stderr);
    assert!(
        outcome.stdout.contains("nothing to read"),
        "{}",
        outcome.stdout
    );
}
