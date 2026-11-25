//! GCP Secret Manager's provider reading with the access token each
//! ambient source gives (issue 1658): the engine's environment's token, a
//! service account's key file, whose signed JWT Google's token endpoint
//! exchanges, and the metadata server's service account, each against a
//! loopback stand-in of its documented endpoint (ADR 0062), so no test
//! reaches Google Cloud (ADR 0076). The value read through each is found
//! in no output, nor is any credential, the key file's private key
//! included (the Secret canary harness, #379); each source's failure is
//! `secret_unavailable` naming it.

#![expect(
    clippy::disallowed_types,
    reason = "the stand-in server, outside the engine and any loom model, opens its own listener and records its requests behind a standard mutex (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use data_encoding::{BASE64, BASE64URL_NOPAD};
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
use goodfetch_test_support::{Canaries, Canary, SecretKind, TestEnv};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use crate::support::{leaks, reference, runtime, table, unavailable};

/// The committed test key a service account's key file holds, which only
/// this stand-in knows (`ci/gitleaks.toml`).
const PRIVATE_KEY: &str = include_str!("../fixtures/service-account-key.pem");

/// The service account the token endpoint gives a token to, and one whose
/// key it refuses.
const ACCOUNT: &str = "reader@shop.iam.gserviceaccount.com";
const REVOKED: &str = "revoked@shop.iam.gserviceaccount.com";

/// The secret version's access path.
const ACCESS: &str = "/v1/projects/shop/secrets/portal/versions/latest:access";

/// The metadata server's token path.
const METADATA_TOKEN: &str = "/computeMetadata/v1/instance/service-accounts/default/token";

/// What the stand-in holds and hands out, each a canary: the secret's
/// value and the access token every source gives.
struct Held {
    password: String,
    access_token: String,
}

/// How the stand-in's metadata server behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Instance {
    /// It gives the default service account's token.
    WithAccount,
    /// The instance has no service account.
    WithoutAccount,
    /// Another server answers, without `Metadata-Flavor: Google`.
    NotGoogle,
    /// It holds the token request unanswered.
    Silent,
}

/// The stand-in's answer to `method` on `path` with `headers` and `body`,
/// with the headers it adds.
async fn answer(
    held: &Held,
    instance: Instance,
    method: &Method,
    path: &str,
    header: impl Fn(&str) -> String,
    body: &[u8],
) -> (StatusCode, Vec<(&'static str, &'static str)>, String) {
    let token = || {
        json!({ "access_token": held.access_token, "expires_in": 3599, "token_type": "Bearer" })
            .to_string()
    };
    let google = vec![("metadata-flavor", "Google")];
    match (method.as_str(), path) {
        ("GET", METADATA_TOKEN) if header("metadata-flavor") == "Google" => match instance {
            Instance::WithAccount => (StatusCode::OK, google, token()),
            Instance::WithoutAccount => (StatusCode::NOT_FOUND, google, String::new()),
            Instance::NotGoogle => (StatusCode::OK, Vec::new(), token()),
            Instance::Silent => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                (StatusCode::GATEWAY_TIMEOUT, Vec::new(), String::new())
            }
        },
        ("POST", "/token") => {
            let form: BTreeMap<String, String> =
                url::form_urlencoded::parse(body).into_owned().collect();
            let claims = form
                .get("assertion")
                .and_then(|jwt| jwt.split('.').nth(1))
                .and_then(|claims| BASE64URL_NOPAD.decode(claims.as_bytes()).ok())
                .and_then(|claims| serde_json::from_slice::<Value>(&claims).ok())
                .unwrap_or_default();
            let granted = form.get("grant_type").map(String::as_str)
                == Some("urn:ietf:params:oauth:grant-type:jwt-bearer")
                && claims["iss"] == ACCOUNT
                && claims["scope"] == "https://www.googleapis.com/auth/cloud-platform";
            if granted {
                (StatusCode::OK, Vec::new(), token())
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    Vec::new(),
                    json!({ "error": "invalid_grant", "error_description": "Invalid JWT Signature." })
                        .to_string(),
                )
            }
        }
        ("GET", ACCESS) => {
            if header("authorization") == format!("Bearer {}", held.access_token) {
                (
                    StatusCode::OK,
                    Vec::new(),
                    json!({
                        "name": "projects/123/secrets/portal/versions/1",
                        "payload": { "data": BASE64.encode(held.password.as_bytes()) }
                    })
                    .to_string(),
                )
            } else {
                (
                    StatusCode::UNAUTHORIZED,
                    Vec::new(),
                    json!({ "error": { "code": 401, "message": "Request had invalid authentication credentials.", "status": "UNAUTHENTICATED" } })
                        .to_string(),
                )
            }
        }
        _ => (StatusCode::NOT_FOUND, Vec::new(), String::new()),
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
    keys: PathBuf,
    env: TestEnv,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the stand-in server, outside the engine, runs its accept loop and connections on the test's runtime (ADR 0062)"
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
    ];
    let key_line = PRIVATE_KEY
        .lines()
        .nth(3)
        .expect("invariant: the test key spans more than three lines");
    canaries.add(Canary::known(
        "service-account-key",
        SecretKind::EndpointToken,
        key_line,
    ));
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            0,
        )))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let keys = env.folder().join("keys");
    write_key_files(&keys, port, &mut canaries);
    for (id, _) in &planted {
        canaries.record_planted(id, "the stand-in's credentials");
    }
    let [password, access_token] = planted.map(|(_, value)| value);
    let held = Arc::new(Held {
        password,
        access_token,
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
                        let headers = request.headers().clone();
                        let header = |name: &str| {
                            headers
                                .get(name)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_owned()
                        };
                        let body = request.into_body().collect().await.unwrap().to_bytes();
                        let (status, added, text) =
                            answer(&held, instance, &method, &path, header, &body).await;
                        let mut response = Response::new(Full::new(Bytes::from(text)));
                        *response.status_mut() = status;
                        for (name, value) in added {
                            response
                                .headers_mut()
                                .insert(name, hyper::header::HeaderValue::from_static(value));
                        }
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
        keys,
        env,
    }
}

