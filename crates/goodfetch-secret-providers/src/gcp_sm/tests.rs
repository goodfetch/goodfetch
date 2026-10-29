//! GCP Secret Manager's translation table (ADR 0127): one named case per
//! status the API documents for `AccessSecretVersion`, in the Google API
//! error shape, and one for a failure it is not; and the payload a
//! successful access holds.

use goodfetch_engine::secrets::ProviderFailure;
use rstest::rstest;
use secrecy::ExposeSecret;

use super::{payload_of, translate};

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
#[case::a_missing_secret(404, r#"{"error":{"code":404,"message":"Secret [projects/123/secrets/portal] not found or has no versions.","status":"NOT_FOUND"}}"#, Meaning::Missing)]
#[case::a_denied_access(403, r#"{"error":{"code":403,"message":"Permission 'secretmanager.versions.access' denied for resource 'projects/shop/secrets/portal/versions/latest' (or it may not exist).","status":"PERMISSION_DENIED"}}"#, Meaning::Refused)]
#[case::a_refused_token(401, r#"{"error":{"code":401,"message":"Request had invalid authentication credentials. Expected OAuth 2 access token, login cookie or other valid authentication credential.","status":"UNAUTHENTICATED"}}"#, Meaning::Refused)]
#[case::a_disabled_version(400, r#"{"error":{"code":400,"message":"Secret Version [projects/123/secrets/portal/versions/1] is in DISABLED state.","status":"FAILED_PRECONDITION"}}"#, Meaning::Refused)]
#[case::an_invalid_name(400, r#"{"error":{"code":400,"message":"The provided Secret Version ID [first] does not match the expected format [[0-9]+|latest]","status":"INVALID_ARGUMENT"}}"#, Meaning::Refused)]
#[case::throttled(429, r#"{"error":{"code":429,"message":"Quota exceeded for quota metric 'Access requests'.","status":"RESOURCE_EXHAUSTED"}}"#, Meaning::Busy)]
#[case::unavailable(503, r#"{"error":{"code":503,"message":"The service is currently unavailable.","status":"UNAVAILABLE"}}"#, Meaning::Busy)]
#[case::an_unknown_failure(
    409,
    r#"{"error":{"code":409,"message":"new","status":"ABORTED"}}"#,
    Meaning::Unrecognized
)]
#[case::no_json(502, "<html>Bad Gateway</html>", Meaning::Unrecognized)]
fn each_known_failure_has_its_meaning(
    #[case] status: u16,
    #[case] body: &str,
    #[case] expected: Meaning,
) {
    assert_eq!(meaning(&translate(status, body.as_bytes())), expected);
}

/// The payload's Base64 data is the value, as UTF-8 text.
#[rstest]
#[case::text(r#"{"name":"projects/123/secrets/portal/versions/1","payload":{"data":"czNjcmV0LXRleHQ=","dataCrc32c":"1"}}"#, Ok("s3cret-text"))]
#[case::no_payload(
    r#"{"name":"projects/123/secrets/portal/versions/1"}"#,
    Err(Meaning::Missing)
)]
#[case::binary(r#"{"payload":{"data":"/w=="}}"#, Err(Meaning::Refused))]
#[case::no_base64(r#"{"payload":{"data":"%%"}}"#, Err(Meaning::Unrecognized))]
fn the_value_is_the_payload_as_text(#[case] body: &str, #[case] expected: Result<&str, Meaning>) {
    let value = payload_of(body.as_bytes());
    assert_eq!(
        value
            .as_ref()
            .map(ExposeSecret::expose_secret)
            .map_err(meaning),
        expected
    );
}
