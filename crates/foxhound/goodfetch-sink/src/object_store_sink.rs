//! Object-store Files-mode destination over `object_store` (ADRs 0026 and 0127;
//! issues 1152 and 1156): serves `s3`, `gcs` and `azure_blob` Sinks.
//!
//! Each part is uploaded in one atomic PUT so an interrupted write is never
//! visible as a part. When the store supports conditional writes (`If-None-Match`
//! / `If-Match` on `_goodfetch_fence.json`), the destination proves
//! [`GuaranteeClass::Fenced`] and fences out zombie copies with an older
//! `run_epoch`; otherwise it proves [`GuaranteeClass::Keyed`].

use std::collections::BTreeMap;
use std::sync::Arc;

use bytes::Bytes;
use futures_util::TryStreamExt as _;
use object_store::aws::AmazonS3Builder;
use object_store::azure::MicrosoftAzureBuilder;
use object_store::gcp::GoogleCloudStorageBuilder;
use object_store::path::Path as ObjectPath;
use object_store::{
    ClientOptions, ObjectStore, ObjectStoreExt as _, PutMode, PutOptions, PutPayload, UpdateVersion,
};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::contract::{Failure, FailureKind, GuaranteeClass, RunCopy};
use crate::files::{FileDestination, FileProbe, PutOutcome, parse_part_key};

/// Supported object-store backends.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectStoreBackend {
    /// AWS S3 or S3-compatible (`RustFS`, `LocalStack`, R2, etc.).
    S3,
    /// Google Cloud Storage (or fake-gcs-server).
    Gcs,
    /// Azure Blob Storage (or Azurite).
    AzureBlob,
}

/// Errors from [`ObjectStoreDestination`] before translation.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ObjectStoreError {
    /// Destination URL could not be parsed or is missing required parameters.
    #[error("invalid object-store destination: {0}")]
    InvalidDestination(String),
    /// Error returned by the `object_store` client.
    #[error("{0}")]
    Store(#[from] object_store::Error),
    /// Fence metadata could not be serialized or parsed.
    #[error("fence object JSON error: {0}")]
    FenceJson(String),
    /// No active Tokio runtime available when called synchronously.
    #[error("object-store Sink requires an active Tokio runtime")]
    NoRuntime,
}

#[derive(Serialize, Deserialize)]
struct FenceRecord {
    run_epoch: u64,
}

/// An object-store [`FileDestination`] backed by `object_store::ObjectStore`.
pub struct ObjectStoreDestination {
    store: Arc<dyn ObjectStore>,
    prefix: String,
    sink_name: String,
    fence_supported: Option<bool>,
    fence_version: Option<UpdateVersion>,
}

impl std::fmt::Debug for ObjectStoreDestination {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ObjectStoreDestination")
            .field("sink_name", &self.sink_name)
            .field("prefix", &self.prefix)
            .field("fence_supported", &self.fence_supported)
            .finish_non_exhaustive()
    }
}

