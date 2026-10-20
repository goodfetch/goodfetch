//! Pinned downloads, the one path for every connection nobody configured
//! (ADR 0043): an HTTPS GET to the pinned URL through the engine's HTTP
//! client, its bytes checked against the pinned digest, with no cookie, no
//! identifier, no query beyond the pinned path and the user agent
//! `goodfetch-download`, which names no version, so a download says nothing
//! about its install. Under `egress.offline` each is refused before it
//! connects, and its caller raises `artifact_unavailable_offline` naming the
//! command that supplies it beforehand (ADR 0115). It streams the answer into
//! a file while hashing it and reports its progress; it is the one
//! translation point of this boundary's failures into [`Failure`] (ADR
//! 0127).

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use goodfetch_protocol::settings::SettingsLayer;
use http_body_util::Empty;
use hyper::body::Bytes;
use hyper::{Request, StatusCode, Uri};
use rustls::pki_types::CertificateDer;
use sha2::{Digest as _, Sha256};
use tokio::io::AsyncWriteExt as _;

use crate::clock::Clock;
use crate::happy_eyeballs::Delays;
use crate::http_client::{self, Resolve, Server};

/// The user agent of every pinned download: the same for every install and
/// version (ADR 0043).
pub const USER_AGENT: &str = "goodfetch-download";

/// How many bytes pass between two progress reports, at most.
const PROGRESS_STEP: u64 = 1024 * 1024;

/// A SHA-256 digest, written `sha256:<hex>`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    /// The digest a SHA-256 hasher finished with.
    pub(crate) fn from_hasher(hasher: Sha256) -> Self {
        Self(hasher.finalize().into())
    }

    /// Its 32 bytes.
    pub(crate) fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// Text that is no `sha256:<hex>` digest.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is no digest `sha256:<64 lowercase hex digits>`")]
pub struct MalformedDigest(String);

impl FromStr for Digest {
    type Err = MalformedDigest;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let malformed = || MalformedDigest(text.to_owned());
        let hex = text.strip_prefix("sha256:").ok_or_else(malformed)?;
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(malformed());
        }
        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte =
                u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).map_err(|_| malformed())?;
        }
        Ok(Self(bytes))
    }
}

/// What a pinned download may do: whether `egress.offline` refuses it,
/// what it may reach and trust, and how it races the host's addresses.
#[derive(Clone, Debug)]
pub struct Fetching {
    /// The layer that set `egress.offline` to `true`, or `None` when
    /// downloads are allowed (ADR 0043).
    pub offline: Option<SettingsLayer>,
    /// The `resolve` setting (ADR 0059).
    pub resolve: Resolve,
    /// Certificate authorities trusted beside Mozilla's roots.
    pub trusted_certificates: Vec<CertificateDer<'static>>,
    /// What the Happy Eyeballs race waits on (ADR 0118).
    pub clock: Clock,
    /// `resolver.resolution_delay` and `resolver.attempt_delay`.
    pub delays: Delays,
}

impl Fetching {
    /// Downloads allowed, with no mapping, no extra certificate authority
    /// and RFC 8305's delays, on `clock`.
    pub fn new(clock: Clock) -> Self {
        Self {
            offline: None,
            resolve: Resolve::new(),
            trusted_certificates: Vec::new(),
            clock,
            delays: Delays::BUILT_IN,
        }
    }
}

/// A pinned URL: `https`, a host, no query, no fragment and no credentials.
pub(crate) struct PinnedUrl {
    uri: Uri,
    host: String,
    port: u16,
}

impl PinnedUrl {
    /// Reads `url`; `None` when it is no pinned URL.
    pub(crate) fn parse(url: &str) -> Option<Self> {
        if url.contains('#') {
            return None;
        }
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
    pub(crate) fn host(&self) -> &str {
        &self.host
    }
}

/// Why a pinned download failed.
#[derive(Debug)]
pub(crate) enum Failure {
    /// `egress.offline`, set at this layer, refused it before it connected.
    Offline(SettingsLayer),
    /// No answer, or a connection that broke.
    Network,
    /// An answer other than `200 OK`.
    Status(u16),
    /// What it received is not the pinned bytes.
    DigestMismatch,
    /// The file it writes into could not be written.
    File(std::io::Error),
}

/// Downloads `url` into the file at `into`, reporting `(received, total)` as
/// bytes arrive, and checks that what it received has the digest `pinned`.
///
/// # Cancel safety
///
/// Dropping the future closes the connection and leaves `into` partly
/// written, for its caller to remove.
pub(crate) async fn download(
    url: &PinnedUrl,
    pinned: Digest,
    fetching: &Fetching,
    into: &Path,
    progress: &mut (dyn FnMut(u64, Option<u64>) + Send),
) -> Result<(), Failure> {
    if let Some(layer) = fetching.offline {
        return Err(Failure::Offline(layer));
    }
    let authority = url.uri.authority().map_or_else(
        || url.host.clone(),
        |authority| authority.as_str().to_owned(),
    );
    let path = url
        .uri
        .path_and_query()
        .map_or("/", |path| path.as_str())
        .to_owned();
    let request = Request::get(path)
        .header(hyper::header::HOST, authority)
        .header(hyper::header::USER_AGENT, USER_AGENT)
        .body(Empty::<Bytes>::new())
        .map_err(|_| Failure::Network)?;
    let network = http_client::Network {
        resolve: &fetching.resolve,
        trusted_certificates: &fetching.trusted_certificates,
        clock: fetching.clock,
        delays: fetching.delays,
    };
    let server = Server {
        host: &url.host,
        port: url.port,
    };
    let mut response = http_client::send(&server, request, &network)
        .await
        .map_err(|_| Failure::Network)?;
    if response.status != StatusCode::OK {
        return Err(Failure::Status(response.status.as_u16()));
    }
    let total = response.content_length;
    let mut file = tokio::fs::File::create(into).await.map_err(Failure::File)?;
    let mut hasher = Sha256::new();
    let mut received = 0_u64;
    let mut reported = 0_u64;
    progress(0, total);
    while let Some(bytes) = response.chunk().await.map_err(|_| Failure::Network)? {
        hasher.update(&bytes);
        file.write_all(&bytes).await.map_err(Failure::File)?;
        received += u64::try_from(bytes.len()).expect("invariant: a frame fits in u64");
        if received - reported >= PROGRESS_STEP {
            reported = received;
            progress(received, total);
        }
    }
    if total.is_some_and(|total| total != received) {
        return Err(Failure::Network);
    }
    file.sync_all().await.map_err(Failure::File)?;
    if reported != received {
        progress(received, total);
    }
    if Digest::from_hasher(hasher) != pinned {
        return Err(Failure::DigestMismatch);
    }
    Ok(())
}
