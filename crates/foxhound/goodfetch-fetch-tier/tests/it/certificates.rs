//! TLS per host on both HTTP transports (ADR 0042, pitfall H03's
//! acceptance test on the HTTP tier, spec #565): reqwest, of `standard` and
//! `identified` Sessions, and `wreq`, of HTTP-only `stealth` ones, whose
//! custom verification runs during the handshake. The engine's verifier
//! decides each host between Mozilla's roots, the domain's
//! `tls.trusted_cas` and its pin: a domain's authority passes its own
//! hosts and no other domain's. Each invalid certificate of the Proving
//! ground's `certificates` family ends the hop with `certificate_invalid`,
//! naming the fix, not retryable, nothing sent, and passes under its
//! domain's exception, the answer carrying the exception. Trust on first
//! use pins the key in the user's store and the Audit log; a pinned host
//! presenting another key ends with `certificate_changed` after a
//! handshake and before any byte of the request goes, so does every later
//! hop to it in the Run, and the key waits for a person to accept it;
//! `any` passes every key without a pin, recorded once per Run, and never
//! past a pin. `wreq`'s scopes present one fingerprint; and each domain's
//! client certificate, its key resolved at the first handshake, goes to
//! its own hosts alone, never to one reached through an exception or
//! stopped as a changed key.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::{ExceptionKind, KeyHash, KnownCertificates};
use goodfetch_engine::secrets::{Carried, Keychain, RunSecrets, Secrets};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::{
    CertificateChanged, CertificateExceptions, ClientCertificates, ExceptionUsed, PinBook, Trust,
    read_certificates,
};
use goodfetch_fetch_tier::{Hop, Reuse, Transport, TransportFailure};
use goodfetch_protocol::audit::AuditVerified;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use rustls::DigitallySignedStruct;
use rustls::SignatureScheme;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use serde_json::{Value, json};
use url::Url;

use crate::{Stack, built, config, running_clock_runtime};

/// Any seed: the `certificates` family is served on every one.
const SEED: u64 = 679;

/// The person an engine without accounts acts for.
const PERSON: &str = "os_user";

/// The Run each transport of [`Fixture::transport`] serves.
const RUN: &str = "certificates";

/// The host whose certificate expired.
const EXPIRED: &str = "expired.certificates.test";

/// A host whose certificate the Proving ground's authority makes valid,
/// once that authority is trusted for it.
const VERIFIED: &str = "scope.test";

/// The mutual-TLS host whose certificate the fixture's own Proving ground
/// signed.
const MUTUAL: &str = "client-certificate.certificates.test";

/// The other mutual-TLS host.
const OTHER_MUTUAL: &str = "other-client-certificate.certificates.test";

/// What a hop to a mutual-TLS host that refuses the client certificate it
/// got, or got none, fails with on each transport: the host's refusal reaches
/// the client as a TLS alert after its side of the handshake, which no other
/// variant names.
const REFUSED_HANDSHAKE: TransportFailure = TransportFailure::Other;

/// A pin of the user's store: its host, its key, its Run and the key that
/// waits to replace it.
type PinRow = (String, Option<String>, Option<String>, Option<String>);

/// What one test holds: its folder, the Proving ground, the user's stores
/// and the Run's Secrets. Its fields drop in order, the Proving ground
/// before the folder whose teardown checks that nothing it started is
/// still running.
struct Fixture {
    proving_ground: ProvingGround,
    audit: Arc<AuditLog>,
    store: KnownCertificates,
    owner: Arc<TaskOwner>,
    env: TestEnv,
}

fn fixture() -> Fixture {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let home = Home::new(env.home().to_path_buf());
    Fixture {
        audit: Arc::new(AuditLog::of(
            &home,
            None,
            Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        )),
        store: KnownCertificates::of(
            &home,
            None,
            Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        ),
        owner: Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        proving_ground,
        env,
    }
}

