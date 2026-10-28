//! Trust on first use on the engine's own HTTP client, against the Proving
//! ground (ADR 0042, pitfall H03): a domain listed in
//! `tls.accept_invalid_certs` without a pin accepts its host's invalid
//! certificate once, pins the SHA-256 of its public key in the user's
//! known-certificates store and records the first pin in the Audit log;
//! every response fetched under the exception carries its kind and the
//! certificate's hash, no Session cookie goes to the host and no Secret
//! reference resolves for it; a pin row the engine cannot read stops the
//! host as a changed key. The `resolve` setting maps every host name to
//! the Proving ground's loopback address, which the Session's private
//! ranges list: no test reaches the internet.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::home::Home;
use goodfetch_engine::http_client::{
    Failure, FetchError, PageTraffic, Resolve, Response, SessionCookie, Waiter,
};
use goodfetch_engine::known_certificates::{ExceptionKind, KeyHash, KnownCertificates};
use goodfetch_engine::secrets::{Carried, Keychain, Purpose, RunSecrets, SecretError, Secrets};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::{
    CertificateChanged, CertificateExceptions, ClientCertificates, ExceptionUsed, PinBook, Trust,
};
use goodfetch_protocol::audit::AuditVerified;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rusqlite::Connection;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::pem::PemObject as _;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

/// Any seed: the families used here are served on every one.
const SEED: u64 = 628;

/// The person an engine without accounts acts for.
const PERSON: &str = "os_user";

/// The expired host of the Proving ground's `certificates` family.
const EXPIRED: &str = "expired.certificates.test";

/// A host whose certificate the Proving ground's authority makes valid,
/// once that authority is trusted for it.
const VERIFIED: &str = "scope.test";

/// What one test holds: its folder, the Proving ground, the user's stores
/// and the Run's Secrets.
/// Its fields drop in order, the Proving ground before the folder whose
/// teardown checks that nothing it started is still running.
struct Fixture {
    proving_ground: ProvingGround,
    audit: Arc<AuditLog>,
    store: KnownCertificates,
    owner: Arc<TaskOwner>,
    secrets: RunSecrets,
    env: TestEnv,
}

fn fixture() -> Fixture {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let home = Home::new(env.home().to_path_buf());
    let audit = Arc::new(AuditLog::of(&home, None));
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let environment: BTreeMap<OsString, OsString> = [(
        OsString::from("SITE_PASSWORD"),
        OsString::from("s3cr3t-value"),
    )]
    .into();
    let secrets = Secrets::new(
        Carried {
            environment,
            keychain: Keychain::absent(),
            home: home.clone(),
        },
        Arc::clone(&audit),
        Arc::clone(&owner),
        Clock::system(),
    )
    .for_run(Duration::from_secs(30));
    Fixture {
        store: KnownCertificates::of(&home, None),
        env,
        proving_ground,
        audit,
        owner,
        secrets,
    }
}

impl Fixture {
    /// The run's traffic: the Proving ground's authority trusted for
    /// `scope.test`, each of `listed` under trust on first use, and the
    /// Session's `session` cookie for both domains.
    fn traffic(&self, listed: &[&str]) -> PageTraffic {
        let accept_invalid_certs = Value::Object(
            listed
                .iter()
                .map(|domain| ((*domain).to_owned(), Value::Null))
                .collect(),
        );
        self.run_traffic(&accept_invalid_certs, "first-use")
    }

    /// The traffic of the Run `run_id`, under `accept_invalid_certs`, the
    /// setting's resolved value, as [`Self::traffic`] builds it otherwise.
    fn run_traffic(&self, accept_invalid_certs: &Value, run_id: &str) -> PageTraffic {
        let description = self.proving_ground.description();
        let resolve =
            description
                .resolve
                .iter()
                .fold(Resolve::new(), |resolve, (host, address)| {
                    resolve.with(host.clone(), vec![address.parse::<IpAddr>().unwrap()])
                });
        let authority = CertificateDer::from_pem_file(&description.ca).unwrap();
        let book = PinBook {
            store: self.store.clone(),
            person: PERSON.to_owned(),
            run_id: Some(RunId::parse(run_id).unwrap()),
            audit: Arc::clone(&self.audit),
            owner: Arc::clone(&self.owner),
            clock: Clock::system(),
        };
        PageTraffic {
            egress: Policy::new(&["*"], &[] as &[&str])
                .unwrap()
                .with_private_ranges(&[self.proving_ground.address().ip().to_string()])
                .unwrap(),
            resolve,
            local: None,
            proxy: None,
            trust: Trust::default_roots()
                .with_trusted_cas(VERIFIED, vec![authority])
                .unwrap(),
            exceptions: CertificateExceptions::listed(
                accept_invalid_certs,
                book,
                self.secrets.exception_hosts(),
            )
            .unwrap(),
            client_certificates: ClientCertificates::default(),
            cookies: [VERIFIED, "certificates.test"]
                .into_iter()
                .map(|domain| SessionCookie {
                    domain: domain.to_owned(),
                    name: "session".to_owned(),
                    value: "c00k1e".to_owned(),
                })
                .collect(),
            clock: Clock::system(),
            delays: Delays::BUILT_IN,
        }
    }

