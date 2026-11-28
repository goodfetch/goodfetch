//! Object-storage client for Run Checkpoints (`s3://`, `gs://`, `az://`,
//! issue 3472): one shared driver over three sets of store primitives
//! (ADR 0129).
//!
//! Every HTTP request goes through the engine's own [`http_client`] under
//! [`ConfiguredKind::CheckpointStorage`] (ADRs 0017 and 0043). Credentials
//! are taken exclusively as [`SecretReference`] settings, resolved through
//! [`RunSecrets`] before any request is sent, and never appear in logs,
//! errors, Checkpoint reports, or Run traces (ADR 0067). Uploads are whole
//! or not at all; retries honour ADR 0118's `Retry-After`.

mod azure;
mod gcs;
mod s3;
#[cfg(test)]
mod tests;
pub mod url;

use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::checkpoint::{
    CHECKPOINT_OBJECT_STORAGE_AZ_ACCOUNT, CHECKPOINT_OBJECT_STORAGE_AZ_ACCOUNT_KEY,
    CHECKPOINT_OBJECT_STORAGE_AZ_ENDPOINT, CHECKPOINT_OBJECT_STORAGE_AZ_SAS_TOKEN,
    CHECKPOINT_OBJECT_STORAGE_GS_ACCESS_TOKEN, CHECKPOINT_OBJECT_STORAGE_GS_ENDPOINT,
    CHECKPOINT_OBJECT_STORAGE_S3_ACCESS_KEY_ID, CHECKPOINT_OBJECT_STORAGE_S3_ENDPOINT,
    CHECKPOINT_OBJECT_STORAGE_S3_REGION, CHECKPOINT_OBJECT_STORAGE_S3_SECRET_ACCESS_KEY,
    CHECKPOINT_OBJECT_STORAGE_S3_SESSION_TOKEN,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use hyper::body::Bytes;
use hyper::{Method, Request};
use secrecy::{ExposeSecret as _, SecretString};

use crate::aws_signing::Credentials as AwsCredentials;
use crate::clock::Clock;
use crate::durable;
use crate::egress::{ConfiguredKind, Refused};
use crate::http_client::{EndpointSettings, WholeFailure};
use crate::retry_after;
use crate::secrets::{Purpose, RunSecrets};
use crate::settings::Resolved;
use crate::tasks::BlockingThreads;
use crate::tls::{CertificateInvalid, ClientCertificateFailure};

pub use self::url::{Location, LocationError, ObjectLocation, Scheme};

/// Default S3 signing region when `checkpoint.object_storage.s3.region` is unset.
const DEFAULT_S3_REGION: &str = "us-east-1";

/// Maximum Checkpoint response size downloaded in one request (512 MiB).
const MAX_RESPONSE_BYTES: usize = 512 * 1024 * 1024;

/// Default single-PUT threshold for S3 (5 MiB): payloads above this use
/// multipart upload with abort on failure.
const S3_MULTIPART_THRESHOLD: usize = 5 * 1024 * 1024;

/// Default single-PUT threshold for Azure Block Blobs (4 MiB): payloads above
/// this use `PutBlock` + `PutBlockList`.
const AZURE_BLOCK_THRESHOLD: usize = 4 * 1024 * 1024;

/// Maximum retry attempts on retryable responses (`Throttled`, `Transient`, `Network`).
const MAX_ATTEMPTS: usize = 4;

/// Base backoff when no `Retry-After` header is present.
const BASE_BACKOFF: Duration = Duration::from_millis(100);

/// Cap on `Retry-After` / backoff waits for one attempt.
const MAX_BACKOFF: Duration = Duration::from_secs(5);

/// Per-request timeout.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(60);

/// Which object store produced a failure.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StoreKind {
    /// S3 / S3-compatible.
    S3,
    /// Google Cloud Storage.
    Gcs,
    /// Azure Blob Storage.
    Azure,
}

impl StoreKind {
    /// Human-readable store name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::S3 => "S3",
            Self::Gcs => "GCS",
            Self::Azure => "Azure Blob",
        }
    }
}