impl ObjectStoreDestination {
    /// Builds an [`ObjectStoreDestination`] from a configured URL and backend.
    ///
    /// Supported URL shapes:
    /// - S3: `s3://[access_key:secret_key@]bucket[/prefix][?endpoint=http://...&region=us-east-1&allow_http=true]`
    /// - GCS: `gs://bucket[/prefix][?endpoint=http://...&allow_http=true&service_account_key=...]`
    /// - Azure: `az://[account:key@]container[/prefix][?endpoint=http://...&allow_http=true&use_emulator=true]`
    ///
    /// # Errors
    ///
    /// [`ObjectStoreError::InvalidDestination`] or [`ObjectStoreError::Store`]
    /// if the URL or client configuration is invalid.
    pub fn from_url(
        backend: ObjectStoreBackend,
        sink_name: impl Into<String>,
        destination: &str,
        target_prefix: Option<&str>,
    ) -> Result<Self, ObjectStoreError> {
        let url = Url::parse(destination)
            .map_err(|err| ObjectStoreError::InvalidDestination(err.to_string()))?;
        let bucket = url
            .host_str()
            .filter(|h| !h.is_empty())
            .ok_or_else(|| ObjectStoreError::InvalidDestination("missing bucket/container".into()))?
            .to_owned();
        let url_prefix = url.path().trim_matches('/').to_owned();
        let prefix = match (
            url_prefix.is_empty(),
            target_prefix.map(|p| p.trim_matches('/')),
        ) {
            (true, Some(t)) if !t.is_empty() => t.to_owned(),
            (false, Some(t)) if !t.is_empty() => format!("{url_prefix}/{t}"),
            (false, _) => url_prefix,
            (true, _) => String::new(),
        };
        let params: BTreeMap<String, String> = url
            .query_pairs()
            .map(|(k, v)| (k.into_owned(), v.into_owned()))
            .collect();
        let allow_http = params
            .get("allow_http")
            .is_some_and(|v| v == "true" || v == "1")
            || params
                .get("endpoint")
                .is_some_and(|e| e.starts_with("http://"));
        let client_options = ClientOptions::new().with_allow_http(allow_http);

        let store = match backend {
            ObjectStoreBackend::S3 => {
                build_s3_store(&bucket, &url, &params, client_options, allow_http)?
            }
            ObjectStoreBackend::Gcs => {
                build_gcs_store(&bucket, &params, client_options, allow_http)?
            }
            ObjectStoreBackend::AzureBlob => {
                build_azure_store(&bucket, &url, &params, client_options, allow_http)?
            }
        };

        Ok(Self {
            store,
            prefix,
            sink_name: sink_name.into(),
            fence_supported: None,
            fence_version: None,
        })
    }

    /// Returns the combined prefix (URL path + target) used for part keys and
    /// fence probes.
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// Creates an [`ObjectStoreDestination`] directly from an existing
    /// [`ObjectStore`] instance.
    #[must_use]
    pub fn from_store(
        store: Arc<dyn ObjectStore>,
        sink_name: impl Into<String>,
        prefix: impl Into<String>,
    ) -> Self {
        Self {
            store,
            prefix: prefix.into().trim_matches('/').to_owned(),
            sink_name: sink_name.into(),
            fence_supported: None,
            fence_version: None,
        }
    }

    /// Returns a clone of the underlying [`ObjectStore`].
    #[must_use]
    pub fn store(&self) -> Arc<dyn ObjectStore> {
        Arc::clone(&self.store)
    }

    fn fence_path(&self, run: &RunCopy) -> ObjectPath {
        let rel = format!("{}/{}/_goodfetch_fence.json", run.run_id, self.sink_name);
        if self.prefix.is_empty() {
            ObjectPath::from(rel)
        } else {
            ObjectPath::from(format!("{}/{rel}", self.prefix))
        }
    }

    fn run_prefix(&self, run: &RunCopy) -> ObjectPath {
        let rel = format!("{}/{}", run.run_id, self.sink_name);
        if self.prefix.is_empty() {
            ObjectPath::from(rel)
        } else {
            ObjectPath::from(format!("{}/{rel}", self.prefix))
        }
    }

    fn run_async<F, T>(future: F) -> Result<T, ObjectStoreError>
    where
        F: std::future::Future<Output = Result<T, ObjectStoreError>>,
    {
        let handle =
            tokio::runtime::Handle::try_current().map_err(|_| ObjectStoreError::NoRuntime)?;
        handle.block_on(future)
    }

    async fn read_fence(
        &self,
        path: &ObjectPath,
    ) -> Result<Option<(u64, UpdateVersion)>, ObjectStoreError> {
        match self.store.get(path).await {
            Ok(result) => {
                let version = UpdateVersion {
                    e_tag: result.meta.e_tag.clone(),
                    version: result.meta.version.clone(),
                };
                let bytes = result.bytes().await?;
                let record: FenceRecord = serde_json::from_slice(&bytes)
                    .map_err(|err| ObjectStoreError::FenceJson(err.to_string()))?;
                Ok(Some((record.run_epoch, version)))
            }
            Err(object_store::Error::NotFound { .. }) => Ok(None),
            Err(err) => Err(ObjectStoreError::Store(err)),
        }
    }

