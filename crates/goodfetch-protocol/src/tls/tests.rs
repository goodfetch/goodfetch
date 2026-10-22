#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use serde_json::json;

use super::{CertificateFailure, CertificateInvalidDetails, KnownCertificateFailure};
use crate::catalog::ValueKind;
use crate::entries::MODULES;
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
