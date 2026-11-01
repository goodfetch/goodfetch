//! The three cloud managers' providers, AWS Secrets Manager's, GCP Secret
//! Manager's and Azure Key Vault's, as the crate's contribution registers
//! them and the driver builds them for a Run from its settings, against
//! one loopback stand-in that answers each service's documented HTTP API
//! (ADR 0062): a value planted in the stand-in resolves at the moment of
//! use and is found in no output (the Secret canary harness, #379); a
//! missing secret is `missing`, and a denied access, refused credentials
//! and an unreachable service are each `provider_failed` naming the
//! service and the failure, never a value; the endpoint is an
//! Operator-configured endpoint, refused on loopback only under
//! `egress.no_private_endpoints`; and missing credentials are
//! `not_configured` with a fix.

#![expect(
    clippy::disallowed_types,
    reason = "the stand-in server, outside the engine and any loom model, opens its own listener and counts its requests with standard atomics (ADR 0062)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
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
use goodfetch_protocol::secret_reference::SecretReference;
use goodfetch_protocol::secrets::SecretFailure;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{Canaries, ProvingGround, SecretKind, TestEnv};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use rstest::rstest;
use secrecy::ExposeSecret as _;
use serde_json::{Value, json};

use crate::support::{leaks, runtime, table, unavailable};

/// The credentials the stand-in takes; none has a real credential's shape.
const AWS_KEY_ID: &str = "stand-in-key-id";
const AWS_SECRET: &str = "stand-in-secret-key";
const GCP_TOKEN: &str = "stand-in-gcp-token";
const AZURE_TENANT: &str = "shop-tenant";
const AZURE_CLIENT: &str = "shop-app";
const AZURE_CLIENT_SECRET: &str = "stand-in-client-secret";
const AZURE_ACCESS_TOKEN: &str = "stand-in-azure-token";

/// Which cloud a case drives.
#[derive(Clone, Copy, Debug)]
enum Cloud {
    Aws,
    Gcp,
    Azure,
}

impl Cloud {
    /// The reference to the planted value, to a secret the stand-in
    /// denies, and to one it does not hold.
    fn reference(self, secret: &str) -> SecretReference {
        let text = match self {
            Self::Aws => format!("aws-sm:prod/{secret}#password"),
            Self::Gcp => format!("gcp-sm:shop/{secret}/latest"),
            Self::Azure => format!("azure-kv:shop-vault/{secret}"),
        };
        SecretReference::parse(&text).unwrap()
    }

    /// The service's name, which its failures give.
    fn service(self) -> &'static str {
        match self {
            Self::Aws => "AWS Secrets Manager",
            Self::Gcp => "GCP Secret Manager",
            Self::Azure => "Azure Key Vault",
        }
    }

    /// User configuration pointing the provider at `base`.
    fn configuration(self, base: &str) -> Value {
        match self {
            Self::Aws => json!({
                "secret_providers": { "aws_sm": { "region": "eu-west-3", "endpoint": base } }
            }),
            Self::Gcp => json!({ "secret_providers": { "gcp_sm": { "endpoint": base } } }),
            Self::Azure => json!({
                "secret_providers": {
                    "azure_kv": { "endpoint": format!("{base}/{{vault}}"), "authority": base }
                }
            }),
        }
    }

    /// The engine's environment signing the provider in, with its secret
    /// credential replaced by `secret` when one is given, or without any.
    fn environment(self, credentials: Credentials) -> BTreeMap<OsString, OsString> {
        let wrong = "a-wrong-credential";
        let variables: Vec<(&str, &str)> = match (self, credentials) {
            (_, Credentials::None) => Vec::new(),
            (Self::Aws, credentials) => vec![
                (
                    "AWS_ACCESS_KEY_ID",
                    if credentials == Credentials::Right {
                        AWS_KEY_ID
                    } else {
                        wrong
                    },
                ),
                ("AWS_SECRET_ACCESS_KEY", AWS_SECRET),
            ],
            (Self::Gcp, credentials) => vec![(
                "GOOGLE_OAUTH_ACCESS_TOKEN",
                if credentials == Credentials::Right {
                    GCP_TOKEN
                } else {
                    wrong
                },
            )],
            (Self::Azure, credentials) => vec![
                ("AZURE_TENANT_ID", AZURE_TENANT),
                ("AZURE_CLIENT_ID", AZURE_CLIENT),
                (
                    "AZURE_CLIENT_SECRET",
                    if credentials == Credentials::Right {
                        AZURE_CLIENT_SECRET
                    } else {
                        wrong
                    },
                ),
            ],
        };
        variables
            .into_iter()
            .map(|(name, value)| (OsString::from(name), OsString::from(value)))
            .collect()
    }
}