/// Typed failure of an object-storage Checkpoint operation (ADR 0127).
///
/// Never carries raw credential values or authorization headers.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    /// Invalid object-storage URL.
    #[error("{0}")]
    InvalidLocation(#[from] LocationError),
    /// Invalid or incomplete object-storage configuration.
    #[error("invalid Checkpoint object-storage configuration: {0}")]
    InvalidConfig(String),
    /// A credential Secret reference failed to resolve before any request was sent.
    #[error("failed to resolve Checkpoint credential `{setting}`: {reason}")]
    CredentialUnavailable {
        /// Setting key naming the Secret reference.
        setting: &'static str,
        /// Masked reason from the Secret provider.
        reason: String,
    },
    /// Egress policy refused the endpoint before connecting.
    #[error("{0}")]
    Refused(Refused),
    /// TLS certificate verification failed.
    #[error("{0}")]
    CertificateInvalid(CertificateInvalid),
    /// Client certificate presentation failed.
    #[error("{0}")]
    ClientCertificate(ClientCertificateFailure),
    /// Network or timeout failure.
    #[error("object-storage request failed: {0}")]
    Network(String),
    /// Object not found (404 / `NoSuchKey` / `BlobNotFound`).
    #[error("{store} object not found", store = store.as_str())]
    NotFound {
        /// Store kind.
        store: StoreKind,
    },
    /// Bucket or container not found (`NoSuchBucket` / `ContainerNotFound`).
    #[error("{store} bucket or container not found", store = store.as_str())]
    BucketNotFound {
        /// Store kind.
        store: StoreKind,
    },
    /// Invalid or expired credentials (401 / `InvalidAccessKeyId` / `AuthenticationFailed`).
    #[error("{store} rejected the credentials", store = store.as_str())]
    InvalidCredentials {
        /// Store kind.
        store: StoreKind,
    },
    /// Permission denied (403 / `AccessDenied` / `AuthorizationFailure`).
    #[error("{store} denied access", store = store.as_str())]
    AccessDenied {
        /// Store kind.
        store: StoreKind,
    },
    /// Rate limited / throttled (429 / `SlowDown` / `ServerBusy`).
    #[error("{store} throttled the request", store = store.as_str())]
    Throttled {
        /// Store kind.
        store: StoreKind,
    },
    /// Transient 5xx error.
    #[error("{store} returned a transient error (HTTP {status})", store = store.as_str())]
    Transient {
        /// Store kind.
        store: StoreKind,
        /// HTTP status code.
        status: u16,
    },
    /// Unrecognized HTTP error response (ADR 0127).
    #[error(
        "{store} returned an unrecognized error (HTTP {status}{})",
        code.as_deref().map_or_else(String::new, |c| format!(", code `{c}`")),
        store = store.as_str()
    )]
    Unrecognized {
        /// Store kind.
        store: StoreKind,
        /// HTTP status code.
        status: u16,
        /// Provider error code, if parsed.
        code: Option<String>,
    },
    /// Local temporary file I/O failure while staging upload or download.
    #[error("local staging I/O failed: {0}")]
    LocalIo(String),
}

impl StoreError {
    /// Whether this failure is retryable by the shared driver.
    #[must_use]
    pub const fn is_retryable(&self) -> bool {
        matches!(
            self,
            Self::Throttled { .. } | Self::Transient { .. } | Self::Network(_)
        )
    }

    /// The `file_write_failed` reason string for this error.
    #[must_use]
    pub const fn write_reason(&self) -> &'static str {
        match self {
            Self::CredentialUnavailable { .. }
            | Self::InvalidCredentials { .. }
            | Self::AccessDenied { .. }
            | Self::Refused(_) => "permission_denied",
            Self::NotFound { .. } | Self::BucketNotFound { .. } => "not_found",
            Self::InvalidLocation(_) | Self::InvalidConfig(_) => "invalid_input",
            Self::Throttled { .. }
            | Self::Transient { .. }
            | Self::Network(_)
            | Self::CertificateInvalid(_)
            | Self::ClientCertificate(_)
            | Self::Unrecognized { .. }
            | Self::LocalIo(_) => "other",
        }
    }
}