impl Fixture {
    /// A Run's Secrets, holding `environment`.
    fn secrets(&self, environment: BTreeMap<OsString, OsString>) -> RunSecrets {
        let home = Home::new(self.env.home().to_path_buf());
        #[expect(
            clippy::disallowed_methods,
            reason = "the test's Secrets draw their retry jitter from a seeded Entropy (ADR 0059)"
        )]
        let entropy = goodfetch_engine::entropy::Entropy::seeded(0);
        Secrets::new(
            Carried {
                environment,
                keychain: Keychain::absent(),
                home,
                entropy,
            },
            Arc::clone(&self.audit),
            Arc::clone(&self.owner),
            Clock::system(),
        )
        .for_run(Duration::from_secs(30))
    }

    /// `stack`'s transport for a Run: the Proving ground's authority, and
    /// those of `others`, trusted for `trusted`, each domain of `excepted`
    /// listed in `tls.accept_invalid_certs`, and the client certificates
    /// `client_certificates` lists, whose keys `secrets` holds.
    fn transport(
        &self,
        stack: Stack,
        others: &[&ProvingGround],
        trusted: &[&str],
        excepted: &Value,
        client_certificates: &Value,
        secrets: RunSecrets,
    ) -> Box<dyn Transport> {
        let mut config = config(&self.proving_ground, None, &[]);
        let authorities: Vec<_> = std::iter::once(&self.proving_ground)
            .chain(others.iter().copied())
            .flat_map(|proving_ground| {
                #[expect(
                    clippy::disallowed_methods,
                    reason = "a test reads the authority's file before any runtime starts"
                )]
                let authority = std::fs::read(&proving_ground.description().ca).unwrap();
                read_certificates(&authority).unwrap()
            })
            .collect();
        config.trust = trusted
            .iter()
            .fold(Trust::default_roots(), |trust, domain| {
                trust.with_trusted_cas(domain, authorities.clone()).unwrap()
            });
        config.exceptions =
            CertificateExceptions::listed(excepted, self.book(RUN), secrets.exception_hosts())
                .unwrap();
        config.client_certificates = Some(
            ClientCertificates::listed(
                client_certificates,
                secrets,
                Some(SettingsLayer::Run),
                Some(RunId::parse(RUN).unwrap()),
            )
            .unwrap(),
        );
        built(stack, config)
    }

    /// Where the Run `run_id`'s exceptions pin: the user's store, for
    /// [`PERSON`], recording each first pin in the Audit log.
    fn book(&self, run_id: &str) -> PinBook {
        PinBook {
            store: self.store.clone(),
            person: PERSON.to_owned(),
            run_id: Some(RunId::parse(run_id).unwrap()),
            audit: Arc::clone(&self.audit),
            clock: Clock::system(),
        }
    }

    /// `stack`'s transport for the Run `run_id`, trusting the Proving
    /// ground's authority for `scope.test` and `certificates.test` and
    /// listing `excepted` in `tls.accept_invalid_certs`: each Run's
    /// transport learns on its own which host it stopped.
    fn excepting_in(&self, stack: Stack, excepted: &Value, run_id: &str) -> Box<dyn Transport> {
        let mut config = config(&self.proving_ground, None, &[VERIFIED, "certificates.test"]);
        config.exceptions = CertificateExceptions::listed(
            excepted,
            self.book(run_id),
            self.secrets(BTreeMap::new()).exception_hosts(),
        )
        .unwrap();
        built(stack, config)
    }

    /// `stack`'s transport listing the Proving ground's own client
    /// certificate for [`MUTUAL`], its key at `env:OWN_KEY`, which the
    /// Run's environment holds when `with_key`; trusting the Proving
    /// ground's authority for `trusted` and listing `excepted` in
    /// `tls.accept_invalid_certs`.
    fn mutual(
        &self,
        stack: Stack,
        trusted: &[&str],
        excepted: &Value,
        with_key: bool,
    ) -> Box<dyn Transport> {
        let own = self.proving_ground.client_certificate().unwrap();
        let environment: BTreeMap<OsString, OsString> = if with_key {
            [(OsString::from("OWN_KEY"), OsString::from(&own.key))].into()
        } else {
            BTreeMap::new()
        };
        self.transport(
            stack,
            &[],
            trusted,
            excepted,
            &json!({
                MUTUAL: { "certificate": own.certificate.display().to_string(), "key": "env:OWN_KEY" },
            }),
            self.secrets(environment),
        )
    }

    /// The Audit log's entries, each its kind, Run and details.
    fn audit_entries(&self) -> Vec<Value> {
        self.audit
            .export()
            .unwrap()
            .into_iter()
            .map(|exported| {
                let entry = serde_json::to_value(exported.entry).unwrap();
                json!([entry["kind"], entry["run_id"], entry["details"]])
            })
            .collect()
    }

    /// The pins of the user's store.
    fn pins(&self) -> Vec<PinRow> {
        goodfetch_test_support::runtime().block_on(self.listed_pins())
    }

    /// [`Self::pins`], for a test already on a runtime, where blocking on
    /// another one panics.
    async fn listed_pins(&self) -> Vec<PinRow> {
        self.store
            .list(PERSON)
            .await
            .unwrap()
            .into_iter()
            .map(|pin| {
                (
                    pin.host,
                    pin.spki_sha256,
                    pin.run_id.map(|run| run.to_string()),
                    pin.changed_spki_sha256,
                )
            })
            .collect()
    }

    /// `stack`'s transport trusting the Proving ground's authority for
    /// `scope.test` and `certificates.test`, listing `excepted` in
    /// `tls.accept_invalid_certs`, with no client certificate.
    fn excepting(&self, stack: Stack, excepted: &Value) -> Box<dyn Transport> {
        self.transport(
            stack,
            &[],
            &[VERIFIED, "certificates.test"],
            excepted,
            &json!({}),
            self.secrets(BTreeMap::new()),
        )
    }
}

/// The hop to `host`'s root on `proving_ground`.
fn hop(proving_ground: &ProvingGround, host: &str) -> Hop {
    hop_at(proving_ground, host, "/")
}

/// The hop to `path` on `host` on `proving_ground`.
fn hop_at(proving_ground: &ProvingGround, host: &str, path: &str) -> Hop {
    Hop::get(
        Url::parse(&format!(
            "https://{host}:{}{path}",
            proving_ground.address().port()
        ))
        .unwrap(),
    )
}

/// The exception `transport` reached `hop` through, if it was answered.
async fn exception(
    transport: &dyn Transport,
    hop: &Hop,
    reuse: Reuse,
) -> Result<Option<ExceptionUsed>, TransportFailure> {
    transport
        .send(hop, reuse)
        .await
        .map(|sent| sent.certificate_exception)
}

/// The key of the leaf `host` presents on `proving_ground`, read apart
/// from the engine's verifier: a TLS client that accepts any certificate,
/// keeps the leaf and hashes its `SubjectPublicKeyInfo`.
#[expect(
    clippy::disallowed_types,
    reason = "rustls calls the verifier synchronously, and the test dials the Proving ground's loopback socket itself"
)]
async fn presented_key(proving_ground: &ProvingGround, host: &str) -> KeyHash {
    use std::sync::Mutex;

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
    KeyHash::of_public_key(parsed.subject_public_key_info().as_ref())
}

/// The status `transport` got for `hop`, and the exception its host was
/// reached through, as its kind.
async fn sent(
    transport: &dyn Transport,
    hop: &Hop,
) -> Result<(u16, Option<ExceptionKind>), TransportFailure> {
    transport.send(hop, Reuse::Allowed).await.map(|sent| {
        (
            sent.status,
            sent.certificate_exception.map(|used| used.kind),
        )
    })
}

