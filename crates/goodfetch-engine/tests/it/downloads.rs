//! The Browser cache's pinned downloads against a local HTTPS stand-in that
//! the `resolve` setting maps the pinned host to (ADRs 0043 and 0059): no
//! test reaches the internet. The stand-in's certificate authority is
//! trusted as `trusted_cas` would trust one.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;

use goodfetch_engine::browser_cache::{
    BrowserCache, BuildPlatform, CHROME_FOR_TESTING, CacheError, CacheLocation, MaxUnusedAge,
    PinnedArtifact, Pins, content_digest,
};
use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::browser_cache::{
    ArtifactId, ArtifactKind, DownloadFailure, DownloadProgress, InstallOutcome, PinnedBy,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_test_support::TestEnv;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use tokio::sync::mpsc;

const HOST: &str = "builds.goodfetch.test";

/// A request the stand-in received: its path and the names of its headers.
#[derive(Debug, PartialEq, Eq)]
struct Received {
    path: String,
    headers: Vec<String>,
}

/// An HTTPS server on loopback that serves fixed files and records what it
/// was asked.
struct StandIn {
    port: u16,
    authority: CertificateDer<'static>,
    received: mpsc::Receiver<Received>,
}

impl StandIn {
    #[expect(
        clippy::disallowed_methods,
        reason = "the stand-in server, outside the engine, runs its accept loop and connections on the test's runtime (ADR 0062)"
    )]
    async fn serve(files: BTreeMap<&'static str, Vec<u8>>) -> Self {
        let authority_key =
            rcgen::KeyPair::generate().expect("invariant: the stand-in and its fixtures set up");
        let mut authority_params = rcgen::CertificateParams::new(Vec::<String>::new())
            .expect("invariant: the stand-in and its fixtures set up");
        authority_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
        let authority = authority_params
            .self_signed(&authority_key)
            .expect("invariant: the stand-in and its fixtures set up");
        let issuer = rcgen::Issuer::new(authority_params, authority_key);
        let leaf_key =
            rcgen::KeyPair::generate().expect("invariant: the stand-in and its fixtures set up");
        let leaf = rcgen::CertificateParams::new(vec![HOST.to_owned()])
            .expect("invariant: the stand-in and its fixtures set up")
            .signed_by(&leaf_key, &issuer)
            .expect("invariant: the stand-in and its fixtures set up");
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ServerConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("invariant: the stand-in and its fixtures set up")
            .with_no_client_auth()
            .with_single_cert(
                vec![leaf.der().clone()],
                PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(leaf_key.serialize_der())),
            )
            .expect("invariant: the stand-in and its fixtures set up");
        let acceptor = tokio_rustls::TlsAcceptor::from(Arc::new(config));
        let listener =
            tokio::net::TcpListener::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
                .await
                .expect("invariant: the stand-in and its fixtures set up");
        let port = listener
            .local_addr()
            .expect("invariant: the stand-in and its fixtures set up")
            .port();
        let (record, received) = mpsc::channel(16);
        let files = Arc::new(files);
        tokio::spawn(async move {
            loop {
                let (tcp, _) = listener
                    .accept()
                    .await
                    .expect("invariant: the stand-in and its fixtures set up");
                let Ok(tls) = acceptor.accept(tcp).await else {
                    continue;
                };
                let (files, record) = (Arc::clone(&files), record.clone());
                let service =
                    hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
                        let (files, record) = (Arc::clone(&files), record.clone());
                        async move {
                            let path = request.uri().path().to_owned();
                            let headers = request
                                .headers()
                                .keys()
                                .map(|name| name.as_str().to_owned())
                                .collect();
                            let _ = record
                                .send(Received {
                                    path: path.clone(),
                                    headers,
                                })
                                .await;
                            let response = if let Some(bytes) = files.get(path.as_str()) {
                                Response::new(Full::new(Bytes::from(bytes.clone())))
                            } else {
                                let mut missing = Response::new(Full::new(Bytes::new()));
                                *missing.status_mut() = StatusCode::NOT_FOUND;
                                missing
                            };
                            Ok::<_, std::convert::Infallible>(response)
                        }
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
            received,
        }
    }

    fn url(&self, path: &str) -> String {
        format!("https://{HOST}:{}{path}", self.port)
    }

    fn fetching(&self) -> Fetching {
        Fetching {
            offline: None,
            resolve: HostMapping::default().with(HOST, vec![IpAddr::V4(Ipv4Addr::LOCALHOST)]),
            trusted_certificates: vec![self.authority.clone()],
            ..Fetching::new(Clock::system())
        }
    }

    fn requests(&mut self) -> Vec<Received> {
        let mut requests = Vec::new();
        while let Ok(request) = self.received.try_recv() {
            requests.push(request);
        }
        requests
    }
}

