//! Azure Key Vault's three translation tables (ADR 0127): one named case
//! per failure Key Vault documents for Get Secret, per OAuth 2.0 error
//! Microsoft Entra ID documents for the client-credentials grant and per
//! status a managed identity endpoint documents, and one for a failure each
//! does not know; which source the environment names; and the value a
//! successful read holds.

use std::collections::BTreeMap;
use std::ffi::OsString;

use goodfetch_engine::secrets::ProviderFailure;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::access_token::{Source, translate_identity, translate_token};
use super::{is_vault_name, translate, value_of};

/// The identity a failure names.
const FROM: &str = "the workload identity of `AZURE_FEDERATED_TOKEN_FILE`";

/// Which [`ProviderFailure`] a failure is, without its text.
#[derive(Debug, PartialEq, Eq)]
enum Meaning {
    Missing,
    Refused,
    Busy,
    NotConfigured,
    Unrecognized,
}

fn meaning(failure: &ProviderFailure) -> Meaning {
    match failure {
        ProviderFailure::Missing => Meaning::Missing,
        ProviderFailure::Refused { .. } => Meaning::Refused,
        ProviderFailure::NotConfigured(_) => Meaning::NotConfigured,
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
    assert_eq!(meaning(&translate(status, body.as_bytes(), FROM)), expected);
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
    assert_eq!(
        meaning(&translate_token(status, body.as_bytes(), FROM)),
        expected
    );
}

/// A managed identity endpoint's failures, by status, in Azure's shape;
/// an answer in no shape of Azure's is none of them.
#[rstest]
#[case::no_identity(
    400,
    r#"{"error":"invalid_request","error_description":"Identity not found"}"#,
    Some(Meaning::NotConfigured)
)]
#[case::updating(
    404,
    r#"{"error":"not_found","error_description":"IMDS is updating"}"#,
    Some(Meaning::Busy)
)]
#[case::going_through_updates(
    410,
    r#"{"error":"gone","error_description":"retry"}"#,
    Some(Meaning::Busy)
)]
#[case::throttled(
    429,
    r#"{"error":"too_many_requests","error_description":"slow down"}"#,
    Some(Meaning::Busy)
)]
#[case::a_transient_failure(500, r#"{"error":"unknown","error_description":"Failed to retrieve token from the Active directory."}"#, Some(Meaning::Busy))]
#[case::unavailable(
    503,
    r#"{"error":"unavailable","error_description":"retry"}"#,
    Some(Meaning::Busy)
)]
#[case::an_unknown_failure(
    401,
    r#"{"error":"unknown_source","error_description":"Unknown Source"}"#,
    Some(Meaning::Unrecognized)
)]
#[case::no_json(404, "Not Found", None)]
#[case::json_without_an_error(404, r#"{"message":"not here"}"#, None)]
fn each_known_identity_failure_has_its_meaning(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Option<Meaning>,
) {
    assert_eq!(
        translate_identity(status, body.as_bytes(), "IMDS")
            .as_ref()
            .map(meaning),
        expected
    );
}

/// The first source the environment names is taken, in
/// `DefaultAzureCredential`'s order, and `AZURE_TOKEN_CREDENTIALS` keeps
/// only the sources it names.
#[rstest]
#[case::a_service_principal(&[("AZURE_CLIENT_SECRET", "s"), ("AZURE_FEDERATED_TOKEN_FILE", "/t")], "service principal")]
#[case::a_workload_identity(&[("AZURE_FEDERATED_TOKEN_FILE", "/t")], "workload identity")]
#[case::the_hosts_managed_identity(&[("IDENTITY_ENDPOINT", "http://127.0.0.1:1/msi/token"), ("IDENTITY_HEADER", "h")], "`IDENTITY_ENDPOINT`")]
#[case::the_virtual_machines_managed_identity(&[], "virtual machine")]
#[case::kept_to_managed_identity(&[("AZURE_CLIENT_SECRET", "s"), ("AZURE_TOKEN_CREDENTIALS", "ManagedIdentityCredential")], "virtual machine")]
#[case::kept_to_the_environment(
    &[("AZURE_TOKEN_CREDENTIALS", "environmentcredential")],
    "no identity"
)]
#[case::kept_to_developer_tools(&[("AZURE_CLIENT_SECRET", "s"), ("AZURE_TOKEN_CREDENTIALS", "dev")], "no identity")]
#[case::kept_to_deployed_services(&[("AZURE_CLIENT_SECRET", "s"), ("AZURE_TOKEN_CREDENTIALS", "prod")], "service principal")]
fn the_first_source_named_is_taken(#[case] variables: &[(&str, &str)], #[case] named: &str) {
    let environment: BTreeMap<OsString, OsString> =
        [("AZURE_TENANT_ID", "t"), ("AZURE_CLIENT_ID", "c")]
            .iter()
            .chain(variables)
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect();

    let source = Source::of(&environment);

    assert!(source.describe().contains(named), "{source:?}");
}

/// A value of `AZURE_TOKEN_CREDENTIALS` the Azure Identity libraries do
/// not take is a configuration problem naming it.
#[test]
fn an_unknown_choice_is_not_configured() {
    let environment = BTreeMap::from([(
        OsString::from("AZURE_TOKEN_CREDENTIALS"),
        OsString::from("everything"),
    )]);

    let problem = Source::of(&environment).check().unwrap_err();

    assert!(problem.fix.contains("`everything`"), "{}", problem.fix);
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
