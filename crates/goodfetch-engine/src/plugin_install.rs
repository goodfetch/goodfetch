//! Installing a Plugin from its OCI artifact (ADR 0033, issue 889): the
//! pull, by digest, of the image index a [`Reference`] pins, the host's
//! platform's manifest it names and that manifest's one layer, the bundle,
//! each checked against its digest before it is read; then the bundle's
//! unpack into a staging folder beside the Plugin store, which the store
//! moves into place whole. Nothing of the Plugin runs.
//!
//! The pull is three GETs through the engine's own HTTP client on the
//! pinned downloads' path (ADR 0043): a digest pins every byte, so the
//! registry is no configured endpoint, and `egress.offline`, the `resolve`
//! mapping and `tls.trusted_cas` hold as for any pinned download. An OCI
//! client crate with its own HTTP client would be a second outbound path
//! past them; the anonymous bearer token and the blob redirects a public
//! registry such as ghcr.io asks for are later work.
//!
//! It is the translation point of the registry's failures (ADR 0127): each
//! becomes a named [`PullFailure`], which `artifact_download_failed`
//! carries as its [`DownloadFailure`]; a status it does not name becomes
//! [`PullFailure::Unrecognized`], which keeps the answer's text and is
//! logged at `warn`.
#![expect(
    clippy::disallowed_methods,
    reason = "the bundle is unpacked and moved on the tasks module's blocking policy, never on an async worker"
)]

use std::fs;
use std::io::{self, BufReader};

use goodfetch_protocol::browser_cache::DownloadFailure;
use goodfetch_protocol::settings::SettingsLayer;

use crate::http_client;
use crate::pinned_download::{self, Digest, Fetching, Receiving};
use crate::plugin_bundle::{self, BundleError};
use crate::plugin_oci::{
    self, DOCUMENT_LIMIT, DocumentError, INDEX_MEDIA_TYPE, MANIFEST_MEDIA_TYPE, Platform, Reference,
};
use crate::plugin_store::{Installed, PluginStore, StoreError};
use crate::tasks::BlockingThreads;

/// The most bytes of an answer's body an [`PullFailure::Unrecognized`]
/// keeps.
const TEXT_LIMIT: usize = 1024;

/// Why the registry did not serve what a Plugin's pull asked for.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum PullFailure {
    /// No answer, or a connection that broke.
    #[error("the registry did not answer, or the connection broke")]
    Unreachable,
    /// The registry's certificate failed verification, its host is no
    /// name TLS verifies, or the client certificate listed for it cannot
    /// be presented.
    #[error("the TLS handshake with the registry failed")]
    Tls,
    /// The registry holds no such repository, manifest or blob: `404`.
    #[error("the registry holds no such artifact (404)")]
    Absent,
    /// The registry asks for credentials, or refuses them: `401` or `403`.
    #[error("the registry refused access ({0})")]
    Unauthorized(u16),
    /// The registry asks to wait: `429`.
    #[error("the registry asks to wait (429)")]
    RateLimited,
    /// The registry failed: a `5xx`.
    #[error("the registry failed ({0})")]
    ServerError(u16),
    /// What arrived is not the bytes its digest and size name.
    #[error("what the registry served is not the pinned bytes")]
    DigestMismatch,
    /// Any other answer: its status and the start of its text.
    #[error("the registry answered {status}: {text}")]
    Unrecognized {
        /// Its status.
        status: u16,
        /// At most the first 1 KiB of its body, as text.
        text: String,
    },
}

impl PullFailure {
    /// The failure `artifact_download_failed` names.
    #[must_use]
    pub fn download_failure(&self) -> DownloadFailure {
        match self {
            Self::Unreachable | Self::Tls => DownloadFailure::Network,
            Self::Absent => DownloadFailure::HttpStatus { status: 404 },
            Self::RateLimited => DownloadFailure::HttpStatus { status: 429 },
            Self::Unauthorized(status)
            | Self::ServerError(status)
            | Self::Unrecognized { status, .. } => DownloadFailure::HttpStatus { status: *status },
            Self::DigestMismatch => DownloadFailure::DigestMismatch,
        }
    }
}

impl From<http_client::Failure> for PullFailure {
    fn from(failure: http_client::Failure) -> Self {
        match failure {
            http_client::Failure::Network => Self::Unreachable,
            http_client::Failure::ServerName
            | http_client::Failure::CertificateInvalid(_)
            | http_client::Failure::ClientCertificate(_) => Self::Tls,
        }
    }
}

/// The failure an answer of `status`, other than `200 OK`, stands for,
/// `text` its body's start; an unrecognized one is logged at `warn`.
fn status_failure(status: u16, text: String) -> PullFailure {
    match status {
        404 => PullFailure::Absent,
        401 | 403 => PullFailure::Unauthorized(status),
        429 => PullFailure::RateLimited,
        500..=599 => PullFailure::ServerError(status),
        _ => {
            tracing::warn!(
                status,
                text,
                "the Plugin registry answered a status the engine does not translate"
            );
            PullFailure::Unrecognized { status, text }
        }
    }
}

/// Why a Plugin was not installed.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    /// `egress.offline`, set at this layer, refused the pull before it
    /// connected.
    #[error("`egress.offline` refused the Plugin's pull")]
    Offline(SettingsLayer),
    /// The registry did not serve the artifact.
    #[error("the Plugin's registry did not serve it: {0}")]
    Pull(PullFailure),
    /// An index or a manifest the engine refuses to follow.
    #[error(transparent)]
    Document(DocumentError),
    /// A bundle the unpack refuses.
    #[error(transparent)]
    Bundle(BundleError),
    /// The store refuses the unpacked bundle.
    #[error(transparent)]
    Store(StoreError),
    /// The staging folder or the layer's file could not be written.
    #[error("the Plugin's staging folder could not be written: {0}")]
    Staging(io::Error),
}