/// A build's files, its ZIP archive and the digests a pin names.
struct Build {
    archive: Vec<u8>,
    archive_digest: Digest,
    digest: Digest,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test lays out a build's files by hand, outside any async worker"
)]
fn build(env: &TestEnv, marker: &str) -> Build {
    let tree = env.folder().join(format!("build {marker}"));
    std::fs::create_dir_all(tree.join("resources"))
        .expect("invariant: the stand-in and its fixtures set up");
    std::fs::write(tree.join("chrome"), format!("#!chrome {marker}\n"))
        .expect("invariant: the stand-in and its fixtures set up");
    std::fs::write(tree.join("resources/locale.pak"), "é\n")
        .expect("invariant: the stand-in and its fixtures set up");
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);
    archive
        .start_file("chrome", options)
        .expect("invariant: the stand-in and its fixtures set up");
    archive
        .write_all(format!("#!chrome {marker}\n").as_bytes())
        .expect("invariant: the stand-in and its fixtures set up");
    archive
        .add_directory("resources/", options)
        .expect("invariant: the stand-in and its fixtures set up");
    archive
        .start_file("resources/locale.pak", options)
        .expect("invariant: the stand-in and its fixtures set up");
    archive
        .write_all("é\n".as_bytes())
        .expect("invariant: the stand-in and its fixtures set up");
    let archive = archive
        .finish()
        .expect("invariant: the stand-in and its fixtures set up")
        .into_inner();
    Build {
        archive_digest: Digest::of(&archive),
        digest: content_digest(&tree).expect("invariant: the stand-in and its fixtures set up"),
        archive,
    }
}

fn pinned_by() -> PinnedBy {
    PinnedBy {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0")
            .expect("invariant: the stand-in and its fixtures set up"),
    }
}

fn pin(build_id: &str, url: String, build: &Build) -> PinnedArtifact {
    PinnedArtifact {
        id: ArtifactId {
            kind: ArtifactKind::Browser,
            name: CHROME_FOR_TESTING.to_owned(),
            build: build_id.to_owned(),
        },
        platform: Some(BuildPlatform::LinuxX64),
        url,
        archive_digest: Some(build.archive_digest),
        application_settings: std::collections::BTreeMap::new(),
        digest: build.digest,
    }
}

fn now(env: &TestEnv) -> jiff::Timestamp {
    jiff::Timestamp::try_from(env.start_time())
        .expect("invariant: the stand-in and its fixtures set up")
}

fn cache(env: &TestEnv) -> BrowserCache {
    BrowserCache::new(
        &CacheLocation::new(env.home(), None),
        TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
    )
}

fn failure_of(error: CacheError) -> (String, DownloadFailure) {
    let CacheError::DownloadFailed { details, .. } = error else {
        panic!("expected artifact_download_failed, got {error:?}");
    };
    (details.host, details.failure)
}

fn entry_folder(cache: &BrowserCache, id: &ArtifactId) -> std::path::PathBuf {
    cache
        .root()
        .join(id.kind.as_str())
        .join(&id.name)
        .join(&id.build)
}

fn launch_path(cache: &BrowserCache, id: &ArtifactId) -> std::path::PathBuf {
    entry_folder(cache, id).join("files").join("chrome")
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test reads a file the engine wrote, outside any async worker"
)]
fn read(path: &Path) -> String {
    std::fs::read_to_string(path).expect("invariant: the stand-in and its fixtures set up")
}