    async fn ensure_fence(
        &mut self,
        run: &RunCopy,
    ) -> Result<(GuaranteeClass, Option<u64>), ObjectStoreError> {
        let path = self.fence_path(run);
        let payload_bytes = serde_json::to_vec(&FenceRecord {
            run_epoch: run.run_epoch,
        })
        .map_err(|err| ObjectStoreError::FenceJson(err.to_string()))?;

        for _ in 0..3 {
            let existing = self.read_fence(&path).await?;
            if let Some((epoch, version)) = existing {
                if epoch > run.run_epoch {
                    self.fence_supported = Some(true);
                    return Ok((GuaranteeClass::Fenced, Some(epoch)));
                }
                if epoch == run.run_epoch && self.fence_supported.is_some() {
                    self.fence_version = Some(version);
                    let class = if self.fence_supported == Some(true) {
                        GuaranteeClass::Fenced
                    } else {
                        GuaranteeClass::Keyed
                    };
                    return Ok((class, None));
                }
                let opts = PutOptions::from(PutMode::Update(version));
                match self
                    .store
                    .put_opts(
                        &path,
                        PutPayload::from(Bytes::from(payload_bytes.clone())),
                        opts,
                    )
                    .await
                {
                    Ok(res) => {
                        self.fence_supported = Some(true);
                        self.fence_version = Some(UpdateVersion {
                            e_tag: res.e_tag,
                            version: res.version,
                        });
                        return Ok((GuaranteeClass::Fenced, None));
                    }
                    Err(object_store::Error::Precondition { .. }) => continue,
                    Err(
                        object_store::Error::NotImplemented { .. }
                        | object_store::Error::NotSupported { .. },
                    ) => {
                        self.store
                            .put(&path, PutPayload::from(Bytes::from(payload_bytes)))
                            .await?;
                        self.fence_supported = Some(false);
                        return Ok((GuaranteeClass::Keyed, None));
                    }
                    Err(err) => return Err(ObjectStoreError::Store(err)),
                }
            }

            let opts = PutOptions::from(PutMode::Create);
            match self
                .store
                .put_opts(
                    &path,
                    PutPayload::from(Bytes::from(payload_bytes.clone())),
                    opts,
                )
                .await
            {
                Ok(res) => {
                    self.fence_supported = Some(true);
                    self.fence_version = Some(UpdateVersion {
                        e_tag: res.e_tag,
                        version: res.version,
                    });
                    return Ok((GuaranteeClass::Fenced, None));
                }
                Err(
                    object_store::Error::AlreadyExists { .. }
                    | object_store::Error::Precondition { .. },
                ) => {}
                Err(
                    object_store::Error::NotImplemented { .. }
                    | object_store::Error::NotSupported { .. },
                ) => {
                    self.store
                        .put(&path, PutPayload::from(Bytes::from(payload_bytes)))
                        .await?;
                    self.fence_supported = Some(false);
                    return Ok((GuaranteeClass::Keyed, None));
                }
                Err(err) => return Err(ObjectStoreError::Store(err)),
            }
        }

        if let Some((epoch, _)) = self.read_fence(&path).await?
            && epoch > run.run_epoch
        {
            self.fence_supported = Some(true);
            return Ok((GuaranteeClass::Fenced, Some(epoch)));
        }
        Err(ObjectStoreError::FenceJson(
            "fence update conflicted repeatedly".to_owned(),
        ))
    }

    async fn check_fence_before_put(
        &mut self,
        run: &RunCopy,
    ) -> Result<Option<u64>, ObjectStoreError> {
        if self.fence_supported != Some(true) {
            return Ok(None);
        }
        let path = self.fence_path(run);
        if let Some((epoch, version)) = self.read_fence(&path).await? {
            if epoch > run.run_epoch {
                return Ok(Some(epoch));
            }
            self.fence_version = Some(version);
        }
        Ok(None)
    }
}

