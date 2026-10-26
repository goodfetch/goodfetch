//! The light-browser tier's Lightpanda build, never bundled, downloaded on
//! first use into the Browser cache with its pinned digest (ADR 0020):
//! Foxhound's own pin for this platform, its URL's host mapped to a local
//! HTTPS stand-in that serves the tests' copy of the release executable
//! (`ci/lightpanda-fetch.sh`), so no test reaches GitHub.
//!
//! Covers: story:5-120
#![expect(
    clippy::disallowed_types,
    reason = "the stand-in, outside the engine, counts its requests under a test-thread lock (ADR 0062)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::{Arc, Mutex};

use goodfetch_engine::browser_cache::{BrowserCache, CacheLocation, PinnedArtifact, Platform};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::http_client::Resolve;
use goodfetch_engine::pinned_download::Fetching;
use goodfetch_fetch_tier::{light_browser_pins, prepare_light_browser};
use goodfetch_protocol::browser_cache::PinnedBy;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_test_support::{TestEnv, lightpanda_copy};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};

/// An HTTPS server on loopback, for `host`, that serves fixed files by path
/// and counts what it was asked.
struct StandIn {
    port: u16,
    authority: CertificateDer<'static>,
    asked: Arc<Mutex<Vec<String>>>,
}

impl StandIn {
    #[expect(
        clippy::disallowed_methods,
        clippy::disallowed_types,
        reason = "the stand-in server, outside the engine, runs its accept loop and connections on the test's runtime (ADR 0062)"
    )]
    async fn serve(host: &str, files: BTreeMap<String, Vec<u8>>) -> Self {
        let authority_key = rcgen::KeyPair::generate().unwrap();
        let mut authority_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
        authority_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let authority = authority_params.self_signed(&authority_key).unwrap();
        let issuer = rcgen::Issuer::new(authority_params, authority_key);
        let leaf_key = rcgen::KeyPair::generate().unwrap();
        let leaf = rcgen::CertificateParams::new(vec![host.to_owned()])
            .unwrap()
            .signed_by(&leaf_key, &issuer)
            .unwrap();
        let config = rustls::ServerConfig::builder_with_provider(Arc::new(
            rustls::crypto::ring::default_provider(),
        ))
        .with_safe_default_protocol_versions()
        .unwrap()
        .with_no_client_auth()
        .with_single_cert(
            vec![leaf.der().clone()],
            PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
        )
        .unwrap();
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener =
            tokio::net::TcpListener::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
                .await
                .unwrap();
        let port = listener.local_addr().unwrap().port();
        let asked = Arc::new(Mutex::new(Vec::new()));
        let (files, asking) = (Arc::new(files), Arc::clone(&asked));
        tokio::spawn(async move {
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                let Ok(tls) = acceptor.accept(tcp).await else {
                    continue;
                };
                let (files, asking) = (Arc::clone(&files), Arc::clone(&asking));
                let service = hyper::service::service_fn(move |request: Request<_>| {
                    let path = request.uri().path().to_owned();
                    asking.lock().unwrap().push(path.clone());
                    let response = files.get(&path).map_or_else(
                        || {
                            let mut missing = Response::new(Full::new(Bytes::new()));
                            *missing.status_mut() = StatusCode::NOT_FOUND;
                            missing
                        },
                        |bytes| Response::new(Full::new(Bytes::from(bytes.clone()))),
                    );
                    async move { Ok::<_, std::convert::Infallible>(response) }
                });
                tokio::spawn(
                    hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tls), service),
                );
            }
        });
        Self {
            port,
            authority: authority.der().clone(),
            asked,
        }
    }
}

/// The host Lightpanda's builds download from.
const HOST: &str = "github.com";

/// Foxhound's Lightpanda pin for this platform.
fn pinned_here() -> PinnedArtifact {
    let platform = Platform::current().build_platform();
    light_browser_pins()
        .into_iter()
        .find(|pin| pin.platform == platform)
        .expect("Lightpanda pins a build for this platform")
}

/// `pin` with its URL moved to the stand-in's port, and what a download
/// of it may do there.
fn moved(mut pin: PinnedArtifact, stand_in: &StandIn) -> (PinnedArtifact, Fetching) {
    let host = HOST;
    pin.url = pin.url.replace(
        &format!("https://{host}/"),
        &format!("https://{host}:{}/", stand_in.port),
    );
    let fetching = Fetching {
        resolve: Resolve::new().with(host, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]),
        trusted_certificates: vec![stand_in.authority.clone()],
        ..Fetching::new(Clock::system())
    };
    (pin, fetching)
}

fn pinned_by() -> PinnedBy {
    PinnedBy {
        product: Product::Foxhound,
        product_version: ProductVersion::parse("1.0.0").unwrap(),
    }
}

/// The path of a pin's URL.
fn path_of(pin: &PinnedArtifact) -> String {
    format!("/{}", pin.url.splitn(4, '/').nth(3).unwrap())
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the tests' copy outside any async worker"
)]
fn copy() -> Vec<u8> {
    std::fs::read(lightpanda_copy().expect("the tests' Lightpanda is fetched")).unwrap()
}

/// The first run on the tier downloads Lightpanda's pinned executable into
/// the Browser cache, under its release name, and a later one takes it
/// from there, the digest checked each time.
#[test]
fn lightpanda_is_downloaded_into_the_browser_cache_on_first_use() {
    let env = TestEnv::new();
    let bytes = copy();
    goodfetch_test_support::runtime().block_on(async {
        let pin = pinned_here();
        let stand_in = StandIn::serve(HOST, BTreeMap::from([(path_of(&pin), bytes.clone())])).await;
        let (pin, fetching) = moved(pin, &stand_in);
        let cache = BrowserCache::new(&CacheLocation::new(env.home(), None));
        let now = jiff::Timestamp::try_from(env.start_time()).unwrap();

        let first = prepare_light_browser(&cache, &pin, &pinned_by(), &fetching, now)
            .await
            .unwrap();
        let second = prepare_light_browser(&cache, &pin, &pinned_by(), &fetching, now)
            .await
            .unwrap();

        let name = pin.url.rsplit('/').next().unwrap();
        assert_eq!(
            (
                first.downloaded,
                second.downloaded,
                first.executable.starts_with(cache.root()),
                first.executable.file_name().and_then(|file| file.to_str()),
                stand_in.asked.lock().unwrap().clone(),
            ),
            (true, false, true, Some(name), vec![path_of(&pin)])
        );
    });
}

/// A download whose bytes are not the pinned build's is refused with
/// `artifact_download_failed`, and nothing is left to run.
///
/// Covers: error:artifact_download_failed
#[test]
fn a_lightpanda_download_without_its_pinned_digest_is_refused() {
    let env = TestEnv::new();
    let mut bytes = copy();
    bytes[0] ^= 0xff;
    goodfetch_test_support::runtime().block_on(async {
        let pin = pinned_here();
        let stand_in = StandIn::serve(HOST, BTreeMap::from([(path_of(&pin), bytes)])).await;
        let (pin, fetching) = moved(pin, &stand_in);
        let cache = BrowserCache::new(&CacheLocation::new(env.home(), None));
        let now = jiff::Timestamp::try_from(env.start_time()).unwrap();

        let refused = prepare_light_browser(&cache, &pin, &pinned_by(), &fetching, now).await;

        let error = refused.err().expect("the download is refused");
        assert_eq!(
            (
                error.code(),
                error.details().map(|details| details["failure"].clone()),
            ),
            (
                "artifact_download_failed",
                Some(serde_json::json!("digest_mismatch"))
            )
        );
    });
}
