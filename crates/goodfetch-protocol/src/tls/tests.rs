#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use rstest::rstest;
use serde_json::{Value, json};

use super::{
    CertificateFailure, CertificateInvalidDetails, ClientCertificateConfiguration,
    KnownCertificateFailure, TLS_CLIENT_CERTIFICATES,
};
use crate::catalog::ValueKind;
use crate::entries::MODULES;
use crate::secret_reference::SecretReference;
use crate::settings::DEPLOYMENT_VARIABLES;

/// Words that name turning certificate checks off, in a setting's key.
const SWITCH_WORDS: [&str; 6] = [
    "verif",
    "insecure",
    "invalid_cert",
    "ignore_cert",
    "skip_tls",
    "danger",
];

/// Words that name certificates or TLS, in a deployment variable.
const VARIABLE_WORDS: [&str; 6] = ["TLS", "SSL", "CERT", "VERIF", "INSECURE", "CA_"];

#[test]
fn certificate_invalid_names_the_host_the_failure_and_the_fix_on_the_wire() {
    let details = CertificateInvalidDetails {
        host: "expired.certificates.test".to_owned(),
        failure: CertificateFailure::Known(KnownCertificateFailure::Expired),
        fix: "List expired.certificates.test under `tls.accept_invalid_certs`.".to_owned(),
    };

    assert_eq!(
        serde_json::to_value(&details).unwrap(),
        json!({
            "host": "expired.certificates.test",
            "failure": "expired",
            "fix": "List expired.certificates.test under `tls.accept_invalid_certs`.",
        })
    );
}

#[test]
fn a_failure_the_wire_does_not_name_travels_as_its_text_and_reads_back() {
    let failure: CertificateFailure =
        serde_json::from_value(json!("unhandled_critical_extension")).unwrap();
    let known: CertificateFailure = serde_json::from_value(json!("self_signed")).unwrap();

    assert_eq!(
        (failure, known),
        (
            CertificateFailure::Unrecognized("unhandled_critical_extension".to_owned()),
            CertificateFailure::Known(KnownCertificateFailure::SelfSigned)
        )
    );
}

/// Story 4-31: no setting or deployment variable turns verification off.
/// Every `tls.` setting maps a domain to a value, so an exception is always
/// one domain's; no switch's key names turning checks off; and no
/// deployment variable is about certificates or TLS at all.
/// Covers: story:4-31, setting:tls.trusted_cas, setting:tls.accept_invalid_certs
#[test]
fn no_setting_or_deployment_variable_switches_certificate_verification_off() {
    let settings: Vec<_> = MODULES
        .iter()
        .flat_map(|entries| entries.settings)
        .collect();
    let tls: Vec<_> = settings
        .iter()
        .filter(|setting| setting.name.starts_with("tls."))
        .collect();
    let not_per_domain: Vec<&str> = tls
        .iter()
        .filter(|setting| !matches!(setting.kind, ValueKind::Map(_)))
        .map(|setting| setting.name)
        .collect();
    let switches: Vec<&str> = settings
        .iter()
        .filter(|setting| matches!(setting.kind, ValueKind::Boolean))
        .filter(|setting| SWITCH_WORDS.iter().any(|word| setting.name.contains(word)))
        .map(|setting| setting.name)
        .collect();
    let variables: Vec<&str> = DEPLOYMENT_VARIABLES
        .iter()
        .copied()
        .filter(|variable| VARIABLE_WORDS.iter().any(|word| variable.contains(word)))
        .collect();

    assert!(
        tls.iter().any(|setting| setting.name == "tls.trusted_cas"),
        "the enumeration sees the per-domain settings"
    );
    assert_eq!(
        (not_per_domain, switches, variables),
        (Vec::<&str>::new(), Vec::<&str>::new(), Vec::<&str>::new())
    );
}

/// A raw private key, as a person might paste one in place of its
/// reference.
const RAW_KEY: &str = "-----BEGIN PRIVATE KEY-----\nc4n4ryk3yAgEAMBMGByqGSM49AgEGCCqGSM49AwEHBG0wawIBAQQg\n-----END PRIVATE KEY-----\n";

/// Story 4-34: a domain's client certificate is a certificate file and the
/// Secret reference of its key, and nothing else.
/// Covers: story:4-34, setting:tls.client_certificates
#[test]
fn a_client_certificate_is_a_file_and_the_secret_reference_of_its_key() {
    let entry = json!({ "certificate": "client.pem", "key": "env:CLIENT_KEY" });

    let read = ClientCertificateConfiguration::read(&entry);
    let checked = TLS_CLIENT_CERTIFICATES
        .kind
        .check(&json!({ "mtls.example": entry }));

    assert_eq!(
        (read, checked),
        (
            Ok(ClientCertificateConfiguration {
                certificate: "client.pem".to_owned(),
                key: SecretReference::parse("env:CLIENT_KEY").unwrap(),
            }),
            Ok(())
        )
    );
}

/// Story 4-34: a key given as a raw value, or an entry that is not a
/// certificate file and a Secret reference, is refused, and the refusal,
/// which a `setting_invalid` message carries, never repeats the value.
/// Covers: story:4-34, setting:tls.client_certificates, error:setting_invalid
#[rstest]
#[case::a_raw_key(json!({ "certificate": "client.pem", "key": RAW_KEY }), "`key`")]
#[case::a_key_that_is_no_text(json!({ "certificate": "client.pem", "key": [RAW_KEY] }), "`key`")]
#[case::no_key(json!({ "certificate": "client.pem" }), "`key`")]
#[case::no_certificate(json!({ "key": "env:CLIENT_KEY" }), "`certificate`")]
#[case::an_unknown_field(
    json!({ "certificate": "client.pem", "key": "env:CLIENT_KEY", "password": RAW_KEY }),
    "`password`"
)]
#[case::the_key_in_place_of_the_entry(json!(RAW_KEY), "a table")]
fn a_client_certificate_that_is_no_file_and_reference_is_refused_without_its_value(
    #[case] entry: Value,
    #[case] named: &str,
) {
    let refused = TLS_CLIENT_CERTIFICATES
        .kind
        .check(&json!({ "mtls.example": entry }))
        .unwrap_err();

    assert!(refused.contains(named), "{refused}");
    assert!(
        !refused.contains("c4n4ryk3y"),
        "the refusal repeats the key: {refused}"
    );
}