/// Resolved store configuration, with all credentials resolved up front.
#[derive(Clone, Debug)]
enum StoreConfig {
    S3(s3::S3Config),
    Gcs(gcs::GcsConfig),
    Azure(azure::AzureConfig),
}

/// Shared object-storage driver for Checkpoints (ADR 0129).
pub struct Driver {
    endpoint: EndpointSettings,
    clock: Clock,
    blocking: BlockingThreads,
    config: StoreConfig,
    s3_part_size: usize,
    azure_block_size: usize,
}

impl Driver {
    /// Resolves settings and Secret references for `scheme` BEFORE any network
    /// request is sent.
    ///
    /// # Errors
    ///
    /// [`StoreError::CredentialUnavailable`] when any configured Secret
    /// reference fails to resolve, or [`StoreError::InvalidConfig`] when a
    /// Secret reference setting is malformed or incomplete.
    pub async fn from_settings(
        scheme: Scheme,
        settings: &Resolved,
        secrets: &RunSecrets,
        run_id: Option<&RunId>,
        clock: Clock,
        blocking: BlockingThreads,
    ) -> Result<Self, StoreError> {
        let config = resolve_config(scheme, settings, secrets, run_id).await?;
        let endpoint = EndpointSettings::of(settings, clock, blocking.clone());
        Ok(Self {
            endpoint,
            clock,
            blocking,
            config,
            s3_part_size: S3_MULTIPART_THRESHOLD,
            azure_block_size: AZURE_BLOCK_THRESHOLD,
        })
    }