/// Which credentials the engine's environment holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Credentials {
    Right,
    Wrong,
    None,
}

/// What the stand-in holds: `portal`'s `password`, the value planted.
struct Store {
    password: String,
}

/// The stand-in's answer to a request of `method` to `path`, with
/// `authorization` and `target` headers and `body`, as each service's API
/// documents it.
fn answer(
    store: &Store,
    method: &Method,
    path: &str,
    authorization: &str,
    target: &str,
    body: &[u8],
) -> (StatusCode, Value) {
    if method == Method::POST && path == "/" {
        aws(store, authorization, target, body)
    } else if let Some(rest) = path.strip_prefix("/v1/projects/shop/secrets/") {
        gcp(store, authorization, rest)
    } else if path == format!("/{AZURE_TENANT}/oauth2/v2.0/token") {
        azure_token(body)
    } else if let Some(secret) = path.strip_prefix("/shop-vault/secrets/") {
        azure(store, authorization, secret)
    } else {
        (StatusCode::NOT_FOUND, json!({}))
    }
}

/// `GetSecretValue`, signed with Signature Version 4 by [`AWS_KEY_ID`].
fn aws(store: &Store, authorization: &str, target: &str, body: &[u8]) -> (StatusCode, Value) {
    let error = |kind: &str, message: &str| {
        (
            StatusCode::BAD_REQUEST,
            json!({ "__type": kind, "Message": message }),
        )
    };
    let signed = authorization.starts_with(&format!("AWS4-HMAC-SHA256 Credential={AWS_KEY_ID}/"))
        && authorization.contains("/eu-west-3/secretsmanager/aws4_request, ")
        && authorization.contains("SignedHeaders=content-type;host;x-amz-date;x-amz-target, ")
        && authorization.contains("Signature=");
    if !signed {
        return error(
            "UnrecognizedClientException",
            "The security token included in the request is invalid.",
        );
    }
    if target != "secretsmanager.GetSecretValue" {
        return error("UnknownOperationException", "");
    }
    let request: Value = serde_json::from_slice(body).unwrap_or_default();
    match request["SecretId"].as_str() {
        Some("prod/portal") => (
            StatusCode::OK,
            json!({
                "ARN": "arn:aws:secretsmanager:eu-west-3:123456789012:secret:prod/portal-a1b2c3",
                "Name": "prod/portal",
                "SecretString": json!({ "password": store.password, "user": "shop" }).to_string(),
                "VersionId": "a1b2c3d4-5678-90ab-cdef-EXAMPLE11111",
                "VersionStages": ["AWSCURRENT"],
                "CreatedDate": 1_761_480_000.0
            }),
        ),
        Some("prod/admin") => error(
            "AccessDeniedException",
            "User: arn:aws:iam::123456789012:user/shop is not authorized to perform: secretsmanager:GetSecretValue on resource: prod/admin",
        ),
        _ => error(
            "ResourceNotFoundException",
            "Secrets Manager can't find the specified secret.",
        ),
    }
}

/// `AccessSecretVersion`, with the bearer token [`GCP_TOKEN`].
fn gcp(store: &Store, authorization: &str, rest: &str) -> (StatusCode, Value) {
    let error = |status: StatusCode, kind: &str, message: &str| {
        (
            status,
            json!({ "error": { "code": status.as_u16(), "message": message, "status": kind } }),
        )
    };
    if authorization != format!("Bearer {GCP_TOKEN}") {
        return error(
            StatusCode::UNAUTHORIZED,
            "UNAUTHENTICATED",
            "Request had invalid authentication credentials.",
        );
    }
    match rest {
        "portal/versions/latest:access" => (
            StatusCode::OK,
            json!({
                "name": "projects/123/secrets/portal/versions/1",
                "payload": {
                    "data": data_encoding::BASE64.encode(store.password.as_bytes()),
                    "dataCrc32c": "1"
                }
            }),
        ),
        "admin/versions/latest:access" => error(
            StatusCode::FORBIDDEN,
            "PERMISSION_DENIED",
            "Permission 'secretmanager.versions.access' denied for resource 'projects/shop/secrets/admin/versions/latest' (or it may not exist).",
        ),
        _ => error(
            StatusCode::NOT_FOUND,
            "NOT_FOUND",
            "Secret [projects/123/secrets/absent] not found or has no versions.",
        ),
    }
}

