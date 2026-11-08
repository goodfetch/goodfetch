//! What the protocol suite's harness does when what a case relies on fails,
//! rather than the contract it judges: each failure fails its case, naming
//! why, and leaves the next case its result. The target is a stand-in that
//! answers every call, since no script can spell a call whose params hold
//! the Proving ground's port; where the Proving ground must fail too, so is
//! it.
#![cfg(unix)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt as _;

use goodfetch_conformance::{
    EngineMode, EngineTarget, ProtocolSuite, ProtocolSuiteError, ProvingGround, run_protocol_suite,
};
use goodfetch_protocol::result_document::{CaseOutcome, ConformanceDocument};
use goodfetch_test_support::{ATTACHED_SCRIPT, ProvingGround as LocalCopy, TestEnv, Tool, tools};
use rstest::rstest;
use serde_json::{Value, json};

/// A crawl's fetch case, whose handler the harness answers, and which reads
/// the requests the Proving ground received.
const FETCHES: &str = "a-request-fetched-over-http-hands-its-handler-the-document-on-a-page";

/// A case that reads nothing of the Proving ground and answers nothing.
const ASKS_THE_ENGINE: &str = "engine-info-names-the-engine-hello-named";

/// A case that reads the Proving ground's observations.
const READS_OBSERVATIONS: &str = "the-socks5-proxy-credentials-reach-it-through-the-sessions-relay";

/// The case that writes user configuration for an Agent's Run, then puts it
/// back.
const CONFIGURES: &str = "a-block-never-replaces-the-programs-or-the-agents-session";

/// A suite of `cases`, in order, on Foxhound's executable or Collie's, one
/// capability each.
fn suite(product: &str, cases: &[&str]) -> ProtocolSuite {
    let mut text = format!(
        "product = \"{product}\"\n\n[suite]\nname = \"harness-failures\"\nversion = \"1\"\n"
    );
    for case in cases {
        text.push_str(&format!(
            "\n[[capability]]\nname = \"{case}\"\n\n[[capability.case]]\nid = \"{case}\"\ncovers = []\n"
        ));
    }
    ProtocolSuite::read(&text).unwrap()
}

/// A stand-in target in `env`'s folder: an Attached engine of whichever
/// Product `hello` needs, which answers each call with an empty result,
/// `crawl.start` with a new Run followed by the line `after_crawl_start`.
fn stand_in_target(env: &TestEnv, after_crawl_start: &str) -> EngineTarget {
    let executable = env.folder().join("stand-in-engine");
    let script = format!(
        r#"#!/bin/sh
product=collie
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  case $method in
    hello)
      product=$(printf '%s' "$line" | sed -n 's/.*"needs":"\([a-z]*\)".*/\1/p')
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocol_version":"0.1","product":"%s","product_version":"0.0.0","engine_instance":"0123456789abcdef"}}}}\n' "$id" "$product"
      ;;
    engine.info)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"product":"%s","product_version":"0.0.0","protocol_version":"0.1","product_major":0}}}}\n' "$id" "$product"
      ;;
    crawl.start)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"started_as":"new"}}}}\n' "$id"
      printf '%s\n' '{after_crawl_start}'
      ;;
    *)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id"
      ;;
  esac
done
"#
    );
    std::fs::write(&executable, script).unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut environment = env.env().clone();
    environment.insert("PATH".to_owned(), "/usr/bin:/bin".to_owned());
    EngineTarget {
        executable,
        mode: EngineMode::Attached,
        environment,
        folder: env.folder().to_path_buf(),
    }
}

/// A handler request `crawl.start` is followed by, which the fetch case
/// answers.
const HANDLER_REQUEST: &str =
    r#"{"jsonrpc":"2.0","id":1,"method":"crawl.request.handle","params":{}}"#;