    /// Uploads the local file at `local_path` to `object`, whole or not at all.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on read, egress, auth, or store failure. On any failure,
    /// no object is left at `object`.
    pub async fn upload(
        &self,
        local_path: &Path,
        object: &ObjectLocation,
    ) -> Result<(), StoreError> {
        let path = local_path.to_owned();
        #[expect(
            clippy::disallowed_methods,
            reason = "reading the staged Checkpoint file runs on the owner's blocking threads"
        )]
        let bytes = self
            .blocking
            .run(move || std::fs::read(&path))
            .await
            .map_err(|error| StoreError::LocalIo(error.to_string()))?;
        self.upload_bytes(Bytes::from(bytes), object).await
    }

    /// Uploads `bytes` to `object`, whole or not at all.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on egress, auth, or store failure.
    pub async fn upload_bytes(
        &self,
        bytes: Bytes,
        object: &ObjectLocation,
    ) -> Result<(), StoreError> {
        match &self.config {
            StoreConfig::S3(cfg) => self.upload_s3(cfg, object, bytes).await,
            StoreConfig::Gcs(cfg) => self.upload_gcs(cfg, object, bytes).await,
            StoreConfig::Azure(cfg) => self.upload_azure(cfg, object, bytes).await,
        }
    }

    /// Downloads `object` into a temporary file beside `destination`, syncs it,
    /// and atomically renames it to `destination`.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on egress, auth, store, or local I/O failure.
    pub async fn download(
        &self,
        object: &ObjectLocation,
        destination: &Path,
        nonce: u64,
    ) -> Result<(), StoreError> {
        let bytes = self.download_bytes(object).await?;
        let dest = destination.to_owned();
        self.blocking
            .run(move || write_atomic(&dest, &bytes, nonce))
            .await
            .map_err(|error| StoreError::LocalIo(error.to_string()))
    }

    /// Downloads the full bytes of `object`.
    ///
    /// # Errors
    ///
    /// [`StoreError`] on egress, auth, or store failure.
    pub async fn download_bytes(&self, object: &ObjectLocation) -> Result<Bytes, StoreError> {
        match &self.config {
            StoreConfig::S3(cfg) => {
                let url = cfg.object_url(object, None)?;
                self.send_with_retry(
                    StoreKind::S3,
                    || cfg.sign(Method::GET, &url, &[], Bytes::new(), self.clock.wall()),
                    |status, body, _| s3::translate_error(status, body),
                )
                .await
                .map(|(_, body)| body)
            }
            StoreConfig::Gcs(cfg) => {
                let url = cfg.download_url(object)?;
                self.send_with_retry(
                    StoreKind::Gcs,
                    || cfg.request(Method::GET, &url, None, Bytes::new()),
                    |status, body, _| gcs::translate_error(status, body),
                )
                .await
                .map(|(_, body)| body)
            }
            StoreConfig::Azure(cfg) => {
                let url = cfg.blob_url(object, &[])?;
                self.send_with_retry(
                    StoreKind::Azure,
                    || cfg.signed_request(&Method::GET, &url, &[], Bytes::new(), self.clock.wall()),
                    azure::translate_error,
                )
                .await
                .map(|(_, body)| body)
            }
        }
    }

    async fn upload_s3(
        &self,
        cfg: &s3::S3Config,
        object: &ObjectLocation,
        bytes: Bytes,
    ) -> Result<(), StoreError> {
        if bytes.len() <= self.s3_part_size {
            let url = cfg.object_url(object, None)?;
            self.send_with_retry(
                StoreKind::S3,
                || {
                    cfg.sign(
                        Method::PUT,
                        &url,
                        &[("content-type", "application/octet-stream")],
                        bytes.clone(),
                        self.clock.wall(),
                    )
                },
                |status, body, _| s3::translate_error(status, body),
            )
            .await?;
            return Ok(());
        }

        // Multipart upload: Initiate -> UploadPart* -> Complete, Abort on any error.
        let init_url = cfg.object_url(object, Some("uploads="))?;
        let (_, init_body) = self
            .send_with_retry(
                StoreKind::S3,
                || {
                    cfg.sign(
                        Method::POST,
                        &init_url,
                        &[("content-type", "application/octet-stream")],
                        Bytes::new(),
                        self.clock.wall(),
                    )
                },
                |status, body, _| s3::translate_error(status, body),
            )
            .await?;
        let init_text = String::from_utf8_lossy(&init_body);
        let upload_id =
            s3::xml_tag(&init_text, "UploadId").ok_or_else(|| StoreError::Unrecognized {
                store: StoreKind::S3,
                status: 200,
                code: Some("MissingUploadId".to_owned()),
            })?;

        let result = self.upload_s3_parts(cfg, object, &upload_id, &bytes).await;
        if result.is_err() {
            // Best-effort abort so no partial multipart upload remains.
            let abort_query = format!("uploadId={}", url::encode_component(&upload_id));
            if let Ok(abort_url) = cfg.object_url(object, Some(&abort_query)) {
                let _ = self
                    .send_once(
                        || {
                            cfg.sign(
                                Method::DELETE,
                                &abort_url,
                                &[],
                                Bytes::new(),
                                self.clock.wall(),
                            )
                        },
                        |status, body, _| s3::translate_error(status, body),
                    )
                    .await;
            }
        }
        result
    }

    async fn upload_s3_parts(
        &self,
        cfg: &s3::S3Config,
        object: &ObjectLocation,
        upload_id: &str,
        bytes: &Bytes,
    ) -> Result<(), StoreError> {
        let mut parts = Vec::new();
        let encoded_id = url::encode_component(upload_id);
        for (idx, chunk) in bytes.chunks(self.s3_part_size).enumerate() {
            let part_number = u32::try_from(idx + 1).map_err(|_| {
                StoreError::InvalidConfig("Checkpoint exceeds S3 part count limit".to_owned())
            })?;
            let query = format!("partNumber={part_number}&uploadId={encoded_id}");
            let part_url = cfg.object_url(object, Some(&query))?;
            let part_bytes = bytes.slice_ref(chunk);
            let (headers, _) = self
                .send_with_retry(
                    StoreKind::S3,
                    || {
                        cfg.sign(
                            Method::PUT,
                            &part_url,
                            &[],
                            part_bytes.clone(),
                            self.clock.wall(),
                        )
                    },
                    |status, body, _| s3::translate_error(status, body),
                )
                .await?;
            let etag = headers
                .get(hyper::header::ETAG)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| StoreError::Unrecognized {
                    store: StoreKind::S3,
                    status: 200,
                    code: Some("MissingETag".to_owned()),
                })?
                .to_owned();
            parts.push((part_number, etag));
        }

        let complete_query = format!("uploadId={encoded_id}");
        let complete_url = cfg.object_url(object, Some(&complete_query))?;
        let body = s3::complete_multipart_xml(&parts);
        self.send_with_retry(
            StoreKind::S3,
            || {
                cfg.sign(
                    Method::POST,
                    &complete_url,
                    &[("content-type", "application/xml")],
                    body.clone(),
                    self.clock.wall(),
                )
            },
            |status, body, _| s3::translate_error(status, body),
        )
        .await?;
        Ok(())
    }

    async fn upload_gcs(
        &self,
        cfg: &gcs::GcsConfig,
        object: &ObjectLocation,
        bytes: Bytes,
    ) -> Result<(), StoreError> {
        let url = cfg.upload_url(object)?;
        self.send_with_retry(
            StoreKind::Gcs,
            || {
                cfg.request(
                    Method::POST,
                    &url,
                    Some("application/octet-stream"),
                    bytes.clone(),
                )
            },
            |status, body, _| gcs::translate_error(status, body),
        )
        .await?;
        Ok(())
    }

    async fn upload_azure(
        &self,
        cfg: &azure::AzureConfig,
        object: &ObjectLocation,
        bytes: Bytes,
    ) -> Result<(), StoreError> {
        if bytes.len() <= self.azure_block_size {
            let url = cfg.blob_url(object, &[])?;
            self.send_with_retry(
                StoreKind::Azure,
                || {
                    cfg.signed_request(
                        &Method::PUT,
                        &url,
                        &[
                            ("x-ms-blob-type", "BlockBlob"),
                            ("content-type", "application/octet-stream"),
                        ],
                        bytes.clone(),
                        self.clock.wall(),
                    )
                },
                azure::translate_error,
            )
            .await?;
            return Ok(());
        }

        // Stage blocks then commit with PutBlockList (uncommitted blocks are never visible).
        let mut block_ids = Vec::new();
        for (idx, chunk) in bytes.chunks(self.azure_block_size).enumerate() {
            let block_id = data_encoding::BASE64.encode(format!("blk-{idx:08}").as_bytes());
            let block_url = cfg.blob_url(object, &[("comp", "block"), ("blockid", &block_id)])?;
            let block_bytes = bytes.slice_ref(chunk);
            self.send_with_retry(
                StoreKind::Azure,
                || {
                    cfg.signed_request(
                        &Method::PUT,
                        &block_url,
                        &[],
                        block_bytes.clone(),
                        self.clock.wall(),
                    )
                },
                azure::translate_error,
            )
            .await?;
            block_ids.push(block_id);
        }
        let list_url = cfg.blob_url(object, &[("comp", "blocklist")])?;
        let list_body = azure::block_list_xml(&block_ids);
        self.send_with_retry(
            StoreKind::Azure,
            || {
                cfg.signed_request(
                    &Method::PUT,
                    &list_url,
                    &[("content-type", "application/xml")],
                    list_body.clone(),
                    self.clock.wall(),
                )
            },
            azure::translate_error,
        )
        .await?;
        Ok(())
    }

    /// Shared retry loop honoring `Retry-After` (ADR 0118) and exponential backoff.
    async fn send_with_retry<F, T>(
        &self,
        _store: StoreKind,
        mut build: F,
        translate: T,
    ) -> Result<(hyper::HeaderMap, Bytes), StoreError>
    where
        F: FnMut() -> Result<Request<Bytes>, StoreError>,
        T: Fn(u16, &str, Option<&str>) -> StoreError,
    {
        for attempt in 0..MAX_ATTEMPTS {
            match self.send_once(&mut build, &translate).await {
                Ok(ok) => return Ok(ok),
                Err(boxed) => {
                    let (error, retry_after_wait) = *boxed;
                    if !error.is_retryable() || attempt + 1 >= MAX_ATTEMPTS {
                        return Err(error);
                    }
                    let backoff = retry_after_wait
                        .unwrap_or_else(|| BASE_BACKOFF.saturating_mul(1_u32 << attempt))
                        .min(MAX_BACKOFF);
                    self.clock.sleep(backoff).await;
                }
            }
        }
        unreachable!("invariant: loop returns on final attempt")
    }

    async fn send_once<F, T>(
        &self,
        build: F,
        translate: T,
    ) -> Result<(hyper::HeaderMap, Bytes), Box<(StoreError, Option<Duration>)>>
    where
        F: FnOnce() -> Result<Request<Bytes>, StoreError>,
        T: Fn(u16, &str, Option<&str>) -> StoreError,
    {
        let request = build().map_err(|e| Box::new((e, None)))?;
        let sent = tokio::time::timeout(
            REQUEST_TIMEOUT,
            self.endpoint.send_whole(
                ConfiguredKind::CheckpointStorage,
                request,
                MAX_RESPONSE_BYTES,
            ),
        )
        .await
        .map_err(|_| Box::new((StoreError::Network("request timed out".to_owned()), None)))?
        .map_err(|failure| Box::new((map_whole_failure(failure), None)))?;

        let status = sent.status().as_u16();
        let headers = sent.headers().clone();
        let wait = retry_after::wait_of(&headers, self.clock.wall());
        let body = sent.into_body();

        if (200..300).contains(&status) {
            Ok((headers, body))
        } else {
            let text = String::from_utf8_lossy(&body);
            let header_code = headers.get("x-ms-error-code").and_then(|v| v.to_str().ok());
            Err(Box::new((translate(status, &text, header_code), wait)))
        }
    }
}

