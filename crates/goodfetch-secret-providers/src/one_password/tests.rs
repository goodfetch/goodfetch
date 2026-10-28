//! `op`'s translation table (ADR 0127): one named case per failure it is
//! known to write and one for a failure it is not, and its own text,
//! planted as a Secret canary, in no log line and no formatted failure.

use goodfetch_engine::secrets::ProviderFailure;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, logged};
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
        other => panic!("`op` never answers {other:?}"),
    }
}

/// Covers: story:4-50
#[rstest]
#[case::no_such_item(
    "[ERROR] 2026/10/26 10:00:00 could not read secret 'op://Shop/portal/password': could not get item Shop/portal: \"portal\" isn't an item in the \"Shop\" vault. Specify the item with its UUID, name, or domain.",
    Meaning::Missing
)]
#[case::no_such_vault(
    "[ERROR] 2026/10/26 10:00:00 could not read secret 'op://Shop/portal/password': \"Shop\" isn't a vault in this account. Specify the vault with its UUID or name.",
    Meaning::Missing
)]
#[case::no_such_field(
    "[ERROR] 2026/10/26 10:00:00 could not read secret 'op://Shop/portal/pin': item 'Shop/portal' does not have a field 'pin'",
    Meaning::Missing
)]
#[case::several_items_match(
    "[ERROR] 2026/10/26 10:00:00 could not read secret 'op://Shop/portal/password': More than one item matches \"portal\". Try again and specify the item by its ID",
    Meaning::Refused
)]
#[case::not_signed_in(
    "[ERROR] 2026/10/26 10:00:00 You are not currently signed in. Please run `op signin --help` for instructions",
    Meaning::Refused
)]
#[case::account_not_signed_in(
    "[ERROR] 2026/10/26 10:00:00 account is not signed in",
    Meaning::Refused
)]
#[case::token_refused(
    "[ERROR] 2026/10/26 10:00:00 (401) Unauthorized: You aren't authorized to perform this action",
    Meaning::Refused
)]
#[case::unreachable(
    "[ERROR] 2026/10/26 10:00:00 Get \"https://my.1password.com/api/v2/account\": dial tcp: lookup my.1password.com: no such host",
    Meaning::Refused
)]
#[case::refused_connection(
    "[ERROR] 2026/10/26 10:00:00 dial tcp 127.0.0.1:443: connect: connection refused",
    Meaning::Refused
)]
#[case::an_unknown_failure(
    "[ERROR] 2026/10/26 10:00:00 the moon is in the wrong phase",
    Meaning::Unrecognized
)]
fn ops_failures_translate_each_to_its_meaning(#[case] stderr: &str, #[case] expected: Meaning) {
    assert_eq!(meaning(&translate(stderr.to_owned(), Some(1))), expected);
}

/// Covers: story:4-50
#[test]
fn ops_own_text_is_in_no_log_line_and_no_formatted_failure() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let echoed = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();

    let (failure, logs) = logged(|| translate(format!("[ERROR] unexpected `{echoed}`"), Some(1)));

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