/// Microsoft Entra ID's client-credentials grant, for [`AZURE_CLIENT`]
/// with [`AZURE_CLIENT_SECRET`].
fn azure_token(body: &[u8]) -> (StatusCode, Value) {
    let form: BTreeMap<String, String> = url::form_urlencoded::parse(body).into_owned().collect();
    let granted = form.get("grant_type").map(String::as_str) == Some("client_credentials")
        && form.get("client_id").map(String::as_str) == Some(AZURE_CLIENT)
        && form.get("client_secret").map(String::as_str) == Some(AZURE_CLIENT_SECRET)
        && form.get("scope").map(String::as_str) == Some("https://vault.azure.net/.default");
    if granted {
        (
            StatusCode::OK,
            json!({ "token_type": "Bearer", "expires_in": 3599, "ext_expires_in": 3599, "access_token": AZURE_ACCESS_TOKEN }),
        )
    } else {
        (
            StatusCode::UNAUTHORIZED,
            json!({
                "error": "invalid_client",
                "error_description": "AADSTS7000215: Invalid client secret provided.",
                "error_codes": [7_000_215]
            }),
        )
    }
}

/// Get Secret, with the bearer token the authority issued.
fn azure(store: &Store, authorization: &str, secret: &str) -> (StatusCode, Value) {
    let error = |status, code: &str, message: &str| {
        (
            status,
            json!({ "error": { "code": code, "message": message } }),
        )
    };
    if authorization != format!("Bearer {AZURE_ACCESS_TOKEN}") {
        return error(
            StatusCode::UNAUTHORIZED,
            "Unauthorized",
            "AKV10000: Request is missing a Bearer or PoP token.",
        );
    }
    match secret {
        "portal" => (
            StatusCode::OK,
            json!({
                "value": store.password,
                "id": "https://shop-vault.vault.azure.net/secrets/portal/1",
                "attributes": { "enabled": true }
            }),
        ),
        "admin" => error(
            StatusCode::FORBIDDEN,
            "Forbidden",
            "Caller is not authorized to perform action on resource.",
        ),
        _ => error(
            StatusCode::NOT_FOUND,
            "SecretNotFound",
            "A secret with (name/id) absent was not found in this key vault.",
        ),
    }
}

/// One test's runtime, home, canaries and stand-in.
struct Fixture {
    env: TestEnv,
    runtime: tokio::runtime::Runtime,
    canaries: Canaries,
    port: u16,
    received: Arc<AtomicUsize>,
    password: String,
}