    fn url(&self, host: &str) -> String {
        format!("https://{host}:{}/", self.proving_ground.address().port())
    }

    /// The requests the Proving ground received: host and cookies.
    fn requests(&self) -> Vec<Value> {
        requests_of(&self.proving_ground)
    }

    /// The Audit log's entries, each its kind, Run and details.
    fn audit_entries(&self) -> Vec<Value> {
        Connection::open(self.audit.path())
            .unwrap()
            .prepare("SELECT entry FROM entries ORDER BY sequence")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|entry| {
                let entry: Value = serde_json::from_str(&entry.unwrap()).unwrap();
                json!([entry["kind"], entry["run_id"], entry["details"]])
            })
            .collect()
    }
}

/// The requests `proving_ground` received: host and cookies.
fn requests_of(proving_ground: &ProvingGround) -> Vec<Value> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .map(|request| json!([request["host"], request["cookies"]]))
        .collect()
}

/// A Proving ground of another seed beside the fixture's: the same host
/// names, each leaf's key another, as a fixture that regenerates its
/// certificate, or an attacker on the path, presents.
fn another_key(fixture: &Fixture) -> ProvingGround {
    ProvingGround::start(&fixture.env, SEED + 1).unwrap()
}

/// `host`'s URL on `proving_ground`.
fn url_on(proving_ground: &ProvingGround, host: &str) -> String {
    format!("https://{host}:{}/", proving_ground.address().port())
}

/// Fetches `url` and answers its status with the exception it was
/// fetched under.
async fn fetched(
    traffic: &PageTraffic,
    url: &str,
) -> Result<(u16, Option<ExceptionUsed>), FetchError> {
    traffic
        .get(url, Waiter::Call)
        .await
        .map(|response: Response| (response.status(), response.exception().cloned()))
}

