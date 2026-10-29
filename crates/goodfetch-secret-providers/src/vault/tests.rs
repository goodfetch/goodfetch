//! Vault's translation table (ADR 0127): one named case per failure a
//! server is known to answer a read with, taken verbatim from Vault 1.20.4
//! and `OpenBao` 2.4.1 dev servers, and one for a failure it is not; the
//! field a successful read answers; and the server's own text, planted as
//! a Secret canary, in no log line and no formatted failure.

use goodfetch_engine::secrets::ProviderFailure;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv, logged};
use rstest::rstest;
use secrecy::ExposeSecret as _;

use super::{field_of, server_url, translate};

/// Which [`ProviderFailure`] a failure is, without its text.
#[derive(Debug, PartialEq, Eq)]
enum Meaning {
    Missing,
    Refused,
    Busy,
    Unrecognized,
}

fn meaning(failure: &ProviderFailure) -> Meaning {
    match failure {
        ProviderFailure::Missing => Meaning::Missing,
        ProviderFailure::Refused { .. } => Meaning::Refused,
        ProviderFailure::Busy { .. } => Meaning::Busy,
        ProviderFailure::Unrecognized(_) => Meaning::Unrecognized,
        other => panic!("Vault never answers {other:?}"),
    }
}

/// Covers: story:4-50
#[rstest]
#[case::no_such_secret(404, r#"{"errors":[]}"#, Meaning::Missing)]
#[case::a_deleted_version(
    404,
    r#"{"request_id":"bf22e6fb-1863-1556-a796-97e1b5cc7019","lease_id":"","renewable":false,"lease_duration":0,"data":{"data":null,"metadata":{"created_time":"2026-10-26T11:47:50Z","custom_metadata":null,"deletion_time":"2026-10-26T11:47:50Z","destroyed":false,"version":1}},"wrap_info":null,"warnings":null,"auth":null}"#,
    Meaning::Missing
)]
#[case::no_secrets_engine_at_the_mount(
    404,
    r#"{"errors":["no handler for route \"nomount/data/x\". route entry not found."]}"#,
    Meaning::Refused
)]
#[case::openbao_refuses_the_token(403, r#"{"errors":["permission denied"]}"#, Meaning::Refused)]
#[case::vault_refuses_an_invalid_token(
    403,
    r#"{"errors":["2 errors occurred:\n\t* permission denied\n\t* invalid token\n\n"]}"#,
    Meaning::Refused
)]
#[case::no_token(400, r#"{"errors":["missing client token"]}"#, Meaning::Refused)]
#[case::sealed(503, r#"{"errors":["Vault is sealed"]}"#, Meaning::Refused)]
#[case::rate_limited(
    429,
    r#"{"errors":["request path \"secret/data/shop/portal\": rate limit quota exceeded"]}"#,
    Meaning::Busy
)]
#[case::an_unknown_failure(
    500,
    r#"{"errors":["the moon is in the wrong phase"]}"#,
    Meaning::Unrecognized
)]
fn a_servers_failures_translate_each_to_its_meaning(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Meaning,
) {
    assert_eq!(meaning(&translate(status, body.as_bytes())), expected);
}

/// A successful read's field: a text value as it is, another JSON value as
/// its JSON text, and an absent or null field, or a deleted version, as
/// `missing`.
/// Covers: story:4-50
#[rstest]
#[case::a_text_field("password", Some("pw"))]
#[case::a_number_field("port", Some("5432"))]
#[case::a_null_field("note", None)]
#[case::an_absent_field("pin", None)]
fn a_successful_reads_field_is_its_value(#[case] field: &str, #[case] expected: Option<&str>) {
    let body = br#"{"request_id":"974b2cc2-0ed1-7c18-c0a5-9bbc3f4b192d","data":{"data":{"password":"pw","port":5432,"note":null},"metadata":{"version":1}}}"#;

    let read = field_of(body, field);

    assert_eq!(
        read.as_ref()
            .map(|value| value.expose_secret().to_owned())
            .map_err(meaning),
        expected.map(str::to_owned).ok_or(Meaning::Missing)
    );
}

/// The server's address: `https:`, or `http:` on loopback only.
/// Covers: story:4-50
#[rstest]
#[case::https("https://vault.example.com:8200", true)]
#[case::https_behind_a_path("https://example.com/vault", true)]
#[case::http_on_loopback("http://127.0.0.1:8200", true)]
#[case::http_on_localhost("http://localhost:8200", true)]
#[case::http_on_ipv6_loopback("http://[::1]:8200", true)]
#[case::http_off_loopback("http://vault.example.com:8200", false)]
#[case::another_scheme("ftp://vault.example.com", false)]
#[case::no_url("vault.example.com:8200", false)]
#[case::a_query("https://vault.example.com/?token=x", false)]
fn a_servers_address_is_https_or_loopback_http(#[case] address: &str, #[case] taken: bool) {
    assert_eq!(server_url(address).is_ok(), taken, "{address}");
}

/// Covers: story:4-50
#[test]
fn a_servers_own_text_is_in_no_log_line_and_no_formatted_failure() {
    let env = TestEnv::new();
    let mut canaries = Canaries::new(env.seed());
    let echoed = canaries
        .plant(SecretKind::SinkCredential)
        .value()
        .to_owned();

    let (failure, logs) = logged(|| {
        translate(
            500,
            format!(r#"{{"errors":["unexpected `{echoed}`"]}}"#).as_bytes(),
        )
    });

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