#[expect(
    clippy::disallowed_methods,
    reason = "the stand-in server, outside the engine, runs its accept loop and connections on the test's runtime (ADR 0062)"
)]
fn fixture() -> Fixture {
    let env = TestEnv::new();
    let runtime = runtime();
    let mut canaries = Canaries::new(env.seed());
    let canary = canaries.plant(SecretKind::TypedPassword);
    let (id, password) = (canary.id().to_owned(), canary.value().to_owned());
    canaries.record_planted(&id, "the stand-in's secret store");
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind(SocketAddr::new(
            Ipv4Addr::LOCALHOST.into(),
            0,
        )))
        .unwrap();
    let port = listener.local_addr().unwrap().port();
    let received = Arc::new(AtomicUsize::new(0));
    let store = Arc::new(Store {
        password: password.clone(),
    });
    let counted = Arc::clone(&received);
    runtime.spawn(async move {
        loop {
            let Ok((tcp, _)) = listener.accept().await else {
                return;
            };
            let (store, counted) = (Arc::clone(&store), Arc::clone(&counted));
            let service =
                hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
                    counted.fetch_add(1, Ordering::SeqCst);
                    let store = Arc::clone(&store);
                    async move {
                        let header = |name: &str| {
                            request
                                .headers()
                                .get(name)
                                .and_then(|value| value.to_str().ok())
                                .unwrap_or_default()
                                .to_owned()
                        };
                        let (method, path) =
                            (request.method().clone(), request.uri().path().to_owned());
                        let (authorization, target) =
                            (header("authorization"), header("x-amz-target"));
                        let body = request.into_body().collect().await.unwrap().to_bytes();
                        let (status, answer) =
                            answer(&store, &method, &path, &authorization, &target, &body);
                        let mut response =
                            Response::new(Full::new(Bytes::from(answer.to_string())));
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
        env,
        runtime,
        canaries,
        port,
        received,
        password,
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

impl Fixture {
    /// The stand-in's base URL.
    fn base(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// The driver over the crate's contribution, with the engine's
    /// `environment`, for a Run whose settings are `settings`.
    fn driver(&self, environment: BTreeMap<OsString, OsString>, settings: &Resolved) -> RunSecrets {
        let home = Home::new(self.env.home().to_path_buf());
        #[expect(
            clippy::disallowed_methods,
            reason = "the test's Secrets draw their retry jitter from a seeded Entropy (ADR 0059)"
        )]
        let entropy = goodfetch_engine::entropy::Entropy::seeded(0);
        self.runtime.block_on(async {
            Composition::default()
                .with(goodfetch_secret_providers::contribution())
                .secret_providers()
                .into_iter()
                .fold(
                    Secrets::new(
                        Carried {
                            environment,
                            keychain: Keychain::absent(),
                            home: home.clone(),
                            entropy,
                        },
                        Arc::new(AuditLog::of(&home, None)),
                        Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
                        Clock::system(),
                    ),
                    |secrets, (provider, factory)| secrets.with_configured(provider, factory),
                )
                .for_run_under(settings, Duration::from_secs(30), Redaction::On)
        })
    }

    /// The driver for `cloud` pointed at the stand-in, with `credentials`,
    /// under the Operator policy `policy`.
    fn over(&self, cloud: Cloud, credentials: Credentials, policy: Value) -> RunSecrets {
        self.driver(
            cloud.environment(credentials),
            &run_settings(policy, cloud.configuration(&self.base())),
        )
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
        self.received.load(Ordering::SeqCst)
    }
}

/// A value planted in each cloud's stand-in resolves at the moment of use,
/// through the provider the crate's contribution builds for the Run, is
/// held for the run and is in no output; a secret the service does not
/// hold is `missing`.
/// Covers: story:4-50, setting:secret_providers.aws_sm.region, setting:secret_providers.aws_sm.endpoint, setting:secret_providers.gcp_sm.endpoint, setting:secret_providers.azure_kv.endpoint, setting:secret_providers.azure_kv.authority
#[rstest]
#[case::aws_secrets_manager(Cloud::Aws)]
#[case::gcp_secret_manager(Cloud::Gcp)]
#[case::azure_key_vault(Cloud::Azure)]
fn a_planted_value_resolves_through_each_cloud_and_is_found_in_no_output(#[case] cloud: Cloud) {
    let fixture = fixture();
    let secrets = fixture.over(cloud, Credentials::Right, json!({}));

    let resolved = fixture
        .through(&secrets, &cloud.reference("portal"))
        .unwrap();
    let absent = unavailable(fixture.through(&secrets, &cloud.reference("absent"))).0;
    let held = fixture.runtime.block_on(secrets.held());
    let formatted = format!("{resolved:?} {held:?} {secrets:?}");

    assert_eq!(
        (
            resolved.expose_secret() == fixture.password,
            absent,
            leaks(
                &fixture.canaries,
                &fixture.env,
                &[("formatted", formatted.as_bytes())]
            ),
        ),
        (true, SecretFailure::Missing, Vec::<String>::new())
    );
}

/// What a failure case points the provider at.
#[derive(Clone, Copy, Debug)]
enum Failing {
    /// A secret the service denies the credentials.
    Denied,
    /// Credentials the service refuses.
    RefusedCredentials,
    /// The Proving ground's `down` address, where nothing listens.
    Unreachable,
}

/// The base URL of a service nothing answers at: the Proving ground's
/// `down` address, where nothing listens and which no other program can
/// take while the Proving ground runs. A port bound and let go could be
/// taken, and answered, by any program before the provider connects
/// (#1796).
fn unreachable_base(proving_ground: &ProvingGround) -> String {
    format!(
        "http://{}",
        proving_ground.description().fixtures.proxies["down"]
    )
}

/// No other program can listen where the unreachable case's provider
/// connects, so it always meets a refused connection (#1796).
#[cfg(unix)]
#[test]
fn no_other_program_can_take_the_unreachable_services_address() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, env.seed()).unwrap();
    let base = unreachable_base(&proving_ground);

    let other = std::net::TcpListener::bind(base.trim_start_matches("http://"));

    assert_eq!(
        other.map(|_| ()).map_err(|error| error.kind()),
        Err(std::io::ErrorKind::AddrInUse)
    );
}

/// A denied access, refused credentials and an unreachable service are
/// each `secret_unavailable`, `provider_failed`, the message naming the
/// reference, the service and the failure, never a value.
/// Covers: story:4-50, error:secret_unavailable
#[rstest]
fn a_denied_refused_or_unreachable_cloud_fails_naming_it_and_no_value(
    #[values(Cloud::Aws, Cloud::Gcp, Cloud::Azure)] cloud: Cloud,
    #[values(Failing::Denied, Failing::RefusedCredentials, Failing::Unreachable)] failing: Failing,
) {
    let fixture = fixture();
    // The unreachable case's Proving ground holds its `down` address until
    // the case ends.
    let (credentials, base, secret, names, _down) = match failing {
        Failing::Denied => (Credentials::Right, fixture.base(), "admin", "may not", None),
        Failing::RefusedCredentials => (
            Credentials::Wrong,
            fixture.base(),
            "portal",
            "refused",
            None,
        ),
        Failing::Unreachable => {
            let proving_ground = ProvingGround::start(&fixture.env, fixture.env.seed()).unwrap();
            (
                Credentials::Right,
                unreachable_base(&proving_ground),
                "portal",
                "did not answer",
                Some(proving_ground),
            )
        }
    };
    let secrets = fixture.driver(
        cloud.environment(credentials),
        &run_settings(json!({}), cloud.configuration(&base)),
    );
    let reference = cloud.reference(secret);

    let (failure, message) = unavailable(fixture.through(&secrets, &reference));

    let service = match (cloud, failing) {
        (Cloud::Azure, Failing::RefusedCredentials | Failing::Unreachable) => "Microsoft Entra ID",
        _ => cloud.service(),
    };
    assert_eq!(
        (
            failure,
            message.contains(reference.as_str()),
            message.contains(service) && message.contains(names),
            leaks(
                &fixture.canaries,
                &fixture.env,
                &[("message", message.as_bytes())]
            ),
        ),
        (
            SecretFailure::ProviderFailed,
            true,
            true,
            Vec::<String>::new()
        ),
        "{message}"
    );
}

/// Story 4-16 for each cloud: its endpoint is an Operator-configured
/// endpoint, so on loopback it is reached by default, and under an Operator
/// policy that sets `egress.no_private_endpoints` it is refused before any
/// request, the failure naming the setting and no value.
/// Covers: story:4-50, setting:egress.no_private_endpoints
#[rstest]
fn each_cloud_endpoint_is_refused_on_loopback_only_under_no_private_endpoints(
    #[values(Cloud::Aws, Cloud::Gcp, Cloud::Azure)] cloud: Cloud,
    #[values(false, true)] refusing: bool,
) {
    let fixture = fixture();
    let policy = if refusing {
        json!({ "egress": { "no_private_endpoints": true } })
    } else {
        json!({})
    };
    let secrets = fixture.over(cloud, Credentials::Right, policy);

    let resolved = fixture.through(&secrets, &cloud.reference("portal"));

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
        assert!(
            resolved.unwrap().expose_secret() == fixture.password,
            "{} requests",
            fixture.received()
        );
    }
}

