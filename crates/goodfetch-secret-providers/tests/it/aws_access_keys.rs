//! AWS Secrets Manager's provider signing with the access key each ambient
//! source gives (issue 1658): a web identity token that AWS STS exchanges,
//! the container credentials endpoint of ECS and EKS, and the EC2
//! instance's role through the instance metadata service's version 2, each
//! against a loopback stand-in of its documented endpoint (ADR 0062), so no
//! test reaches a cloud (ADR 0076). The value read through each is found in
//! no output, nor is any credential (the Secret canary harness, #379); each
//! source's failure is `secret_unavailable` naming it.

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

/// The role the web identity token assumes and the one the instance holds.
const ROLE_ARN: &str = "arn:aws:iam::123456789012:role/shop-reader";
const INSTANCE_ROLE: &str = "shop-reader";
/// The temporary access key ID every source hands out.
const KEY_ID: &str = "ASIASTANDIN";
/// The instance metadata service's session token, which is no credential.
const METADATA_TOKEN: &str = "stand-in-metadata-session";

/// What the stand-in holds and hands out: the secret's value and the
/// credentials, each a canary.
struct Held {
    password: String,
    secret_key: String,
    session_token: String,
    web_identity_token: String,
    container_token: String,
}

/// How the stand-in's instance metadata service behaves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Instance {
    /// It gives a token and the role's credentials.
    WithRole,
    /// It gives a token but lists no role.
    WithoutRole,
    /// It answers 404 to the token request, as another cloud's service does.
    Absent,
    /// It holds the token request unanswered.
    Silent,
}