/// The hosts of the requests `proving_ground` received.
fn requested(proving_ground: &ProvingGround) -> Vec<String> {
    proving_ground
        .received_requests()
        .unwrap()
        .every()
        .map(|request| request.host.as_str().to_owned())
        .collect()
}

/// The server names of the TLS handshakes `proving_ground` saw, each with
/// its JA4 fingerprint.
fn handshakes(proving_ground: &ProvingGround) -> Vec<(String, String)> {
    proving_ground.observations().unwrap()["tls_handshakes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|handshake| {
            (
                handshake["sni"].as_str().unwrap_or_default().to_owned(),
                handshake["ja4"].as_str().unwrap_or_default().to_owned(),
            )
        })
        .collect()
}

/// The fix `certificate_invalid` names for `host`, whose certificate
/// failed as `failure`: the setting that would trust or except it.
fn h03_fix(host: &str, failure: KnownCertificateFailure) -> String {
    match failure {
        KnownCertificateFailure::UnknownIssuer => format!(
            "Trust the certificate authority of {host} for its domain under `tls.trusted_cas` in user or project configuration or the Operator policy, or list its domain under `tls.accept_invalid_certs` to pin its certificate on first use."
        ),
        _ => format!(
            "List the domain of {host} under `tls.accept_invalid_certs` to pin its certificate on first use, or give its pin there in user or project configuration or the Operator policy."
        ),
    }
}

/// H03 on each transport (stories 4-31 and 4-32): each invalid
/// certificate ends the hop with `certificate_invalid` naming the host, how
/// it failed and the setting that would make an exception, not retryable,
/// and nothing reaches the host, though the Proving ground's authority is
/// trusted for its domain; under its domain's exception the same hop is
/// answered and carries the exception, the extra handshake its decision
/// took sending nothing. The verified host is the positive control:
/// answered, with no exception.
/// Covers: story:4-31, story:4-32, story:4-104, story:4-106, error:certificate_invalid, setting:tls.accept_invalid_certs
#[rstest]
#[case::expired("expired.certificates.test", KnownCertificateFailure::Expired)]
#[case::unknown_issuer(
    "unknown-issuer.certificates.test",
    KnownCertificateFailure::UnknownIssuer
)]
#[case::name_mismatch(
    "name-mismatch.certificates.test",
    KnownCertificateFailure::NameMismatch
)]
#[case::self_signed("self-signed.certificates.test", KnownCertificateFailure::SelfSigned)]
#[case::not_yet_valid(
    "not-yet-valid.certificates.test",
    KnownCertificateFailure::NotYetValid
)]
fn an_invalid_certificate_ends_the_hop_unsent_and_passes_under_its_exception(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
    #[case] host: &str,
    #[case] failure: KnownCertificateFailure,
) {
    let fixture = fixture();
    let strict = fixture.excepting(stack, &json!({}));
    let excepting = fixture.excepting(stack, &json!({ "certificates.test": null }));
    let (refused, verified, excepted) = running_clock_runtime().block_on(async {
        let refused = sent(&*strict, &hop(&fixture.proving_ground, host)).await;
        let verified = sent(&*strict, &hop(&fixture.proving_ground, VERIFIED)).await;
        let excepted = sent(&*excepting, &hop(&fixture.proving_ground, host)).await;
        (refused, verified, excepted)
    });

    let Err(refused) = refused else {
        panic!("the invalid certificate is refused: {refused:?}");
    };
    assert_eq!(
        refused,
        TransportFailure::CertificateInvalid(CertificateFailure::Known(failure))
    );
    let error = refused.error(&format!("https://{host}/"));
    let details = error.details().cloned().unwrap_or_default();
    assert_eq!(
        (
            error.code(),
            error.retryable(),
            &details["host"],
            &details["failure"],
            &details["fix"],
        ),
        (
            "certificate_invalid",
            false,
            &json!(host),
            &json!(CertificateFailure::Known(failure)),
            &json!(h03_fix(host, failure)),
        )
    );
    assert_eq!(verified, Ok((200, None)));
    assert_eq!(excepted, Ok((200, Some(ExceptionKind::TrustOnFirstUse))));
    assert_eq!(
        requested(&fixture.proving_ground),
        [VERIFIED, host],
        "the refused hop and the exception's decision sent nothing"
    );
}

/// Concurrent first hops to a host its exception covers each pass under
/// it, on each transport: a verdict one handshake left for the host is
/// never lost to another handshake to it at the same time (finding 565-5).
/// Covers: story:4-104, setting:tls.accept_invalid_certs
#[rstest]
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn concurrent_first_hops_to_an_excepted_host_each_pass(#[case] stack: Stack) {
    let fixture = fixture();
    let excepting = fixture.excepting(stack, &json!({ "certificates.test": null }));
    let host = "self-signed.certificates.test";
    let hop = hop(&fixture.proving_ground, host);

    // Each hop on a new connection, so their handshakes run at once.
    let anew = || async {
        excepting.send(&hop, Reuse::Never).await.map(|sent| {
            (
                sent.status,
                sent.certificate_exception.map(|used| used.kind),
            )
        })
    };
    // Worker threads run the handshakes in parallel, as the engine's do.
    #[expect(
        clippy::disallowed_methods,
        reason = "the test builds its own multi-threaded runtime (ADR 0060)"
    )]
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()
        .unwrap();
    let answers = runtime.block_on(async {
        let (a, b, c, d, e, f) = tokio::join!(anew(), anew(), anew(), anew(), anew(), anew());
        vec![a, b, c, d, e, f]
    });

    let passed = Ok((200, Some(ExceptionKind::TrustOnFirstUse)));
    assert_eq!(answers, vec![passed; 6]);
}