/// With no credentials in the engine's environment, or no AWS Region,
/// each provider is `not_configured` before any request, its fix naming
/// what to set.
/// Covers: story:4-50, error:secret_unavailable
#[rstest]
#[case::aws_without_credentials(Cloud::Aws, None, "AWS_ACCESS_KEY_ID")]
#[case::aws_without_a_region(Cloud::Aws, Some(json!({ "secret_providers": { "aws_sm": { "endpoint": "http://127.0.0.1:9" } } })), "secret_providers.aws_sm.region")]
#[case::gcp_without_a_token(Cloud::Gcp, None, "GOOGLE_OAUTH_ACCESS_TOKEN")]
#[case::azure_without_a_service_principal(Cloud::Azure, None, "AZURE_CLIENT_SECRET")]
#[case::azure_without_a_vault_in_its_endpoint(Cloud::Azure, Some(json!({ "secret_providers": { "azure_kv": { "endpoint": "https://vault.example.com" } } })), "{vault}")]
fn missing_credentials_or_settings_are_not_configured_with_a_fix(
    #[case] cloud: Cloud,
    #[case] configuration: Option<Value>,
    #[case] fix_names: &str,
) {
    let fixture = fixture();
    let credentials = if configuration.is_some() {
        Credentials::Right
    } else {
        Credentials::None
    };
    let configuration = configuration.unwrap_or_else(|| cloud.configuration(&fixture.base()));
    let secrets = fixture.driver(
        cloud.environment(credentials),
        &run_settings(json!({}), configuration),
    );

    let (failure, message) = unavailable(fixture.through(&secrets, &cloud.reference("portal")));

    assert_eq!(
        (failure, message.contains(fix_names), fixture.received()),
        (SecretFailure::NotConfigured, true, 0),
        "{message}"
    );
}
