//! Azure Key Vault's provider reading with the access token each ambient
//! source gives (issue 1658): a workload identity's federated token, which
//! Microsoft Entra ID exchanges, the host's managed identity endpoint, and
//! the instance metadata service, each against a loopback stand-in of its
//! documented endpoint (ADR 0062), so no test reaches Azure (ADR 0076). The
//! value read through each is found in no output, nor is any credential
//! (the Secret canary harness, #379); each source's failure is
//! `secret_unavailable` naming it.

#![expect(
    clippy::disallowed_types,
    reason = "the stand-in server, outside the engine and any loom model, opens its own listener and records its requests behind a standard mutex (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_engine::Composition;
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::home::Home;
use goodfetch_engine::mask::Redaction;
use goodfetch_engine::secrets::{Carried, Keychain, Purpose, RunSecrets, SecretError, Secrets};
use goodfetch_engine::settings::{Layers, Policy, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{Canaries, SecretKind, TestEnv};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use crate::support::{leaks, reference, runtime, table, unavailable};

/// The tenant and the app the workload identity signs in as, and the
/// user-assigned managed identity the host holds.
const TENANT: &str = "shop-tenant";
const CLIENT: &str = "shop-app";
const IDENTITY: &str = "shop-identity";

/// The secret's read path in the vault `shop-vault`.
const READ: &str = "/shop-vault/secrets/portal";

/// The token paths of the authority, the host's managed identity endpoint
/// and the instance metadata service.
const AUTHORITY_TOKEN: &str = "/shop-tenant/oauth2/v2.0/token";
const HOST_TOKEN: &str = "/msi/token";
const IMDS_TOKEN: &str = "/metadata/identity/oauth2/token";

/// What the stand-in holds and hands out, each a canary: the secret's
/// value, the access token every source gives, the federated token the
/// workload identity's file holds and the host's identity header.
struct Held {
    password: String,
    access_token: String,
    federated_token: String,
    identity_header: String,
}

/// How the stand-in's instance metadata service behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Instance {
    /// It gives the token of [`IDENTITY`], or of the system-assigned
    /// identity when none is named.
    WithIdentity,
    /// The virtual machine has no managed identity.
    WithoutIdentity,
    /// Another server answers, in no shape of Azure's.
    NotAzure,
    /// It holds the token request unanswered.
    Silent,
}

/// The stand-in's answer to `method` on `path` with `query`, `header` and
/// `body`.
async fn answer(
    held: &Held,
    instance: Instance,
    method: &Method,
    path: &str,
    query: &BTreeMap<String, String>,
    header: impl Fn(&str) -> String,
    body: &[u8],
) -> (StatusCode, String) {
    let token = || {
        json!({ "access_token": held.access_token, "expires_in": "3599", "token_type": "Bearer", "resource": "https://vault.azure.net" })
            .to_string()
    };
    let asked = |version: &str| {
        query.get("api-version").map(String::as_str) == Some(version)
            && query.get("resource").map(String::as_str) == Some("https://vault.azure.net")
    };
    match (method.as_str(), path) {
        ("POST", AUTHORITY_TOKEN) => {
            let form: BTreeMap<String, String> =
                url::form_urlencoded::parse(body).into_owned().collect();
            let field = |name: &str| form.get(name).map(String::as_str);
            let granted = field("grant_type") == Some("client_credentials")
                && field("client_id") == Some(CLIENT)
                && field("client_assertion_type")
                    == Some("urn:ietf:params:oauth:client-assertion-type:jwt-bearer")
                && field("client_assertion") == Some(held.federated_token.as_str())
                && field("scope") == Some("https://vault.azure.net/.default");
            if granted {
                (StatusCode::OK, token())
            } else {
                (
                    StatusCode::UNAUTHORIZED,
                    json!({ "error": "invalid_client", "error_description": "AADSTS700024: Client assertion is not within its valid time range.", "error_codes": [700_024] })
                        .to_string(),
                )
            }
        }
        ("GET", HOST_TOKEN)
            if asked("2019-08-01") && header("x-identity-header") == held.identity_header =>
        {
            (StatusCode::OK, token())
        }
        ("GET", IMDS_TOKEN) if asked("2018-02-01") && header("metadata") == "true" => match instance
        {
            Instance::WithIdentity if query.get("client_id").is_none_or(|id| id == IDENTITY) => {
                (StatusCode::OK, token())
            }
            Instance::WithIdentity | Instance::WithoutIdentity => (
                StatusCode::BAD_REQUEST,
                json!({ "error": "invalid_request", "error_description": "Identity not found" })
                    .to_string(),
            ),
            Instance::NotAzure => (StatusCode::NOT_FOUND, "Not Found".to_owned()),
            Instance::Silent => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                (StatusCode::GATEWAY_TIMEOUT, String::new())
            }
        },
        ("GET", READ) => {
            if header("authorization") == format!("Bearer {}", held.access_token) {
                (
                    StatusCode::OK,
                    json!({ "value": held.password, "id": "https://shop-vault.vault.azure.net/secrets/portal/1" })
                        .to_string(),
                )
            } else {
                (
                    StatusCode::UNAUTHORIZED,
                    json!({ "error": { "code": "Unauthorized", "message": "AKV10000: Request is missing a Bearer or PoP token." } })
                        .to_string(),
                )
            }
        }
        _ => (StatusCode::NOT_FOUND, String::new()),
    }
}