/// A pinned host presenting another key ends the hop with
/// `certificate_changed` during the handshake, before any byte of the
/// request is sent: the regenerated server saw the handshake and no
/// request. Every later hop to the host in the Run is refused too,
/// whichever server it reaches; the pinned server answered first, and a
/// host no exception covers is still reached.
/// Covers: story:4-104, error:certificate_changed
#[rstest]
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn a_pinned_host_presenting_another_key_is_stopped_before_anything_is_sent(#[case] stack: Stack) {
    let fixture = fixture();
    let regenerated = ProvingGround::start(&fixture.env, SEED + 1).unwrap();
    let transport = fixture.excepting(stack, &json!({ "certificates.test": null }));
    let expired = "expired.certificates.test";

    let (pinned, changed, again, verified) = running_clock_runtime().block_on(async {
        let pinned = sent(&*transport, &hop(&fixture.proving_ground, expired)).await;
        let changed = sent(&*transport, &hop(&regenerated, expired)).await;
        let again = sent(&*transport, &hop(&fixture.proving_ground, expired)).await;
        let verified = sent(&*transport, &hop(&fixture.proving_ground, VERIFIED)).await;
        (pinned, changed, again, verified)
    });

    assert_eq!(pinned, Ok((200, Some(ExceptionKind::TrustOnFirstUse))));
    for stopped in [changed, again] {
        let Err(TransportFailure::CertificateChanged(stopped)) = &stopped else {
            panic!("the changed key stops the host for the Run: {stopped:?}");
        };
        assert_eq!(stopped.host, expired);
        assert_ne!(stopped.pinned, Some(stopped.presented));
        let error = TransportFailure::CertificateChanged(stopped.clone())
            .error(&format!("https://{expired}/"));
        assert_eq!(
            (error.code(), error.retryable()),
            ("certificate_changed", false)
        );
    }
    assert_eq!(verified, Ok((200, None)));
    assert_eq!(
        (
            requested(&regenerated),
            handshakes(&regenerated)
                .into_iter()
                .map(|(name, _)| name)
                .collect::<Vec<_>>()
        ),
        (Vec::<String>::new(), vec![expired.to_owned()]),
        "the changed key was seen in a handshake, and no request followed"
    );
    assert_eq!(requested(&fixture.proving_ground), [expired, VERIFIED]);
}

/// Story 4-105: `wreq` keeps one client per trust scope of a Session, each
/// presenting the same HTTP profile, so a site cannot tell which scope
/// served it. Three hosts in three scopes (two domains with their own
/// `tls.trusted_cas`, and one reached through its exception under the
/// default roots) are each answered, and the JA4 fingerprints the Proving
/// ground saw for them are one.
/// Covers: story:4-105
#[test]
fn every_trust_scope_of_a_stealth_session_presents_one_fingerprint() {
    let fixture = fixture();
    let self_signed = "self-signed.certificates.test";
    let transport = fixture.transport(
        Stack::Wreq,
        &[],
        &[VERIFIED, "access.test"],
        &json!({ "certificates.test": "any" }),
        &json!({}),
        fixture.secrets(BTreeMap::new()),
    );

    let answered = running_clock_runtime().block_on(async {
        let mut answered = Vec::new();
        for host in [VERIFIED, "1.access.test", self_signed] {
            let sent = sent(&*transport, &hop(&fixture.proving_ground, host)).await;
            answered.push(sent.map(|(_, exception)| exception));
        }
        answered
    });

    assert_eq!(
        answered,
        [Ok(None), Ok(None), Ok(Some(ExceptionKind::Any))],
        "each scope's host answered, the last through its exception"
    );
    let seen = handshakes(&fixture.proving_ground);
    let fingerprints: BTreeMap<&str, BTreeSet<&str>> =
        seen.iter()
            .fold(BTreeMap::new(), |mut by_host, (host, ja4)| {
                by_host
                    .entry(host.as_str())
                    .or_default()
                    .insert(ja4.as_str());
                by_host
            });
    let all: BTreeSet<&str> = fingerprints.values().flatten().copied().collect();
    assert_eq!(
        (fingerprints.len(), all.len()),
        (3, 1),
        "one fingerprint across the scopes: {fingerprints:?}"
    );
}

