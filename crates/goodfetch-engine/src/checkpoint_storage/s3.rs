//! S3 and S3-compatible object-storage primitives (AWS `SigV4`, single-PUT and
//! multipart upload with abort-on-error, GET, and XML error translation).

use std::time::SystemTime;

use hyper::body::Bytes;
use hyper::{Method, Request};
use url::Url;

use crate::aws_signing::{Credentials, Scope, Unsignable, signed_request};

use super::url::{ObjectLocation, encode_component, encode_path};
use super::{StoreError, StoreKind};

/// S3 configuration resolved from `checkpoint.object_storage.s3.*`.
#[derive(Clone, Debug)]
pub(super) struct S3Config {
    pub(super) endpoint: Option<String>,
    pub(super) region: String,
    pub(super) credentials: Option<Credentials>,
}

impl S3Config {
    /// Builds the HTTP URL for `object`, using path-style addressing when a
    /// custom endpoint is configured (for `RustFS`, R2, `MinIO`, etc.) and
    /// standard regional endpoint otherwise.
    pub(super) fn object_url(
        &self,
        object: &ObjectLocation,
        query: Option<&str>,
    ) -> Result<Url, StoreError> {
        let base = match &self.endpoint {
            Some(endpoint) => endpoint.trim_end_matches('/').to_owned(),
            None => format!("https://s3.{}.amazonaws.com", self.region),
        };
        let encoded_bucket = encode_component(&object.bucket);
        let encoded_key = encode_path(&object.key);
        let raw = match query {
            Some(q) => format!("{base}/{encoded_bucket}/{encoded_key}?{q}"),
            None => format!("{base}/{encoded_bucket}/{encoded_key}"),
        };
        Url::parse(&raw).map_err(|error| StoreError::InvalidConfig(format!("S3 URL: {error}")))
    }

    pub(super) fn sign(
        &self,
        method: Method,
        url: &Url,
        headers: &[(&str, &str)],
        body: Bytes,
        now: SystemTime,
    ) -> Result<Request<Bytes>, StoreError> {
        if let Some(credentials) = &self.credentials {
            signed_request(
                method,
                url,
                headers,
                body,
                Scope {
                    region: &self.region,
                    service: "s3",
                },
                credentials,
                now,
            )
            .map_err(|error| match error {
                Unsignable::AccessKeyId | Unsignable::SessionToken => {
                    StoreError::InvalidConfig(error.to_string())
                }
            })
        } else {
            let mut builder = Request::builder().method(method).uri(url.as_str());
            for (name, value) in headers {
                builder = builder.header(*name, *value);
            }
            builder
                .body(body)
                .map_err(|error| StoreError::InvalidConfig(error.to_string()))
        }
    }
}

/// Extracts `<tag>value</tag>` from an S3 XML response body.
pub(super) fn xml_tag(body: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let start = body.find(&open)? + open.len();
    let end = body[start..].find(&close)?;
    Some(body[start..start + end].trim().to_owned())
}

/// Translates an S3 HTTP error response at one translation point (ADR 0127).
pub(crate) fn translate_error(status: u16, body: &str) -> StoreError {
    let code = xml_tag(body, "Code");
    match (status, code.as_deref()) {
        (_, Some("NoSuchKey")) | (404, None) => StoreError::NotFound {
            store: StoreKind::S3,
        },
        (_, Some("NoSuchBucket")) => StoreError::BucketNotFound {
            store: StoreKind::S3,
        },
        (
            _,
            Some(
                "InvalidAccessKeyId"
                | "SignatureDoesNotMatch"
                | "ExpiredToken"
                | "InvalidToken"
                | "AuthFailure",
            ),
        )
        | (401, _) => StoreError::InvalidCredentials {
            store: StoreKind::S3,
        },
        (_, Some("AccessDenied" | "AccountProblem")) | (403, None) => StoreError::AccessDenied {
            store: StoreKind::S3,
        },
        (_, Some("SlowDown" | "Throttling" | "ThrottlingException" | "RequestLimitExceeded"))
        | (429, _) => StoreError::Throttled {
            store: StoreKind::S3,
        },
        (
            _,
            Some("InternalError" | "ServiceUnavailable" | "RequestTimeout" | "OperationAborted"),
        )
        | (500 | 502 | 503 | 504, None) => StoreError::Transient {
            store: StoreKind::S3,
            status,
        },
        _ => StoreError::Unrecognized {
            store: StoreKind::S3,
            status,
            code,
        },
    }
}

/// Builds the `<CompleteMultipartUpload>` XML body for `(part_number, etag)`.
pub(super) fn complete_multipart_xml(parts: &[(u32, String)]) -> Bytes {
    let mut xml = String::from("<CompleteMultipartUpload>");
    for (number, etag) in parts {
        use std::fmt::Write as _;
        let _ = write!(
            xml,
            "<Part><PartNumber>{number}</PartNumber><ETag>{etag}</ETag></Part>"
        );
    }
    xml.push_str("</CompleteMultipartUpload>");
    Bytes::from(xml)
}
