//! Azure Key Vault's two translation tables (ADR 0127): one named case per
//! failure Key Vault documents for Get Secret and per OAuth 2.0 error
//! Microsoft Entra ID documents for the client-credentials grant, and one
//! for a failure each does not know; and the value a successful read holds.

use goodfetch_engine::secrets::ProviderFailure;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::{is_vault_name, translate, translate_token, value_of};

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
        other => panic!("not a translation's answer: {other:?}"),
    }
}

#[rstest]
#[case::a_missing_secret(404, r#"{"error":{"code":"SecretNotFound","message":"A secret with (name/id) portal was not found in this key vault. If you recently deleted this secret you may be able to recover it using the correct recovery command. For help resolving this issue, please see https://go.microsoft.com/fwlink/?linkid=2125182"}}"#, Meaning::Missing)]
#[case::a_denied_access(403, r#"{"error":{"code":"Forbidden","message":"Caller is not authorized to perform action on resource.","innererror":{"code":"ForbiddenByRbac"}}}"#, Meaning::Refused)]
#[case::a_disabled_secret(403, r#"{"error":{"code":"Forbidden","message":"Operation get is not allowed on a disabled secret.","innererror":{"code":"SecretDisabled"}}}"#, Meaning::Refused)]
#[case::a_refused_token(
    401,
    r#"{"error":{"code":"Unauthorized","message":"AKV10032: Invalid issuer."}}"#,
    Meaning::Refused
)]
#[case::throttled(429, r#"{"error":{"code":"Throttled","message":"Request was not processed because too many requests were received."}}"#, Meaning::Busy)]
#[case::an_unknown_failure(
    409,
    r#"{"error":{"code":"Conflict","message":"new"}}"#,
    Meaning::Unrecognized
)]
#[case::no_json(502, "Bad Gateway", Meaning::Unrecognized)]
fn each_known_vault_failure_has_its_meaning(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Meaning,
) {
    assert_eq!(meaning(&translate(status, body.as_bytes())), expected);
}

#[rstest]
#[case::a_wrong_client_secret(401, r#"{"error":"invalid_client","error_description":"AADSTS7000215: Invalid client secret provided. Ensure the secret being sent in the request is the client secret value, not the client secret ID.","error_codes":[7000215]}"#, Meaning::Refused)]
#[case::an_unknown_application(400, r#"{"error":"unauthorized_client","error_description":"AADSTS700016: Application with identifier 'shop' was not found in the directory 'Shop'.","error_codes":[700016]}"#, Meaning::Refused)]
#[case::an_unknown_tenant(400, r#"{"error":"invalid_request","error_description":"AADSTS90002: Tenant 'shop' not found.","error_codes":[90002]}"#, Meaning::Refused)]
#[case::a_refused_scope(400, r#"{"error":"invalid_scope","error_description":"AADSTS70011: The provided request must include a 'scope' input parameter.","error_codes":[70011]}"#, Meaning::Refused)]
#[case::unavailable(
    503,
    r#"{"error":"temporarily_unavailable","error_description":"AADSTS50196"}"#,
    Meaning::Busy
)]
#[case::an_unknown_failure(400, r#"{"error":"something_new"}"#, Meaning::Unrecognized)]
#[case::no_json(502, "Bad Gateway", Meaning::Unrecognized)]
fn each_known_token_failure_has_its_meaning(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Meaning,
) {
    assert_eq!(meaning(&translate_token(status, body.as_bytes())), expected);
}

#[rstest]
#[case::text(r#"{"value":"s3cret-text","id":"https://shop.vault.azure.net/secrets/portal/1","attributes":{"enabled":true}}"#, Ok("s3cret-text"))]
#[case::no_value(
    r#"{"id":"https://shop.vault.azure.net/secrets/portal/1"}"#,
    Err(Meaning::Missing)
)]
fn the_value_is_the_secret_value(#[case] body: &str, #[case] expected: Result<&str, Meaning>) {
    let value = value_of(body.as_bytes());
    assert_eq!(
        value
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .map_err(meaning),
        expected
    );
}

/// A vault's name stands in a host name, so only one that can is taken.
#[rstest]
#[case::a_name("shop-vault", true)]
#[case::too_short("sh", false)]
#[case::a_leading_digit("1shop", false)]
#[case::a_dot("shop.evil.test", false)]
#[case::an_at("evil.test@shop", false)]
fn only_a_vault_name_stands_in_the_host(#[case] name: &str, #[case] taken: bool) {
    assert_eq!(is_vault_name(name), taken);
}