impl From<PullFailure> for InstallError {
    fn from(failure: PullFailure) -> Self {
        Self::Pull(failure)
    }
}

/// Installs the Plugin `reference` pins into `store`, for the host's
/// platform: pulls its index, its manifest and its bundle, each checked
/// against its digest, unpacks the bundle into a staging folder beside the
/// store and adds it, its files and links made inert on `blocking`.
///
/// # Errors
///
/// [`InstallError`] when `egress.offline` is set, the registry does not
/// serve the artifact, a document is refused, the bundle is refused, the
/// store refuses it, or the staging folder cannot be written; the store is
/// then left as it was and the staging folder removed.
///
/// # Cancel safety
///
/// Dropping the future before the bundle's unpack leaves the store as it
/// was; an unpack already on `blocking` finishes, and adds the Plugin or
/// not as if the future were kept. The staging folder is removed either
/// way.
pub async fn install(
    reference: &Reference,
    fetching: &Fetching,
    blocking: &BlockingThreads,
    store: &PluginStore,
) -> Result<Installed, InstallError> {
    if let Some(layer) = fetching.offline {
        return Err(InstallError::Offline(layer));
    }
    let index = document(
        reference,
        reference.index(),
        INDEX_MEDIA_TYPE,
        None,
        fetching,
    )
    .await?;
    let manifest =
        plugin_oci::manifest_for(&index, Platform::host()).map_err(InstallError::Document)?;
    let manifest = document(
        reference,
        manifest.digest,
        MANIFEST_MEDIA_TYPE,
        Some(manifest.size),
        fetching,
    )
    .await?;
    let layer = plugin_oci::bundle_layer(&manifest).map_err(InstallError::Document)?;
    let staging = {
        let store = store.clone();
        blocking
            .run(move || store.staging())
            .await
            .map_err(InstallError::Staging)?
    };
    let archive = staging.path().join("layer");
    let mut response = get(
        reference,
        &reference.blob_path(layer.descriptor.digest),
        None,
        fetching,
    )
    .await?;
    let received = pinned_download::receive(
        &mut response,
        &archive,
        blocking,
        layer.descriptor.size,
        &mut |_, _| {},
    )
    .await
    .map_err(|failure| match failure {
        Receiving::Network => InstallError::Pull(PullFailure::Unreachable),
        Receiving::TooLong => InstallError::Pull(PullFailure::DigestMismatch),
        Receiving::File(error) => InstallError::Staging(error),
    })?;
    if received != layer.descriptor.digest {
        return Err(PullFailure::DigestMismatch.into());
    }
    let name = reference.plugin().to_owned();
    let digest = reference.index().to_string();
    let store = store.clone();
    blocking
        .run(move || {
            let bundle = staging.path().join("bundle");
            fs::create_dir(&bundle).map_err(InstallError::Staging)?;
            let file = fs::File::open(&archive).map_err(InstallError::Staging)?;
            plugin_bundle::unpack(BufReader::new(file), layer.compression, &bundle)
                .map_err(InstallError::Bundle)?;
            store
                .add(&name, &bundle, &digest)
                .map_err(InstallError::Store)
        })
        .await
}

/// The index or manifest of digest `digest`, asked for as `media_type`,
/// at most `size` bytes when its descriptor gives it, else
/// [`DOCUMENT_LIMIT`], checked against its digest.
async fn document(
    reference: &Reference,
    digest: Digest,
    media_type: &str,
    size: Option<u64>,
    fetching: &Fetching,
) -> Result<Vec<u8>, InstallError> {
    let mut response = get(
        reference,
        &reference.manifest_path(digest),
        Some(media_type),
        fetching,
    )
    .await?;
    let limit = size.unwrap_or(DOCUMENT_LIMIT);
    let mut body = Vec::new();
    while let Some(bytes) = response.chunk().await.map_err(PullFailure::from)? {
        body.extend_from_slice(&bytes);
        let received = u64::try_from(body.len()).unwrap_or(u64::MAX);
        if received > limit {
            return Err(match size {
                Some(_) => PullFailure::DigestMismatch.into(),
                None => InstallError::Document(DocumentError::TooLarge {
                    document: plugin_oci::INDEX,
                    size: received,
                }),
            });
        }
    }
    if Digest::of(&body) != digest {
        return Err(PullFailure::DigestMismatch.into());
    }
    Ok(body)
}

/// Sends the GET of `path` on the registry of `reference`, with an
/// `Accept` of `accept` when given, and answers its head once it is
/// `200 OK`.
async fn get(
    reference: &Reference,
    path: &str,
    accept: Option<&str>,
    fetching: &Fetching,
) -> Result<http_client::Response, PullFailure> {
    let url = fetching
        .located(&format!("https://{}{path}", reference.registry()))
        .expect("invariant: a Reference's registry and paths make an https URL");
    let mut response = pinned_download::send_get(&url, accept, fetching).await?;
    let status = response.status();
    if status == 200 {
        return Ok(response);
    }
    let mut text = Vec::new();
    while text.len() < TEXT_LIMIT {
        match response.chunk().await {
            Ok(Some(bytes)) => text.extend_from_slice(&bytes),
            Ok(None) | Err(_) => break,
        }
    }
    text.truncate(TEXT_LIMIT);
    Err(status_failure(
        status,
        String::from_utf8_lossy(&text).into_owned(),
    ))
}

#[cfg(test)]
mod tests;