/// The stand-in's answer to `method` on `path` with `headers` and `body`.
async fn answer(
    held: &Held,
    instance: Instance,
    method: &Method,
    path: &str,
    header: impl Fn(&str) -> String,
    body: &[u8],
) -> (StatusCode, String) {
    let ok = |text: String| (StatusCode::OK, text);
    let credentials = || {
        json!({
            "Code": "Success",
            "AccessKeyId": KEY_ID,
            "SecretAccessKey": held.secret_key,
            "Token": held.session_token,
            "Expiration": "2026-11-25T18:00:00Z"
        })
        .to_string()
    };
    let metadata = header("x-aws-ec2-metadata-token") == METADATA_TOKEN;
    match (method.as_str(), path) {
        ("PUT", "/latest/api/token") => match instance {
            Instance::Absent => (StatusCode::NOT_FOUND, String::new()),
            Instance::Silent => {
                tokio::time::sleep(Duration::from_secs(30)).await;
                (StatusCode::GATEWAY_TIMEOUT, String::new())
            }
            _ if header("x-aws-ec2-metadata-token-ttl-seconds").is_empty() => {
                (StatusCode::BAD_REQUEST, String::new())
            }
            _ => ok(METADATA_TOKEN.to_owned()),
        },
        ("GET", "/latest/meta-data/iam/security-credentials/") if metadata => {
            if instance == Instance::WithRole {
                ok(format!("{INSTANCE_ROLE}\n"))
            } else {
                (StatusCode::NOT_FOUND, String::new())
            }
        }
        ("GET", "/latest/meta-data/iam/security-credentials/shop-reader") if metadata => {
            ok(credentials())
        }
        ("GET", "/container/credentials") => {
            if header("authorization") == held.container_token {
                ok(credentials())
            } else {
                (
                    StatusCode::UNAUTHORIZED,
                    json!({ "message": "Unauthorized" }).to_string(),
                )
            }
        }
        ("POST", "/sts/") => {
            let form: BTreeMap<String, String> =
                url::form_urlencoded::parse(body).into_owned().collect();
            let field = |name: &str| form.get(name).map(String::as_str);
            if field("Action") == Some("AssumeRoleWithWebIdentity")
                && field("RoleArn") == Some(ROLE_ARN)
                && field("WebIdentityToken") == Some(held.web_identity_token.as_str())
            {
                ok(format!(
                    "<AssumeRoleWithWebIdentityResponse xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\"><AssumeRoleWithWebIdentityResult><Credentials><AccessKeyId>{KEY_ID}</AccessKeyId><SecretAccessKey>{}</SecretAccessKey><SessionToken>{}</SessionToken><Expiration>2026-11-25T18:00:00Z</Expiration></Credentials><SubjectFromWebIdentityToken>system:serviceaccount:shop:reader</SubjectFromWebIdentityToken></AssumeRoleWithWebIdentityResult><ResponseMetadata><RequestId>ad4156e9-bce1-11e2-82e6-6b6efEXAMPLE</RequestId></ResponseMetadata></AssumeRoleWithWebIdentityResponse>",
                    held.secret_key, held.session_token
                ))
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    "<ErrorResponse xmlns=\"https://sts.amazonaws.com/doc/2011-06-15/\"><Error><Type>Sender</Type><Code>InvalidIdentityToken</Code><Message>Couldn't retrieve verification key from your identity provider, please reference AssumeRoleWithWebIdentity documentation for requirements</Message></Error><RequestId>ad4156e9-bce1-11e2-82e6-6b6efEXAMPLE</RequestId></ErrorResponse>".to_owned(),
                )
            }
        }
        ("POST", "/") => {
            let signed = header("authorization")
                .starts_with(&format!("AWS4-HMAC-SHA256 Credential={KEY_ID}/"))
                && header("x-amz-security-token") == held.session_token;
            if signed {
                ok(json!({
                    "Name": "prod/portal",
                    "SecretString": json!({ "password": held.password }).to_string()
                })
                .to_string())
            } else {
                (
                    StatusCode::BAD_REQUEST,
                    json!({ "__type": "UnrecognizedClientException" }).to_string(),
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
        plant(SecretKind::EndpointToken),
        plant(SecretKind::EndpointToken),
        plant(SecretKind::EndpointToken),
    ];
    let tokens = env.folder().join("tokens");
    std::fs::create_dir_all(&tokens).unwrap();
    std::fs::write(tokens.join("web-identity"), &planted[3].1).unwrap();
    std::fs::write(tokens.join("container"), format!("{}\n", planted[4].1)).unwrap();
    for (id, _) in &planted {
        canaries.record_planted(id, "the stand-in's credentials");
    }
    canaries.record_planted(
        &planted[3].0,
        &tokens.join("web-identity").display().to_string(),
    );
    canaries.record_planted(
        &planted[4].0,
        &tokens.join("container").display().to_string(),
    );
    let [
        password,
        secret_key,
        session_token,
        web_identity_token,
        container_token,
    ] = planted.map(|(_, value)| value);
    let held = Arc::new(Held {
        password,
        secret_key,
        session_token,
        web_identity_token,
        container_token,
    });
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            0,
        )))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
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
                        let (status, text) =
                            answer(&held, instance, &method, &path, header, &body).await;
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
#[derive(Clone, Copy, Debug)]
enum Ambient {
    /// A web identity token, in a file, for [`ROLE_ARN`].
    WebIdentity,
    /// A web identity token file that holds another token.
    WrongWebIdentity,
    /// A web identity token file that does not exist.
    MissingWebIdentity,
    /// The container credentials endpoint, its token in a file.
    ContainerTokenFile,
    /// The container credentials endpoint, its token in a variable.
    ContainerToken,
    /// The container credentials endpoint, with another token.
    WrongContainerToken,
    /// A container credentials endpoint on plain HTTP off loopback.
    PlainContainerEndpoint,
    /// The instance metadata service.
    InstanceMetadata,
    /// Nothing, with the instance metadata service turned off.
    Nothing,
}

