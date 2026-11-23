//! The protocol suite's gate mode (ADR 0047): the suite runs twice, with
//! Filter-list blocking on and off, and a case fails only where blocking
//! alone breaks it. The target is a stand-in on the local copy of the
//! Proving ground, whose break the test toggles for each blocking state, so
//! no outside network is reached.
#![cfg(unix)]

use std::os::unix::fs::PermissionsExt as _;

use goodfetch_conformance::{
    EngineMode, EngineTarget, ProvingGround, run_protocol_suite_in_gate_mode,
};
use goodfetch_protocol::result_document::{Blocking, CaseOutcome};
use goodfetch_test_support::{ProvingGround as LocalCopy, TestEnv};
use rstest::rstest;

use crate::harness_failures::{failed_checks, suite};

/// A case whose two pages must share one Session.
const SHARES_A_SESSION: &str = "pages-open-in-the-runs-default-session";

/// The check the stand-in breaks.
const SAME_SESSION: &str = "the second page is in the first page's Session";

/// A stand-in Collie in `env`'s folder, Attached, which answers every call
/// with an empty result and opens each page of a Run in a Session of its
/// own, breaking [`SAME_SESSION`], with blocking on when `breaks_on` and
/// with blocking off when `breaks_off`. A `run.start` whose settings turn
/// the `ads` Filter list off is a Run with blocking off.
fn stand_in_target(env: &TestEnv, breaks_on: bool, breaks_off: bool) -> EngineTarget {
    let executable = env.folder().join("stand-in-collie");
    let script = format!(
        r#"#!/bin/sh
breaks={breaks_on}
pages=0
while IFS= read -r line; do
  id=$(printf '%s' "$line" | sed -n 's/.*"id":\([0-9][0-9]*\).*/\1/p')
  method=$(printf '%s' "$line" | sed -n 's/.*"method":"\([^"]*\)".*/\1/p')
  case $method in
    hello)
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"protocol_version":"0.1","product":"collie","product_version":"0.0.0","engine_instance":"0123456789abcdef"}}}}\n' "$id"
      ;;
    run.start)
      case $line in
        *'"ads":false'*) breaks={breaks_off} ;;
        *) breaks={breaks_on} ;;
      esac
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{}}}}\n' "$id"
      ;;
    run.new_page)
      pages=$((pages + 1))
      session=session-1
      if [ "$breaks" = true ]; then session=session-$pages; fi
      printf '{{"jsonrpc":"2.0","id":%s,"result":{{"page_id":"page-%s","session_id":"%s"}}}}\n' "$id" "$pages" "$session"
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

/// A gate run keeps each case's result with blocking on and with blocking
/// off, and the mode, in its document. A case that breaks only with
/// blocking on fails and makes the run block; one that breaks both ways is
/// inconclusive, which does not block.
///
/// Covers: story:2-73
#[rstest]
#[case::breaks_only_with_blocking_on(true, false, CaseOutcome::Failed, true)]
#[case::breaks_both_ways(true, true, CaseOutcome::Inconclusive, false)]
#[case::breaks_only_with_blocking_off(false, true, CaseOutcome::Passed, false)]
fn a_gate_run_fails_a_case_only_where_blocking_alone_breaks_it(
    #[case] breaks_on: bool,
    #[case] breaks_off: bool,
    #[case] outcome: CaseOutcome,
    #[case] blocks: bool,
) {
    let env = TestEnv::new();
    let copy = LocalCopy::start(&env, env.seed()).unwrap();
    let proving_ground = ProvingGround::local(&copy.address().to_string());
    let target = stand_in_target(&env, breaks_on, breaks_off);

    let document = run_protocol_suite_in_gate_mode(
        &suite("collie", &[SHARES_A_SESSION]),
        &target,
        &proving_ground,
    )
    .unwrap();

    assert!(document.header().gate);
    let found = document
        .capabilities()
        .iter()
        .flat_map(goodfetch_protocol::result_document::CapabilityResult::cases)
        .find(|found| found.facts().id == SHARES_A_SESSION)
        .unwrap();
    let ran: Vec<(Option<Blocking>, bool)> = found
        .facts()
        .entries
        .iter()
        .map(|entry| {
            (
                entry.blocking,
                entry.checks.iter().all(|check| check.passed),
            )
        })
        .collect();
    assert_eq!(
        ran,
        vec![
            (Some(Blocking::On), !breaks_on),
            (Some(Blocking::Off), !breaks_off)
        ]
    );
    let (found_outcome, failed) = failed_checks(&document, SHARES_A_SESSION);
    assert_eq!(found_outcome, outcome);
    assert!(failed.iter().all(|name| name == SAME_SESSION), "{failed:?}");
    assert_eq!(document.blocks(), blocks);
}