/// The SHA-256 of the public key of the leaf `host` presents, read apart
/// from the engine: a TLS client that accepts any certificate, keeps the
/// leaf, and hashes its `SubjectPublicKeyInfo`.
async fn presented_key(proving_ground: &ProvingGround, host: &str) -> String {
    #[derive(Debug)]
    struct Keeping(
        Mutex<Option<CertificateDer<'static>>>,
        Arc<rustls::crypto::CryptoProvider>,
    );
    impl ServerCertVerifier for Keeping {
        fn verify_server_cert(
            &self,
            end_entity: &CertificateDer<'_>,
            _: &[CertificateDer<'_>],
            _: &ServerName<'_>,
            _: &[u8],
            _: UnixTime,
        ) -> Result<ServerCertVerified, rustls::Error> {
            *self.0.lock().unwrap() = Some(end_entity.clone().into_owned());
            Ok(ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _: &[u8],
            _: &CertificateDer<'_>,
            _: &DigitallySignedStruct,
        ) -> Result<HandshakeSignatureValid, rustls::Error> {
            Ok(HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
            self.1.signature_verification_algorithms.supported_schemes()
        }
    }
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let keeping = Arc::new(Keeping(Mutex::new(None), Arc::clone(&provider)));
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .unwrap()
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&keeping) as Arc<dyn ServerCertVerifier>)
        .with_no_client_auth();
    let tcp = tokio::net::TcpStream::connect(proving_ground.address())
        .await
        .unwrap();
    let _tls = tokio_rustls::TlsConnector::from(Arc::new(config))
        .connect(ServerName::try_from(host.to_owned()).unwrap(), tcp)
        .await
        .unwrap();
    let leaf = keeping.0.lock().unwrap().take().unwrap();
    let parsed = rustls::server::ParsedCertificate::try_from(&leaf).unwrap();
    let digest = Sha256::digest(parsed.subject_public_key_info().as_ref());
    format!("sha256/{}", data_encoding::BASE64.encode(&digest))
}

/// Story 4-35: the expired host fails with `certificate_invalid` while its
/// domain is not listed; once listed for trust on first use, the first
/// connection succeeds and pins the key it presented, in `certs list`'s
/// store and in the Audit log, which still verifies; the response carries
/// the exception's kind and the certificate's hash, and a second
/// connection is let through by the pin.
/// Covers: story:4-35, setting:tls.accept_invalid_certs, format:known-certificates
#[test]
fn a_listed_domains_invalid_certificate_is_accepted_once_and_pinned() {
    let fixture = fixture();
    let unlisted = fixture.traffic(&[]);
    let listed = fixture.traffic(&["certificates.test"]);
    let runtime = goodfetch_test_support::runtime();

    let (refused, first, second, presented) = runtime.block_on(async {
        let refused = fetched(&unlisted, &fixture.url(EXPIRED)).await;
        let first = fetched(&listed, &fixture.url(EXPIRED)).await.unwrap();
        let second = fetched(&listed, &fixture.url(EXPIRED)).await.unwrap();
        let presented = presented_key(&fixture.proving_ground, EXPIRED).await;
        (refused, first, second, presented)
    });

    let Err(FetchError::Failed(Failure::CertificateInvalid(refused))) = refused else {
        panic!("an unlisted domain's expired certificate is refused: {refused:?}");
    };
    assert_eq!(
        refused.failure,
        CertificateFailure::Known(KnownCertificateFailure::Expired)
    );
    assert_eq!(
        (
            first.0,
            first
                .1
                .as_ref()
                .map(|used| (used.kind, used.certificate.to_string(), used.first_pin))
        ),
        (
            200,
            Some((ExceptionKind::TrustOnFirstUse, presented.clone(), true))
        )
    );
    assert_eq!(
        (
            second.0,
            second
                .1
                .as_ref()
                .map(|used| (used.kind, used.certificate.to_string(), used.first_pin))
        ),
        (
            200,
            Some((ExceptionKind::TrustOnFirstUse, presented.clone(), false))
        )
    );
    let pins = fixture.store.list(PERSON).unwrap();
    assert_eq!(
        pins.iter()
            .map(|pin| (
                pin.host.as_str(),
                pin.spki_sha256.clone(),
                pin.run_id.clone()
            ))
            .collect::<Vec<_>>(),
        [(
            EXPIRED,
            Some(presented.clone()),
            Some(RunId::parse("first-use").unwrap())
        )]
    );
    let entries: Vec<Value> = Connection::open(fixture.audit.path())
        .unwrap()
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| serde_json::from_str(&entry.unwrap()).unwrap())
        .collect();
    assert_eq!(
        entries
            .iter()
            .map(|entry| (
                entry["kind"].clone(),
                entry["run_id"].clone(),
                entry["details"].clone()
            ))
            .collect::<Vec<_>>(),
        [(
            json!("certificate_pinned"),
            json!("first-use"),
            json!({ "host": EXPIRED, "spki_sha256": presented, "failure": "expired" }),
        )]
    );
    assert_eq!(
        fixture.audit.verify().unwrap(),
        AuditVerified {
            intact: true,
            entries: 1,
            first_broken: None,
        }
    );
}