impl FileDestination for ObjectStoreDestination {
    type Error = ObjectStoreError;

    fn probe(&mut self, run: &RunCopy) -> Result<FileProbe, Self::Error> {
        let run_cloned = run.clone();
        let store = Arc::clone(&self.store);
        let prefix = self.prefix.clone();
        let sink_name = self.sink_name.clone();
        let list_prefix = self.run_prefix(&run_cloned);

        let handle =
            tokio::runtime::Handle::try_current().map_err(|_| ObjectStoreError::NoRuntime)?;
        let (class, superseded_by) = handle.block_on(self.ensure_fence(&run_cloned))?;
        if let Some(newer) = superseded_by {
            return Ok(FileProbe {
                class,
                delivered: BTreeMap::new(),
                next_parts: BTreeMap::new(),
                superseded_by: Some(newer),
            });
        }

        let (delivered, next_parts) = handle.block_on(async move {
            let mut delivered = BTreeMap::<String, u64>::new();
            let mut next_parts = BTreeMap::<String, u64>::new();
            let mut stream = store.list(Some(&list_prefix));
            while let Some(meta) = stream.try_next().await? {
                let key = meta.location.as_ref();
                if let Some(parsed) = parse_part_key(&prefix, &run_cloned.run_id, &sink_name, key)
                    && parsed.run_epoch == run_cloned.run_epoch
                {
                    delivered
                        .entry(parsed.dataset.clone())
                        .and_modify(|max| *max = (*max).max(parsed.through))
                        .or_insert(parsed.through);
                    next_parts
                        .entry(parsed.dataset)
                        .and_modify(|next| *next = (*next).max(parsed.part_number + 1))
                        .or_insert(parsed.part_number + 1);
                }
            }
            Ok::<_, ObjectStoreError>((delivered, next_parts))
        })?;

        Ok(FileProbe {
            class,
            delivered,
            next_parts,
            superseded_by: None,
        })
    }

    fn put_part(
        &mut self,
        key: &str,
        bytes: Vec<u8>,
        run: &RunCopy,
    ) -> Result<PutOutcome, Self::Error> {
        let run_cloned = run.clone();
        let handle =
            tokio::runtime::Handle::try_current().map_err(|_| ObjectStoreError::NoRuntime)?;
        if let Some(newer) = handle.block_on(self.check_fence_before_put(&run_cloned))? {
            return Ok(PutOutcome::Superseded { run_epoch: newer });
        }

        let store = Arc::clone(&self.store);
        let path = ObjectPath::from(key);
        Self::run_async(async move {
            store
                .put(&path, PutPayload::from(Bytes::from(bytes)))
                .await?;
            Ok(PutOutcome::Written)
        })
    }

    fn map_failure(&self, error: Self::Error) -> Failure {
        match error {
            ObjectStoreError::InvalidDestination(msg) | ObjectStoreError::FenceJson(msg) => {
                Failure::new(FailureKind::Permanent, msg)
            }
            ObjectStoreError::NoRuntime => Failure::new(
                FailureKind::Permanent,
                "object-store Sink requires an active Tokio runtime",
            ),
            ObjectStoreError::Store(err) => map_object_store_error(&err),
        }
    }
}

/// Translates `object_store::Error` into the driver's [`Failure`] (ADR 0127).
fn build_s3_store(
    bucket: &str,
    url: &Url,
    params: &BTreeMap<String, String>,
    client_options: ClientOptions,
    allow_http: bool,
) -> Result<Arc<dyn ObjectStore>, ObjectStoreError> {
    let mut builder = AmazonS3Builder::new()
        .with_bucket_name(bucket)
        .with_client_options(client_options)
        .with_allow_http(allow_http);
    let region = params.get("region").map_or("us-east-1", String::as_str);
    builder = builder.with_region(region);
    if let Some(endpoint) = params.get("endpoint") {
        builder = builder
            .with_endpoint(endpoint)
            .with_virtual_hosted_style_request(false);
    }
    let access_key = params
        .get("access_key_id")
        .cloned()
        .or_else(|| (!url.username().is_empty()).then(|| url.username().to_owned()));
    let secret_key = params
        .get("secret_access_key")
        .cloned()
        .or_else(|| url.password().map(str::to_owned));
    if let Some(key) = access_key {
        builder = builder.with_access_key_id(key);
    }
    if let Some(secret) = secret_key {
        builder = builder.with_secret_access_key(secret);
    }
    if params
        .get("skip_signature")
        .is_some_and(|v| v == "true" || v == "1")
    {
        builder = builder.with_skip_signature(true);
    }
    Ok(Arc::new(builder.build()?))
}

