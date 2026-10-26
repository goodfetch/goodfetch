//! TLS per host on reqwest, the transport of `standard` and `identified`
//! Sessions (ADR 0042, pitfall H03's acceptance test on the HTTP tier,
//! spec #565): the engine's rustls verifier decides each host between
//! Mozilla's roots, the domain's `tls.trusted_cas` and its pin. Each
//! invalid certificate of the Proving ground's `certificates` family ends
//! the hop with `certificate_invalid`, not retryable, nothing sent, and
//! passes under its domain's exception, the answer carrying the exception;
//! a pinned host presenting another key ends with `certificate_changed`
//! before any byte of the request goes, and so does every later hop to it;
//! and each domain's client certificate goes to its own hosts alone.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::known_certificates::KnownCertificates;
use goodfetch_engine::secrets::{Carried, Keychain, RunSecrets, Secrets};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::{
    CertificateExceptions, ClientCertificates, ExceptionKind, PinBook, Trust, read_certificates,
};
use goodfetch_fetch_tier::{Hop, Reqwest, Reuse, Transport, TransportFailure};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::{Value, json};
use url::Url;

use crate::{config, running_clock_runtime};

/// Any seed: the `certificates` family is served on every one.
const SEED: u64 = 679;

/// The person an engine without accounts acts for.
const PERSON: &str = "os_user";

/// A host whose certificate the Proving ground's authority makes valid,
/// once that authority is trusted for it.
const VERIFIED: &str = "scope.test";

/// The mutual-TLS host whose certificate the fixture's own Proving ground
/// signed.
const MUTUAL: &str = "client-certificate.certificates.test";

/// The other mutual-TLS host.
const OTHER_MUTUAL: &str = "other-client-certificate.certificates.test";

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
        audit: Arc::new(AuditLog::of(&home, None)),
        store: KnownCertificates::of(&home, None),
        owner: Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        proving_ground,
        env,
    }
}

impl Fixture {
    /// A Run's Secrets, holding `environment`.
    fn secrets(&self, environment: BTreeMap<OsString, OsString>) -> RunSecrets {
        let home = Home::new(self.env.home().to_path_buf());
        Secrets::new(
            Carried {
                environment,
                keychain: Keychain::absent(),
                home,
            },
            Arc::clone(&self.audit),
            Arc::clone(&self.owner),
            Clock::system(),
        )
        .for_run(Duration::from_secs(30))
    }

    /// reqwest for a Run: the Proving ground's authority, and those of
    /// `others`, trusted for `scope.test` and `certificates.test`, each
    /// domain of `excepted` listed in `tls.accept_invalid_certs`, and the
    /// client certificates `client_certificates` lists, whose keys
    /// `secrets` holds.
    fn transport(
        &self,
        others: &[&ProvingGround],
        excepted: &Value,
        client_certificates: &Value,
        secrets: RunSecrets,
    ) -> Reqwest {
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
        config.trust = [VERIFIED, "certificates.test"]
            .iter()
            .fold(Trust::default_roots(), |trust, domain| {
                trust.with_trusted_cas(domain, authorities.clone()).unwrap()
            });
        let run_id = RunId::parse("certificates").unwrap();
        let book = PinBook {
            store: self.store.clone(),
            person: PERSON.to_owned(),
            run_id: Some(run_id.clone()),
            audit: Arc::clone(&self.audit),
            owner: Arc::clone(&self.owner),
            clock: Clock::system(),
        };
        config.exceptions =
            CertificateExceptions::listed(excepted, book, secrets.exception_hosts()).unwrap();
        config.client_certificates = Some(
            ClientCertificates::listed(
                client_certificates,
                secrets,
                Some(SettingsLayer::Run),
                Some(run_id),
            )
            .unwrap(),
        );
        Reqwest::new(config).unwrap()
    }

    /// reqwest listing `excepted` in `tls.accept_invalid_certs`, with no
    /// client certificate.
    fn excepting(&self, excepted: &Value) -> Reqwest {
        self.transport(&[], excepted, &json!({}), self.secrets(BTreeMap::new()))
    }
}

/// The hop to `host`'s root on `proving_ground`.
fn hop(proving_ground: &ProvingGround, host: &str) -> Hop {
    Hop::get(
        Url::parse(&format!(
            "https://{host}:{}/",
            proving_ground.address().port()
        ))
        .unwrap(),
    )
}

