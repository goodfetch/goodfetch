//! Nothing leaves the machine but pinned downloads (ADR 0043), against the
//! Proving ground: the Browser cache's first-use download and `browsers
//! install` send one plain HTTPS GET, which the Proving ground records with
//! no cookie, no identifier, no query beyond the pinned path and the user
//! agent `goodfetch-download`; under `egress.offline` a missing build is
//! refused before any connection, which the connection record shows, while
//! a copy prepared beforehand still runs. The `resolve` setting maps every
//! host name to the Proving ground's loopback address, whose certificate
//! authority the download trusts as `tls.trusted_cas` would: no test
//! reaches the internet.

use std::net::IpAddr;

use goodfetch_engine::browser_cache::{
    BrowserCache, BuildPlatform, CHROME_FOR_TESTING, CacheError, CacheLocation, PinnedArtifact,
    Pins,
};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::pinned_download::{Digest, Fetching};
use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, InstallOutcome, PinnedBy};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use serde_json::{Value, json};

/// The seed whose `files` family serves the archive below.
const SEED: u64 = 640;

/// The `files` family's ZIP archive, `/files/archive.zip` on `files.test`.
const ARCHIVE: &str = "/files/archive.zip";

/// The SHA-256 of that archive on [`SEED`], read once from the Proving
/// ground and pinned as a Product version pins a build.
const ARCHIVE_DIGEST: &str =
    "sha256:cf75a6eed592bffd2a6ec6f8bc2f9896b4aebb58eb47099b9b5cc0d5c65d82ec";

/// The digest of the files it holds (`readme.txt`), worked out by hand from
/// the Browser cache's tree rule.
const FILES_DIGEST: &str =
    "sha256:50c89b91fefe72c340bc2ded6b3675acc1d432b020c88607db1c19015a8b2084";

/// What `readme.txt` holds on [`SEED`].
const README: &str = "juniper keystone annual budget delta orbit northern orchard\n";

/// A host name only the `resolve` mapping knows, mapped to the Proving
/// ground's watched target.
const UNSERVED_HOST: &str = "builds.goodfetch.test";

fn pinned_by() -> PinnedBy {
    PinnedBy {
        product: Product::Collie,
        product_version: ProductVersion::parse("1.4.0").unwrap(),
    }
}

fn pin(build: &str, url: String) -> PinnedArtifact {
    PinnedArtifact {
        id: ArtifactId {
            kind: ArtifactKind::Browser,
            name: CHROME_FOR_TESTING.to_owned(),
            build: build.to_owned(),
        },
        platform: Some(BuildPlatform::LinuxX64),
        url,
        archive_digest: Some(ARCHIVE_DIGEST.parse::<Digest>().unwrap()),
        application_settings: std::collections::BTreeMap::new(),
        digest: FILES_DIGEST.parse::<Digest>().unwrap(),
    }
}

/// The archive's pin, on the Proving ground's port.
fn archive_pin(proving_ground: &ProvingGround) -> PinnedArtifact {
    pin(
        "archive",
        format!(
            "https://files.test:{}{ARCHIVE}",
            proving_ground.address().port()
        ),
    )
}

/// What a download may reach: every host name the Proving ground serves, and
/// [`UNSERVED_HOST`] at its watched target, under its certificate authority.
fn fetching(proving_ground: &ProvingGround, offline: Option<SettingsLayer>) -> Fetching {
    let description = proving_ground.description();
    let target: std::net::SocketAddr = description.fixtures.target.parse().unwrap();
    let resolve = description
        .resolve
        .iter()
        .fold(HostMapping::default(), |resolve, (host, address)| {
            resolve.with(host, vec![address.parse::<IpAddr>().unwrap()])
        })
        .with(UNSERVED_HOST, vec![target.ip()]);
    Fetching {
        offline,
        resolve,
        trusted_certificates: vec![CertificateDer::from_pem_file(&description.ca).unwrap()],
        ..Fetching::new(Clock::system())
    }
}

fn now(env: &TestEnv) -> jiff::Timestamp {
    jiff::Timestamp::try_from(env.start_time()).unwrap()
}

/// What a test reads of each request the Proving ground recorded.
fn requests(observations: &Value) -> Vec<Value> {
    ReceivedRequests::of(observations)
        .every()
        .map(|request| {
            json!({
                "host": request.host,
                "method": request.method,
                "target": request.target,
                "headers": request.headers,
                "cookies": request.cookies,
            })
        })
        .collect()
}

/// The plain GET ADR 0043 allows: the pinned path, and nothing about the
/// install but the user agent every install sends.
fn plain_get(proving_ground: &ProvingGround) -> Value {
    json!({
        "host": "files.test",
        "method": "GET",
        "target": ARCHIVE,
        "headers": [
            ["host", format!("files.test:{}", proving_ground.address().port())],
            ["user-agent", "goodfetch-download"],
        ],
        "cookies": [],
    })
}

/// The connections the record holds to the listener `name`.
fn connections_to(observations: &Value, name: &str) -> Vec<Value> {
    observations["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == name)
        .cloned()
        .collect()
}

/// Covers: story:4-81, row:SC-10
#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the file the download wrote, outside the runtime"
)]
fn a_first_use_download_sends_one_plain_get_naming_nothing_about_the_install() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None));
    let pinned = archive_pin(&proving_ground);

    let prepared = goodfetch_test_support::runtime()
        .block_on(cache.prepare(
            &pinned,
            &pinned_by(),
            &fetching(&proving_ground, None),
            now(&env),
            &mut |_| {},
        ))
        .unwrap();

    assert!(prepared.downloaded);
    assert_eq!(
        std::fs::read_to_string(prepared.artifact.files().join("readme.txt")).unwrap(),
        README
    );
    assert_eq!(
        requests(&proving_ground.observations().unwrap()),
        [plain_get(&proving_ground)]
    );
}