/// Covers: row:SDK-08, story:3-140, event:browser_cache.download_progress
#[test]
fn first_use_downloads_a_missing_build_with_a_plain_get_and_reports_its_progress() {
    let env = TestEnv::new();
    let build = build(&env, "a");
    goodfetch_test_support::runtime().block_on(async {
        let mut stand_in =
            StandIn::serve(BTreeMap::from([("/chrome-a.zip", build.archive.clone())])).await;
        let cache = cache(&env);
        let pinned = pin("a", stand_in.url("/chrome-a.zip"), &build);
        let mut reported = Vec::new();

        let prepared = cache
            .prepare(
                &pinned,
                &pinned_by(),
                &stand_in.fetching(),
                now(&env),
                &mut |progress: DownloadProgress| {
                    reported.push(progress);
                },
            )
            .await
            .unwrap();

        assert!(prepared.downloaded);
        assert_eq!(
            read(&prepared.artifact.files().join("chrome")),
            "#!chrome a\n"
        );
        assert_eq!(
            stand_in.requests(),
            [Received {
                path: "/chrome-a.zip".to_owned(),
                headers: vec!["host".to_owned(), "user-agent".to_owned()],
            }]
        );
        let total = u64::try_from(build.archive.len()).unwrap();
        assert_eq!(
            reported,
            [0, total].map(|received_bytes| DownloadProgress {
                artifact: pinned.id.clone(),
                host: HOST.to_owned(),
                received_bytes,
                total_bytes: Some(total),
            })
        );
    });
}

/// Under `BlockingPolicy::Inline`, the Simulation's and the in-process
/// tests', a download creates, writes and syncs its archive, then unpacks
/// it, on no thread of the runtime's blocking pool, here held by another
/// job: work on such a thread lets the runtime's other tasks run for as long
/// as the thread takes, so a run would replay by the host's load, not its
/// seed (issues 2106 and 2132).
#[test]
fn a_download_under_inline_blocking_takes_no_thread_of_the_runtime() {
    let env = TestEnv::new();
    let build = build(&env, "a");

    let (downloaded, held_throughout) = goodfetch_test_support::runtime_with_one_blocking_thread()
        .block_on(async {
            let stand_in =
                StandIn::serve(BTreeMap::from([("/chrome-a.zip", build.archive.clone())])).await;
            let cache = cache(&env);
            let pinned = pin("a", stand_in.url("/chrome-a.zip"), &build);
            goodfetch_test_support::with_the_blocking_pool_held(cache.prepare(
                &pinned,
                &pinned_by(),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            ))
            .await
        });

    assert_eq!(
        (
            downloaded
                .map(|prepared| prepared.downloaded)
                .map_err(|error| format!("{error:?}")),
            held_throughout
        ),
        (Ok(true), true),
        "the download waited for the blocking pool"
    );
}

/// Covers: row:SDK-08
#[test]
fn browsers_install_fetches_each_pinned_build_once() {
    let env = TestEnv::new();
    let (first, second) = (build(&env, "a"), build(&env, "b"));
    goodfetch_test_support::runtime().block_on(async {
        let mut stand_in = StandIn::serve(BTreeMap::from([
            ("/chrome-a.zip", first.archive.clone()),
            ("/chrome-b.zip", second.archive.clone()),
        ]))
        .await;
        let cache = cache(&env);
        let mut for_mac = pin("b", stand_in.url("/chrome-b.zip"), &second);
        for_mac.platform = Some(BuildPlatform::MacArm64);
        let pins = Pins {
            pinned_by: pinned_by(),
            artifacts: vec![pin("a", stand_in.url("/chrome-a.zip"), &first), for_mac],
        };

        let installed = cache
            .install(
                &pins,
                Some(BuildPlatform::LinuxX64),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            )
            .await
            .unwrap();
        let again = cache
            .install(
                &pins,
                Some(BuildPlatform::LinuxX64),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            )
            .await
            .unwrap();

        let id = pins.artifacts[0].id.clone();
        assert_eq!(installed, [(id.clone(), InstallOutcome::Installed)]);
        assert_eq!(again, [(id.clone(), InstallOutcome::AlreadyInstalled)]);
        assert_eq!(stand_in.requests().len(), 1);
        assert_eq!(read(&launch_path(&cache, &id)), "#!chrome a\n");
    });
}

