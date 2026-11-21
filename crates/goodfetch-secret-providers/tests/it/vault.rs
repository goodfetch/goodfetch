//! Vault's and `OpenBao`'s provider through the driver, against a loopback
//! stand-in that answers the KV version 2 API as Vault 1.20 and `OpenBao`
//! 2.4 do (ADR 0062), over HTTPS with a certificate from the test's own
//! authority or over plain HTTP: a value planted in the stand-in resolves
//! at the moment of use and is found in no output (the Secret canary
//! harness, #379); the server on a private address is refused under
//! `egress.no_private_endpoints` and reached without it; an invalid
//! certificate is refused even where `tls.accept_invalid_certs` lists its
//! domain, and the server's own `tls.trusted_cas` entry trusts its
//! authority; and a missing or malformed configuration is `not_configured`
//! with a fix.

#![expect(
    clippy::disallowed_types,
    reason = "the stand-in server, outside the engine and any loom model, opens its own listener and counts its requests with standard atomics (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::mask::Redaction;
use goodfetch_engine::secrets::{
    Carried, Keychain, ProviderInputs, Purpose, RunSecrets, SecretError, SecretProvider, Secrets,
};
use goodfetch_engine::settings::{Layers, Policy, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::secret_reference::{Provider, SecretReference};
use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_secret_providers::Vault;
use goodfetch_test_support::{Canaries, SecretKind, TestEnv};
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rstest::rstest;
use rustls::pki_types::{PrivateKeyDer, PrivatePkcs8KeyDer};
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use crate::support::{leaks, reference, runtime, table, unavailable};

/// The token the stand-in takes; no real token's shape.
const TOKEN: &str = "stand-in-token";

/// The host the stand-in's certificate names, which `network.resolve`
/// maps to loopback.
const HOST: &str = "vault.test";

/// Which server a case's stand-in is, by the variable its token is in.
#[derive(Clone, Copy, Debug)]
enum Server {
    Vault,
    OpenBao,
}

impl Server {
    fn token_variable(self) -> &'static str {
        match self {
            Self::Vault => "VAULT_TOKEN",
            Self::OpenBao => "BAO_TOKEN",
        }
    }
}

/// How the stand-in's server stands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    Unsealed,
    Sealed,
}

/// A KV version 2 server on loopback: the secret `shop/portal` in the
/// engine mounted at `secret`, read by [`TOKEN`] alone, over HTTPS with a
/// certificate for [`HOST`] that its own authority signed, or over plain
/// HTTP. It counts the requests it received.
struct StandIn {
    port: u16,
    /// Its authority's certificate, as a DER file.
    authority: PathBuf,
    received: Arc<AtomicUsize>,
    sealed: Arc<AtomicBool>,
}

/// One test's runtime, home, canaries and stand-in.
struct Fixture {
    env: TestEnv,
    runtime: tokio::runtime::Runtime,
    canaries: Canaries,
    stand_in: StandIn,
    /// The value planted as `shop/portal#password`.
    password: String,
    /// The value planted as `shop/portal#pin`.
    pin: String,
}

/// The secret's fields, as the stand-in answers them.
fn secret(password: &str, pin: &str) -> Value {
    json!({
        "request_id": "974b2cc2-0ed1-7c18-c0a5-9bbc3f4b192d",
        "lease_id": "",
        "renewable": false,
        "lease_duration": 0,
        "data": {
            "data": { "password": password, "pin": pin, "port": 5432 },
            "metadata": {
                "created_time": "2026-10-26T11:47:50Z",
                "custom_metadata": null,
                "deletion_time": "",
                "destroyed": false,
                "version": 1
            }
        },
        "wrap_info": null,
        "warnings": null,
        "auth": null
    })
}