/// The key files the tests name, in `keys`, their token endpoint the
/// stand-in on `port`: a service account's, a revoked one's, one whose
/// token endpoint is plain HTTP off loopback, and gcloud's user
/// credentials; each a planted copy of the private key's canary.
#[expect(
    clippy::disallowed_methods,
    reason = "the test writes its fixture files before any engine runs, on no async worker"
)]
fn write_key_files(keys: &Path, port: u16, canaries: &mut Canaries) {
    std::fs::create_dir_all(keys).unwrap();
    let key_file = |email: &str, token_uri: String| {
        json!({
            "type": "service_account",
            "project_id": "shop",
            "private_key_id": "k1",
            "private_key": PRIVATE_KEY,
            "client_email": email,
            "client_id": "1",
            "token_uri": token_uri,
        })
        .to_string()
    };
    let token_uri = format!("http://127.0.0.1:{port}/token");
    let files = [
        ("service-account.json", key_file(ACCOUNT, token_uri.clone())),
        ("revoked.json", key_file(REVOKED, token_uri)),
        (
            "plain.json",
            key_file(ACCOUNT, "http://192.0.2.7/token".to_owned()),
        ),
        (
            "user.json",
            json!({ "type": "authorized_user", "client_id": "c", "client_secret": "s", "refresh_token": "r" })
                .to_string(),
        ),
    ];
    for (name, text) in &files {
        std::fs::write(keys.join(name), text).unwrap();
        canaries.record_planted(
            "service-account-key",
            &keys.join(name).display().to_string(),
        );
    }
}

/// Which source the engine's environment names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Ambient {
    /// The access token itself.
    Token,
    /// Another access token.
    WrongToken,
    /// A service account's key file.
    KeyFile,
    /// The key file of a service account whose key is refused.
    RevokedKeyFile,
    /// A key file whose token endpoint is plain HTTP off loopback.
    PlainTokenUri,
    /// gcloud's user credentials, not a service account's key.
    UserCredentials,
    /// A key file that does not exist.
    MissingKeyFile,
    /// The metadata server.
    Metadata,
    /// Nothing, with the metadata server turned off.
    Nothing,
}

