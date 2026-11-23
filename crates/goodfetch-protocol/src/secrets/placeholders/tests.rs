use rstest::rstest;
use serde_json::{Value, json};

use super::{BoundPlaceholder, PlaceholderSource, SECRETS_PLACEHOLDERS};
use crate::catalog::SettingJson as _;
use crate::secret_reference::SecretReference;

/// A value's check by the setting, as every layer reads it: `Ok` when
/// accepted, else what `setting_invalid` would say.
fn checked(bound: &Value) -> Result<(), String> {
    SECRETS_PLACEHOLDERS.kind.check(&json!({ "github": bound }))
}

/// A Secret reference, or registrable domains only, each name of a
/// multi-label public suffix's included, is bound.
#[rstest]
/// Covers: setting:secrets.placeholders
#[case::a_reference(json!({ "reference": "env:GITHUB_PASSWORD", "domains": ["github.com"] }))]
#[case::a_totp_seed(json!({ "reference": "op://vault/github/otp", "domains": ["github.com"], "totp": true }))]
#[case::a_code_source(json!({ "code_source": "inbox", "domains": ["example.com"] }))]
#[case::a_multi_label_suffix(json!({ "reference": "env:BANK", "domains": ["bank.co.uk", "shop.com.au"] }))]
#[case::every_domain_when_written(json!({ "reference": "env:ANY", "domains": ["*"] }))]
fn a_placeholder_bound_to_registrable_domains_is_accepted(#[case] bound: Value) {
    assert_eq!(checked(&bound), Ok(()));
}

/// A binding without `domains`, with none, with a host below a registrable
/// domain, a public suffix, a value in place of a reference, or both
/// sources, is refused with what makes it right.
#[rstest]
/// Covers: setting:secrets.placeholders
#[case::no_domains(json!({ "reference": "env:GITHUB_PASSWORD" }), "`domains`")]
#[case::empty_domains(json!({ "reference": "env:GITHUB_PASSWORD", "domains": [] }), "at least one")]
#[case::a_subdomain(json!({ "reference": "env:GITHUB_PASSWORD", "domains": ["www.example.com"] }), "`example.com`")]
#[case::a_public_suffix(json!({ "reference": "env:BANK", "domains": ["co.uk"] }), "public suffix")]
#[case::a_value(json!({ "reference": "hunter2", "domains": ["github.com"] }), "Secret reference")]
#[case::both_sources(json!({ "reference": "env:A", "code_source": "inbox", "domains": ["github.com"] }), "either")]
#[case::totp_beside_a_code_source(json!({ "code_source": "inbox", "domains": ["github.com"], "totp": true }), "`totp`")]
fn a_placeholder_bound_otherwise_is_refused(#[case] bound: Value, #[case] says: &str) {
    let refused = checked(&bound).expect_err("refused");
    assert!(refused.contains(says), "{refused}");
}

/// What a typed SDK key sends is what the engine reads back.
#[rstest]
#[case::a_reference(json!({ "reference": "env:GITHUB_PASSWORD", "domains": ["github.com"] }))]
#[case::a_totp_seed(json!({ "reference": "op://vault/github/otp", "totp": true, "domains": ["github.com"] }))]
#[case::a_code_source(json!({ "code_source": "inbox", "domains": ["example.com", "*"] }))]
fn a_bound_placeholder_sends_what_it_reads(#[case] bound: Value) {
    let read = BoundPlaceholder::read(&bound).expect("bound");

    assert_eq!(read.into_setting_json(), bound);
}

#[test]
fn a_bound_placeholder_reads_as_its_source_and_domains() {
    let bound = BoundPlaceholder::read(&json!({
        "reference": "op://vault/github/otp",
        "domains": ["github.com"],
        "totp": true,
    }));

    assert_eq!(
        bound,
        Ok(BoundPlaceholder {
            source: PlaceholderSource::Reference {
                reference: SecretReference::parse("op://vault/github/otp").expect("a reference"),
                totp: true,
            },
            domains: vec!["github.com".to_owned()],
        })
    );
}
