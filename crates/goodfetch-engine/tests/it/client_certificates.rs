//! Client certificates on the engine's own HTTP client, against the
//! Proving ground's hosts that require mutual TLS (ADRs 0015 and 0042,
//! story 4-34): `tls.client_certificates` lists a certificate file and the
//! Secret reference of its key for a domain, and nothing is installed on
//! the machine. The key is resolved through the Run's Secret-provider
//! driver when a handshake with a listed host starts, never before, and is
//! held for the rest of the Run; the certificate goes to the listed
//! domain's hosts alone, and never to a host reached through an exception
//! of `tls.accept_invalid_certs`, a pinned host presenting another key
//! included. What the Proving ground observes is what the hosts saw: the
//! handshakes that presented a client certificate, and the requests. The
//! `resolve` setting maps every host name to the Proving ground's loopback
//! address, which the Session's private ranges list: no test reaches the
//! internet.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::IpAddr;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::home::Home;
use goodfetch_engine::http_client::{Failure, FetchError, PageTraffic, Resolve, Waiter};
use goodfetch_engine::known_certificates::{KeyHash, KnownCertificates};
use goodfetch_engine::secrets::{Carried, Keychain, RunSecrets, SecretError, Secrets};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::{
    CertificateExceptions, ClientCertificateFailure, ClientCertificates, PinBook, Trust,
};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::secrets::SecretInputRefusedReason;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{ClientCertificate, ProvingGround, ReceivedRequests, TestEnv};
use rusqlite::Connection;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use serde_json::{Value, json};

/// Any seed: the families used here are served on every one.
const SEED: u64 = 626;

/// The person an engine without accounts acts for.
const PERSON: &str = "os_user";

/// The host that requires a client certificate.
const MUTUAL: &str = "client-certificate.certificates.test";

/// Another host that requires one, outside the first's name.
const OTHER: &str = "other-client-certificate.certificates.test";

/// The variable the client certificate's key is read from.
const KEY_VARIABLE: &str = "GF_TEST_CLIENT_KEY";

/// What one test holds: the Proving ground, its client certificate, the
/// user's stores and the engine's Secret-provider driver.
/// Its fields drop in order, the Proving ground before the folder whose
/// teardown checks that nothing it started is still running.
struct Fixture {
    proving_ground: ProvingGround,
    client: ClientCertificate,
    audit: Arc<AuditLog>,
    store: KnownCertificates,
    owner: Arc<TaskOwner>,
    secrets: Secrets,
    _env: TestEnv,
}

/// A fixture whose engine environment holds the client certificate's key
/// in [`KEY_VARIABLE`] when `with_key`.
fn fixture(with_key: bool) -> Fixture {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let client = proving_ground.client_certificate().unwrap();
    let home = Home::new(env.home().to_path_buf());
    #[expect(
        clippy::disallowed_methods,
        reason = "the test's Secrets draw their retry jitter from a seeded Entropy (ADR 0059)"
    )]
    let entropy = goodfetch_engine::entropy::Entropy::seeded(0);
    let audit = Arc::new(AuditLog::of(&home, None));
    let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
    let environment: BTreeMap<OsString, OsString> = if with_key {
        [(OsString::from(KEY_VARIABLE), OsString::from(&client.key))].into()
    } else {
        BTreeMap::new()
    };
    let secrets = Secrets::new(
        Carried {
            environment,
            keychain: Keychain::absent(),
            home: home.clone(),
            entropy,
        },
        Arc::clone(&audit),
        Arc::clone(&owner),
        Clock::system(),
    );
    Fixture {
        store: KnownCertificates::of(&home, None),
        client,
        audit,
        owner,
        secrets,
        proving_ground,
        _env: env,
    }
}

impl Fixture {
    /// A Run's Secrets.
    fn run_secrets(&self) -> RunSecrets {
        self.secrets.for_run(Duration::from_secs(30))
    }

    /// `tls.client_certificates` listing the Proving ground's client
    /// certificate for each of `domains`, its key by reference.
    fn client_certificates(&self, domains: &[&str]) -> Value {
        let entry = json!({
            "certificate": self.client.certificate.display().to_string(),
            "key": format!("env:{KEY_VARIABLE}"),
        });
        Value::Object(
            domains
                .iter()
                .map(|domain| ((*domain).to_owned(), entry.clone()))
                .collect(),
        )
    }