/// Story 4-34 on each transport, both of which pick a client certificate
/// without the server name: two domains listing two client certificates,
/// each signed by another Proving ground's client authority, are each sent
/// their own. Each mutual-TLS host answers the hop that presents its own
/// authority's certificate, and refuses the handshake of the other one's;
/// a host no listed domain covers is answered too. `wreq` keeps a client
/// per listed domain with the same profile, so its handshakes show one
/// fingerprint whatever certificate they present (step 7 review, finding
/// 563-565-3).
/// Covers: story:4-34, setting:tls.client_certificates
#[rstest]
fn two_domains_with_different_client_certificates_are_routed_by_host(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let other = ProvingGround::start(&fixture.env, SEED + 1).unwrap();
    let (own, others) = (
        fixture.proving_ground.client_certificate().unwrap(),
        other.client_certificate().unwrap(),
    );
    let environment: BTreeMap<OsString, OsString> = [
        (OsString::from("OWN_KEY"), OsString::from(&own.key)),
        (OsString::from("OTHER_KEY"), OsString::from(&others.key)),
    ]
    .into();
    let listed = json!({
        MUTUAL: { "certificate": own.certificate.display().to_string(), "key": "env:OWN_KEY" },
        OTHER_MUTUAL: { "certificate": others.certificate.display().to_string(), "key": "env:OTHER_KEY" },
    });
    let transport = fixture.transport(
        stack,
        &[&other],
        &[VERIFIED, "certificates.test"],
        &json!({}),
        &listed,
        fixture.secrets(environment),
    );

    let (verified, mutual, other_mutual, crossed) = running_clock_runtime().block_on(async {
        let verified = sent(&*transport, &hop(&fixture.proving_ground, VERIFIED)).await;
        let mutual = sent(&*transport, &hop(&fixture.proving_ground, MUTUAL)).await;
        let other_mutual = sent(&*transport, &hop(&other, OTHER_MUTUAL)).await;
        let crossed = sent(&*transport, &hop(&fixture.proving_ground, OTHER_MUTUAL)).await;
        (verified, mutual, other_mutual, crossed)
    });

    assert_eq!(
        (verified, mutual, other_mutual),
        (Ok((200, None)), Ok((200, None)), Ok((200, None)))
    );
    assert_eq!(crossed, Err(REFUSED_HANDSHAKE));
    assert_eq!(
        (
            fixture.proving_ground.observations().unwrap()["client_certificates"].clone(),
            other.observations().unwrap()["client_certificates"].clone(),
        ),
        (json!([MUTUAL]), json!([OTHER_MUTUAL]))
    );
    if matches!(stack, Stack::Wreq) {
        let fingerprints: BTreeSet<String> = handshakes(&fixture.proving_ground)
            .into_iter()
            .chain(handshakes(&other))
            .map(|(_, ja4)| ja4)
            .collect();
        assert_eq!(fingerprints.len(), 1, "{fingerprints:?}");
    }
}

/// Story 4-34 on each transport, the negative control: a client certificate
/// listed for one host goes to no other, even one of the same parent domain
/// that requires one, and that would take the listed certificate, its
/// authority being the Proving ground's own. So it refuses the handshake that
/// presents none, and has seen no certificate.
/// Covers: story:4-34, setting:tls.client_certificates
#[rstest]
fn a_host_no_listed_domain_covers_gets_no_client_certificate(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let listing_one = fixture.mutual(stack, &[VERIFIED, "certificates.test"], &json!({}), true);

    let (listed, unlisted) = running_clock_runtime().block_on(async {
        (
            sent(&*listing_one, &hop(&fixture.proving_ground, MUTUAL)).await,
            sent(&*listing_one, &hop(&fixture.proving_ground, OTHER_MUTUAL)).await,
        )
    });

    assert_eq!(listed, Ok((200, None)));
    assert_eq!(unlisted, Err(REFUSED_HANDSHAKE));
    assert_eq!(
        (
            fixture.proving_ground.observations().unwrap()["client_certificates"].clone(),
            requested(&fixture.proving_ground)
        ),
        (json!([MUTUAL]), vec![MUTUAL.to_owned()])
    );
}

/// Story 4-33 on each transport: the Proving ground's private authority,
/// listed for `scope.test`, makes that domain's certificates pass, its
/// subdomains' too, and leaves another domain's certificate from the same
/// authority refused as an unknown issuer's, nothing sent to it.
/// Covers: story:4-33, setting:tls.trusted_cas
#[rstest]
fn a_domains_trusted_authority_passes_its_certificates_and_no_other_domains(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let scope_only = fixture.transport(
        stack,
        &[],
        &[VERIFIED],
        &json!({}),
        &json!({}),
        fixture.secrets(BTreeMap::new()),
    );

    let (listed, subdomain, other) = running_clock_runtime().block_on(async {
        let pg = &fixture.proving_ground;
        (
            sent(&*scope_only, &hop_at(pg, VERIFIED, "/")).await,
            sent(&*scope_only, &hop_at(pg, "www.scope.test", "/news")).await,
            sent(&*scope_only, &hop_at(pg, "catalog.test", "/catalog/1")).await,
        )
    });

    assert_eq!((listed, subdomain), (Ok((200, None)), Ok((200, None))));
    let Err(other) = other else {
        panic!("another domain's certificate is refused: {other:?}");
    };
    let error = other.error("https://catalog.test/catalog/1");
    assert_eq!(
        (
            error.code(),
            error.details().cloned().unwrap_or_default()["fix"].clone()
        ),
        (
            "certificate_invalid",
            json!(h03_fix(
                "catalog.test",
                KnownCertificateFailure::UnknownIssuer
            ))
        ),
        "{other:?}"
    );
    assert_eq!(
        requested(&fixture.proving_ground),
        [VERIFIED, "www.scope.test"]
    );
}

/// The use of the key of [`MUTUAL`]'s client certificate, as the Audit
/// log records it: its kind and what its details say of the reference.
fn key_used(entries: &[Value]) -> Vec<Value> {
    entries
        .iter()
        .map(|entry| {
            json!([
                entry[0],
                entry[2]["placeholder"],
                entry[2]["reference"],
                entry[2]["domain"]
            ])
        })
        .collect()
}

/// Story 4-34 on each transport: the host that requires mutual TLS refuses
/// a Run that lists no client certificate for it, and serves one that
/// lists the Proving ground's; the key's reference is resolved when the
/// first handshake starts, not when the transport is built, and held for
/// the Run's next connection, so the Audit log records one use.
/// Covers: story:4-34, setting:tls.client_certificates
#[rstest]
fn a_mutual_tls_hosts_key_is_resolved_at_its_first_handshake_and_held_for_the_run(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let trusted = [VERIFIED, "certificates.test"];
    let without = fixture.excepting(stack, &json!({}));
    let with = fixture.mutual(stack, &trusted, &json!({}), true);
    let before_any_handshake = fixture.audit_entries();
    let hop = hop(&fixture.proving_ground, MUTUAL);

    let (refused, first, second) = running_clock_runtime().block_on(async {
        (
            sent(&*without, &hop).await,
            sent(&*with, &hop).await,
            exception(&*with, &hop, Reuse::Never).await,
        )
    });

    assert_eq!(refused, Err(REFUSED_HANDSHAKE));
    assert_eq!((first, second), (Ok((200, None)), Ok(None)));
    assert_eq!(requested(&fixture.proving_ground), [MUTUAL, MUTUAL]);
    assert_eq!(
        (before_any_handshake, key_used(&fixture.audit_entries())),
        (
            Vec::new(),
            vec![json!([
                "secret_reference_used",
                "tls.client_certificates",
                "env:OWN_KEY",
                MUTUAL
            ])]
        )
    );
}

