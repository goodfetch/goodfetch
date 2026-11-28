//! Google Cloud Storage JSON API primitives (media upload, media download,
//! and JSON error translation).

use hyper::body::Bytes;
use hyper::header::HeaderValue;
use hyper::{Method, Request};
use secrecy::{ExposeSecret as _, SecretString};
use serde::Deserialize;
use url::Url;

use super::url::{ObjectLocation, encode_component};
use super::{StoreError, StoreKind};

const DEFAULT_GCS_ENDPOINT: &str = "https://storage.googleapis.com";

/// GCS configuration resolved from `checkpoint.object_storage.gs.*`.
#[derive(Clone)]
pub(super) struct GcsConfig {
    pub(super) endpoint: Option<String>,
    pub(super) access_token: Option<SecretString>,
}

impl std::fmt::Debug for GcsConfig {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GcsConfig")
            .field("endpoint", &self.endpoint)
            .finish_non_exhaustive()
    }
}

impl GcsConfig {
    fn base(&self) -> &str {
        self.endpoint
            .as_deref()
            .unwrap_or(DEFAULT_GCS_ENDPOINT)
            .trim_end_matches('/')
    }

    pub(super) fn upload_url(&self, object: &ObjectLocation) -> Result<Url, StoreError> {
        let bucket = encode_component(&object.bucket);
        let name = encode_component(&object.key);
        let raw = format!(
            "{}/upload/storage/v1/b/{bucket}/o?uploadType=media&name={name}",
            self.base()
        );
        Url::parse(&raw).map_err(|error| StoreError::InvalidConfig(format!("GCS URL: {error}")))
    }

    pub(super) fn download_url(&self, object: &ObjectLocation) -> Result<Url, StoreError> {
        let bucket = encode_component(&object.bucket);
        let name = encode_component(&object.key);
        let raw = format!("{}/storage/v1/b/{bucket}/o/{name}?alt=media", self.base());
        Url::parse(&raw).map_err(|error| StoreError::InvalidConfig(format!("GCS URL: {error}")))
    }

    pub(super) fn request(
        &self,
        method: Method,
        url: &Url,
        content_type: Option<&str>,
        body: Bytes,
    ) -> Result<Request<Bytes>, StoreError> {
        let mut builder = Request::builder().method(method).uri(url.as_str());
        if let Some(ct) = content_type {
            builder = builder.header("content-type", ct);
        }
        if let Some(token) = &self.access_token {
            let mut value = HeaderValue::from_str(&format!("Bearer {}", token.expose_secret()))
                .map_err(|_| {
                    StoreError::InvalidConfig(
                        "the GCS access token holds characters no HTTP header may carry".to_owned(),
                    )
                })?;
            value.set_sensitive(true);
            builder = builder.header("authorization", value);
        }
        builder
            .body(body)
            .map_err(|error| StoreError::InvalidConfig(error.to_string()))
    }
}

#[derive(Deserialize)]
struct GcsErrorEnvelope {
    error: Option<GcsErrorBody>,
}

#[derive(Deserialize)]
struct GcsErrorBody {
    errors: Option<Vec<GcsErrorItem>>,
    status: Option<String>,
}

#[derive(Deserialize)]
struct GcsErrorItem {
    reason: Option<String>,
}

/// Translates a GCS JSON API error response at one translation point (ADR 0127).
pub(crate) fn translate_error(status: u16, body: &str) -> StoreError {
    let parsed = serde_json::from_str::<GcsErrorEnvelope>(body)
        .ok()
        .and_then(|env| env.error);
    let reason = parsed
        .as_ref()
        .and_then(|err| err.errors.as_ref())
        .and_then(|items| items.first())
        .and_then(|item| item.reason.clone())
        .or_else(|| parsed.and_then(|err| err.status));

    match (status, reason.as_deref()) {
        (_, Some("notFound" | "NOT_FOUND")) | (404, None) => StoreError::NotFound {
            store: StoreKind::Gcs,
        },
        (_, Some("authError" | "UNAUTHENTICATED")) | (401, _) => StoreError::InvalidCredentials {
            store: StoreKind::Gcs,
        },
        (_, Some("forbidden" | "PERMISSION_DENIED" | "insufficientPermissions")) | (403, None) => {
            StoreError::AccessDenied {
                store: StoreKind::Gcs,
            }
        }
        (
            _,
            Some(
                "rateLimitExceeded"
                | "userRateLimitExceeded"
                | "quotaExceeded"
                | "RESOURCE_EXHAUSTED",
            ),
        )
        | (429, _) => StoreError::Throttled {
            store: StoreKind::Gcs,
        },
        (_, Some("backendError" | "internalError" | "UNAVAILABLE" | "INTERNAL"))
        | (500 | 502 | 503 | 504, None) => StoreError::Transient {
            store: StoreKind::Gcs,
            status,
        },
        _ => StoreError::Unrecognized {
            store: StoreKind::Gcs,
            status,
            code: reason,
        },
    }
}