    /// A Run's traffic: the Proving ground's authority trusted for each of
    /// `trusted`, each of `excepted` listed for trust on first use, and the
    /// client certificate listed for each of `certified`.
    fn traffic(&self, trusted: &[&str], excepted: &[&str], certified: &[&str]) -> PageTraffic {
        let description = self.proving_ground.description();
        let resolve =
            description
                .resolve
                .iter()
                .fold(Resolve::new(), |resolve, (host, address)| {
                    resolve.with(host.clone(), vec![address.parse::<IpAddr>().unwrap()])
                });
        let authority = CertificateDer::from_pem_file(&description.ca).unwrap();
        let trust = trusted
            .iter()
            .fold(Trust::default_roots(), |trust, domain| {
                trust
                    .with_trusted_cas(domain, vec![authority.clone()])
                    .unwrap()
            });
        let run_id = RunId::parse("mutual").unwrap();
        let secrets = self.run_secrets();
        let accept_invalid_certs = Value::Object(
            excepted
                .iter()
                .map(|domain| ((*domain).to_owned(), Value::Null))
                .collect(),
        );
        let book = PinBook {
            store: self.store.clone(),
            person: PERSON.to_owned(),
            run_id: Some(run_id.clone()),
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
            trust,
            exceptions: CertificateExceptions::listed(
                &accept_invalid_certs,
                book,
                secrets.exception_hosts(),
            )
            .unwrap(),
            client_certificates: ClientCertificates::listed(
                &self.client_certificates(certified),
                secrets,
                Some(SettingsLayer::Run),
                Some(run_id),
            )
            .unwrap(),
            cookies: Vec::new(),
            clock: Clock::system(),
            delays: Delays::BUILT_IN,
        }
    }

    fn url(&self, host: &str) -> String {
        format!("https://{host}:{}/", self.proving_ground.address().port())
    }

    /// The hosts whose handshakes presented a client certificate, and the
    /// hosts of the requests, as the Proving ground saw them.
    fn seen(&self) -> (Value, Vec<Value>) {
        let observations = self.proving_ground.observations().unwrap();
        let requests = ReceivedRequests::of(&observations)
            .every()
            .map(|request| request["host"].clone())
            .collect();
        (observations["client_certificates"].clone(), requests)
    }

    /// The Audit log's entries, each its kind and details.
    fn audit_entries(&self) -> Vec<Value> {
        if !self.audit.path().exists() {
            return Vec::new();
        }
        Connection::open(self.audit.path())
            .unwrap()
            .prepare("SELECT entry FROM entries ORDER BY sequence")
            .unwrap()
            .query_map([], |row| row.get::<_, String>(0))
            .unwrap()
            .map(|entry| {
                let entry: Value = serde_json::from_str(&entry.unwrap()).unwrap();
                json!([entry["kind"], entry["details"]])
            })
            .collect()
    }
}

/// Fetches `url` and answers its status.
async fn status(traffic: &PageTraffic, url: &str) -> Result<u16, FetchError> {
    traffic
        .get(url, Waiter::Call)
        .await
        .map(|response| response.status())
}

/// The use of the client certificate's key for `host`, as the Audit log
/// records it.
fn key_used(host: &str) -> Value {
    json!([
        "secret_reference_used",
        {
            "placeholder": "tls.client_certificates",
            "reference": format!("env:{KEY_VARIABLE}"),
            "domain": host,
        }
    ])
}

/// Story 4-34: the host that requires mutual TLS refuses a Run that lists
/// no client certificate for it, and serves one that lists the Proving
/// ground's; the key's reference is resolved when the first handshake
/// starts, not when the Run's traffic is set up, and held for the next
/// connection, so the Audit log records one use.
/// Covers: story:4-34, setting:tls.client_certificates
#[test]
fn a_mutual_tls_host_takes_the_listed_certificate_its_key_resolved_at_the_handshake() {
    let fixture = fixture(true);
    let without = fixture.traffic(&[MUTUAL], &[], &[]);
    let with = fixture.traffic(&[MUTUAL], &[], &[MUTUAL]);
    let before_any_handshake = fixture.audit_entries();
    let runtime = goodfetch_test_support::runtime();

    let (refused, first, second) = runtime.block_on(async {
        let refused = status(&without, &fixture.url(MUTUAL)).await;
        let first = status(&with, &fixture.url(MUTUAL)).await.unwrap();
        let second = status(&with, &fixture.url(MUTUAL)).await.unwrap();
        (refused, first, second)
    });

    assert!(
        matches!(refused, Err(FetchError::Failed(Failure::Network))),
        "{refused:?}"
    );
    assert_eq!((first, second), (200, 200));
    assert_eq!(
        fixture.seen(),
        (json!([MUTUAL, MUTUAL]), vec![json!(MUTUAL), json!(MUTUAL)])
    );
    assert_eq!(
        (before_any_handshake, fixture.audit_entries()),
        (Vec::new(), vec![key_used(MUTUAL)])
    );
}

