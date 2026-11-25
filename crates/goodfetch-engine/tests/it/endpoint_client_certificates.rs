//! An Operator-configured endpoint's own client certificate (ADR 0042,
//! issue 1648): an endpoint whose host `tls.client_certificates` lists
//! presents that certificate, its key resolved when the handshake starts,
//! never before, and only through a provider every engine carries; a
//! certificate listed for another host is never presented to it. Proved on
//! the Secret-provider endpoint as a Run's provider receives it, against the
//! Proving ground's host that requires mutual TLS; its request log and the
//! Audit log are the oracle.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::mask::Redaction;
use goodfetch_engine::secret_endpoint::{EndpointFailure, SecretProviderEndpoint};
use goodfetch_engine::secrets::{
    Carried, ConfigProblem, Keychain, ProviderFailure, ProviderInputs, Resolving, SecretProvider,
    Secrets,
};
use goodfetch_engine::settings::Resolved;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::ClientCertificateFailure;
use goodfetch_protocol::secret_reference::{Provider, SecretReference};
use goodfetch_test_support::{ProvingGround, TestEnv};
use hyper::Request;
use hyper::body::Bytes;
use serde_json::{Value, json};

use crate::operator_endpoints::{at_catalog, authority, machine_settings};

const SEED: u64 = 1648;

/// The Proving ground's host that requires a client certificate its client
/// authority signed.
const MUTUAL: &str = "client-certificate.certificates.test";

/// Its other host that requires one, outside the first's name.
const OTHER_MUTUAL: &str = "other-client-certificate.certificates.test";

/// A provider that holds no value: what a test reads is the endpoint its
/// factory was given.
struct Holding;

impl SecretProvider for Holding {
    fn resolve<'a>(&'a self, _reference: &'a SecretReference) -> Resolving<'a> {
        Box::pin(async { Err(ProviderFailure::Missing) })
    }

    fn check_config(&self) -> Result<(), ConfigProblem> {
        Ok(())
    }
}

/// The engine's Secrets over `environment`, recording each resolution in
/// `audit`, with a Vault provider whose factory keeps the endpoint it is
/// given in `endpoint`.
fn secrets(
    env: &TestEnv,
    environment: BTreeMap<OsString, OsString>,
    audit: &Arc<AuditLog>,
    endpoint: &Arc<OnceLock<SecretProviderEndpoint>>,
) -> Secrets {
    #[expect(
        clippy::disallowed_methods,
        reason = "the test's Secrets draw their retry jitter from a seeded Entropy (ADR 0059)"
    )]
    let entropy = Entropy::seeded(0);
    let kept = Arc::clone(endpoint);
    Secrets::new(
        Carried {
            environment,
            keychain: Keychain::absent(),
            home: Home::new(env.home().to_path_buf()),
            entropy,
        },
        Arc::clone(audit),
        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        Clock::system(),
    )
    .with_configured(
        Provider::Vault,
        Arc::new(move |inputs: &ProviderInputs<'_>| {
            let _ = kept.set(SecretProviderEndpoint::of(inputs));
            Arc::new(Holding) as Arc<dyn SecretProvider>
        }),
    )
}

/// A Run's Secret-provider endpoint, as its Vault provider is built with
/// it, and the Audit log its key's resolution is recorded in. The fields
/// drop in order, so the Proving ground stops before the environment checks
/// that nothing it started is left.
struct Fixture {
    proving_ground: ProvingGround,
    audit: Arc<AuditLog>,
    env: TestEnv,
}

impl Fixture {
    fn new() -> Self {
        let env = TestEnv::new();
        let proving_ground = ProvingGround::start(&env, SEED).unwrap();
        let audit = Arc::new(AuditLog::of(
            &Home::new(env.home().to_path_buf()),
            None,
            Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
        ));
        Self {
            proving_ground,
            audit,
            env,
        }
    }

    /// The settings of a Run whose `tls.client_certificates` lists the
    /// Proving ground's client certificate for `listed`, its key at
    /// `key`, trusting the Proving ground's authority for
    /// `certificates.test`.
    fn settings(&self, listed: &str, key: &str) -> Resolved {
        let own = self.proving_ground.client_certificate().unwrap();
        let tls = json!({
            "trusted_cas": { "certificates.test": [authority(&self.proving_ground)] },
            "client_certificates": {
                listed: { "certificate": own.certificate.display().to_string(), "key": key },
            },
        });
        machine_settings(json!({}), at_catalog(&self.proving_ground, &tls))
    }