/// The status `transport` got for `hop`, and the exception its host was
/// reached through, as its kind.
async fn sent(
    transport: &Reqwest,
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
        .map(|request| request["host"].as_str().unwrap_or_default().to_owned())
        .collect()
}

/// H03 on reqwest: each invalid certificate ends the hop with
/// `certificate_invalid` naming how it failed, not retryable, and nothing
/// reaches the host; under its domain's exception the same hop is answered
/// and carries the exception, the extra handshake its decision took
/// sending nothing. The verified host is the positive control: answered,
/// with no exception.
/// Covers: story:4-104, story:4-106, error:certificate_invalid, setting:tls.accept_invalid_certs
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
fn an_invalid_certificate_ends_the_hop_unsent_and_passes_under_its_exception(
    #[case] host: &str,
    #[case] failure: KnownCertificateFailure,
) {
    let fixture = fixture();
    let strict = fixture.excepting(&json!({}));
    let excepting = fixture.excepting(&json!({ "certificates.test": null }));
    let (refused, verified, excepted) = running_clock_runtime().block_on(async {
        let refused = sent(&strict, &hop(&fixture.proving_ground, host)).await;
        let verified = sent(&strict, &hop(&fixture.proving_ground, VERIFIED)).await;
        let excepted = sent(&excepting, &hop(&fixture.proving_ground, host)).await;
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
            &details["failure"]
        ),
        (
            "certificate_invalid",
            false,
            &json!(host),
            &json!(CertificateFailure::Known(failure))
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

/// A pinned host presenting another key ends the hop with
/// `certificate_changed` before any byte of the request is sent, and every
/// later hop to it in the Run is refused too, whichever server it reaches;
/// the pinned server answered first, and a host no exception covers is
/// still reached.
/// Covers: story:4-104, error:certificate_changed
#[test]
fn a_pinned_host_presenting_another_key_is_stopped_before_anything_is_sent() {
    let fixture = fixture();
    let regenerated = ProvingGround::start(&fixture.env, SEED + 1).unwrap();
    let transport = fixture.excepting(&json!({ "certificates.test": null }));
    let expired = "expired.certificates.test";

    let (pinned, changed, again, verified) = running_clock_runtime().block_on(async {
        let pinned = sent(&transport, &hop(&fixture.proving_ground, expired)).await;
        let changed = sent(&transport, &hop(&regenerated, expired)).await;
        let again = sent(&transport, &hop(&fixture.proving_ground, expired)).await;
        let verified = sent(&transport, &hop(&fixture.proving_ground, VERIFIED)).await;
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
    assert_eq!(requested(&regenerated), Vec::<String>::new());
    assert_eq!(requested(&fixture.proving_ground), [expired, VERIFIED]);
}

/// Story 4-34 on reqwest, which picks a client certificate without the
/// server name: two domains listing two client certificates, each signed
/// by another Proving ground's client authority, are each sent their own.
/// Each mutual-TLS host answers the hop that presents its own authority's
/// certificate, and refuses the handshake of the other one's.
/// Covers: setting:tls.client_certificates
#[test]
fn two_domains_with_different_client_certificates_are_routed_by_host() {
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
    let transport = fixture.transport(&[&other], &json!({}), &listed, fixture.secrets(environment));

    let (mutual, other_mutual, crossed) = running_clock_runtime().block_on(async {
        let mutual = sent(&transport, &hop(&fixture.proving_ground, MUTUAL)).await;
        let other_mutual = sent(&transport, &hop(&other, OTHER_MUTUAL)).await;
        let crossed = sent(&transport, &hop(&fixture.proving_ground, OTHER_MUTUAL)).await;
        (mutual, other_mutual, crossed)
    });

    assert_eq!((mutual, other_mutual), (Ok((200, None)), Ok((200, None))));
    assert!(crossed.is_err(), "{crossed:?}");
    assert_eq!(
        (
            fixture.proving_ground.observations().unwrap()["client_certificates"].clone(),
            other.observations().unwrap()["client_certificates"].clone(),
        ),
        (json!([MUTUAL]), json!([OTHER_MUTUAL]))
    );
}