fn build_gcs_store(
    bucket: &str,
    params: &BTreeMap<String, String>,
    client_options: ClientOptions,
    allow_http: bool,
) -> Result<Arc<dyn ObjectStore>, ObjectStoreError> {
    let mut builder = GoogleCloudStorageBuilder::new()
        .with_bucket_name(bucket)
        .with_client_options(client_options);
    if let Some(endpoint) = params.get("endpoint") {
        builder = builder.with_base_url(endpoint);
    }
    if let Some(key_json) = params.get("service_account_key") {
        builder = builder.with_service_account_key(key_json);
    } else if allow_http {
        builder = builder
            .with_skip_signature(true)
            .with_bearer_token("anonymous");
    }
    Ok(Arc::new(builder.build()?))
}

fn build_azure_store(
    container: &str,
    url: &Url,
    params: &BTreeMap<String, String>,
    client_options: ClientOptions,
    allow_http: bool,
) -> Result<Arc<dyn ObjectStore>, ObjectStoreError> {
    let mut builder = MicrosoftAzureBuilder::new()
        .with_container_name(container)
        .with_client_options(client_options)
        .with_allow_http(allow_http);
    if params
        .get("use_emulator")
        .is_some_and(|v| v == "true" || v == "1")
    {
        builder = builder.with_use_emulator(true);
    }
    if let Some(endpoint) = params.get("endpoint") {
        builder = builder.with_endpoint(endpoint.clone());
    }
    let account = params
        .get("account")
        .cloned()
        .or_else(|| (!url.username().is_empty()).then(|| url.username().to_owned()));
    let key = params
        .get("access_key")
        .cloned()
        .or_else(|| url.password().map(str::to_owned));
    if let Some(account) = account {
        builder = builder.with_account(account);
    }
    if let Some(key) = key {
        builder = builder.with_access_key(key);
    }
    Ok(Arc::new(builder.build()?))
}

/// Translates `object_store::Error` into the driver's [`Failure`] (ADR 0127).
fn map_object_store_error(error: &object_store::Error) -> Failure {
    match error {
        object_store::Error::PermissionDenied { .. }
        | object_store::Error::Unauthenticated { .. } => {
            Failure::new(FailureKind::Auth, error.to_string())
        }
        object_store::Error::NotFound { .. } | object_store::Error::InvalidPath { .. } => {
            Failure::new(FailureKind::Permanent, error.to_string())
        }
        object_store::Error::Precondition { .. }
        | object_store::Error::AlreadyExists { .. }
        | object_store::Error::JoinError { .. } => {
            Failure::new(FailureKind::Transient, error.to_string())
        }
        other => {
            let text = other.to_string();
            if text.contains("429") || text.contains("Too Many Requests") {
                Failure::new(FailureKind::QuotaExhausted, text)
            } else if text.contains("410") && text.contains("Gone") {
                Failure::new(FailureKind::Gone, text)
            } else if text.contains("403") || text.contains("401") {
                Failure::new(FailureKind::Auth, text)
            } else if text.contains("500")
                || text.contains("502")
                || text.contains("503")
                || text.contains("504")
                || text.contains("408")
                || text.contains("timed out")
                || text.contains("connection")
            {
                Failure::new(FailureKind::Transient, text)
            } else {
                tracing::warn!(error = %text, "unrecognized object_store failure; treating as transient");
                Failure::new(FailureKind::Unrecognized, text)
            }
        }
    }
}