/// The stand-in's answer to `request`.
fn answer(
    request: &Request<hyper::body::Incoming>,
    secret: &Value,
    sealed: bool,
) -> (StatusCode, String) {
    let errors = |status, errors: Value| (status, json!({ "errors": errors }).to_string());
    if sealed {
        return errors(StatusCode::SERVICE_UNAVAILABLE, json!(["Vault is sealed"]));
    }
    let token = request
        .headers()
        .get("x-vault-token")
        .and_then(|value| value.to_str().ok());
    if token != Some(TOKEN) {
        return errors(StatusCode::FORBIDDEN, json!(["permission denied"]));
    }
    let path = request.uri().path();
    match path.strip_prefix("/v1/secret/data/") {
        Some("shop/portal") if request.method() == hyper::Method::GET => {
            (StatusCode::OK, secret.to_string())
        }
        Some(_) => errors(StatusCode::NOT_FOUND, json!([])),
        None => errors(
            StatusCode::NOT_FOUND,
            json!([format!(
                "no handler for route \"{}\". route entry not found.",
                path.trim_start_matches("/v1/")
            )]),
        ),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the stand-in server, outside the engine, runs its accept loop and connections on the test's runtime and writes its authority before any engine runs (ADR 0062)"
)]
fn serve(
    runtime: &tokio::runtime::Runtime,
    folder: &std::path::Path,
    tls: bool,
    secret: Value,
) -> StandIn {
    let authority_key = rcgen::KeyPair::generate().unwrap();
    let mut authority_params = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    authority_params.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let authority_certificate = authority_params.self_signed(&authority_key).unwrap();
    let authority = folder.join("authority.der");
    std::fs::write(&authority, authority_certificate.der()).unwrap();
    let issuer = rcgen::Issuer::new(authority_params, authority_key);
    let leaf_key = rcgen::KeyPair::generate().unwrap();
    let leaf = rcgen::CertificateParams::new(vec![HOST.to_owned()])
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
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            0,
        )))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(AtomicUsize::new(0));
    let sealed = Arc::new(AtomicBool::new(false));
    let secret = Arc::new(secret);
    let (counted, seal) = (Arc::clone(&received), Arc::clone(&sealed));
    runtime.spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let (secret, counted, seal) =
                (Arc::clone(&secret), Arc::clone(&counted), Arc::clone(&seal));
            let service = hyper::service::service_fn(move |request| {
                counted.fetch_add(1, Ordering::SeqCst);
                let (status, body) = answer(&request, &secret, seal.load(Ordering::SeqCst));
                let mut response = Response::new(Full::new(Bytes::from(body)));
                *response.status_mut() = status;
                async move { Ok::<_, std::convert::Infallible>(response) }
            });
            let acceptor = acceptor.clone();
            tokio::spawn(async move {
                let builder = hyper::server::conn::http1::Builder::new();
                if tls {
                    if let Ok(stream) = acceptor.accept(tcp).await {
                        let _ = builder
                            .serve_connection(TokioIo::new(stream), service)
                            .await;
                    }
                } else {
                    let _ = builder.serve_connection(TokioIo::new(tcp), service).await;
                }
            });
        }
    });
    StandIn {
        port,
        authority,
        received,
        sealed,
    }
}

fn fixture(tls: bool) -> Fixture {
    let env = TestEnv::new();
    let runtime = runtime();
    let mut canaries = Canaries::new(env.seed());
    let mut plant = || {
        let canary = canaries.plant(SecretKind::TypedPassword);
        (canary.id().to_owned(), canary.value().to_owned())
    };
    let (password_id, password) = plant();
    let (pin_id, pin) = plant();
    for id in [&password_id, &pin_id] {
        canaries.record_planted(id, "the stand-in's KV store");
    }
    let stand_in = serve(&runtime, env.folder(), tls, secret(&password, &pin));
    Fixture {
        env,
        runtime,
        canaries,
        stand_in,
        password,
        pin,
    }
}

