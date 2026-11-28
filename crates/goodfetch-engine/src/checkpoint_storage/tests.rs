//! Unit tests for Checkpoint object storage: URL parsing, per-store error
//! translation (every known failure answer + `Unrecognized`, ADR 0127), and
//! request signing.

use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use hyper::Method;
use hyper::body::Bytes;
use rstest::rstest;
use secrecy::SecretString;

use super::azure::{self, AzureAuth, AzureConfig};
use super::gcs;
use super::s3;
use super::url::{Location, LocationError, ObjectLocation, Scheme};
use super::{StoreError, StoreKind};

#[test]
fn location_parses_local_and_object_urls() {
    assert_eq!(
        Location::parse("/tmp/run/checkpoint.sqlite"),
        Ok(Location::Local(PathBuf::from("/tmp/run/checkpoint.sqlite")))
    );
    assert_eq!(
        Location::parse("C:\\runs\\cp.sqlite"),
        Ok(Location::Local(PathBuf::from("C:\\runs\\cp.sqlite")))
    );
    assert_eq!(
        Location::parse("s3://my-bucket/runs/r1/cp.sqlite"),
        Ok(Location::Object(ObjectLocation {
            scheme: Scheme::S3,
            bucket: "my-bucket".to_owned(),
            key: "runs/r1/cp.sqlite".to_owned(),
            raw: "s3://my-bucket/runs/r1/cp.sqlite".to_owned(),
        }))
    );
    assert_eq!(
        Location::parse("gs://gcs-bucket/nested/cp.sqlite"),
        Ok(Location::Object(ObjectLocation {
            scheme: Scheme::Gs,
            bucket: "gcs-bucket".to_owned(),
            key: "nested/cp.sqlite".to_owned(),
            raw: "gs://gcs-bucket/nested/cp.sqlite".to_owned(),
        }))
    );
    assert_eq!(
        Location::parse("az://checkpoints/r1.sqlite"),
        Ok(Location::Object(ObjectLocation {
            scheme: Scheme::Az,
            bucket: "checkpoints".to_owned(),
            key: "r1.sqlite".to_owned(),
            raw: "az://checkpoints/r1.sqlite".to_owned(),
        }))
    );
}

#[rstest]
#[case::s3_missing_bucket("s3://", LocationError::MissingBucket { scheme: "s3", raw: "s3://".to_owned() })]
#[case::s3_empty_bucket("s3:///key", LocationError::MissingBucket { scheme: "s3", raw: "s3:///key".to_owned() })]
#[case::gs_missing_key("gs://bucket", LocationError::MissingKey { scheme: "gs", raw: "gs://bucket".to_owned() })]
#[case::gs_trailing_slash("gs://bucket/", LocationError::MissingKey { scheme: "gs", raw: "gs://bucket/".to_owned() })]
#[case::az_query("az://container/blob?sig=1", LocationError::QueryOrFragment { scheme: "az", raw: "az://container/blob?sig=1".to_owned() })]
#[case::s3_fragment("s3://bucket/key#frag", LocationError::QueryOrFragment { scheme: "s3", raw: "s3://bucket/key#frag".to_owned() })]
fn location_rejects_malformed_object_urls(#[case] raw: &str, #[case] expected: LocationError) {
    assert_eq!(Location::parse(raw), Err(expected));
}

// --- S3 translation point (ADR 0127) ---

#[rstest]
#[case::no_such_key(404, "<Error><Code>NoSuchKey</Code></Error>", "not_found")]
#[case::no_such_bucket(404, "<Error><Code>NoSuchBucket</Code></Error>", "bucket_not_found")]
#[case::invalid_access_key_id(
    403,
    "<Error><Code>InvalidAccessKeyId</Code></Error>",
    "invalid_credentials"
)]
#[case::signature_does_not_match(
    403,
    "<Error><Code>SignatureDoesNotMatch</Code></Error>",
    "invalid_credentials"
)]
#[case::expired_token(400, "<Error><Code>ExpiredToken</Code></Error>", "invalid_credentials")]
#[case::access_denied(403, "<Error><Code>AccessDenied</Code></Error>", "access_denied")]
#[case::slow_down(503, "<Error><Code>SlowDown</Code></Error>", "throttled")]
#[case::too_many_requests(429, "", "throttled")]
#[case::internal_error(500, "<Error><Code>InternalError</Code></Error>", "transient")]
#[case::service_unavailable(503, "<Error><Code>ServiceUnavailable</Code></Error>", "transient")]
#[case::unrecognized(418, "<Error><Code>TeapotError</Code></Error>", "unrecognized")]
fn s3_error_translation(#[case] status: u16, #[case] body: &str, #[case] kind: &str) {
    let err = s3::translate_error(status, body);
    assert_store_error_kind(&err, StoreKind::S3, kind);
}

// --- GCS translation point (ADR 0127) ---

