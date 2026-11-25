//! GCP Secret Manager's translation table (ADR 0127): one named case per
//! status the API documents for `AccessSecretVersion`, in the Google API
//! error shape, and one for a failure it is not; Google's token
//! endpoint's, and the JWT a service account's key file signs for it; and
//! the payload a successful access holds.

use data_encoding::{BASE64, BASE64URL_NOPAD};
use goodfetch_engine::secrets::ProviderFailure;
use rstest::rstest;
use rustls::SignatureScheme;
use secrecy::ExposeSecret;

use super::access_token::{assertion, service_account, translate_token};
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
    assert_eq!(
        meaning(&translate(
            status,
            body.as_bytes(),
            "`GOOGLE_OAUTH_ACCESS_TOKEN`"
        )),
        expected
    );
}

/// Google's token endpoint's failures of a JWT exchange, in the OAuth 2.0
/// error shape, by the codes RFC 6749 and RFC 7523 define.
#[rstest]
#[case::an_invalid_grant("invalid_grant", 400, Meaning::Refused)]
#[case::an_invalid_client("invalid_client", 401, Meaning::Refused)]
#[case::an_unauthorized_client("unauthorized_client", 400, Meaning::Refused)]
#[case::a_denied_access("access_denied", 403, Meaning::Refused)]
#[case::temporarily_unavailable("temporarily_unavailable", 503, Meaning::Busy)]
#[case::an_unknown_failure("invalid_scope", 400, Meaning::Unrecognized)]
fn each_known_token_failure_has_its_meaning(
    #[case] error: &str,
    #[case] status: u16,
    #[case] expected: Meaning,
) {
    let body = format!(r#"{{"error":"{error}","error_description":"d"}}"#);

    assert_eq!(meaning(&translate_token(status, body.as_bytes())), expected);
}

/// The token endpoint's answer is no JSON: an unrecognized failure.
#[test]
fn a_token_answer_of_no_json_is_unrecognized() {
    assert_eq!(
        meaning(&translate_token(502, b"<html>Bad Gateway</html>")),
        Meaning::Unrecognized
    );
}

/// The committed test key's public half, as an `RSAPublicKey` in DER and
/// Base64.
const PUBLIC_KEY: &str = "MIIBCgKCAQEA6CRm9FSThbunB65enqLjtz6e0ZxbYumgnxDZtUCmFyG5Z8RIe7UtGw/tfgDgPAxK8x86g3K/r11SBisIWFFMR7GVEUwBBJgcl42VZx/VVRPROKqkWMzTgDot5OACgFKrHMQ8mHimK0OcdXzSthKqYRjP7Fe+GH8rvGn4H9UZHrxIZgj8PaD8Vs50ANJpn6f8/1sGLgIppaMQfRjlD4phETbB2G/G200936XVxwF/cR3y8gdrOr/J2xU38PFTBzlOJZl4uAf1cc/lop4WKDLCbZOeDYeY1wj6xkpNIho9qmHx4orvVRJ6E6Dtwwps52UG/sG9TJ2W99kFVurlULwxdQIDAQAB";

/// The JWT a key file signs is RFC 7523's: RS256 over its header and
/// claims, which name the account, the scope, the token endpoint and an
/// hour from now.
#[test]
fn the_key_file_signs_an_rs256_jwt_for_its_token_endpoint() {
    let key_file = serde_json::json!({
        "type": "service_account",
        "client_email": "reader@shop.iam.gserviceaccount.com",
        "private_key": include_str!("../../tests/fixtures/service-account-key.pem"),
        "private_key_id": "k1",
        "token_uri": "https://oauth2.googleapis.com/token",
    });
    let account = service_account(&key_file.to_string()).unwrap();

    let jwt = assertion(&account, 1_764_000_000).unwrap();

    let parts = jwt.expose_secret().split('.').collect::<Vec<_>>();
    let [header, claims, signature] = parts[..] else {
        panic!("a JWT has three parts: {}", parts.len());
    };
    let decoded = |part: &str| -> serde_json::Value {
        serde_json::from_slice(&BASE64URL_NOPAD.decode(part.as_bytes()).unwrap()).unwrap()
    };
    let rs256 = rustls::crypto::ring::default_provider()
        .signature_verification_algorithms
        .mapping
        .iter()
        .find(|(scheme, _)| *scheme == SignatureScheme::RSA_PKCS1_SHA256)
        .unwrap()
        .1[0];
    let verified = rs256.verify_signature(
        &BASE64.decode(PUBLIC_KEY.as_bytes()).unwrap(),
        format!("{header}.{claims}").as_bytes(),
        &BASE64URL_NOPAD.decode(signature.as_bytes()).unwrap(),
    );
    assert_eq!(
        (decoded(header), decoded(claims), verified.is_ok()),
        (
            serde_json::json!({ "alg": "RS256", "typ": "JWT", "kid": "k1" }),
            serde_json::json!({
                "iss": "reader@shop.iam.gserviceaccount.com",
                "scope": "https://www.googleapis.com/auth/cloud-platform",
                "aud": "https://oauth2.googleapis.com/token",
                "iat": 1_764_000_000,
                "exp": 1_764_003_600,
            }),
            true
        )
    );
}

/// Only a service account's key file is read: gcloud's user credentials
/// and the other types are named, not taken.
#[rstest]
#[case::a_users_credentials(
    r#"{"type":"authorized_user","client_id":"c","client_secret":"s","refresh_token":"r"}"#,
    "`authorized_user`"
)]
#[case::no_type(r#"{"client_email":"e","private_key":"k"}"#, "no `type`")]
#[case::no_key(r#"{"type":"service_account","client_email":"e"}"#, "`private_key`")]
#[case::no_json("not json", "no JSON")]
fn only_a_service_accounts_key_file_is_read(#[case] file: &str, #[case] named: &str) {
    let problem = service_account(file).err().unwrap();

    assert!(problem.contains(named), "{problem}");
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