/// The machine's settings: the Operator policy's `policy`, and user
/// configuration's `configuration`.
fn machine_settings(policy: Value, configuration: Value) -> Resolved {
    let layers = Layers {
        operator_policy: Policy {
            settings: Values::read(
                SettingsLayer::OperatorPolicy,
                &table(policy),
                Product::Collie,
            )
            .unwrap(),
            ..Policy::default()
        },
        user_configuration: Values::read(
            SettingsLayer::UserConfiguration,
            &table(configuration),
            Product::Collie,
        )
        .unwrap(),
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

impl Fixture {
    /// The stand-in's address over HTTPS, at [`HOST`].
    fn https(&self) -> String {
        format!("https://{HOST}:{}", self.stand_in.port)
    }

    /// The stand-in's address over plain HTTP, at its loopback address.
    fn http(&self) -> String {
        format!("http://127.0.0.1:{}", self.stand_in.port)
    }

    /// User configuration naming the server at `address`, [`HOST`] mapped
    /// to loopback, and `tls`.
    fn configuration(address: &str, tls: &Value) -> Value {
        json!({
            "secret_providers": { "vault": { "address": address } },
            "network": { "resolve": { HOST: ["127.0.0.1"] } },
            "tls": tls,
        })
    }

    /// `tls.trusted_cas` trusting the stand-in's authority for [`HOST`].
    fn trusted(&self) -> Value {
        json!({ "trusted_cas": { HOST: [self.stand_in.authority.display().to_string()] } })
    }

    fn environment(server: Server, token: Option<&str>) -> BTreeMap<OsString, OsString> {
        token
            .map(|token| {
                (
                    OsString::from(server.token_variable()),
                    OsString::from(token),
                )
            })
            .into_iter()
            .collect()
    }

    /// The driver over the test's home with `vault` answering `vault:`
    /// references.
    fn driver(&self, vault: Vault) -> RunSecrets {
        let home = Home::new(self.env.home().to_path_buf());
        #[expect(
            clippy::disallowed_methods,
            reason = "the test's Secrets draw their retry jitter from a seeded Entropy (ADR 0059)"
        )]
        let entropy = goodfetch_engine::entropy::Entropy::seeded(0);
        let implementation: Arc<dyn SecretProvider> = Arc::new(vault);
        self.runtime.block_on(async {
            Secrets::new(
                Carried {
                    environment: BTreeMap::new(),
                    keychain: Keychain::absent(),
                    home: home.clone(),
                    entropy,
                },
                Arc::new(AuditLog::of(
                    &home,
                    None,
                    Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
                )),
                Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
                Clock::system(),
            )
            .with_configured(
                Provider::Vault,
                Arc::new(move |_: &ProviderInputs<'_>| Arc::clone(&implementation)),
            )
            .for_run_under(
                &resolve(&Layers::default(), Product::Collie, Product::Collie).unwrap(),
                Duration::from_secs(30),
                Redaction::On,
            )
        })
    }

    /// The driver over a provider `settings` configure, signed in as
    /// `server` with `token`.
    fn over(&self, settings: &Resolved, server: Server, token: Option<&str>) -> RunSecrets {
        self.driver(Vault::configured(
            settings,
            &Self::environment(server, token),
            Clock::system(),
            TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
        ))
    }

    /// The driver over the stand-in at its HTTPS address, its authority
    /// trusted, signed in as `server`.
    fn signed_in(&self, server: Server) -> RunSecrets {
        let settings = machine_settings(
            json!({}),
            Self::configuration(&self.https(), &self.trusted()),
        );
        self.over(&settings, server, Some(TOKEN))
    }

    fn through(
        &self,
        secrets: &RunSecrets,
        reference: &SecretReference,
    ) -> Result<secrecy::SecretString, SecretError> {
        self.runtime.block_on(secrets.resolve(
            reference,
            &Purpose {
                placeholder: "portal_password",
                domain: Some("shop.test"),
                domains: &["shop.test".to_owned()],
                layer: Some(SettingsLayer::UserConfiguration),
                run_id: None,
            },
        ))
    }

    fn received(&self) -> usize {
        self.stand_in.received.load(Ordering::SeqCst)
    }
}

const PASSWORD: &str = "vault:secret/shop/portal#password";

/// A value planted in the server's KV store resolves at the moment of use,
/// over HTTPS to a server whose own authority `tls.trusted_cas` trusts, is
/// held for the run and is in no output; another field of the secret
/// resolves too, and a field or a secret the server does not hold is
/// `missing`.
/// Covers: story:4-50, setting:secret_providers.vault.address, setting:tls.trusted_cas
#[rstest]
#[case::vault(Server::Vault)]
#[case::openbao(Server::OpenBao)]
fn a_planted_value_resolves_through_vault_and_is_found_in_no_output(#[case] server: Server) {
    let fixture = fixture(true);
    let secrets = fixture.signed_in(server);

    let resolved = fixture.through(&secrets, &reference(PASSWORD)).unwrap();
    let pin = fixture
        .through(&secrets, &reference("vault:secret/shop/portal#pin"))
        .unwrap();
    let absent_field =
        unavailable(fixture.through(&secrets, &reference("vault:secret/shop/portal#badge"))).0;
    let absent_secret =
        unavailable(fixture.through(&secrets, &reference("vault:secret/shop/admin#password"))).0;
    let held = fixture.runtime.block_on(secrets.held());
    let formatted = format!("{resolved:?} {pin:?} {held:?} {secrets:?}");

    assert_eq!(
        (
            resolved.expose_secret() == fixture.password,
            pin.expose_secret() == fixture.pin,
            absent_field,
            absent_secret,
            leaks(
                &fixture.canaries,
                &fixture.env,
                &[("formatted", formatted.as_bytes())]
            ),
        ),
        (
            true,
            true,
            SecretFailure::Missing,
            SecretFailure::Missing,
            Vec::<String>::new()
        )
    );
}

/// Story 4-16 for a Secret provider: the server on loopback is reached by
/// default, and under an Operator policy that sets
/// `egress.no_private_endpoints` it is refused before any request, the
/// failure naming the setting and no value.
/// Covers: story:4-50, setting:egress.no_private_endpoints
#[rstest]
#[case::by_default(false)]
#[case::under_no_private_endpoints(true)]
fn a_vault_server_on_a_private_address_is_refused_only_under_no_private_endpoints(
    #[case] refusing: bool,
) {
    let fixture = fixture(false);
    let policy = if refusing {
        json!({ "egress": { "no_private_endpoints": true } })
    } else {
        json!({})
    };
    let settings = machine_settings(policy, Fixture::configuration(&fixture.http(), &json!({})));
    let secrets = fixture.over(&settings, Server::Vault, Some(TOKEN));

    let resolved = fixture.through(&secrets, &reference(PASSWORD));

    if refusing {
        let (failure, message) = unavailable(resolved);
        assert_eq!(
            (
                failure,
                message.contains("egress.no_private_endpoints"),
                fixture.received(),
                leaks(
                    &fixture.canaries,
                    &fixture.env,
                    &[("message", message.as_bytes())]
                ),
            ),
            (SecretFailure::ProviderFailed, true, 0, Vec::<String>::new()),
            "{message}"
        );
    } else {
        assert_eq!(
            (
                resolved.unwrap().expose_secret() == fixture.password,
                fixture.received()
            ),
            (true, 1)
        );
    }
}

/// ADR 0042 for a Secret provider: the server's certificate, from an
/// authority nothing trusts, is refused before any request even where
/// `tls.accept_invalid_certs` lists its domain, for trust on first use or
/// with `any`; its own `tls.trusted_cas` entry trusts the authority
/// (positive control).
/// Covers: story:4-50, setting:tls.accept_invalid_certs, setting:tls.trusted_cas
#[rstest]
#[case::listed_for_trust_on_first_use(Some(json!({ "accept_invalid_certs": { HOST: null } })))]
#[case::listed_with_any(Some(json!({ "accept_invalid_certs": { HOST: "any" } })))]
#[case::its_own_trusted_authority(None)]
fn an_invalid_certificate_is_refused_whatever_accept_invalid_certs_lists(
    #[case] tls: Option<Value>,
) {
    let fixture = fixture(true);
    let trusted = tls.is_none();
    let tls = tls.unwrap_or_else(|| fixture.trusted());
    let settings = machine_settings(json!({}), Fixture::configuration(&fixture.https(), &tls));
    let secrets = fixture.over(&settings, Server::Vault, Some(TOKEN));

    let resolved = fixture.through(&secrets, &reference(PASSWORD));

    if trusted {
        assert_eq!(
            (
                resolved.unwrap().expose_secret() == fixture.password,
                fixture.received()
            ),
            (true, 1)
        );
    } else {
        let (failure, message) = unavailable(resolved);
        assert_eq!(
            (
                failure,
                message.contains("certificate") && message.contains("tls.trusted_cas"),
                fixture.received(),
            ),
            (SecretFailure::ProviderFailed, true, 0),
            "{message}"
        );
    }
}

/// A token the server refuses, or a sealed server, fails with
/// `secret_unavailable`, `provider_failed`, the message naming the
/// reference and the fix, never a value.
/// Covers: story:4-50, error:secret_unavailable
#[rstest]
#[case::a_refused_token(Some("another-token"), State::Unsealed, "VAULT_TOKEN")]
#[case::a_sealed_server(Some(TOKEN), State::Sealed, "unseal")]
fn a_refused_token_or_a_sealed_server_fails_naming_it_and_no_value(
    #[case] token: Option<&str>,
    #[case] state: State,
    #[case] fix_names: &str,
) {
    let fixture = fixture(true);
    fixture
        .stand_in
        .sealed
        .store(state == State::Sealed, Ordering::SeqCst);
    let settings = machine_settings(
        json!({}),
        Fixture::configuration(&fixture.https(), &fixture.trusted()),
    );
    let secrets = fixture.over(&settings, Server::Vault, token);

    let (failure, message) = unavailable(fixture.through(&secrets, &reference(PASSWORD)));

    assert_eq!(
        (
            failure,
            message.contains(PASSWORD) && message.contains(fix_names),
            leaks(
                &fixture.canaries,
                &fixture.env,
                &[("message", message.as_bytes())]
            ),
        ),
        (SecretFailure::ProviderFailed, true, Vec::<String>::new()),
        "{message}"
    );
}

/// `check_config` refuses an unset or malformed address, or no token, with
/// a fix naming what to set, and the driver answers `not_configured`
/// before any request.
/// Covers: story:4-50, error:secret_unavailable, setting:secret_providers.vault.address
#[rstest]
#[case::an_unset_address(None, Some(TOKEN), "secret_providers.vault.address")]
#[case::plain_http_off_loopback(Some("http://vault.example.com:8200"), Some(TOKEN), "https:")]
#[case::no_url(Some("vault.example.com"), Some(TOKEN), "no URL")]
#[case::no_token(Some("https://vault.test"), None, "VAULT_TOKEN")]
fn a_missing_or_malformed_configuration_is_not_configured_with_a_fix(
    #[case] address: Option<&str>,
    #[case] token: Option<&str>,
    #[case] fix_names: &str,
) {
    let fixture = fixture(true);
    let configuration = match address {
        Some(address) => Fixture::configuration(address, &fixture.trusted()),
        None => json!({}),
    };
    let vault = Vault::configured(
        &machine_settings(json!({}), configuration),
        &Fixture::environment(Server::Vault, token),
        Clock::system(),
        TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
    );
    let checked = vault.check_config().err().map(|problem| problem.fix);
    let secrets = fixture.driver(vault);

    let (failure, message) = unavailable(fixture.through(&secrets, &reference(PASSWORD)));

    assert_eq!(
        (
            failure,
            message.contains(fix_names),
            checked.is_some_and(|fix| fix.contains(fix_names)),
            fixture.received(),
        ),
        (SecretFailure::NotConfigured, true, true, 0),
        "{message}"
    );
}