/// Covers: story:4-81, row:SDK-08
#[test]
fn browsers_install_sends_the_same_plain_get_for_each_pinned_build() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None));
    let pins = Pins {
        pinned_by: pinned_by(),
        artifacts: vec![archive_pin(&proving_ground)],
    };

    let installed = goodfetch_test_support::runtime()
        .block_on(cache.install(
            &pins,
            Some(BuildPlatform::LinuxX64),
            &fetching(&proving_ground, None),
            now(&env),
            &mut |_| {},
        ))
        .unwrap();

    assert_eq!(
        installed,
        [(pins.artifacts[0].id.clone(), InstallOutcome::Installed)]
    );
    assert_eq!(
        requests(&proving_ground.observations().unwrap()),
        [plain_get(&proving_ground)]
    );
}

/// A build missing under `egress.offline` is refused naming `browsers
/// install`, and nothing connects to where it would download from; the same
/// Product version's build in a copy prepared beforehand, as
/// `GOODFETCH_BROWSERS_PATH` points at, still runs (ADRs 0043 and 0115).
///
/// Covers: story:4-82, error:artifact_unavailable_offline, setting:egress.offline
#[test]
fn under_egress_offline_a_missing_build_is_refused_before_any_connection() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let runtime = goodfetch_test_support::runtime();
    let copy = BrowserCache::new(&CacheLocation::new(
        env.home(),
        Some(env.folder().join("prepared copy")),
    ));
    let prepared = archive_pin(&proving_ground);
    runtime
        .block_on(copy.install(
            &Pins {
                pinned_by: pinned_by(),
                artifacts: vec![prepared.clone()],
            },
            Some(BuildPlatform::LinuxX64),
            &fetching(&proving_ground, None),
            now(&env),
            &mut |_| {},
        ))
        .unwrap();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None));
    let missing = pin(
        "missing",
        format!(
            "https://{UNSERVED_HOST}:{}/chrome-linux64.zip",
            proving_ground
                .description()
                .fixtures
                .target
                .rsplit(':')
                .next()
                .unwrap()
        ),
    );
    let offline = fetching(&proving_ground, Some(SettingsLayer::OperatorPolicy));

    let refused = runtime
        .block_on(cache.prepare(&missing, &pinned_by(), &offline, now(&env), &mut |_| {}))
        .err()
        .unwrap();
    let from_copy = runtime
        .block_on(copy.prepare(&prepared, &pinned_by(), &offline, now(&env), &mut |_| {}))
        .unwrap();

    let CacheError::Offline { details, .. } = refused else {
        panic!("expected artifact_unavailable_offline, got {refused:?}");
    };
    assert_eq!(
        (details.limit.key.as_str(), details.limit.layer),
        ("egress.offline", SettingsLayer::OperatorPolicy)
    );
    assert!(
        details
            .fix
            .starts_with("Run `goodfetch-collie browser install` beforehand"),
        "{}",
        details.fix
    );
    assert!(!from_copy.downloaded);
    assert_eq!(
        connections_to(&proving_ground.observations().unwrap(), "target"),
        Vec::<Value>::new()
    );
}

/// The names of the spans an exporting process ends, kept in memory.
#[derive(Clone, Debug, Default)]
struct Collector(std::sync::Arc<std::sync::Mutex<Vec<String>>>);

impl opentelemetry_sdk::trace::SpanExporter for Collector {
    fn export(
        &self,
        batch: Vec<opentelemetry_sdk::trace::SpanData>,
    ) -> impl std::future::Future<Output = opentelemetry_sdk::error::OTelSdkResult> + Send {
        self.0
            .lock()
            .unwrap()
            .extend(batch.into_iter().map(|span| span.name.into_owned()));
        std::future::ready(Ok(()))
    }
}

/// While the engine exports its spans, a download made in a request's span
/// sends the same plain GET: no `traceparent`, `tracestate` or `baggage`
/// header reaches a site from the engine's HTTP client (ADR 0132).
///
/// Covers: story:3-146
#[test]
fn with_spans_exported_a_download_in_a_requests_span_sends_no_trace_header() {
    use opentelemetry::trace::TracerProvider as _;
    use tracing::Instrument as _;
    use tracing_subscriber::Layer as _;
    use tracing_subscriber::layer::SubscriberExt as _;

    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let cache = BrowserCache::new(&CacheLocation::new(env.home(), None));
    let pinned = archive_pin(&proving_ground);
    let collector = Collector::default();
    let provider = opentelemetry_sdk::trace::SdkTracerProvider::builder()
        .with_simple_exporter(collector.clone())
        .build();
    let subscriber = tracing_subscriber::registry().with(
        tracing_opentelemetry::layer()
            .with_tracer(provider.tracer("goodfetch"))
            .with_filter(tracing_subscriber::filter::filter_fn(|metadata| {
                metadata.is_span()
            })),
    );

    tracing::subscriber::with_default(subscriber, || {
        let request = tracing::info_span!("request", otel.kind = "server");
        goodfetch_test_support::runtime()
            .block_on(
                cache
                    .prepare(
                        &pinned,
                        &pinned_by(),
                        &fetching(&proving_ground, None),
                        now(&env),
                        &mut |_| {},
                    )
                    .instrument(request),
            )
            .unwrap();
    });
    provider.shutdown().unwrap();

    assert!(
        collector.0.lock().unwrap().contains(&"request".to_owned()),
        "the request's span was not exported"
    );
    assert_eq!(
        requests(&proving_ground.observations().unwrap()),
        [plain_get(&proving_ground)]
    );
}