    /// The Secret-provider endpoint a Run under `settings` builds its
    /// providers with, the engine's environment holding the client
    /// certificate's key at `OWN_KEY`.
    fn endpoint(&self, settings: &Resolved) -> SecretProviderEndpoint {
        let own = self.proving_ground.client_certificate().unwrap();
        let environment = [(OsString::from("OWN_KEY"), OsString::from(&own.key))].into();
        let endpoint = Arc::new(OnceLock::new());
        drop(
            secrets(&self.env, environment, &self.audit, &endpoint).for_run_under(
                settings,
                Duration::from_secs(30),
                Redaction::On,
            ),
        );
        endpoint
            .get()
            .cloned()
            .expect("the Run built its providers")
    }

    /// Reads a secret from `endpoint` at [`MUTUAL`]: the status, or why
    /// there was none.
    fn read(&self, endpoint: &SecretProviderEndpoint) -> Result<u16, EndpointFailure> {
        let url = format!(
            "https://{MUTUAL}:{}/v1/secret/data/shop",
            self.proving_ground.address().port()
        );
        let request = Request::get(url).body(Bytes::new()).unwrap();
        goodfetch_test_support::runtime().block_on(async {
            endpoint
                .send(request)
                .await
                .map(|answer| answer.status().as_u16())
        })
    }

    /// The hosts of the requests the Proving ground received.
    fn requested(&self) -> Vec<String> {
        self.proving_ground
            .received_requests()
            .unwrap()
            .every()
            .map(|request| request.host.as_str().to_owned())
            .collect()
    }

    /// Each use of a Secret reference the Audit log records: its
    /// placeholder, reference and domain.
    fn keys_used(&self) -> Vec<Value> {
        self.audit
            .export()
            .unwrap()
            .into_iter()
            .map(|exported| serde_json::to_value(exported.entry).unwrap())
            .filter(|entry| entry["kind"] == "secret_reference_used")
            .map(|entry| {
                let details = &entry["details"];
                json!([
                    details["placeholder"],
                    details["reference"],
                    details["domain"]
                ])
            })
            .collect()
    }
}

/// Issue 1648: the Secret-provider endpoint at a host that requires mutual
/// TLS presents the client certificate `tls.client_certificates` lists for
/// it, so the request is answered; the key's reference is resolved when
/// the handshake starts, not when the endpoint is built, and the Audit log
/// records that use.
/// Covers: story:4-34, setting:tls.client_certificates
#[test]
fn a_provider_endpoint_presents_the_client_certificate_listed_for_its_host() {
    let fixture = Fixture::new();
    let endpoint = fixture.endpoint(&fixture.settings(MUTUAL, "env:OWN_KEY"));
    let before_any_handshake = fixture.keys_used();

    let read = fixture.read(&endpoint);

    assert_eq!(
        (
            read.is_ok(),
            fixture.requested(),
            before_any_handshake,
            fixture.keys_used()
        ),
        (
            true,
            vec![MUTUAL.to_owned()],
            Vec::new(),
            vec![json!(["tls.client_certificates", "env:OWN_KEY", MUTUAL])]
        ),
        "{read:?}"
    );
}

/// Issue 1648: a client certificate listed for another host is never
/// presented to the endpoint, so the host that requires one refuses the
/// handshake, nothing is requested and no key is resolved (the negative
/// control of the test above).
/// Covers: story:4-34, setting:tls.client_certificates
#[test]
fn a_provider_endpoint_presents_no_certificate_listed_for_another_host() {
    let fixture = Fixture::new();
    let endpoint = fixture.endpoint(&fixture.settings(OTHER_MUTUAL, "env:OWN_KEY"));

    let read = fixture.read(&endpoint);

    assert_eq!(
        (
            matches!(read, Err(EndpointFailure::Network)),
            fixture.requested(),
            fixture.keys_used()
        ),
        (true, Vec::<String>::new(), Vec::<Value>::new()),
        "{read:?}"
    );
}

/// Issue 1648: an endpoint's key is read only through a provider every
/// engine carries, never through one that itself reaches an endpoint, such
/// as Vault's: the certificate cannot be used, and nothing is sent.
/// Covers: setting:tls.client_certificates
#[test]
fn an_endpoints_key_is_read_only_through_a_carried_provider() {
    let fixture = Fixture::new();
    let endpoint = fixture.endpoint(&fixture.settings(MUTUAL, "vault:secret/endpoint#key"));

    let read = fixture.read(&endpoint);

    let Err(EndpointFailure::ClientCertificate(ClientCertificateFailure::Unusable {
        domain, ..
    })) = &read
    else {
        panic!("the endpoint's key is refused: {read:?}");
    };
    assert_eq!(
        (domain.as_str(), fixture.requested(), fixture.keys_used()),
        (MUTUAL, Vec::<String>::new(), Vec::<Value>::new())
    );
}