impl Fixture {
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The engine's environment naming `ambient`.
    fn environment(&self, ambient: Ambient) -> BTreeMap<OsString, OsString> {
        let token = |name: &str| self.tokens.join(name).display().to_string();
        let web_identity = |file: String| {
            vec![
                ("AWS_WEB_IDENTITY_TOKEN_FILE", file),
                ("AWS_ROLE_ARN", ROLE_ARN.to_owned()),
                ("AWS_ROLE_SESSION_NAME", "shop-run".to_owned()),
                ("AWS_ENDPOINT_URL_STS", format!("{}/sts/", self.base())),
            ]
        };
        let container = |address: String| {
            (
                "AWS_CONTAINER_CREDENTIALS_FULL_URI",
                format!("{address}/container/credentials"),
            )
        };
        let variables = match ambient {
            Ambient::WebIdentity => web_identity(token("web-identity")),
            Ambient::WrongWebIdentity => web_identity(token("container")),
            Ambient::MissingWebIdentity => web_identity(token("absent")),
            Ambient::ContainerTokenFile => vec![
                container(self.base()),
                ("AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE", token("container")),
            ],
            Ambient::ContainerToken => vec![
                container(self.base()),
                (
                    "AWS_CONTAINER_AUTHORIZATION_TOKEN",
                    self.held.container_token.clone(),
                ),
            ],
            Ambient::WrongContainerToken => vec![
                container(self.base()),
                ("AWS_CONTAINER_AUTHORIZATION_TOKEN", "another".to_owned()),
            ],
            Ambient::PlainContainerEndpoint => vec![container("http://192.0.2.7".to_owned())],
            Ambient::InstanceMetadata => {
                vec![("AWS_EC2_METADATA_SERVICE_ENDPOINT", self.base())]
            }
            Ambient::Nothing => vec![("AWS_EC2_METADATA_DISABLED", "true".to_owned())],
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
            "secret_providers": { "aws_sm": { "region": "eu-west-3", "endpoint": self.base() } }
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
            &reference("aws-sm:prod/portal#password"),
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

/// Each ambient source gives the temporary access key that signs
/// `GetSecretValue`, asked at its documented endpoint in its documented
/// order; the value read is held for the run, and neither it nor any
/// credential, the token files' included, is in any output.
/// Covers: story:4-50
#[rstest]
#[case::web_identity(Ambient::WebIdentity, &["POST /sts/"])]
#[case::container_with_a_token_file(Ambient::ContainerTokenFile, &["GET /container/credentials"])]
#[case::container_with_a_token(Ambient::ContainerToken, &["GET /container/credentials"])]
#[case::instance_metadata(Ambient::InstanceMetadata, &[
    "PUT /latest/api/token",
    "GET /latest/meta-data/iam/security-credentials/",
    "GET /latest/meta-data/iam/security-credentials/shop-reader",
])]
fn each_ambient_source_signs_the_read_and_no_credential_reaches_an_output(
    #[case] ambient: Ambient,
    #[case] identity: &[&str],
) {
    let fixture = fixture(Instance::WithRole);
    let secrets = fixture.driver(ambient, json!({}));

    let resolved = fixture.read(&secrets).unwrap();
    let held = fixture.runtime.block_on(secrets.held());
    let formatted = format!("{resolved:?} {held:?} {secrets:?}");

    let mut expected: Vec<String> = identity.iter().map(|&line| line.to_owned()).collect();
    expected.push("POST /".to_owned());
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
/// what to fix, and no credential: a refused or unreadable token, an
/// endpoint the egress policy or the plain-HTTP rule refuses, and a host
/// whose instance metadata service is absent, silent or lists no role.
/// Covers: story:4-50, error:secret_unavailable, setting:egress.no_private_endpoints
#[rstest]
#[case::a_refused_web_identity_token(Ambient::WrongWebIdentity, Instance::WithRole, false, SecretFailure::ProviderFailed, &["AWS STS", "AWS_WEB_IDENTITY_TOKEN_FILE"])]
#[case::a_missing_web_identity_token(Ambient::MissingWebIdentity, Instance::WithRole, false, SecretFailure::NotConfigured, &["AWS_WEB_IDENTITY_TOKEN_FILE", "cannot be read"])]
#[case::a_refused_container_token(Ambient::WrongContainerToken, Instance::WithRole, false, SecretFailure::ProviderFailed, &["container credentials endpoint", "401"])]
#[case::a_plain_container_endpoint_off_loopback(Ambient::PlainContainerEndpoint, Instance::WithRole, false, SecretFailure::NotConfigured, &["AWS_CONTAINER_CREDENTIALS_FULL_URI", "https:"])]
#[case::an_instance_without_a_role(Ambient::InstanceMetadata, Instance::WithoutRole, false, SecretFailure::NotConfigured, &["EC2 instance metadata service", "no IAM role"])]
#[case::a_host_off_ec2(Ambient::InstanceMetadata, Instance::Absent, false, SecretFailure::NotConfigured, &["EC2 instance metadata service", "AWS_ACCESS_KEY_ID"])]
#[case::a_silent_instance_metadata_service(Ambient::InstanceMetadata, Instance::Silent, false, SecretFailure::NotConfigured, &["EC2 instance metadata service", "no token within 1 s"])]
#[case::instance_metadata_under_no_private_endpoints(Ambient::InstanceMetadata, Instance::WithRole, true, SecretFailure::ProviderFailed, &["EC2 instance metadata service", "egress.no_private_endpoints"])]
#[case::no_source(Ambient::Nothing, Instance::WithRole, false, SecretFailure::NotConfigured, &["AWS_ACCESS_KEY_ID", "AWS_WEB_IDENTITY_TOKEN_FILE", "IAM role"])]
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
    assert_eq!(
        (
            found,
            missing,
            fixture.requests().contains(&"POST /".to_owned()),
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