/// One test's runtime, home, canaries and stand-in, with the requests it
/// received, in order.
struct Fixture {
    runtime: tokio::runtime::Runtime,
    canaries: Canaries,
    port: u16,
    requests: Arc<Mutex<Vec<String>>>,
    held: Arc<Held>,
    tokens: PathBuf,
    env: TestEnv,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the stand-in server, outside the engine, runs its accept loop and connections on the test's runtime; the test writes its token files before any engine runs (ADR 0062)"
)]
fn fixture(instance: Instance) -> Fixture {
    let env = TestEnv::new();
    let runtime = runtime();
    let mut canaries = Canaries::new(env.seed());
    let mut plant = |kind| {
        let canary = canaries.plant(kind);
        (canary.id().to_owned(), canary.value().to_owned())
    };
    let planted = [
        plant(SecretKind::TypedPassword),
        plant(SecretKind::EndpointToken),
        plant(SecretKind::EndpointToken),
        plant(SecretKind::EndpointToken),
    ];
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            0,
        )))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let tokens = env.folder().join("tokens");
    std::fs::create_dir_all(&tokens).unwrap();
    let (federated_id, federated_token) = &planted[2];
    std::fs::write(tokens.join("azure-identity-token"), federated_token).unwrap();
    std::fs::write(tokens.join("expired-token"), "an-expired-assertion").unwrap();
    canaries.record_planted(
        federated_id,
        &tokens.join("azure-identity-token").display().to_string(),
    );
    for (id, _) in &planted {
        canaries.record_planted(id, "the stand-in's credentials");
    }
    let [password, access_token, federated_token, identity_header] =
        planted.map(|(_, value)| value);
    let held = Arc::new(Held {
        password,
        access_token,
        federated_token,
        identity_header,
    });
    let requests = Arc::new(Mutex::new(Vec::new()));
    let (serving, recorded) = (Arc::clone(&held), Arc::clone(&requests));
    runtime.spawn(async move {
        while let Ok((tcp, _)) = listener.accept().await {
            let (held, recorded) = (Arc::clone(&serving), Arc::clone(&recorded));
            let service =
                hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
                    let (held, recorded) = (Arc::clone(&held), Arc::clone(&recorded));
                    async move {
                        let (method, path) =
                            (request.method().clone(), request.uri().path().to_owned());
                        recorded.lock().unwrap().push(format!("{method} {path}"));
                        let query: BTreeMap<String, String> = url::form_urlencoded::parse(
                            request.uri().query().unwrap_or_default().as_bytes(),
                        )
                        .into_owned()
                        .collect();
                        let headers = request.headers().clone();
                        let header = |name: &str| {
                            headers
                                .get(name)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_owned()
                        };
                        let body = request.into_body().collect().await.unwrap().to_bytes();
                        let (status, text) =
                            answer(&held, instance, &method, &path, &query, header, &body).await;
                        let mut response = Response::new(Full::new(Bytes::from(text)));
                        *response.status_mut() = status;
                        Ok::<_, std::convert::Infallible>(response)
                    }
                });
            tokio::spawn(async move {
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(tcp), service)
                    .await;
            });
        }
    });
    Fixture {
        runtime,
        canaries,
        port,
        requests,
        held,
        tokens,
        env,
    }
}

/// Which source the engine's environment names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ambient {
    /// A workload identity's federated token file.
    WorkloadIdentity,
    /// A federated token file whose token the authority refuses.
    ExpiredFederatedToken,
    /// A federated token file that does not exist.
    MissingTokenFile,
    /// The host's managed identity endpoint.
    HostIdentity,
    /// The instance metadata service, for the system-assigned identity.
    Imds,
    /// The instance metadata service, for the user-assigned [`IDENTITY`].
    UserAssigned,
    /// The instance metadata service, for an identity the host lacks.
    OtherIdentity,
    /// Nothing: `AZURE_TOKEN_CREDENTIALS` keeps only the service principal,
    /// which the environment lacks.
    Nothing,
    /// An `AZURE_TOKEN_CREDENTIALS` the Azure Identity libraries do not take.
    UnknownChoice,
}