fn map_whole_failure(failure: WholeFailure) -> StoreError {
    match failure {
        WholeFailure::Refused(refused) => StoreError::Refused(refused),
        WholeFailure::CertificateInvalid(invalid) => StoreError::CertificateInvalid(invalid),
        WholeFailure::ClientCertificate(err) => StoreError::ClientCertificate(err),
        WholeFailure::Network => StoreError::Network("endpoint did not answer".to_owned()),
        WholeFailure::TooLong { .. } => StoreError::Network(format!(
            "object-storage response exceeded {MAX_RESPONSE_BYTES} bytes"
        )),
    }
}

async fn resolve_secret_setting(
    setting: &'static Setting,
    settings: &Resolved,
    secrets: &RunSecrets,
    run_id: Option<&RunId>,
) -> Result<Option<SecretString>, StoreError> {
    let Some(reference) = settings.get::<Option<SecretReference>>(setting) else {
        return Ok(None);
    };
    let layer = settings.effective(setting).map(|e| e.layer);
    let purpose = Purpose {
        placeholder: setting.name,
        domain: None,
        domains: &[],
        layer,
        run_id,
    };
    let value = secrets
        .resolve(&reference, &purpose)
        .await
        .map_err(|error| StoreError::CredentialUnavailable {
            setting: setting.name,
            reason: error.to_string(),
        })?;
    Ok(Some(value))
}