/// Story 4-38: under the exception the engine sends the host no Session
/// cookie and resolves no Secret reference for it, refused with
/// `secret_input_refused`; the same cookie and the same reference go to a
/// verified host (positive control).
/// Covers: story:4-38, error:secret_input_refused
#[test]
fn a_host_reached_through_an_exception_gets_no_cookie_and_no_secret() {
    let fixture = fixture();
    let listed = fixture.traffic(&["certificates.test"]);
    let reference = SecretReference::parse("env:SITE_PASSWORD").unwrap();
    let purpose = |domain| Purpose {
        placeholder: "%password%",
        domain: Some(domain),
        domains: &[],
        layer: None,
        run_id: None,
    };
    let runtime = goodfetch_test_support::runtime();

    let (verified, excepted, to_verified, to_excepted) = runtime.block_on(async {
        let verified = fetched(&listed, &fixture.url(VERIFIED)).await.unwrap();
        let excepted = fetched(&listed, &fixture.url(EXPIRED)).await.unwrap();
        let to_verified = fixture
            .secrets
            .resolve(&reference, &purpose(VERIFIED))
            .await;
        let to_excepted = fixture.secrets.resolve(&reference, &purpose(EXPIRED)).await;
        (verified, excepted, to_verified, to_excepted)
    });

    assert_eq!((verified.0, verified.1.is_none()), (200, true));
    assert_eq!((excepted.0, excepted.1.is_some()), (200, true));
    assert_eq!(
        fixture.requests(),
        [
            json!([VERIFIED, [["session", "c00k1e"]]]),
            json!([EXPIRED, []]),
        ]
    );
    assert!(to_verified.is_ok(), "{to_verified:?}");
    let Err(SecretError::InputRefused {
        placeholder,
        domain,
        ..
    }) = to_excepted
    else {
        panic!("no Secret resolves for a host reached through an exception: {to_excepted:?}");
    };
    assert_eq!(
        (placeholder.as_str(), domain.as_str()),
        ("%password%", EXPIRED)
    );
}

/// Story 4-43: a pin row damaged on disk, its key's bytes cut short, makes
/// the host stop as a changed key before anything is sent, never let
/// through as unpinned.
/// Covers: story:4-43, error:certificate_changed, format:known-certificates
#[test]
fn a_damaged_pin_row_stops_the_host_as_a_changed_key() {
    let fixture = fixture();
    let listed = fixture.traffic(&["certificates.test"]);
    let runtime = goodfetch_test_support::runtime();

    let (first, damaged, presented) = runtime.block_on(async {
        let first = fetched(&listed, &fixture.url(EXPIRED)).await.unwrap();
        Connection::open(fixture.store.path())
            .unwrap()
            .execute("UPDATE pins SET spki_sha256 = x'0102'", [])
            .unwrap();
        let damaged = fetched(&listed, &fixture.url(EXPIRED)).await;
        let presented = presented_key(&fixture.proving_ground, EXPIRED).await;
        (first, damaged, presented)
    });

    assert_eq!(first.0, 200);
    let Err(FetchError::Failed(Failure::CertificateChanged(changed))) = damaged else {
        panic!("a damaged pin stops the host: {damaged:?}");
    };
    assert_eq!(
        (
            changed.host.as_str(),
            changed.pinned,
            changed.presented.to_string()
        ),
        (EXPIRED, None::<KeyHash>, presented.clone())
    );
    assert_eq!(
        answered(&changed),
        (
            "certificate_changed",
            json!({ "host": EXPIRED, "presented_spki_sha256": presented, "fix": changed_fix() })
        )
    );
    assert_eq!(
        fixture.requests().len(),
        1,
        "nothing is sent past a damaged pin"
    );
}

/// The `certificate_changed` the engine answers for `changed`: its code and
/// its details on the wire.
fn answered(changed: &CertificateChanged) -> (&'static str, Value) {
    (
        changed.error().code(),
        serde_json::to_value(changed.details()).unwrap(),
    )
}

/// The fix `certificate_changed` names for the expired host.
fn changed_fix() -> String {
    format!(
        "A person accepts the new key of {EXPIRED} with `certs accept {EXPIRED}` in a terminal or the Console, or removes its pin with `certs forget {EXPIRED}`; no agent, Recipe or `--json` command can."
    )
}

