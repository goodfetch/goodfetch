//! AWS Secrets Manager's translation table (ADR 0127): one named case per
//! `__type` the service documents for `GetSecretValue` and the common
//! errors of every AWS API, and one for a failure it is not; AWS STS's,
//! and the credentials its web identity exchange answers; and the value a
//! successful answer holds. Signature Version 4 is tested where it lives,
//! in the engine's `aws_signing`.

use goodfetch_engine::secrets::ProviderFailure;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::access_key::{assumed, translate_sts};
use super::{translate, value_of};

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

/// The bodies follow the `awsJson1.1` protocol's error shape and the
/// messages of Secrets Manager's API reference for `GetSecretValue`.
#[rstest]
#[case::a_missing_secret(400, r#"{"__type":"ResourceNotFoundException","Message":"Secrets Manager can't find the specified secret."}"#, Meaning::Missing)]
#[case::a_denied_access(400, r#"{"__type":"AccessDeniedException","Message":"User: arn:aws:iam::123456789012:user/shop is not authorized to perform: secretsmanager:GetSecretValue on resource: prod/shop because no identity-based policy allows the secretsmanager:GetSecretValue action"}"#, Meaning::Refused)]
#[case::an_unknown_access_key(400, r#"{"__type":"UnrecognizedClientException","message":"The security token included in the request is invalid."}"#, Meaning::Refused)]
#[case::a_wrong_secret_key(400, r#"{"__type":"InvalidSignatureException","message":"The request signature we calculated does not match the signature you provided. Check your AWS Secret Access Key and signing method."}"#, Meaning::Refused)]
#[case::expired_credentials(400, r#"{"__type":"ExpiredTokenException","message":"The security token included in the request is expired"}"#, Meaning::Refused)]
#[case::a_secret_scheduled_for_deletion(400, r#"{"__type":"InvalidRequestException","Message":"You can't perform this operation on the secret because it was marked for deletion."}"#, Meaning::Refused)]
#[case::a_kms_key_it_cannot_use(400, r#"{"__type":"DecryptionFailure","Message":"Secrets Manager can't decrypt the protected secret text using the provided KMS key."}"#, Meaning::Refused)]
#[case::throttled(
    400,
    r#"{"__type":"ThrottlingException","message":"Rate exceeded"}"#,
    Meaning::Busy
)]
#[case::a_service_failure(
    500,
    r#"{"__type":"InternalServiceError","Message":"An error occurred on the server side."}"#,
    Meaning::Refused
)]
#[case::a_namespaced_type(400, r#"{"__type":"com.amazonaws.secretsmanager#ResourceNotFoundException","Message":"Secrets Manager can't find the specified secret."}"#, Meaning::Missing)]
#[case::an_unknown_failure(
    400,
    r#"{"__type":"SomethingNewException","Message":"new"}"#,
    Meaning::Unrecognized
)]
#[case::no_json(502, "Bad Gateway", Meaning::Unrecognized)]
fn each_known_failure_has_its_meaning(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Meaning,
) {
    assert_eq!(
        meaning(&translate(
            status,
            body.as_bytes(),
            "the engine's environment"
        )),
        expected
    );
}

/// AWS STS's failures of `AssumeRoleWithWebIdentity`, in the XML error
/// shape of its query protocol, by the codes its API reference lists.
#[rstest]
#[case::an_invalid_token("InvalidIdentityToken", 400, Meaning::Refused)]
#[case::an_expired_token("ExpiredTokenException", 400, Meaning::Refused)]
#[case::a_rejected_claim("IDPRejectedClaim", 403, Meaning::Refused)]
#[case::a_denied_role("AccessDenied", 403, Meaning::Refused)]
#[case::a_malformed_policy("MalformedPolicyDocument", 400, Meaning::Refused)]
#[case::a_packed_policy_too_large("PackedPolicyTooLarge", 400, Meaning::Refused)]
#[case::a_disabled_region("RegionDisabledException", 403, Meaning::Refused)]
#[case::an_unreachable_identity_provider("IDPCommunicationError", 400, Meaning::Busy)]
#[case::throttled("Throttling", 400, Meaning::Busy)]
#[case::an_unknown_failure("SomethingNew", 400, Meaning::Unrecognized)]
fn each_known_sts_failure_has_its_meaning(
    #[case] code: &str,
    #[case] status: u16,
    #[case] expected: Meaning,
) {
    let body = format!(
        "<ErrorResponse xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\"><Error><Type>Sender</Type><Code>{code}</Code><Message>m</Message></Error><RequestId>r</RequestId></ErrorResponse>"
    );

    assert_eq!(meaning(&translate_sts(status, body.as_bytes())), expected);
}

/// STS's answer is no XML: an unrecognized failure.
#[test]
fn an_sts_answer_of_no_xml_is_unrecognized() {
    assert_eq!(
        meaning(&translate_sts(502, b"<html><body>Bad Gateway")),
        Meaning::Unrecognized
    );
}

/// A successful `AssumeRoleWithWebIdentity` answer's temporary
/// credentials, its XML's escapes read.
#[test]
fn the_assumed_roles_credentials_are_read_from_the_xml() {
    let credentials = assumed(
        b"<AssumeRoleWithWebIdentityResponse><AssumeRoleWithWebIdentityResult><Credentials><AccessKeyId>ASIAEXAMPLE</AccessKeyId><SecretAccessKey>a&amp;b&#47;c</SecretAccessKey><SessionToken>t&#x2B;k</SessionToken><Expiration>2026-11-25T18:00:00Z</Expiration></Credentials></AssumeRoleWithWebIdentityResult></AssumeRoleWithWebIdentityResponse>",
    )
    .unwrap();

    assert_eq!(
        (
            credentials.access_key_id.as_str(),
            credentials.secret_access_key.expose_secret(),
            credentials
                .session_token
                .as_ref()
                .map(ExposeSecret::expose_secret),
        ),
        ("ASIAEXAMPLE", "a&b/c", Some("t+k"))
    );
}

/// A `SecretString` is the value; with `#key`, its JSON object's `key`, a
/// text as it is and any other value as its JSON text.
#[rstest]
#[case::the_whole_text(
    r#"{"Name":"prod/shop","SecretString":"s3cret-text"}"#,
    None,
    Ok("s3cret-text")
)]
#[case::a_key(
    r#"{"SecretString":"{\"password\":\"pw-value\",\"port\":5432}"}"#,
    Some("password"),
    Ok("pw-value")
)]
#[case::a_number_key(
    r#"{"SecretString":"{\"password\":\"pw-value\",\"port\":5432}"}"#,
    Some("port"),
    Ok("5432")
)]
#[case::an_absent_key(
    r#"{"SecretString":"{\"password\":\"pw-value\"}"}"#,
    Some("pin"),
    Err(Meaning::Missing)
)]
#[case::a_key_of_plain_text(r#"{"SecretString":"plain"}"#, Some("pin"), Err(Meaning::Refused))]
#[case::binary_only(r#"{"SecretBinary":"AAEC"}"#, None, Err(Meaning::Refused))]
fn the_value_is_the_secret_text_or_its_key(
    #[case] body: &str,
    #[case] key: Option<&str>,
    #[case] expected: Result<&str, Meaning>,
) {
    let value = value_of(body.as_bytes(), key);
    assert_eq!(
        value
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .map_err(meaning),
        expected
    );
}