async fn resolve_config(
    scheme: Scheme,
    settings: &Resolved,
    secrets: &RunSecrets,
    run_id: Option<&RunId>,
) -> Result<StoreConfig, StoreError> {
    match scheme {
        Scheme::S3 => resolve_s3_config(settings, secrets, run_id)
            .await
            .map(StoreConfig::S3),
        Scheme::Gs => resolve_gcs_config(settings, secrets, run_id)
            .await
            .map(StoreConfig::Gcs),
        Scheme::Az => resolve_azure_config(settings, secrets, run_id)
            .await
            .map(StoreConfig::Azure),
    }
}

async fn resolve_s3_config(
    settings: &Resolved,
    secrets: &RunSecrets,
    run_id: Option<&RunId>,
) -> Result<s3::S3Config, StoreError> {
    let endpoint = settings
        .get::<Option<&str>>(&CHECKPOINT_OBJECT_STORAGE_S3_ENDPOINT)
        .map(str::to_owned);
    let region = settings
        .get::<Option<&str>>(&CHECKPOINT_OBJECT_STORAGE_S3_REGION)
        .unwrap_or(DEFAULT_S3_REGION)
        .to_owned();
    let access_key = resolve_secret_setting(
        &CHECKPOINT_OBJECT_STORAGE_S3_ACCESS_KEY_ID,
        settings,
        secrets,
        run_id,
    )
    .await?;
    let secret_key = resolve_secret_setting(
        &CHECKPOINT_OBJECT_STORAGE_S3_SECRET_ACCESS_KEY,
        settings,
        secrets,
        run_id,
    )
    .await?;
    let session_token = resolve_secret_setting(
        &CHECKPOINT_OBJECT_STORAGE_S3_SESSION_TOKEN,
        settings,
        secrets,
        run_id,
    )
    .await?;
    let credentials = match (access_key, secret_key) {
        (Some(id), Some(secret)) => Some(AwsCredentials {
            access_key_id: id.expose_secret().to_owned(),
            secret_access_key: secret,
            session_token,
        }),
        (None, None) => {
            if session_token.is_some() {
                return Err(StoreError::InvalidConfig(
                    "`checkpoint.object_storage.s3.session_token` requires `access_key_id` and `secret_access_key`".to_owned(),
                ));
            }
            None
        }
        _ => {
            return Err(StoreError::InvalidConfig(
                "`checkpoint.object_storage.s3.access_key_id` and `secret_access_key` must both be set".to_owned(),
            ));
        }
    };
    Ok(s3::S3Config {
        endpoint,
        region,
        credentials,
    })
}

