//! `bw`'s translation table (ADR 0127): one named case per failure its
//! `get` command is known to write (`apps/cli/src/commands/get.command.ts`
//! and `base-program.ts` in `bitwarden/clients`) and one for a failure it
//! is not, and its own text, planted as a Secret canary, in no log line
//! and no formatted failure.
#![expect(
    clippy::disallowed_types,
    reason = "the test's log writer holds its bytes behind a standard Mutex, outside any loom model"
)]

use std::io;
use std::sync::{Arc, Mutex};

use goodfetch_engine::secrets::ProviderFailure;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use rstest::rstest;

use super::translate;

/// Which [`ProviderFailure`] a failure is, without its text.
#[derive(Debug, PartialEq, Eq)]
enum Meaning {
    Missing,
    NotConfigured,
    Refused,
    Unrecognized,
}

fn meaning(failure: &ProviderFailure) -> Meaning {
    match failure {
        ProviderFailure::Missing => Meaning::Missing,
        ProviderFailure::NotConfigured(_) => Meaning::NotConfigured,
        ProviderFailure::Refused { .. } => Meaning::Refused,
        ProviderFailure::Unrecognized(_) => Meaning::Unrecognized,
        other => panic!("`bw` never answers {other:?}"),
    }
}

/// Covers: story:4-50
#[rstest]
#[case::not_found("Not found.", Meaning::Missing)]
#[case::not_a_login("Not a login.", Meaning::Missing)]
#[case::no_password("No password available for this login.", Meaning::Missing)]
#[case::no_username("No username available for this login.", Meaning::Missing)]
#[case::no_uri("No uri available for this login.", Meaning::Missing)]
#[case::no_totp("No TOTP available for this login.", Meaning::Missing)]
#[case::no_notes("No notes available for this item.", Meaning::Missing)]
#[case::several_items_match(
    "More than one result was found. Try getting a specific object by `id` instead. The following objects were found:\n6d1f0b2e-0000-0000-0000-000000000001\n6d1f0b2e-0000-0000-0000-000000000002",
    Meaning::Refused
)]
#[case::not_logged_in("You are not logged in.", Meaning::Refused)]
#[case::locked("Vault is locked.", Meaning::Refused)]
#[case::locked_by_key_connector(
    "Your vault is locked. You must unlock your vault using your session key.\nIf you do not have your session key, you can get a new one by logging out and logging in again.",
    Meaning::Refused
)]
#[case::premium_required("Premium status is required to use this feature.", Meaning::Refused)]
#[case::restricted_by_policy(
    "Access to this item type is restricted by organizational policy.",
    Meaning::Refused
)]
#[case::totp_failed("Couldn't generate TOTP code.", Meaning::Refused)]
#[case::unreachable(
    "FetchError: request to https://vault.example.test/identity/connect/token failed, reason: getaddrinfo ENOTFOUND vault.example.test",
    Meaning::Refused
)]
#[case::refused_connection(
    "FetchError: request to http://127.0.0.1:8080/api/sync failed, reason: connect ECONNREFUSED 127.0.0.1:8080",
    Meaning::Refused
)]
#[case::an_unknown_failure("The moon is in the wrong phase.", Meaning::Unrecognized)]
fn bws_failures_translate_each_to_its_meaning(#[case] stderr: &str, #[case] expected: Meaning) {
    assert_eq!(meaning(&translate(stderr.to_owned(), Some(1))), expected);
}

/// What `tracing` wrote while `work` ran, at every level.
fn logged<T>(work: impl FnOnce() -> T) -> (T, Vec<u8>) {
    #[derive(Clone, Default)]
    struct Captured(Arc<Mutex<Vec<u8>>>);
    impl io::Write for Captured {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let captured = Captured::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_writer(move || writer.clone())
        .with_max_level(tracing::Level::TRACE)
        .with_ansi(false)
        .finish();
    let answer = tracing::subscriber::with_default(subscriber, work);
    let bytes = captured.0.lock().unwrap().clone();
    (answer, bytes)
}

/// Covers: story:4-50
#[test]
fn bws_own_text_is_in_no_log_line_and_no_formatted_failure() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let echoed = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();

    let (failure, logs) = logged(|| translate(format!("Unexpected `{echoed}`."), Some(1)));

    let formatted = format!("{failure:?}");
    assert_eq!(
        (
            meaning(&failure),
            canaries
                .leaks(&[
                    Output::Bytes {
                        name: "logs",
                        bytes: &logs,
                    },
                    Output::Bytes {
                        name: "formatted",
                        bytes: formatted.as_bytes(),
                    },
                ])
                .unwrap(),
            logs.is_empty(),
        ),
        (Meaning::Unrecognized, vec![], false)
    );
}