/// Story 4-34: a client certificate listed for one host goes to no other,
/// even one of the same parent domain that asks for a client certificate
/// (negative control), while the listed host gets it (positive control).
/// Covers: story:4-34, setting:tls.client_certificates
#[test]
fn a_client_certificate_goes_to_its_listed_domain_and_no_other() {
    let fixture = fixture(true);
    let traffic = fixture.traffic(&["certificates.test"], &[], &[MUTUAL]);
    let runtime = goodfetch_test_support::runtime();

    let (other, listed) = runtime.block_on(async {
        let other = status(&traffic, &fixture.url(OTHER)).await;
        let listed = status(&traffic, &fixture.url(MUTUAL)).await.unwrap();
        (other, listed)
    });

    assert!(
        matches!(other, Err(FetchError::Failed(Failure::Network))),
        "{other:?}"
    );
    assert_eq!(listed, 200);
    assert_eq!(fixture.seen(), (json!([MUTUAL]), vec![json!(MUTUAL)]));
    assert_eq!(fixture.audit_entries(), vec![key_used(MUTUAL)]);
}

/// Story 4-34: a key whose reference gives no value fails the request with
/// `secret_unavailable` before any handshake, and nothing reaches the host.
/// Covers: story:4-34, error:secret_unavailable
#[test]
fn a_key_reference_that_gives_no_value_fails_the_request_before_the_handshake() {
    let fixture = fixture(false);
    let traffic = fixture.traffic(&[MUTUAL], &[], &[MUTUAL]);
    let runtime = goodfetch_test_support::runtime();

    let failed = runtime.block_on(status(&traffic, &fixture.url(MUTUAL)));

    let Err(FetchError::Failed(Failure::ClientCertificate(ClientCertificateFailure::Secret(
        SecretError::Unavailable { placeholder, .. },
    )))) = failed
    else {
        panic!("an unset key variable gives no client certificate: {failed:?}");
    };
    assert_eq!(placeholder, "tls.client_certificates");
    assert_eq!(fixture.seen(), (json!([]), Vec::<Value>::new()));
}

/// Story 4-34 (A04-18): a pinned host that presents another key than its
/// pin is stopped with `certificate_changed` before the client certificate
/// is sent, even though its certificate verifies; once a person forgets the
/// pin, the next Run's handshake presents the certificate (positive
/// control).
/// Covers: story:4-34, error:certificate_changed
#[test]
fn a_pinned_host_presenting_another_key_gets_no_client_certificate() {
    let fixture = fixture(true);
    let another_key = KeyHash::from_stored(&[7; 32]).unwrap();
    fixture
        .store
        .pin(
            PERSON,
            MUTUAL,
            &another_key,
            None,
            jiff::Timestamp::UNIX_EPOCH,
        )
        .unwrap();
    let pinned_run = fixture.traffic(&[MUTUAL], &["certificates.test"], &[MUTUAL]);
    let runtime = goodfetch_test_support::runtime();

    let stopped = runtime.block_on(status(&pinned_run, &fixture.url(MUTUAL)));
    let seen_while_pinned = fixture.seen();
    assert!(fixture.store.forget(PERSON, MUTUAL).unwrap());
    let next_run = fixture.traffic(&[MUTUAL], &["certificates.test"], &[MUTUAL]);
    let reached = runtime
        .block_on(status(&next_run, &fixture.url(MUTUAL)))
        .unwrap();

    let Err(FetchError::Failed(Failure::CertificateChanged(changed))) = stopped else {
        panic!("a pinned host presenting another key is stopped: {stopped:?}");
    };
    assert_eq!(
        (changed.host.as_str(), changed.pinned),
        (MUTUAL, Some(another_key))
    );
    assert_eq!(seen_while_pinned, (json!([]), Vec::<Value>::new()));
    assert_eq!(reached, 200);
    assert_eq!(fixture.seen(), (json!([MUTUAL]), vec![json!(MUTUAL)]));
}

/// ADR 0042's guard: a host whose invalid certificate a trust-on-first-use
/// exception lets through gets no client certificate, as it gets no
/// Secret: the request fails with `secret_input_refused` before the
/// certificate is sent, while the same host whose authority is trusted
/// gets it (positive control).
/// Covers: story:4-34, error:secret_input_refused
#[test]
fn a_host_reached_through_an_exception_gets_no_client_certificate() {
    let fixture = fixture(true);
    let excepted = fixture.traffic(&[], &["certificates.test"], &[MUTUAL]);
    let trusted = fixture.traffic(&[MUTUAL], &[], &[MUTUAL]);
    let runtime = goodfetch_test_support::runtime();

    let (refused, reached) = runtime.block_on(async {
        let refused = status(&excepted, &fixture.url(MUTUAL)).await;
        let reached = status(&trusted, &fixture.url(MUTUAL)).await.unwrap();
        (refused, reached)
    });

    let Err(FetchError::Failed(Failure::ClientCertificate(ClientCertificateFailure::Secret(
        SecretError::InputRefused { domain, reason, .. },
    )))) = refused
    else {
        panic!("no client certificate goes to a host reached through an exception: {refused:?}");
    };
    assert_eq!(
        (domain.as_str(), reason),
        (
            MUTUAL,
            SecretInputRefusedReason::InvalidCertificateException
        )
    );
    assert_eq!(reached, 200);
    assert_eq!(fixture.seen(), (json!([MUTUAL]), vec![json!(MUTUAL)]));
}