impl Fixture {
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The engine's environment naming `ambient`.
    fn environment(&self, ambient: Ambient) -> BTreeMap<OsString, OsString> {
        let key_file = |name: &str| {
            vec![(
                "GOOGLE_APPLICATION_CREDENTIALS",
                self.keys.join(name).display().to_string(),
            )]
        };
        let variables = match ambient {
            Ambient::Token => vec![("GOOGLE_OAUTH_ACCESS_TOKEN", self.held.access_token.clone())],
            Ambient::WrongToken => vec![("GOOGLE_OAUTH_ACCESS_TOKEN", "another".to_owned())],
            Ambient::KeyFile => key_file("service-account.json"),
            Ambient::RevokedKeyFile => key_file("revoked.json"),
            Ambient::PlainTokenUri => key_file("plain.json"),
            Ambient::UserCredentials => key_file("user.json"),
            Ambient::MissingKeyFile => key_file("absent.json"),
            Ambient::Metadata => vec![("GCE_METADATA_HOST", format!("127.0.0.1:{}", self.port))],
            Ambient::Nothing => vec![("NO_GCE_CHECK", "True".to_owned())],
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
            "secret_providers": { "gcp_sm": { "endpoint": self.base() } }
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
            &reference("gcp-sm:shop/portal/latest"),
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

/// Each ambient source gives the access token that reads the secret's
/// version, asked at its documented endpoint; the value read is held for
/// the run, and neither it nor any credential, the key file's private key
/// included, is in any output.
/// Covers: story:4-50
#[rstest]
#[case::the_environments_token(Ambient::Token, &[])]
#[case::a_service_accounts_key_file(Ambient::KeyFile, &["POST /token"])]
#[case::the_metadata_server(Ambient::Metadata, &[
    "GET /computeMetadata/v1/instance/service-accounts/default/token",
])]
fn each_ambient_source_gives_the_token_and_no_credential_reaches_an_output(
    #[case] ambient: Ambient,
    #[case] identity: &[&str],
) {
    let fixture = fixture(Instance::WithAccount);
    let secrets = fixture.driver(ambient, json!({}));

    let resolved = fixture.read(&secrets).unwrap();
    let held = fixture.runtime.block_on(secrets.held());
    let formatted = format!("{resolved:?} {held:?} {secrets:?}");

    let mut expected: Vec<String> = identity.iter().map(|&line| line.to_owned()).collect();
    expected.push(format!("GET {ACCESS}"));
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
        (true, expected, Vec::<String>::new())
    );
}

/// Each source's failure is `secret_unavailable` naming the source and
/// what to fix, and no credential: a refused token or key, a key file
/// that is missing or no service account's, a token endpoint the
/// plain-HTTP rule refuses, a metadata server the egress policy refuses,
/// and a host whose metadata server lacks an account, is not Google's or
/// is silent.
/// Covers: story:4-50, error:secret_unavailable, setting:egress.no_private_endpoints
#[rstest]
#[case::a_refused_token(Ambient::WrongToken, Instance::WithAccount, false, SecretFailure::ProviderFailed, &["access token from `GOOGLE_OAUTH_ACCESS_TOKEN` was refused"])]
#[case::a_refused_key(Ambient::RevokedKeyFile, Instance::WithAccount, false, SecretFailure::ProviderFailed, &["Google's token endpoint", "GOOGLE_APPLICATION_CREDENTIALS"])]
#[case::a_plain_token_endpoint_off_loopback(Ambient::PlainTokenUri, Instance::WithAccount, false, SecretFailure::NotConfigured, &["`token_uri`", "https:"])]
#[case::a_users_credentials(Ambient::UserCredentials, Instance::WithAccount, false, SecretFailure::NotConfigured, &["GOOGLE_APPLICATION_CREDENTIALS", "`authorized_user`"])]
#[case::a_missing_key_file(Ambient::MissingKeyFile, Instance::WithAccount, false, SecretFailure::NotConfigured, &["GOOGLE_APPLICATION_CREDENTIALS", "cannot be read"])]
#[case::an_instance_without_an_account(Ambient::Metadata, Instance::WithoutAccount, false, SecretFailure::NotConfigured, &["GCP metadata server", "has no service account"])]
#[case::a_host_off_google_cloud(Ambient::Metadata, Instance::NotGoogle, false, SecretFailure::NotConfigured, &["GCP metadata server", "Metadata-Flavor: Google"])]
#[case::a_silent_metadata_server(Ambient::Metadata, Instance::Silent, false, SecretFailure::NotConfigured, &["GCP metadata server", "no token within 1 s"])]
#[case::the_metadata_server_under_no_private_endpoints(Ambient::Metadata, Instance::WithAccount, true, SecretFailure::ProviderFailed, &["GCP metadata server", "egress.no_private_endpoints"])]
#[case::no_source(Ambient::Nothing, Instance::WithAccount, false, SecretFailure::NotConfigured, &["GOOGLE_OAUTH_ACCESS_TOKEN", "GOOGLE_APPLICATION_CREDENTIALS", "service account attached"])]
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
    let read =
        ambient != Ambient::WrongToken && fixture.requests().contains(&format!("GET {ACCESS}"));
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