/// Story 4-34 on each transport: a key whose reference gives no value
/// fails the hop with `secret_unavailable` before any handshake, and
/// nothing reaches the host.
/// Covers: story:4-34, error:secret_unavailable
#[rstest]
fn a_key_reference_that_gives_no_value_fails_the_hop_before_any_handshake(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let keyless = fixture.mutual(stack, &[VERIFIED, "certificates.test"], &json!({}), false);

    let failed =
        running_clock_runtime().block_on(sent(&*keyless, &hop(&fixture.proving_ground, MUTUAL)));

    let Err(failed) = failed else {
        panic!("an unset key variable gives no client certificate: {failed:?}");
    };
    assert_eq!(
        failed.error(&format!("https://{MUTUAL}/")).code(),
        "secret_unavailable",
        "{failed:?}"
    );
    assert_eq!(
        (
            handshakes(&fixture.proving_ground),
            requested(&fixture.proving_ground)
        ),
        (Vec::new(), Vec::<String>::new())
    );
}

/// Story 4-34 on each transport (A04-18): a pinned host that presents
/// another key than its pin is stopped with `certificate_changed` before
/// the client certificate is sent, though its certificate verifies; once a
/// person forgets the pin, the next Run's handshake presents the
/// certificate (positive control).
/// Covers: story:4-34, error:certificate_changed
#[rstest]
fn a_pinned_host_presenting_another_key_gets_no_client_certificate(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let another_key = KeyHash::from_stored(&[7; 32]).unwrap();
    goodfetch_test_support::runtime()
        .block_on(fixture.store.pin(
            PERSON,
            MUTUAL,
            &another_key,
            None,
            jiff::Timestamp::UNIX_EPOCH,
        ))
        .unwrap();
    let trusted = [VERIFIED, "certificates.test"];
    let excepted = json!({ "certificates.test": null });
    let pinned_run = fixture.mutual(stack, &trusted, &excepted, true);
    let hop = hop(&fixture.proving_ground, MUTUAL);

    let stopped = running_clock_runtime().block_on(sent(&*pinned_run, &hop));
    let seen_while_pinned = (
        fixture.proving_ground.observations().unwrap()["client_certificates"].clone(),
        requested(&fixture.proving_ground),
    );
    assert!(
        goodfetch_test_support::runtime()
            .block_on(fixture.store.forget(PERSON, MUTUAL))
            .unwrap()
    );
    let next_run = fixture.mutual(stack, &trusted, &excepted, true);
    let reached = running_clock_runtime().block_on(sent(&*next_run, &hop));

    let Err(TransportFailure::CertificateChanged(changed)) = stopped else {
        panic!("a pinned host presenting another key is stopped: {stopped:?}");
    };
    assert_eq!(
        (changed.host.as_str(), changed.pinned),
        (MUTUAL, Some(another_key))
    );
    assert_eq!(seen_while_pinned, (json!([]), Vec::<String>::new()));
    assert_eq!(reached, Ok((200, None)));
    assert_eq!(
        (
            fixture.proving_ground.observations().unwrap()["client_certificates"].clone(),
            requested(&fixture.proving_ground)
        ),
        (json!([MUTUAL]), vec![MUTUAL.to_owned()])
    );
}

/// ADR 0042's guard on each transport: a host whose invalid certificate a
/// trust-on-first-use exception lets through gets no client certificate,
/// as it gets no Secret: the hop fails with `secret_input_refused` before
/// the certificate is sent, while the same host whose authority is trusted
/// gets it (positive control).
/// Covers: story:4-34, error:secret_input_refused
#[rstest]
fn a_host_reached_through_an_exception_gets_no_client_certificate(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let excepted = fixture.mutual(
        stack,
        &[VERIFIED],
        &json!({ "certificates.test": null }),
        true,
    );
    let trusted = fixture.mutual(stack, &[VERIFIED, "certificates.test"], &json!({}), true);
    let hop = hop(&fixture.proving_ground, MUTUAL);

    let (refused, reached) = running_clock_runtime()
        .block_on(async { (sent(&*excepted, &hop).await, sent(&*trusted, &hop).await) });

    let Err(refused) = refused else {
        panic!("no client certificate goes to a host reached through an exception: {refused:?}");
    };
    let error = refused.error(&format!("https://{MUTUAL}/"));
    assert_eq!(
        (
            error.code(),
            error.details().cloned().unwrap_or_default()["domain"].clone()
        ),
        ("secret_input_refused", json!(MUTUAL)),
        "{refused:?}"
    );
    assert_eq!(reached, Ok((200, None)));
    assert_eq!(
        (
            fixture.proving_ground.observations().unwrap()["client_certificates"].clone(),
            requested(&fixture.proving_ground)
        ),
        (json!([MUTUAL]), vec![MUTUAL.to_owned()])
    );
}

/// What an exception reached its host as: its kind, the key it accepted
/// and whether the connection pinned it.
fn used(exception: Option<ExceptionUsed>) -> Option<(ExceptionKind, KeyHash, bool)> {
    exception.map(|used| (used.kind, used.certificate, used.first_pin))
}