/// Covers: error:artifact_download_failed
#[test]
fn a_download_answered_with_an_error_status_fails_naming_the_host_and_status() {
    let env = TestEnv::new();
    let build = build(&env, "a");
    goodfetch_test_support::runtime().block_on(async {
        let stand_in = StandIn::serve(BTreeMap::new()).await;
        let cache = cache(&env);
        let pinned = pin("a", stand_in.url("/gone.zip"), &build);

        let failed = cache
            .prepare(
                &pinned,
                &pinned_by(),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            )
            .await
            .err()
            .unwrap();

        assert_eq!(
            failure_of(failed),
            (HOST.to_owned(), DownloadFailure::HttpStatus { status: 404 })
        );
        assert!(!entry_folder(&cache, &pinned.id).exists());
    });
}

/// Covers: error:artifact_download_failed
#[test]
fn a_download_whose_bytes_are_not_the_pinned_ones_is_refused() {
    let env = TestEnv::new();
    let (pinned_build, served_build) = (build(&env, "a"), build(&env, "tampered"));
    goodfetch_test_support::runtime().block_on(async {
        let stand_in = StandIn::serve(BTreeMap::from([(
            "/chrome-a.zip",
            served_build.archive.clone(),
        )]))
        .await;
        let cache = cache(&env);
        let pinned = pin("a", stand_in.url("/chrome-a.zip"), &pinned_build);

        let failed = cache
            .prepare(
                &pinned,
                &pinned_by(),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            )
            .await
            .err()
            .unwrap();

        assert_eq!(
            failure_of(failed),
            (HOST.to_owned(), DownloadFailure::DigestMismatch)
        );
        assert!(!entry_folder(&cache, &pinned.id).exists());
    });
}

/// A downloaded archive whose bytes are the pinned ones but whose files
/// cannot be written out, as on a full disk, fails as the cache's own
/// failure, naming the OS's reason, not as a tampered download.
///
/// Covers: story:3-140
#[test]
fn an_archive_that_cannot_be_unpacked_is_the_caches_failure_not_a_tampered_download() {
    let env = TestEnv::new();
    let pinned_build = build(&env, "a");
    // `chrome` is a file, then a folder: the OS refuses the second.
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    archive.start_file("chrome", options).unwrap();
    archive.write_all(b"#!chrome\n").unwrap();
    archive.start_file("chrome/locale.pak", options).unwrap();
    archive.write_all(b"\n").unwrap();
    let archive = archive.finish().unwrap().into_inner();
    let served = Build {
        archive_digest: Digest::of(&archive),
        digest: pinned_build.digest,
        archive,
    };
    goodfetch_test_support::runtime().block_on(async {
        let stand_in =
            StandIn::serve(BTreeMap::from([("/chrome-a.zip", served.archive.clone())])).await;
        let cache = cache(&env);
        let pinned = pin("a", stand_in.url("/chrome-a.zip"), &served);

        let failed = cache
            .prepare(
                &pinned,
                &pinned_by(),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            )
            .await
            .err()
            .unwrap();

        assert_eq!(
            (failed.code(), entry_folder(&cache, &pinned.id).exists()),
            ("internal_error", false),
            "{failed:?}"
        );
    });
}

/// Covers: error:artifact_download_failed
#[test]
fn a_download_nothing_answers_fails_on_the_network_and_is_retryable() {
    let env = TestEnv::new();
    let build = build(&env, "a");
    goodfetch_test_support::runtime().block_on(async {
        let stand_in = StandIn::serve(BTreeMap::new()).await;
        // A loopback port nothing listens on: the stand-in's, once closed, is
        // not reliably free, so the pin names port 1.
        let pinned = pin("a", format!("https://{HOST}:1/chrome-a.zip"), &build);
        let cache = cache(&env);

        let failed = cache
            .prepare(
                &pinned,
                &pinned_by(),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            )
            .await
            .err()
            .unwrap();

        assert_eq!(failed.code(), "artifact_download_failed");
        assert_eq!(
            failure_of(failed),
            (HOST.to_owned(), DownloadFailure::Network)
        );
    });
}