async fn resolve_gcs_config(
    settings: &Resolved,
    secrets: &RunSecrets,
    run_id: Option<&RunId>,
) -> Result<gcs::GcsConfig, StoreError> {
    let endpoint = settings
        .get::<Option<&str>>(&CHECKPOINT_OBJECT_STORAGE_GS_ENDPOINT)
        .map(str::to_owned);
    let access_token = resolve_secret_setting(
        &CHECKPOINT_OBJECT_STORAGE_GS_ACCESS_TOKEN,
        settings,
        secrets,
        run_id,
    )
    .await?;
    Ok(gcs::GcsConfig {
        endpoint,
        access_token,
    })
}

async fn resolve_azure_config(
    settings: &Resolved,
    secrets: &RunSecrets,
    run_id: Option<&RunId>,
) -> Result<azure::AzureConfig, StoreError> {
    let endpoint = settings
        .get::<Option<&str>>(&CHECKPOINT_OBJECT_STORAGE_AZ_ENDPOINT)
        .map(str::to_owned);
    let account = settings
        .get::<Option<&str>>(&CHECKPOINT_OBJECT_STORAGE_AZ_ACCOUNT)
        .map(str::to_owned);
    let account_key = resolve_secret_setting(
        &CHECKPOINT_OBJECT_STORAGE_AZ_ACCOUNT_KEY,
        settings,
        secrets,
        run_id,
    )
    .await?;
    let sas_token = resolve_secret_setting(
        &CHECKPOINT_OBJECT_STORAGE_AZ_SAS_TOKEN,
        settings,
        secrets,
        run_id,
    )
    .await?;
    let auth = match (account_key, sas_token) {
        (Some(key), _) => {
            let acct = account.clone().ok_or_else(|| {
                StoreError::InvalidConfig(
                    "`checkpoint.object_storage.az.account_key` requires `checkpoint.object_storage.az.account`".to_owned(),
                )
            })?;
            Some(azure::AzureAuth::SharedKey { account: acct, key })
        }
        (None, Some(sas)) => Some(azure::AzureAuth::SasToken(sas)),
        (None, None) => None,
    };
    Ok(azure::AzureConfig {
        endpoint,
        account,
        auth,
    })
}

#[expect(
    clippy::disallowed_methods,
    reason = "invoked on the owner's blocking threads to stage downloaded Checkpoint bytes atomically"
)]
fn write_atomic(destination: &Path, bytes: &[u8], nonce: u64) -> std::io::Result<()> {
    use std::io::Write as _;
    let parent = destination.parent().unwrap_or_else(|| Path::new("."));
    std::fs::create_dir_all(parent)?;
    let name = destination
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("checkpoint.sqlite");
    let tmp = parent.join(format!(".{name}.{nonce}.tmp"));
    let write_res = (|| -> std::io::Result<()> {
        let mut file = std::fs::File::create(&tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(())
    })();
    if let Err(error) = write_res {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    if let Err(error) = std::fs::rename(&tmp, destination) {
        let _ = std::fs::remove_file(&tmp);
        return Err(error);
    }
    durable::sync_folder(parent);
    Ok(())
}