/// A stand-in Proving ground that serves the fetch case's task and the
/// `socks5` proxy, and answers `GET /observations` with `observations`, a
/// status and a body. It serves until the test's process ends.
fn failing_proving_ground(observations: (u16, &'static str)) -> ProvingGround {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap();
    let tasks = json!({
        "proving_ground": { "version": "0.0.0", "seed": 1 },
        "tasks": [{
            "id": "content-coding/8",
            "family": "content-coding",
            "kind": "crawl",
            "start_url": format!("http://content-coding.test:{}/8", address.port()),
            "description": "A page.",
            "digest": "0",
        }],
    })
    .to_string();
    let fixtures = json!({
        "proxies": { "socks5": "127.0.0.1:1" },
        "proxy_credentials": { "username": "user", "password": "password" },
    })
    .to_string();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut stream = stream.unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut request = String::new();
            reader.read_line(&mut request).unwrap();
            loop {
                let mut header = String::new();
                reader.read_line(&mut header).unwrap();
                if header.trim().is_empty() {
                    break;
                }
            }
            let path = request.split(' ').nth(1).unwrap_or_default();
            let (status, body) = match path {
                "/tasks" => (200, tasks.as_str()),
                "/network-fixtures" => (200, fixtures.as_str()),
                "/observations" => observations,
                _ => (404, ""),
            };
            write!(
                stream,
                "HTTP/1.1 {status} Answered\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        }
    });
    ProvingGround::local(&address.to_string())
}

/// `case`'s outcome and the names of its failed checks.
fn failed_checks(document: &ConformanceDocument, case: &str) -> (CaseOutcome, Vec<String>) {
    let found = document
        .capabilities()
        .iter()
        .flat_map(|capability| capability.cases())
        .find(|found| found.facts().id == case)
        .unwrap_or_else(|| panic!("the document holds no case {case}"));
    let failed = found
        .facts()
        .entries
        .iter()
        .flat_map(|entry| &entry.checks)
        .filter(|check| !check.passed)
        .map(|check| check.name.clone())
        .collect();
    (found.outcome(), failed)
}

/// A message with no ID, which a handler request's reply cannot carry back,
/// is the case's failure: the Connection is fine, so it stays open for the
/// cases after it, and nothing blames it.
#[rstest]
#[case::a_notification(
    r#"{"jsonrpc":"2.0","method":"run.progress","params":{}}"#,
    "the message the case answers carries a request's ID, not null"
)]
#[case::an_error_with_no_id(
    r#"{"jsonrpc":"2.0","id":null,"error":{"code":-32600,"message":"no","data":{"code":"invalid_request","retryable":false}}}"#,
    "the message the case answers carries a request's ID, not null"
)]
fn a_message_with_no_id_to_answer_fails_its_case_and_the_next_case_still_runs(
    #[case] message: &str,
    #[case] check: &str,
) {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = stand_in_target(&env, message);

    let document = run_protocol_suite(
        &suite("foxhound", &[FETCHES, ASKS_THE_ENGINE]),
        &target,
        &proving_ground,
    )
    .unwrap();

    let (outcome, failed) = failed_checks(&document, FETCHES);
    assert_eq!(outcome, CaseOutcome::Failed);
    assert!(failed.iter().any(|name| name == check), "{failed:?}");
    assert!(
        !failed.iter().any(|name| name.contains("the Connection")),
        "{failed:?}"
    );
    assert_eq!(
        failed_checks(&document, ASKS_THE_ENGINE),
        (CaseOutcome::Passed, vec![])
    );
}

/// A case that could not read what the Proving ground received has no
/// evidence for its checks, whichever of them it reached: its case fails
/// naming why, and the next case starts with nothing against it.
#[rstest]
#[case::the_proving_ground_refuses((500, "no"))]
#[case::it_holds_no_list_of_requests((200, "{}"))]
fn requests_the_proving_ground_cannot_list_fail_the_case_alone(
    #[case] observations: (u16, &'static str),
) {
    let env = TestEnv::new();
    let proving_ground = failing_proving_ground(observations);
    let target = stand_in_target(&env, HANDLER_REQUEST);

    let document = run_protocol_suite(
        &suite("foxhound", &[FETCHES, ASKS_THE_ENGINE]),
        &target,
        &proving_ground,
    )
    .unwrap();

    let (outcome, failed) = failed_checks(&document, FETCHES);
    assert_eq!(outcome, CaseOutcome::Failed);
    assert!(
        failed.iter().any(|name| {
            name.starts_with("the Proving ground lists the requests its page hosts received (")
        }),
        "{failed:?}"
    );
    assert_eq!(
        failed_checks(&document, ASKS_THE_ENGINE),
        (CaseOutcome::Passed, vec![])
    );
}