#[rstest]
#[case::not_found(404, r#"{"error":{"errors":[{"reason":"notFound"}]}}"#, "not_found")]
#[case::auth_error(
    401,
    r#"{"error":{"errors":[{"reason":"authError"}]}}"#,
    "invalid_credentials"
)]
#[case::unauthenticated(
    401,
    r#"{"error":{"status":"UNAUTHENTICATED"}}"#,
    "invalid_credentials"
)]
#[case::forbidden(
    403,
    r#"{"error":{"errors":[{"reason":"forbidden"}]}}"#,
    "access_denied"
)]
#[case::permission_denied(403, r#"{"error":{"status":"PERMISSION_DENIED"}}"#, "access_denied")]
#[case::rate_limit(
    429,
    r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}]}}"#,
    "throttled"
)]
#[case::backend_error(
    500,
    r#"{"error":{"errors":[{"reason":"backendError"}]}}"#,
    "transient"
)]
#[case::unavailable(503, r#"{"error":{"status":"UNAVAILABLE"}}"#, "transient")]
#[case::unrecognized(
    418,
    r#"{"error":{"errors":[{"reason":"customGcsReason"}]}}"#,
    "unrecognized"
)]
fn gcs_error_translation(#[case] status: u16, #[case] body: &str, #[case] kind: &str) {
    let err = gcs::translate_error(status, body);
    assert_store_error_kind(&err, StoreKind::Gcs, kind);
}

// --- Azure Blob translation point (ADR 0127) ---

#[rstest]
#[case::blob_not_found(404, "<Error><Code>BlobNotFound</Code></Error>", None, "not_found")]
#[case::container_not_found(
    404,
    "<Error><Code>ContainerNotFound</Code></Error>",
    Some("ContainerNotFound"),
    "bucket_not_found"
)]
#[case::authentication_failed(
    403,
    "<Error><Code>AuthenticationFailed</Code></Error>",
    Some("AuthenticationFailed"),
    "invalid_credentials"
)]
#[case::authorization_failure(
    403,
    "<Error><Code>AuthorizationFailure</Code></Error>",
    Some("AuthorizationFailure"),
    "access_denied"
)]
#[case::server_busy(
    503,
    "<Error><Code>ServerBusy</Code></Error>",
    Some("ServerBusy"),
    "throttled"
)]
#[case::operation_timed_out(
    500,
    "<Error><Code>OperationTimedOut</Code></Error>",
    None,
    "throttled"
)]
#[case::internal_error(500, "<Error><Code>InternalError</Code></Error>", None, "transient")]
#[case::unrecognized(
    418,
    "<Error><Code>UnknownAzureCode</Code></Error>",
    None,
    "unrecognized"
)]
fn azure_error_translation(
    #[case] status: u16,
    #[case] body: &str,
    #[case] header_code: Option<&str>,
    #[case] kind: &str,
) {
    let err = azure::translate_error(status, body, header_code);
    assert_store_error_kind(&err, StoreKind::Azure, kind);
}

fn assert_store_error_kind(err: &StoreError, expected_store: StoreKind, kind: &str) {
    match (kind, err) {
        ("not_found", StoreError::NotFound { store })
        | ("bucket_not_found", StoreError::BucketNotFound { store })
        | ("invalid_credentials", StoreError::InvalidCredentials { store })
        | ("access_denied", StoreError::AccessDenied { store }) => {
            assert_eq!(*store, expected_store);
        }
        ("throttled", StoreError::Throttled { store })
        | ("transient", StoreError::Transient { store, .. }) => {
            assert_eq!(*store, expected_store);
            assert!(err.is_retryable());
        }
        ("unrecognized", StoreError::Unrecognized { store, code, .. }) => {
            assert_eq!(*store, expected_store);
            assert!(code.is_some());
        }
        _ => panic!("expected {kind} for {expected_store:?}, got {err:?}"),
    }
}

#[test]
fn azure_shared_key_marks_authorization_sensitive() {
    let cfg = AzureConfig {
        endpoint: Some("http://127.0.0.1:10000/devstoreaccount1".to_owned()),
        account: Some("devstoreaccount1".to_owned()),
        auth: Some(AzureAuth::SharedKey {
            account: "devstoreaccount1".to_owned(),
            key: SecretString::from(
                "Eby8vdM02xNOcqFlqUwJPLlmEtlCDXJ1OUzFT50uSRZ6IFsuFq2UVErCz4I6tq/K1SZFPTOtr/KBHBeksoGMGw==".to_owned(),
            ),
        }),
    };
    let object = ObjectLocation {
        scheme: Scheme::Az,
        bucket: "checkpoints".to_owned(),
        key: "run-1/cp.sqlite".to_owned(),
        raw: "az://checkpoints/run-1/cp.sqlite".to_owned(),
    };
    let url = cfg.blob_url(&object, &[]).unwrap();
    let req = cfg
        .signed_request(
            &Method::PUT,
            &url,
            &[("x-ms-blob-type", "BlockBlob")],
            Bytes::from_static(b"hello"),
            SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000),
        )
        .unwrap();
    let auth = req
        .headers()
        .get("authorization")
        .expect("authorization header");
    assert!(auth.is_sensitive());
    assert!(
        auth.to_str()
            .unwrap()
            .starts_with("SharedKey devstoreaccount1:")
    );
}
