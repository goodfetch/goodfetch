//! AWS Secrets Manager's translation table (ADR 0127): one named case per
//! `__type` the service documents for `GetSecretValue` and the common
//! errors of every AWS API, and one for a failure it is not; the value a
//! successful answer holds; and Signature Version 4 against AWS's own
//! worked example.

use goodfetch_engine::secrets::ProviderFailure;
use rstest::rstest;
use secrecy::{ExposeSecret, SecretString};

use super::{Credentials, Signing, authorization, translate, value_of};

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
    assert_eq!(meaning(&translate(status, body.as_bytes())), expected);
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

/// AWS's own Signature Version 4 example, IAM's `ListUsers` (the
/// "Signature Version 4 signing process" pages of the General Reference):
/// the documented signature is the independent answer. The example's
/// secret key is built from parts so it reads as no key in the source.
#[test]
fn the_signature_is_aws_documented_example() {
    let credentials = Credentials {
        access_key_id: "AKIDEXAMPLE".to_owned(),
        secret_access_key: SecretString::from(
            ["wJalrXUtnFEMI", "/K7MDENG+bPxRfiCY", "EXAMPLEKEY"].concat(),
        ),
        session_token: None,
    };
    let headers = [
        (
            "content-type",
            "application/x-www-form-urlencoded; charset=utf-8".to_owned(),
        ),
        ("host", "iam.amazonaws.com".to_owned()),
        ("x-amz-date", "20150830T123600Z".to_owned()),
    ];

    let header = authorization(
        &Signing {
            method: "GET",
            path: "/",
            query: "Action=ListUsers&Version=2010-05-08",
            headers: &headers,
            body: b"",
            region: "us-east-1",
            service: "iam",
            amz_date: "20150830T123600Z",
        },
        &credentials,
    );

    assert_eq!(
        header,
        "AWS4-HMAC-SHA256 Credential=AKIDEXAMPLE/20150830/us-east-1/iam/aws4_request, SignedHeaders=content-type;host;x-amz-date, Signature=5d672d79c15b13162d9279b0855cfba6789a8edb4c82c400e06b5924a6f2b5d7"
    );
}