#[test]
fn observations_the_proving_ground_cannot_serve_fail_the_case() {
    let env = TestEnv::new();
    let proving_ground = failing_proving_ground((500, "no"));
    let target = stand_in_target(&env, HANDLER_REQUEST);

    let document = run_protocol_suite(
        &suite("collie", &[READS_OBSERVATIONS]),
        &target,
        &proving_ground,
    )
    .unwrap();

    let (outcome, failed) = failed_checks(&document, READS_OBSERVATIONS);
    assert_eq!(outcome, CaseOutcome::Failed);
    assert!(
        failed
            .iter()
            .any(|name| name.starts_with("the Proving ground's observations can be read (")),
        "{failed:?}"
    );
}

/// A Daemon that answered every case and refuses the stop is still running,
/// so the suite gives up its document rather than report a clean run.
#[test]
fn a_daemon_that_refuses_the_stop_gives_up_the_document() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let conforming = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/it/fixtures/protocol/conforming-daemon.json"
    );
    let mut script: Value =
        serde_json::from_str(&std::fs::read_to_string(conforming).unwrap()).unwrap();
    let steps = script["steps"].as_array_mut().unwrap();
    let answer = steps.len() - 2;
    assert_eq!(steps[answer - 1]["expect"]["method"], json!("daemon.stop"));
    steps[answer] = json!({ "reply": { "error": {
        "code": -32000,
        "message": "Runs are live.",
        "data": { "code": "daemon_busy", "retryable": true },
    } } });
    let installed = env.home().join(ATTACHED_SCRIPT);
    std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
    std::fs::write(&installed, script.to_string()).unwrap();
    let target = EngineTarget {
        executable: tools()
            .unwrap()
            .path(Tool::ScriptedEngine)
            .unwrap()
            .to_path_buf(),
        mode: EngineMode::Daemon,
        environment: env.env().clone(),
        folder: env.folder().to_path_buf(),
    };

    let refused =
        run_protocol_suite(&ProtocolSuite::embedded(), &target, &proving_ground).unwrap_err();

    assert!(
        matches!(refused, ProtocolSuiteError::NotStopped { .. }),
        "{refused}"
    );
}

/// A user configuration that reads and will not write: writing to a file of
/// `/proc` fails whoever runs the test.
///
/// The case saves what is there, fails to write, and must leave it: what it
/// read is the only copy, and the Home's later cases rely on the file.
#[cfg(target_os = "linux")]
#[test]
fn a_user_configuration_the_case_cannot_write_is_left_in_place_and_no_agent_runs_on_the_old_one() {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = stand_in_target(&env, HANDLER_REQUEST);
    let major = env.home().join("major");
    std::fs::create_dir_all(
        major
            .join(goodfetch_protocol::home::RUNS)
            .join("conformance-blocked-program"),
    )
    .unwrap();
    let configuration = major.join(goodfetch_protocol::home::USER_CONFIGURATION_FILE);
    drop(std::fs::remove_file(&configuration));
    std::os::unix::fs::symlink("/proc/version", &configuration).unwrap();

    let document =
        run_protocol_suite(&suite("collie", &[CONFIGURES]), &target, &proving_ground).unwrap();

    let (outcome, failed) = failed_checks(&document, CONFIGURES);
    assert_eq!(outcome, CaseOutcome::Failed);
    assert!(
        failed
            .iter()
            .any(|name| name.starts_with("user configuration lists the site's hosts (")),
        "{failed:?}"
    );
    assert!(configuration.is_symlink(), "the case removed the file");
    let evidence = serde_json::to_string(&document).unwrap();
    assert!(
        !evidence.contains("conformance-blocked-agent"),
        "an Agent's Run started on the old configuration"
    );
}