/// Covers: story:3-140
#[test]
fn the_daily_clean_up_after_a_download_deletes_an_unpinned_build_unused_for_the_age() {
    let env = TestEnv::new();
    let (old_build, new_build) = (build(&env, "old"), build(&env, "new"));
    goodfetch_test_support::runtime().block_on(async {
        let stand_in = StandIn::serve(BTreeMap::from([
            ("/chrome-old.zip", old_build.archive.clone()),
            ("/chrome-new.zip", new_build.archive.clone()),
        ]))
        .await;
        let cache = cache(&env);
        let old = pin("old", stand_in.url("/chrome-old.zip"), &old_build);
        let new = pin("new", stand_in.url("/chrome-new.zip"), &new_build);
        let forty_days_ago = now(&env) - jiff::SignedDuration::from_hours(40 * 24);
        cache
            .prepare(
                &old,
                &pinned_by(),
                &stand_in.fetching(),
                forty_days_ago,
                &mut |_| {},
            )
            .await
            .unwrap();

        let prepared = cache
            .prepare(
                &new,
                &pinned_by(),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            )
            .await
            .unwrap();
        let report = cache
            .clean_up_daily(
                BTreeSet::from([new.id.clone()]),
                now(&env),
                MaxUnusedAge::default(),
            )
            .await
            .unwrap();

        assert!(prepared.downloaded);
        assert_eq!(
            report.map(|report| report.deleted),
            Some(vec![old.id.clone()])
        );
        assert!(launch_path(&cache, &new.id).exists());
        assert!(!entry_folder(&cache, &old.id).exists());
    });
}

/// A pin whose URL names no ZIP archive downloads a single executable, as
/// Lightpanda ships its builds: it is placed under the URL's last segment,
/// executable, its digest the tree of that one file, so the light-browser
/// tier runs it from the cache (ADR 0020).
///
/// Covers: row:SDK-08, story:5-120
#[test]
fn a_pinned_single_executable_is_placed_under_its_name_and_runs() {
    let env = TestEnv::new();
    let bytes = b"#!/bin/sh\necho light\n".to_vec();
    let tree = env.folder().join("single");
    write_file(&tree.join("lightpanda-x86_64-linux"), &bytes);
    let digest = content_digest(&tree).expect("invariant: the stand-in and its fixtures set up");
    goodfetch_test_support::runtime().block_on(async {
        let stand_in = StandIn::serve(BTreeMap::from([(
            "/lightpanda-x86_64-linux",
            bytes.clone(),
        )]))
        .await;
        let cache = cache(&env);
        let pinned = PinnedArtifact {
            id: ArtifactId {
                kind: ArtifactKind::Browser,
                name: "lightpanda".to_owned(),
                build: "1".to_owned(),
            },
            platform: Some(BuildPlatform::LinuxX64),
            url: stand_in.url("/lightpanda-x86_64-linux"),
            archive_digest: Some(Digest::of(&bytes)),
            application_settings: BTreeMap::new(),
            digest,
        };

        let prepared = cache
            .prepare(
                &pinned,
                &pinned_by(),
                &stand_in.fetching(),
                now(&env),
                &mut |_| {},
            )
            .await
            .unwrap();

        let placed = prepared.artifact.files().join("lightpanda-x86_64-linux");
        assert_eq!(read(&placed), "#!/bin/sh\necho light\n");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            #[expect(
                clippy::disallowed_methods,
                reason = "the test reads a file the engine wrote, outside any async worker"
            )]
            let mode = std::fs::metadata(&placed).unwrap().permissions().mode();
            assert_eq!(mode & 0o111, 0o111, "{mode:o}");
        }
    });
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test lays out a build's files by hand, outside any async worker"
)]
fn write_file(path: &Path, bytes: &[u8]) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, bytes).unwrap();
}
