use proptest::prelude::*;
use rstest::rstest;
use serde_json::json;

use super::{Provider, ReferenceError, SecretReference};
use crate::catalog::ValueKind;

#[rstest]
#[case::the_environment("env:SHOP_PASSWORD", Provider::Environment)]
#[case::the_environment_with_a_leading_underscore("env:_KEY_2", Provider::Environment)]
#[case::the_os_keychain("keychain:goodfetch/portal", Provider::Keychain)]
#[case::the_encrypted_local_file("secretfile:shop.password", Provider::SecretFile)]
#[case::the_caller("caller:shop-login", Provider::Caller)]
#[case::one_password("op://Private/Shop/password", Provider::OnePassword)]
#[case::one_password_with_a_section("op://Private/Shop/login/password", Provider::OnePassword)]
#[case::bitwarden("bw:shop/password", Provider::Bitwarden)]
#[case::hashicorp_vault("vault:secret/shop/login#password", Provider::Vault)]
#[case::aws_secrets_manager("aws-sm:prod/shop#password", Provider::AwsSecretsManager)]
#[case::aws_secrets_manager_whole_secret("aws-sm:prod/shop", Provider::AwsSecretsManager)]
#[case::gcp_secret_manager("gcp-sm:my-project/shop-password/latest", Provider::GcpSecretManager)]
#[case::azure_key_vault("azure-kv:shop-vault/password", Provider::AzureKeyVault)]
#[case::a_plugin("plugin:custom-http:shop/password", Provider::Plugin)]
fn a_reference_of_each_provider_parses(#[case] text: &str, #[case] provider: Provider) {
    let reference = SecretReference::parse(text).unwrap();

    assert_eq!(
        (reference.provider(), reference.to_string()),
        (provider, text.to_owned())
    );
}

#[rstest]
#[case::a_raw_password("hunter2-correct-horse", ReferenceError::NoProvider)]
#[case::empty("", ReferenceError::NoProvider)]
#[case::an_unknown_provider("pass:word", ReferenceError::UnknownProvider)]
#[case::a_url("https://vault.example/secret", ReferenceError::UnknownProvider)]
#[case::an_empty_variable("env:", ReferenceError::Malformed(Provider::Environment))]
#[case::a_variable_starting_with_a_digit(
    "env:1KEY",
    ReferenceError::Malformed(Provider::Environment)
)]
#[case::a_variable_with_a_dash("env:SHOP-KEY", ReferenceError::Malformed(Provider::Environment))]
#[case::a_keychain_item_without_account(
    "keychain:goodfetch",
    ReferenceError::Malformed(Provider::Keychain)
)]
#[case::a_keychain_item_with_an_empty_account(
    "keychain:goodfetch/",
    ReferenceError::Malformed(Provider::Keychain)
)]
#[case::a_file_entry_with_a_slash(
    "secretfile:a/b",
    ReferenceError::Malformed(Provider::SecretFile)
)]
#[case::one_password_without_field(
    "op://Private/Shop",
    ReferenceError::Malformed(Provider::OnePassword)
)]
#[case::one_password_with_an_empty_segment(
    "op://Private//password",
    ReferenceError::Malformed(Provider::OnePassword)
)]
#[case::bitwarden_without_field("bw:shop", ReferenceError::Malformed(Provider::Bitwarden))]
#[case::vault_without_field("vault:secret/shop", ReferenceError::Malformed(Provider::Vault))]
#[case::vault_without_path("vault:secret#password", ReferenceError::Malformed(Provider::Vault))]
#[case::aws_with_an_empty_key(
    "aws-sm:prod/shop#",
    ReferenceError::Malformed(Provider::AwsSecretsManager)
)]
#[case::gcp_without_version(
    "gcp-sm:my-project/shop",
    ReferenceError::Malformed(Provider::GcpSecretManager)
)]
#[case::azure_without_secret(
    "azure-kv:shop-vault",
    ReferenceError::Malformed(Provider::AzureKeyVault)
)]
#[case::a_plugin_without_rest("plugin:custom-http:", ReferenceError::Malformed(Provider::Plugin))]
#[case::a_plugin_without_name("plugin::shop", ReferenceError::Malformed(Provider::Plugin))]
#[case::a_space("env:SHOP PASSWORD", ReferenceError::NotPrintable)]
#[case::a_line_feed("env:SHOP\nPASSWORD", ReferenceError::NotPrintable)]
fn text_that_is_no_reference_is_refused(#[case] text: &str, #[case] error: ReferenceError) {
    assert_eq!(SecretReference::parse(text), Err(error));
}

#[test]
fn a_reference_past_its_length_is_refused() {
    let text = format!("env:{}", "A".repeat(1_021));

    assert_eq!(SecretReference::parse(&text), Err(ReferenceError::TooLong));
}

/// Covers: story:4-49
#[test]
fn a_raw_value_for_a_secret_reference_setting_is_refused_without_its_value() {
    let raw = "gfcanary-password-0123456789abcdef";

    let reason = ValueKind::SecretReference.check(&json!(raw)).unwrap_err();

    assert!(!reason.contains(raw), "{reason}");
}

/// Covers: story:4-49
#[test]
fn a_reference_for_a_secret_reference_setting_is_accepted() {
    assert_eq!(
        ValueKind::SecretReference.check(&json!("env:SHOP_PASSWORD")),
        Ok(())
    );
}

proptest! {
    /// A reference reads back as the text it was parsed from, and that text
    /// parses to the same reference.
    #[test]
    fn a_parsed_reference_is_its_own_text(text in "(env|keychain|op|bw|vault|aws-sm|gcp-sm|azure-kv|plugin|caller|secretfile)(:|://)[A-Za-z0-9_/#:.-]{1,24}") {
        if let Ok(reference) = SecretReference::parse(&text) {
            prop_assert_eq!(reference.to_string(), text.clone());
            prop_assert_eq!(SecretReference::parse(&reference.to_string()), Ok(reference));
        }
    }

    /// A refusal never repeats the text it refused, so a raw password
    /// given where a reference belongs reaches no message (ADR 0092).
    #[test]
    fn a_refusal_never_repeats_the_refused_text(text in "[A-Za-z0-9:/#@!._-]{12,40}") {
        if let Err(error) = SecretReference::parse(&text) {
            prop_assert!(!error.to_string().contains(&text));
        }
    }
}