/// Story 4-36: a pinned host that presents another key is stopped with
/// `certificate_changed`, carrying the pinned and the presented hashes,
/// before anything is sent; every later request to it in the run is
/// stopped too, even one to the server that still presents the pinned
/// key, and the store keeps the new key waiting for a person. The next
/// run reaches the pinned server again (positive control).
/// Covers: story:4-36, error:certificate_changed, format:known-certificates
#[test]
fn a_pinned_host_presenting_another_key_is_stopped_for_the_rest_of_the_run() {
    let fixture = fixture();
    let regenerated = another_key(&fixture);
    let run = fixture.traffic(&["certificates.test"]);
    let next_run = fixture.traffic(&["certificates.test"]);
    let runtime = goodfetch_test_support::runtime();

    let (pinned, changed, again, next, keys) = runtime.block_on(async {
        let pinned = fetched(&run, &fixture.url(EXPIRED)).await.unwrap();
        let changed = fetched(&run, &url_on(&regenerated, EXPIRED)).await;
        let again = fetched(&run, &fixture.url(EXPIRED)).await;
        let next = fetched(&next_run, &fixture.url(EXPIRED)).await.unwrap();
        let keys = (
            presented_key(&fixture.proving_ground, EXPIRED).await,
            presented_key(&regenerated, EXPIRED).await,
        );
        (pinned, changed, again, next, keys)
    });

    let (pinned_key, new_key) = keys;
    assert_ne!(pinned_key, new_key, "the two seeds' leaves hold two keys");
    assert_eq!((pinned.0, next.0), (200, 200));
    let expected = (
        "certificate_changed",
        json!({
            "host": EXPIRED,
            "pinned_spki_sha256": pinned_key,
            "presented_spki_sha256": new_key,
            "fix": changed_fix(),
        }),
    );
    for stopped in [changed, again] {
        let Err(FetchError::Failed(Failure::CertificateChanged(stopped))) = stopped else {
            panic!("the changed key stops the host for the run: {stopped:?}");
        };
        assert_eq!(answered(&stopped), expected);
    }
    assert_eq!(
        (requests_of(&regenerated).len(), fixture.requests().len()),
        (0, 2),
        "nothing is sent to the host once it is stopped"
    );
    assert_eq!(
        fixture
            .store
            .list(PERSON)
            .unwrap()
            .into_iter()
            .map(|pin| (pin.spki_sha256, pin.changed_spki_sha256))
            .collect::<Vec<_>>(),
        [(Some(pinned_key), Some(new_key))]
    );
}

/// Story 4-36: once a person accepted the key that stopped the host, the
/// next run reaches the server presenting it, and the old key's server is
/// stopped instead.
/// Covers: story:4-36
#[test]
fn an_accepted_key_lets_the_host_through_in_the_next_run() {
    let fixture = fixture();
    let regenerated = another_key(&fixture);
    let run = fixture.traffic(&["certificates.test"]);
    let next_run = fixture.traffic(&["certificates.test"]);
    let runtime = goodfetch_test_support::runtime();

    let (waiting, accepted, old, new_key) = runtime.block_on(async {
        fetched(&run, &fixture.url(EXPIRED)).await.unwrap();
        let _ = fetched(&run, &url_on(&regenerated, EXPIRED)).await;
        let new_key = presented_key(&regenerated, EXPIRED).await;
        let waiting = fixture.store.list(PERSON).unwrap()[0]
            .changed_spki_sha256
            .clone();
        let key = KeyHash::from_text(&new_key).unwrap();
        fixture
            .store
            .accept(PERSON, EXPIRED, &key, jiff::Timestamp::now())
            .unwrap();
        let accepted = fetched(&next_run, &url_on(&regenerated, EXPIRED)).await;
        let old = fetched(&next_run, &fixture.url(EXPIRED)).await;
        (waiting, accepted, old, new_key)
    });

    assert_eq!(waiting, Some(new_key));
    assert_eq!(accepted.map(|(status, _)| status).ok(), Some(200));
    assert!(
        matches!(old, Err(FetchError::Failed(Failure::CertificateChanged(_)))),
        "{old:?}"
    );
}