/// Story 4-35 on each transport: once its domain is listed for trust on
/// first use, the expired host's first hop pins the key it presented, in
/// `certs list`'s store and in the Audit log, which still verifies; the
/// answer carries the exception's kind, the certificate's hash and that
/// the Run pinned it, and the next connection is let through by the pin
/// with no second pin or entry.
/// Covers: story:4-35, setting:tls.accept_invalid_certs, format:known-certificates
#[rstest]
fn a_listed_domains_invalid_certificate_is_pinned_on_first_use_and_recorded_in_the_audit_log(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let listed = fixture.excepting_in(stack, &json!({ "certificates.test": null }), "first-use");
    let hop = hop(&fixture.proving_ground, EXPIRED);

    let (first, second, presented) = running_clock_runtime().block_on(async {
        (
            exception(&*listed, &hop, Reuse::Allowed).await,
            exception(&*listed, &hop, Reuse::Never).await,
            presented_key(&fixture.proving_ground, EXPIRED).await,
        )
    });

    assert_eq!(
        (
            first.map(used),
            second.map(|second| second.map(|used| (used.kind, used.certificate)))
        ),
        (
            Ok(Some((ExceptionKind::TrustOnFirstUse, presented, true))),
            Ok(Some((ExceptionKind::TrustOnFirstUse, presented)))
        )
    );
    assert_eq!(
        fixture.pins(),
        [(
            EXPIRED.to_owned(),
            Some(presented.to_string()),
            Some("first-use".to_owned()),
            None
        )]
    );
    assert_eq!(
        fixture.audit_entries(),
        [json!([
            "certificate_pinned",
            "first-use",
            { "host": EXPIRED, "spki_sha256": presented.to_string(), "failure": "expired" }
        ])]
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

/// The `certificate_changed` the engine answers for `changed`: its code
/// and its details on the wire.
fn answered(changed: &CertificateChanged) -> (&'static str, Value) {
    (
        changed.error().code(),
        serde_json::to_value(changed.details()).unwrap(),
    )
}

/// The fix `certificate_changed` names for the expired host.
fn changed_fix() -> String {
    format!(
        "A person accepts the new key of {EXPIRED} with `certs accept {EXPIRED}` in a terminal, or removes its pin with `certs forget {EXPIRED}`; no agent, Recipe or `--json` command can."
    )
}

/// Story 4-36 on each transport: a pinned host that presents another key
/// is stopped with `certificate_changed`, carrying the pinned and the
/// presented hashes and how a person accepts the new key, and the store
/// keeps the new key waiting for that person; the next Run reaches the
/// pinned server again (positive control).
/// Covers: story:4-36, error:certificate_changed, format:known-certificates
#[rstest]
fn a_changed_key_waits_in_the_store_and_the_next_run_reaches_the_pinned_server(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let regenerated = ProvingGround::start(&fixture.env, SEED + 1).unwrap();
    let listed = json!({ "certificates.test": null });
    let run = fixture.excepting_in(stack, &listed, "run");
    let next_run = fixture.excepting_in(stack, &listed, "next-run");

    let (pinned, changed, next, keys) = running_clock_runtime().block_on(async {
        let pinned = sent(&*run, &hop(&fixture.proving_ground, EXPIRED)).await;
        let changed = sent(&*run, &hop(&regenerated, EXPIRED)).await;
        let next = sent(&*next_run, &hop(&fixture.proving_ground, EXPIRED)).await;
        let keys = (
            presented_key(&fixture.proving_ground, EXPIRED).await,
            presented_key(&regenerated, EXPIRED).await,
        );
        (pinned, changed, next, keys)
    });

    let (pinned_key, new_key) = keys;
    assert_ne!(pinned_key, new_key, "the two seeds' leaves hold two keys");
    let passed = Ok((200, Some(ExceptionKind::TrustOnFirstUse)));
    assert_eq!((pinned, next), (passed.clone(), passed));
    let Err(TransportFailure::CertificateChanged(changed)) = changed else {
        panic!("the changed key stops the host: {changed:?}");
    };
    assert_eq!(
        answered(&changed),
        (
            "certificate_changed",
            json!({
                "host": EXPIRED,
                "pinned_spki_sha256": pinned_key.to_string(),
                "presented_spki_sha256": new_key.to_string(),
                "fix": changed_fix(),
            })
        )
    );
    assert_eq!(
        fixture.pins(),
        [(
            EXPIRED.to_owned(),
            Some(pinned_key.to_string()),
            Some("run".to_owned()),
            Some(new_key.to_string())
        )]
    );
}

/// Story 4-36 on each transport: once a person accepted the key that
/// stopped the host, the next Run reaches the server presenting it, and
/// the old key's server is stopped instead.
/// Covers: story:4-36
#[rstest]
fn an_accepted_key_lets_the_host_through_in_the_next_run(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let regenerated = ProvingGround::start(&fixture.env, SEED + 1).unwrap();
    let listed = json!({ "certificates.test": null });
    let run = fixture.excepting_in(stack, &listed, "run");
    let next_run = fixture.excepting_in(stack, &listed, "next-run");

    let (waiting, accepted, old, new_key) = running_clock_runtime().block_on(async {
        sent(&*run, &hop(&fixture.proving_ground, EXPIRED))
            .await
            .unwrap();
        let _ = sent(&*run, &hop(&regenerated, EXPIRED)).await;
        let new_key = presented_key(&regenerated, EXPIRED).await;
        let waiting = fixture.listed_pins().await.pop().and_then(|pin| pin.3);
        fixture
            .store
            .accept(
                PERSON,
                EXPIRED,
                &new_key,
                jiff::Timestamp::try_from(Clock::system().wall()).unwrap(),
            )
            .await
            .unwrap();
        let accepted = sent(&*next_run, &hop(&regenerated, EXPIRED)).await;
        let old = sent(&*next_run, &hop(&fixture.proving_ground, EXPIRED)).await;
        (waiting, accepted, old, new_key)
    });

    assert_eq!(waiting, Some(new_key.to_string()));
    assert_eq!(accepted, Ok((200, Some(ExceptionKind::TrustOnFirstUse))));
    assert!(
        matches!(old, Err(TransportFailure::CertificateChanged(_))),
        "{old:?}"
    );
}

/// Story 4-37 on each transport: under `any`, a fixture that regenerates
/// its certificate is reached with each key it presents and no pin is
/// written; each Run that uses `any` records it once in the Audit log,
/// which verifies, and the answer carries the exception's kind.
/// Covers: story:4-37, setting:tls.accept_invalid_certs
#[rstest]
fn any_accepts_a_regenerated_certificate_without_a_pin_and_is_recorded_once_per_run(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let regenerated = ProvingGround::start(&fixture.env, SEED + 1).unwrap();
    let any = json!({ "certificates.test": "any" });
    let first_run = fixture.excepting_in(stack, &any, "first-run");
    let second_run = fixture.excepting_in(stack, &any, "second-run");

    let (fetched, keys) = running_clock_runtime().block_on(async {
        let pg = &fixture.proving_ground;
        let fetched = [
            exception(&*first_run, &hop(pg, EXPIRED), Reuse::Allowed).await,
            exception(&*first_run, &hop(pg, EXPIRED), Reuse::Never).await,
            exception(&*second_run, &hop(&regenerated, EXPIRED), Reuse::Allowed).await,
        ];
        let keys = (
            presented_key(pg, EXPIRED).await,
            presented_key(&regenerated, EXPIRED).await,
        );
        (fetched, keys)
    });

    let (first_key, regenerated_key) = keys;
    assert_eq!(
        fetched.map(|fetched| fetched.map(used)),
        [
            Ok(Some((ExceptionKind::Any, first_key, false))),
            Ok(Some((ExceptionKind::Any, first_key, false))),
            Ok(Some((ExceptionKind::Any, regenerated_key, false))),
        ]
    );
    assert_eq!(fixture.pins(), []);
    assert_eq!(
        fixture.audit_entries(),
        [
            json!(["certificate_any_used", "first-run", { "host": EXPIRED, "spki_sha256": first_key.to_string(), "failure": "expired" }]),
            json!(["certificate_any_used", "second-run", { "host": EXPIRED, "spki_sha256": regenerated_key.to_string(), "failure": "expired" }]),
        ]
    );
    assert!(fixture.audit.verify().unwrap().intact);
}

/// Story 4-37 on each transport: `any` never overrides a pin: a pinned
/// host listed with `any` still passes only with its pinned key, another
/// key stops it with `certificate_changed`, and the pin stays.
/// Covers: story:4-37, error:certificate_changed
#[rstest]
fn any_never_replaces_an_existing_pin(#[values(Stack::Reqwest, Stack::Wreq)] stack: Stack) {
    let fixture = fixture();
    let regenerated = ProvingGround::start(&fixture.env, SEED + 1).unwrap();
    let pinning = fixture.excepting_in(stack, &json!({ "certificates.test": null }), "pinning");
    let any = fixture.excepting_in(stack, &json!({ "certificates.test": "any" }), "any-run");

    let (pinned, kept, changed, pinned_key) = running_clock_runtime().block_on(async {
        let pg = &fixture.proving_ground;
        (
            sent(&*pinning, &hop(pg, EXPIRED)).await,
            sent(&*any, &hop(pg, EXPIRED)).await,
            sent(&*any, &hop(&regenerated, EXPIRED)).await,
            presented_key(pg, EXPIRED).await,
        )
    });

    let passed = Ok((200, Some(ExceptionKind::TrustOnFirstUse)));
    assert_eq!((pinned, kept), (passed.clone(), passed));
    let Err(TransportFailure::CertificateChanged(changed)) = changed else {
        panic!("`any` does not let another key past a pin: {changed:?}");
    };
    assert_eq!(changed.pinned, Some(pinned_key));
    assert_eq!(
        fixture
            .pins()
            .into_iter()
            .map(|pin| pin.1)
            .collect::<Vec<_>>(),
        [Some(pinned_key.to_string())]
    );
}

/// Story 4-37 on each transport: once a person forgot a host's pin, its
/// next connection under trust on first use pins again, whatever key it
/// presents.
/// Covers: story:4-37
#[rstest]
fn a_forgotten_host_is_pinned_on_first_use_again(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let fixture = fixture();
    let regenerated = ProvingGround::start(&fixture.env, SEED + 1).unwrap();
    let listed = json!({ "certificates.test": null });
    let first = fixture.excepting_in(stack, &listed, "first");
    let next = fixture.excepting_in(stack, &listed, "next");

    let (forgotten, pinned_again, regenerated_key) = running_clock_runtime().block_on(async {
        sent(&*first, &hop(&fixture.proving_ground, EXPIRED))
            .await
            .unwrap();
        let forgotten = fixture.store.forget(PERSON, EXPIRED).await.unwrap();
        let pinned_again = exception(&*next, &hop(&regenerated, EXPIRED), Reuse::Allowed).await;
        (
            forgotten,
            pinned_again,
            presented_key(&regenerated, EXPIRED).await,
        )
    });

    assert_eq!(
        (forgotten, pinned_again.map(used)),
        (
            true,
            Ok(Some((
                ExceptionKind::TrustOnFirstUse,
                regenerated_key,
                true
            )))
        )
    );
}
