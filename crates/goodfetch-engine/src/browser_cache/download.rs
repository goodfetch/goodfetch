//! A pinned download (ADR 0043): a plain HTTPS GET to the pinned URL over
//! HTTP/1.1, with no cookie, no identifier, no query beyond the pinned path
//! and no user agent, the `resolve` setting applied before any lookup (ADR
//! 0059). It streams the answer into a file while hashing it and reports
//! its progress. The one translation point of this boundary's failures into
//! [`Failure`] (ADR 0127).

use std::collections::BTreeMap;
use std::net::{IpAddr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use http_body_util::BodyExt as _;
use hyper::{Request, StatusCode, Uri};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, ServerName};
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;

use super::artifact::Digest;
use crate::transport;

/// How many bytes pass between two progress reports, at most.
const PROGRESS_STEP: u64 = 1024 * 1024;

/// The `resolve` setting (ADR 0059): host names mapped to addresses before
/// any lookup.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Resolve(BTreeMap<String, Vec<IpAddr>>);

impl Resolve {
    /// No mapping: every name goes to the OS's resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// Maps `host` to `addresses`.
    #[must_use]
    pub fn with(mut self, host: impl Into<String>, addresses: Vec<IpAddr>) -> Self {
        self.0.insert(host.into().to_ascii_lowercase(), addresses);
        self
    }

    fn addresses(&self, host: &str) -> Option<&[IpAddr]> {
        self.0.get(&host.to_ascii_lowercase()).map(Vec::as_slice)
    }
}

/// What a download is allowed to trust and reach.
pub(super) struct Network {
    /// The `resolve` setting.
    pub(super) resolve: Resolve,
    /// Certificate authorities trusted beside Mozilla's roots, such as those
    /// of `trusted_cas`.
    pub(super) trusted_certificates: Vec<CertificateDer<'static>>,
}

/// Why a download failed.
#[derive(Debug)]
pub(super) enum Failure {
    /// The URL is not one the engine downloads from.
    Url,
    /// No answer, or a connection that broke.
    Network,
    /// An answer other than `200 OK`.
    Status(u16),
    /// The cache's own file could not be written.
    Cache(std::io::Error),
}

/// The pinned URL's parts.
pub(super) struct Target {
    uri: Uri,
    host: String,
    port: u16,
}

impl Target {
    /// Reads `url`: `https`, a host, no query and no credentials.
    pub(super) fn parse(url: &str) -> Option<Self> {
        let uri: Uri = url.parse().ok()?;
        let authority = uri.authority()?;
        if uri.scheme_str() != Some("https")
            || uri.query().is_some()
            || authority.as_str().contains('@')
        {
            return None;
        }
        let host = authority
            .host()
            .trim_start_matches('[')
            .trim_end_matches(']');
        Some(Self {
            host: host.to_owned(),
            port: authority.port_u16().unwrap_or(443),
            uri,
        })
    }

    /// The host, which errors and progress name, never the full URL.
    pub(super) fn host(&self) -> &str {
        &self.host
    }
}

/// Downloads `target` into the file at `into`, reporting `(received, total)`
/// as bytes arrive, and returns the digest of what it received.
pub(super) async fn download(
    target: &Target,
    network: &Network,
    into: &Path,
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<Digest, Failure> {
    let addresses = addresses(target, network).await?;
    let tcp = transport::connect_tcp(&addresses)
        .await
        .map_err(|_| Failure::Network)?;
    let server_name = ServerName::try_from(target.host.clone()).map_err(|_| Failure::Url)?;
    let tls = tokio_rustls::TlsConnector::from(Arc::new(tls_config(network)))
        .connect(server_name, tcp)
        .await
        .map_err(|_| Failure::Network)?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(tls))
        .await
        .map_err(|_| Failure::Network)?;
    let authority = target.uri.authority().map_or_else(
        || target.host.clone(),
        |authority| authority.as_str().to_owned(),
    );
    let path = target
        .uri
        .path_and_query()
        .map_or("/", |path| path.as_str())
        .to_owned();
    let request = Request::get(path)
        .header(hyper::header::HOST, authority)
        .body(http_body_util::Empty::<hyper::body::Bytes>::new())
        .map_err(|_| Failure::Url)?;
    let exchange = async {
        let response = sender
            .send_request(request)
            .await
            .map_err(|_| Failure::Network)?;
        if response.status() != StatusCode::OK {
            return Err(Failure::Status(response.status().as_u16()));
        }
        let total = response
            .headers()
            .get(hyper::header::CONTENT_LENGTH)
            .and_then(|length| length.to_str().ok())
            .and_then(|length| length.parse().ok());
        let mut file = tokio::fs::File::create(into)
            .await
            .map_err(Failure::Cache)?;
        let mut body = response.into_body();
        let mut hasher = Sha256::new();
        let mut received = 0_u64;
        let mut reported = 0_u64;
        progress(0, total);
        while let Some(frame) = body.frame().await {
            let frame = frame.map_err(|_| Failure::Network)?;
            if let Ok(bytes) = frame.into_data() {
                hasher.update(&bytes);
                file.write_all(&bytes).await.map_err(Failure::Cache)?;
                received += u64::try_from(bytes.len()).expect("invariant: a frame fits in u64");
                if received - reported >= PROGRESS_STEP {
                    reported = received;
                    progress(received, total);
                }
            }
        }
        if total.is_some_and(|total| total != received) {
            return Err(Failure::Network);
        }
        file.sync_all().await.map_err(Failure::Cache)?;
        if reported != received {
            progress(received, total);
        }
        Ok(Digest::from_bytes(hasher.finalize().into()))
    };
    let mut exchange = std::pin::pin!(exchange);
    let mut connection = std::pin::pin!(connection);
    let mut connection_done = false;
    loop {
        tokio::select! {
            outcome = &mut exchange => return outcome,
            ended = &mut connection, if !connection_done => {
                connection_done = true;
                if ended.is_err() {
                    return Err(Failure::Network);
                }
            }
        }
    }
}

async fn addresses(target: &Target, network: &Network) -> Result<Vec<SocketAddr>, Failure> {
    if let Some(mapped) = network.resolve.addresses(&target.host) {
        return Ok(mapped
            .iter()
            .map(|address| SocketAddr::new(*address, target.port))
            .collect());
    }
    if let Ok(address) = target.host.parse::<IpAddr>() {
        return Ok(vec![SocketAddr::new(address, target.port)]);
    }
    let found: Vec<SocketAddr> = tokio::net::lookup_host((target.host.as_str(), target.port))
        .await
        .map_err(|_| Failure::Network)?
        .collect();
    if found.is_empty() {
        return Err(Failure::Network);
    }
    Ok(found)
}

fn tls_config(network: &Network) -> rustls::ClientConfig {
    let mut roots = rustls::RootCertStore::empty();
    roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    for certificate in &network.trusted_certificates {
        // A certificate that is no CA certificate trusts nothing; it is the
        // setting's to refuse, not the download's.
        let _ = roots.add(certificate.clone());
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let mut config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("invariant: ring supports rustls's default protocol versions")
        .with_root_certificates(roots)
        .with_no_client_auth();
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}