impl Fixture {
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The engine's environment naming `ambient`.
    fn environment(&self, ambient: Ambient) -> BTreeMap<OsString, OsString> {
        let workload = |file: &str| {
            vec![
                ("AZURE_TENANT_ID", TENANT.to_owned()),
                ("AZURE_CLIENT_ID", CLIENT.to_owned()),
                (
                    "AZURE_FEDERATED_TOKEN_FILE",
                    self.tokens.join(file).display().to_string(),
                ),
            ]
        };
        let imds = |client_id: Option<&str>| {
            let mut variables = vec![("AZURE_POD_IDENTITY_AUTHORITY_HOST", self.base())];
            variables.extend(client_id.map(|id| ("AZURE_CLIENT_ID", id.to_owned())));
            variables
        };
        let variables = match ambient {
            Ambient::WorkloadIdentity => workload("azure-identity-token"),
            Ambient::ExpiredFederatedToken => workload("expired-token"),
            Ambient::MissingTokenFile => workload("absent-token"),
            Ambient::HostIdentity => vec![
                ("IDENTITY_ENDPOINT", format!("{}{HOST_TOKEN}", self.base())),
                ("IDENTITY_HEADER", self.held.identity_header.clone()),
            ],
            Ambient::Imds => imds(None),
            Ambient::UserAssigned => imds(Some(IDENTITY)),
            Ambient::OtherIdentity => imds(Some("another-identity")),
            Ambient::Nothing => vec![(
                "AZURE_TOKEN_CREDENTIALS",
                "EnvironmentCredential".to_owned(),
            )],
            Ambient::UnknownChoice => vec![("AZURE_TOKEN_CREDENTIALS", "everything".to_owned())],
        };
        variables
            .into_iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }

    /// The driver over the crate's contribution, with the engine's
    /// environment naming `ambient`, under the Operator policy `policy`.
    fn driver(&self, ambient: Ambient, policy: Value) -> RunSecrets {
        let home = Home::new(self.env.home().to_path_buf());
        #[expect(
            clippy::disallowed_methods,
            reason = "the test's Secrets draw their retry jitter from a seeded Entropy (ADR 0059)"
        )]
        let entropy = goodfetch_engine::entropy::Entropy::seeded(0);
        let configuration = json!({
            "secret_providers": {
                "azure_kv": { "endpoint": format!("{}/{{vault}}", self.base()), "authority": self.base() }
            }
        });
        let settings = run_settings(policy, configuration);
        self.runtime.block_on(async {
            Composition::default()
                .with(goodfetch_secret_providers::contribution())
                .secret_providers()
                .into_iter()
                .fold(
                    Secrets::new(
                        Carried {
                            environment: self.environment(ambient),
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
                    ),
                    |secrets, (provider, factory)| secrets.with_configured(provider, factory),
                )
                .for_run_under(&settings, Duration::from_secs(30), Redaction::On)
        })
    }

    fn read(&self, secrets: &RunSecrets) -> Result<secrecy::SecretString, SecretError> {
        self.runtime.block_on(secrets.resolve(
            &reference("azure-kv:shop-vault/portal"),
            &Purpose {
                placeholder: "portal_password",
                domain: Some("shop.test"),
                domains: &["shop.test".to_owned()],
                layer: Some(SettingsLayer::UserConfiguration),
                run_id: None,
            },
        ))
    }

    fn requests(&self) -> Vec<String> {
        self.requests.lock().unwrap().clone()
    }
}