/// Story 4-37: under `any`, a fixture that regenerates its certificate is
/// reached with each key it presents and no pin is written; each run that
/// uses `any` records it once in the Audit log, which verifies, and the
/// response carries the exception's kind.
/// Covers: story:4-37, setting:tls.accept_invalid_certs
#[test]
fn any_accepts_a_regenerated_certificate_without_a_pin_and_is_recorded_on_each_run() {
    let fixture = fixture();
    let regenerated = another_key(&fixture);
    let any = json!({ "certificates.test": "any" });
    let first_run = fixture.run_traffic(&any, "first-run");
    let second_run = fixture.run_traffic(&any, "second-run");
    let runtime = goodfetch_test_support::runtime();

    let (fetches, keys) = runtime.block_on(async {
        let fetches = [
            fetched(&first_run, &fixture.url(EXPIRED)).await.unwrap(),
            fetched(&first_run, &fixture.url(EXPIRED)).await.unwrap(),
            fetched(&second_run, &url_on(&regenerated, EXPIRED))
                .await
                .unwrap(),
        ];
        let keys = (
            presented_key(&fixture.proving_ground, EXPIRED).await,
            presented_key(&regenerated, EXPIRED).await,
        );
        (fetches, keys)
    });

    let (first_key, regenerated_key) = keys;
    assert_eq!(
        fetches
            .iter()
            .map(|(status, used)| (
                *status,
                used.as_ref()
                    .map(|used| (used.kind, used.certificate.to_string(), used.first_pin))
            ))
            .collect::<Vec<_>>(),
        [
            (200, Some((ExceptionKind::Any, first_key.clone(), false))),
            (200, Some((ExceptionKind::Any, first_key.clone(), false))),
            (
                200,
                Some((ExceptionKind::Any, regenerated_key.clone(), false))
            ),
        ]
    );
    assert_eq!(fixture.store.list(PERSON).unwrap(), []);
    assert_eq!(
        fixture.audit_entries(),
        [
            json!(["certificate_any_used", "first-run", { "host": EXPIRED, "spki_sha256": first_key, "failure": "expired" }]),
            json!(["certificate_any_used", "second-run", { "host": EXPIRED, "spki_sha256": regenerated_key, "failure": "expired" }]),
        ]
    );
    assert!(fixture.audit.verify().unwrap().intact);
}

/// Story 4-37: `any` never overrides a pin: a pinned host listed with
/// `any` still passes only with its pinned key, another key stops it with
/// `certificate_changed`, and the pin stays.
/// Covers: story:4-37, error:certificate_changed
#[test]
fn any_never_replaces_an_existing_pin() {
    let fixture = fixture();
    let regenerated = another_key(&fixture);
    let pinning = fixture.traffic(&["certificates.test"]);
    let any = fixture.run_traffic(&json!({ "certificates.test": "any" }), "any-run");
    let runtime = goodfetch_test_support::runtime();

    let (pinned, kept, changed, pinned_key) = runtime.block_on(async {
        let pinned = fetched(&pinning, &fixture.url(EXPIRED)).await.unwrap();
        let kept = fetched(&any, &fixture.url(EXPIRED)).await.unwrap();
        let changed = fetched(&any, &url_on(&regenerated, EXPIRED)).await;
        let pinned_key = presented_key(&fixture.proving_ground, EXPIRED).await;
        (pinned, kept, changed, pinned_key)
    });

    assert_eq!(
        (pinned.0, kept.0, kept.1.map(|used| used.kind)),
        (200, 200, Some(ExceptionKind::TrustOnFirstUse))
    );
    let Err(FetchError::Failed(Failure::CertificateChanged(changed))) = changed else {
        panic!("`any` does not let another key past a pin: {changed:?}");
    };
    assert_eq!(
        changed.pinned.map(|key| key.to_string()),
        Some(pinned_key.clone())
    );
    assert_eq!(
        fixture
            .store
            .list(PERSON)
            .unwrap()
            .into_iter()
            .map(|pin| pin.spki_sha256)
            .collect::<Vec<_>>(),
        [Some(pinned_key)]
    );
}

/// Story 4-37: once a person forgot a host's pin, its next connection under
/// trust on first use pins again, whatever key it presents.
/// Covers: story:4-37
#[test]
fn a_forgotten_host_is_pinned_on_first_use_again() {
    let fixture = fixture();
    let regenerated = another_key(&fixture);
    let first = fixture.traffic(&["certificates.test"]);
    let next = fixture.traffic(&["certificates.test"]);
    let runtime = goodfetch_test_support::runtime();

    let (forgotten, pinned_again, regenerated_key) = runtime.block_on(async {
        fetched(&first, &fixture.url(EXPIRED)).await.unwrap();
        let forgotten = fixture.store.forget(PERSON, EXPIRED).unwrap();
        let pinned_again = fetched(&next, &url_on(&regenerated, EXPIRED))
            .await
            .unwrap();
        (
            forgotten,
            pinned_again,
            presented_key(&regenerated, EXPIRED).await,
        )
    });

    assert_eq!(
        (
            forgotten,
            pinned_again
                .1
                .map(|used| (used.kind, used.certificate.to_string(), used.first_pin))
        ),
        (
            true,
            Some((ExceptionKind::TrustOnFirstUse, regenerated_key, true))
        )
    );
}