/// The Run's settings: the Operator policy's `policy`, and user
/// configuration's `configuration`.
fn run_settings(policy: Value, configuration: Value) -> Resolved {
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

/// Each ambient source gives the access token that reads the secret, asked
/// at its documented endpoint; the value read is held for the run, and
/// neither it nor any credential is in any output.
/// Covers: story:4-50
#[rstest]
#[case::a_workload_identity(Ambient::WorkloadIdentity, AUTHORITY_TOKEN)]
#[case::the_hosts_managed_identity(Ambient::HostIdentity, HOST_TOKEN)]
#[case::the_virtual_machines_identity(Ambient::Imds, IMDS_TOKEN)]
#[case::a_user_assigned_identity(Ambient::UserAssigned, IMDS_TOKEN)]
fn each_ambient_source_gives_the_token_and_no_credential_reaches_an_output(
    #[case] ambient: Ambient,
    #[case] identity: &str,
) {
    let fixture = fixture(Instance::WithIdentity);
    let secrets = fixture.driver(ambient, json!({}));

    let resolved = fixture.read(&secrets).unwrap();
    let held = fixture.runtime.block_on(secrets.held());
    let formatted = format!("{resolved:?} {held:?} {secrets:?}");

    let identity = if identity == AUTHORITY_TOKEN {
        format!("POST {identity}")
    } else {
        format!("GET {identity}")
    };
    assert_eq!(
        (
            resolved.expose_secret() == fixture.held.password,
            fixture.requests(),
            leaks(
                &fixture.canaries,
                &fixture.env,
                &[("formatted", formatted.as_bytes())]
            ),
        ),
        (
            true,
            vec![identity, format!("GET {READ}")],
            Vec::<String>::new()
        )
    );
}

/// Each source's failure is `secret_unavailable` naming the source and
/// what to fix, and no credential: a refused federated token, a missing
/// token file, an identity the virtual machine lacks, a host off Azure or
/// whose metadata service is silent, an identity endpoint the egress
/// policy refuses, no source and an unknown `AZURE_TOKEN_CREDENTIALS`.
/// Covers: story:4-50, error:secret_unavailable, setting:egress.no_private_endpoints
#[rstest]
#[case::a_refused_federated_token(Ambient::ExpiredFederatedToken, Instance::WithIdentity, false, SecretFailure::ProviderFailed, &["Microsoft Entra ID", "workload identity of `AZURE_FEDERATED_TOKEN_FILE` was refused"])]
#[case::a_missing_token_file(Ambient::MissingTokenFile, Instance::WithIdentity, false, SecretFailure::NotConfigured, &["AZURE_FEDERATED_TOKEN_FILE", "cannot be read"])]
#[case::an_identity_the_host_lacks(Ambient::OtherIdentity, Instance::WithIdentity, false, SecretFailure::NotConfigured, &["Azure instance metadata service", "`AZURE_CLIENT_ID`"])]
#[case::a_virtual_machine_without_identity(Ambient::Imds, Instance::WithoutIdentity, false, SecretFailure::NotConfigured, &["Azure instance metadata service", "has no managed identity"])]
#[case::a_host_off_azure(Ambient::Imds, Instance::NotAzure, false, SecretFailure::NotConfigured, &["Azure instance metadata service", "no shape of Azure's"])]
#[case::a_silent_metadata_service(Ambient::Imds, Instance::Silent, false, SecretFailure::NotConfigured, &["Azure instance metadata service", "no token within 1 s"])]
#[case::the_metadata_service_under_no_private_endpoints(Ambient::Imds, Instance::WithIdentity, true, SecretFailure::ProviderFailed, &["Azure instance metadata service", "egress.no_private_endpoints"])]
#[case::the_hosts_endpoint_under_no_private_endpoints(Ambient::HostIdentity, Instance::WithIdentity, true, SecretFailure::ProviderFailed, &["`IDENTITY_ENDPOINT`", "egress.no_private_endpoints"])]
#[case::no_source(Ambient::Nothing, Instance::WithIdentity, false, SecretFailure::NotConfigured, &["AZURE_CLIENT_SECRET", "AZURE_FEDERATED_TOKEN_FILE", "managed identity"])]
#[case::an_unknown_choice(Ambient::UnknownChoice, Instance::WithIdentity, false, SecretFailure::NotConfigured, &["AZURE_TOKEN_CREDENTIALS", "`everything`"])]
fn each_sources_failure_names_it_and_no_credential(
    #[case] ambient: Ambient,
    #[case] instance: Instance,
    #[case] no_private_endpoints: bool,
    #[case] failure: SecretFailure,
    #[case] names: &[&str],
) {
    let fixture = fixture(instance);
    let policy = if no_private_endpoints {
        json!({ "egress": { "no_private_endpoints": true } })
    } else {
        json!({})
    };
    let secrets = fixture.driver(ambient, policy);

    let (found, message) = unavailable(fixture.read(&secrets));

    let missing: Vec<&str> = names
        .iter()
        .copied()
        .filter(|name| !message.contains(name))
        .collect();
    let read = fixture.requests().contains(&format!("GET {READ}"));
    assert_eq!(
        (
            found,
            missing,
            read,
            leaks(
                &fixture.canaries,
                &fixture.env,
                &[("message", message.as_bytes())]
            ),
        ),
        (failure, Vec::<&str>::new(), false, Vec::<String>::new()),
        "{message}"
    );
}
