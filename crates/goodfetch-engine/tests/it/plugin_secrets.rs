//! The Secrets a Plugin's `net.fetch` names by reference (ADR 0033; ticket
//! #842), against a loopback server that records each request and echoes
//! what it received in Goodfetch's encodings: the engine inserts a value
//! only toward a domain the Run binds its reference to, refuses every
//! other request before it resolves one, and hands the Plugin the response
//! with each value it resolved masked.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::mask::{Masks, Redaction, Scope};
use goodfetch_engine::plugin_gate::{Inserting, PluginFetch};
use goodfetch_engine::secrets::{Carried, Keychain, RunSecrets, Secrets};
use goodfetch_engine::settings::{Layers, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::plugin::approvals::Approvals;
use goodfetch_protocol::plugin::net::{NetFetchParams, NetFetchResult};
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{Canaries, Output, SecretKind, TestEnv};
use http_body_util::{BodyExt as _, Full};
use hyper::body::Bytes;
use hyper::{Request, Response};
use hyper_util::rt::TokioIo;
use serde_json::{Map, Value, json};
use tokio::sync::mpsc;

/// The Plugin whose `net.fetch` each test sends.
const PLUGIN: &str = "kettle";

/// The host the Run binds the Secret to, through its domain.
const BOUND: &str = "api.example.com";

/// A host the Run does not bind the Secret to.
const ELSEWHERE: &str = "elsewhere.test";

/// The reference the Plugin declares, the Operator policy approves and the
/// Run binds, with a form field.
const TOKEN: &str = "env:WEATHER_TOKEN";

/// A request the server received: its target, headers and body.
#[derive(Debug)]
struct Received {
    target: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
}

/// A plain HTTP server on loopback that records each request and answers
/// with what it received: its `authorization` header as it came, as a JSON
/// string, in Base64, Base64url and hex, and its query and body as they
/// came, as an echoing API does.
/// It serves on its own runtime, on real time, which every `net.fetch` of
/// a test is sent on too.
struct Echo {
    port: u16,
    received: mpsc::Receiver<Received>,
    runtime: tokio::runtime::Runtime,
}

impl Echo {
    fn serve() -> Self {
        let runtime = goodfetch_test_support::real_time_runtime();
        let (port, received) = runtime.block_on(Self::listen());
        Self {
            port,
            received,
            runtime,
        }
    }

    #[expect(
        clippy::disallowed_methods,
        reason = "the echoing server, outside the engine, runs its accept loop and connections on the test's runtime (ADR 0062)"
    )]
    async fn listen() -> (u16, mpsc::Receiver<Received>) {
        let listener =
            tokio::net::TcpListener::bind(SocketAddr::new(Ipv4Addr::LOCALHOST.into(), 0))
                .await
                .unwrap();
        let port = listener.local_addr().unwrap().port();
        let (record, received) = mpsc::channel(16);
        tokio::spawn(async move {
            loop {
                let (tcp, _) = listener.accept().await.unwrap();
                let record = record.clone();
                let service =
                    hyper::service::service_fn(move |request: Request<hyper::body::Incoming>| {
                        let record = record.clone();
                        async move {
                            let (parts, body) = request.into_parts();
                            let body = body.collect().await?.to_bytes().to_vec();
                            let authorization = parts
                                .headers
                                .get("authorization")
                                .map(|value| value.as_bytes().to_vec())
                                .unwrap_or_default();
                            let text = String::from_utf8_lossy(&authorization).into_owned();
                            let query = parts.uri.query().unwrap_or_default().to_owned();
                            let mut echoed = Vec::new();
                            for line in [
                                format!("raw={text}"),
                                format!("json={}", Value::String(text.clone())),
                                format!("base64={}", data_encoding::BASE64.encode(&authorization)),
                                format!(
                                    "base64url={}",
                                    data_encoding::BASE64URL_NOPAD.encode(&authorization)
                                ),
                                format!("hex={}", data_encoding::HEXLOWER.encode(&authorization)),
                                format!("query={query}"),
                            ] {
                                echoed.extend_from_slice(line.as_bytes());
                                echoed.push(b'\n');
                            }
                            echoed.extend_from_slice(b"form=");
                            echoed.extend_from_slice(&body);
                            let _ = record
                                .send(Received {
                                    target: parts.uri.to_string(),
                                    headers: parts
                                        .headers
                                        .iter()
                                        .map(|(name, value)| {
                                            (
                                                name.as_str().to_owned(),
                                                String::from_utf8_lossy(value.as_bytes())
                                                    .into_owned(),
                                            )
                                        })
                                        .collect(),
                                    body,
                                })
                                .await;
                            let response = Response::builder()
                                .header("x-echo-authorization", authorization)
                                .header("x-echo-query", query)
                                .body(Full::new(Bytes::from(echoed)))
                                .unwrap();
                            Ok::<_, hyper::Error>(response)
                        }
                    });
                tokio::spawn(
                    hyper::server::conn::http1::Builder::new()
                        .serve_connection(TokioIo::new(tcp), service),
                );
            }
        });
        (port, received)
    }

    /// Sends `params` through `fetch` while it serves.
    fn send(
        &self,
        fetch: &PluginFetch,
        params: NetFetchParams,
    ) -> Result<NetFetchResult, CallError> {
        self.runtime.block_on(fetch.fetch(params))
    }

    /// The URL of `path` at `host`, which the Run's `network.resolve` maps
    /// to the server.
    fn url(&self, host: &str, path: &str) -> String {
        format!("http://{host}:{}{path}", self.port)
    }

    /// The requests it received since the last look.
    fn received(&mut self) -> Vec<Received> {
        std::iter::from_fn(|| self.received.try_recv().ok()).collect()
    }
}

fn table(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("settings are a table"),
    }
}

/// The settings of a Run whose user configuration maps both hosts to the
/// server, gives the Plugin both as its endpoints, and binds the
/// placeholder `weather` to [`TOKEN`] for `example.com`, its value going in
/// a form field only as `api_key`.
fn run_settings(echo: &Echo) -> Resolved {
    let configuration = json!({
        "network": { "resolve": { BOUND: ["127.0.0.1"], ELSEWHERE: ["127.0.0.1"] } },
        "plugins": { PLUGIN: { "settings": {
            "api": echo.url(BOUND, "/"),
            "mirror": echo.url(ELSEWHERE, "/"),
        } } },
        "secrets": { "placeholders": {
            "weather": { "reference": TOKEN, "domains": ["example.com"], "form_field": "api_key" },
        } },
    });
    let layers = Layers {
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

/// The Plugin's manifest, declaring [`TOKEN`], `env:UNAPPROVED` and
/// `env:UNBOUND` in `permissions.secrets`.
fn manifest() -> PluginManifest {
    let manifest = json!({
        "manifest_version": "0.1",
        "name": PLUGIN,
        "version": "1.0.0",
        "kinds": ["extract"],
        "protocol": { "min": "0.1", "max": "0.1" },
        "entrypoint": "plugin",
        "permissions": { "secrets": [TOKEN, "env:UNAPPROVED", "env:UNBOUND"] },
    });
    PluginManifest::read(manifest.to_string().as_bytes()).unwrap()
}

/// What the Operator policy approved: [`TOKEN`] and `env:UNBOUND`.
fn approvals() -> Approvals {
    serde_json::from_value(json!({ "secrets": [TOKEN, "env:UNBOUND"] })).unwrap()
}

/// A Run's Secrets, [`TOKEN`]'s value `token`, recording each use in an
/// Audit log in the home of `env`, and the Plugin's `net.fetch` in it.
struct Fixture {
    audit: Arc<AuditLog>,
    secrets: RunSecrets,
    settings: Resolved,
}

impl Fixture {
    fn new(env: &TestEnv, echo: &Echo, token: &str) -> Self {
        let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let home = Home::new(env.home().to_path_buf());
        let audit = Arc::new(AuditLog::of(&home, None, Arc::clone(&owner)));
        #[expect(
            clippy::disallowed_methods,
            reason = "the test's Secrets draw their retry jitter from a seeded Entropy (ADR 0059)"
        )]
        let entropy = Entropy::seeded(0);
        let environment: BTreeMap<OsString, OsString> =
            [(OsString::from("WEATHER_TOKEN"), OsString::from(token))].into();
        let settings = run_settings(echo);
        let secrets = Secrets::new(
            Carried {
                environment,
                keychain: Keychain::absent(),
                home,
                entropy,
            },
            Arc::clone(&audit),
            owner,
            Clock::system(),
        )
        .for_run_under(&settings, Duration::from_secs(30), Redaction::On);
        Self {
            audit,
            secrets,
            settings,
        }
    }

    /// The Plugin's `net.fetch` outside a Run, inserting nothing.
    fn outside_a_run(&self) -> PluginFetch {
        PluginFetch::of(
            &self.settings,
            PLUGIN,
            Clock::system(),
            TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
        )
        .unwrap()
    }

    /// The Plugin's `net.fetch` in the Run.
    fn plugin_fetch(&self) -> PluginFetch {
        self.outside_a_run().inserting(Inserting::new(
            PLUGIN,
            (&manifest(), &approvals()),
            &self.settings,
            self.secrets.clone(),
            RunId::parse("01jx0000000000000000000842").unwrap(),
        ))
    }

    /// The engine's masks, which each value the Run resolves joins.
    fn masks(&self) -> &Masks {
        self.secrets.mask().masks()
    }

    /// Each use of a Secret reference the Audit log records: its
    /// placeholder, reference and domain.
    fn uses(&self) -> Vec<Value> {
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

/// A `net.fetch` of `params` more its `call_id`.
fn params(params: Value) -> NetFetchParams {
    let mut params = params;
    params["call_id"] = json!(1);
    serde_json::from_value(params).unwrap()
}

/// A `POST` of a form to the bound host's `/forecast?city=paris`, naming
/// [`TOKEN`] in the `authorization` header as `Bearer {}`, in the query
/// parameter `key`, and in its form field.
fn inserting_everywhere(echo: &Echo) -> NetFetchParams {
    params(json!({
        "method": "POST",
        "url": echo.url(BOUND, "/forecast?city=paris"),
        "headers": [{ "name": "content-type", "value": "application/x-www-form-urlencoded" }],
        "body": data_encoding::BASE64.encode(b"days=3"),
        "secrets": [
            { "in": "header", "name": "authorization", "reference": TOKEN, "template": "Bearer {}" },
            { "in": "query", "name": "key", "reference": TOKEN },
            { "in": "form", "reference": TOKEN },
        ],
    }))
}

/// Runs `test` with the server serving, a Run's fixture whose
/// [`TOKEN`] holds a planted header value, which ends with characters JSON
/// escaping and URL encoding change, and the canaries.
fn with_echo(test: impl FnOnce(&mut Echo, &Fixture, &Canaries)) {
    let mut canaries = Canaries::new(842);
    let token = canaries.plant(SecretKind::HeaderValue).value().to_owned();
    let env = TestEnv::new();
    let mut echo = Echo::serve();
    let fixture = Fixture::new(&env, &echo, &token);
    test(&mut echo, &fixture, &canaries);
}

/// Story 7-43: a `net.fetch` naming its Secret by reference toward a host
/// of the domain the Run binds it to reaches the server with the value in
/// the `authorization` header's template, in the query parameter and in
/// the form field the operator set, after the request's own; the Audit log
/// records each of the three uses for the host.
/// Covers: story:7-43, setting:secrets.placeholders
#[test]
fn a_plugin_secret_goes_in_its_header_query_and_form_field_toward_a_bound_domain() {
    with_echo(|echo, fixture, canaries| {
        let token = canaries.iter().next().unwrap().value().to_owned();

        let answered = echo.send(&fixture.plugin_fetch(), inserting_everywhere(echo));

        let received = echo.received();
        let request = &received[0];
        let header = |name: &str| {
            request
                .headers
                .iter()
                .find(|(header, _)| header == name)
                .map(|(_, value)| value.clone())
        };
        let query: Vec<(String, String)> =
            url::form_urlencoded::parse(request.target.split_once('?').unwrap().1.as_bytes())
                .into_owned()
                .collect();
        let form: Vec<(String, String)> = url::form_urlencoded::parse(&request.body)
            .into_owned()
            .collect();
        assert_eq!(
            (
                answered
                    .map(|result| result.status)
                    .map_err(|failure| failure.code().to_owned()),
                received.len(),
                header("authorization"),
                query,
                form,
                fixture.uses(),
            ),
            (
                Ok(200),
                1,
                Some(format!("Bearer {token}")),
                vec![
                    ("city".to_owned(), "paris".to_owned()),
                    ("key".to_owned(), token.clone()),
                ],
                vec![
                    ("days".to_owned(), "3".to_owned()),
                    ("api_key".to_owned(), token.clone()),
                ],
                vec![json!(["weather", TOKEN, BOUND]); 3],
            )
        );
    });
}

/// Story 7-44: the server echoes the value it received raw, JSON-escaped,
/// URL-encoded, in Base64 at another alignment, in Base64url and in hex, in
/// its headers and its body; the Plugin's answer holds it in none of them,
/// while the server did receive it. The URL the value was inserted in, as
/// the trace, Usage and the journal keep it, holds it in no encoding.
/// Covers: story:7-44
#[test]
fn a_plugin_secret_is_scrubbed_from_the_response_and_masked_in_the_url_it_went_in() {
    with_echo(|echo, fixture, canaries| {
        let answered = echo
            .send(&fixture.plugin_fetch(), inserting_everywhere(echo))
            .unwrap();

        let received = echo.received();
        let sent_url = format!("http://{BOUND}:{}{}", echo.port, received[0].target);
        let answer = serde_json::to_vec(&answered).unwrap();
        let body = answered.body.bytes();
        let kept = fixture.masks().apply(&sent_url, Scope::Kept);
        let kept_output = fixture.masks().apply(&sent_url, Scope::KeptOutput);
        let leaks = |outputs: &[Output<'_>]| canaries.leaks(outputs).unwrap().len();
        assert_eq!(
            (
                leaks(&[Output::Bytes {
                    name: "the server's request",
                    bytes: sent_url.as_bytes(),
                }]) > 0,
                leaks(&[
                    Output::Bytes {
                        name: "net.fetch's answer",
                        bytes: &answer,
                    },
                    Output::Bytes {
                        name: "net.fetch's body",
                        bytes: body,
                    },
                    Output::Bytes {
                        name: "the URL as kept",
                        bytes: kept.as_bytes(),
                    },
                    Output::Bytes {
                        name: "the URL as kept output",
                        bytes: kept_output.as_bytes(),
                    },
                ]),
                answered
                    .headers
                    .iter()
                    .any(|header| header.name == "x-echo-authorization"
                        && header.value == "Bearer %weather%"),
            ),
            (true, 0, true)
        );
    });
}

/// ADR 0033's refusals of a Secret a `net.fetch` names, each before any
/// value is resolved: toward a host outside the binding's domains, a
/// reference the manifest does not declare, one the Operator policy has
/// not approved, one no placeholder binds, a template without its `{}`, a
/// header the request sets already, and any Secret outside a Run. None
/// reaches the server, and the Audit log records no use.
/// Covers: story:7-43, error:secret_domain_not_allowed, error:secret_unbound, error:plugin_permission_denied, error:plugin_call_back_refused
#[test]
fn a_plugin_secret_is_refused_toward_an_unbound_domain_and_unless_declared_approved_and_bound() {
    with_echo(|echo, fixture, _| {
        let header = |url: &str, reference: &str, template: &str| {
            params(json!({
                "method": "GET",
                "url": url,
                "secrets": [{ "in": "header", "name": "authorization", "reference": reference, "template": template }],
            }))
        };
        let bound = echo.url(BOUND, "/");
        let cases = [
            (
                "elsewhere",
                header(&echo.url(ELSEWHERE, "/"), TOKEN, "Bearer {}"),
            ),
            ("undeclared", header(&bound, "env:UNDECLARED", "{}")),
            ("unapproved", header(&bound, "env:UNAPPROVED", "{}")),
            ("unbound", header(&bound, "env:UNBOUND", "{}")),
            ("no slot", header(&bound, TOKEN, "Bearer")),
            (
                "set twice",
                params(json!({
                    "method": "GET",
                    "url": bound,
                    "headers": [{ "name": "Authorization", "value": "Basic none" }],
                    "secrets": [{ "in": "header", "name": "authorization", "reference": TOKEN }],
                })),
            ),
        ];
        let fetch = fixture.plugin_fetch();
        let mut refused: Vec<(&str, String)> = cases
            .into_iter()
            .map(|(case, params)| {
                let code = echo.send(&fetch, params).map_or_else(
                    |failure| failure.code().to_owned(),
                    |result| format!("answered {}", result.status),
                );
                (case, code)
            })
            .collect();
        refused.push((
            "outside a Run",
            echo.send(&fixture.outside_a_run(), header(&bound, TOKEN, "Bearer {}"))
                .map_or_else(
                    |failure| failure.code().to_owned(),
                    |_| "answered".to_owned(),
                ),
        ));

        assert_eq!(
            (refused, echo.received().len(), fixture.uses()),
            (
                vec![
                    ("elsewhere", "secret_domain_not_allowed".to_owned()),
                    ("undeclared", "plugin_call_back_refused".to_owned()),
                    ("unapproved", "plugin_permission_denied".to_owned()),
                    ("unbound", "secret_unbound".to_owned()),
                    ("no slot", "invalid_params".to_owned()),
                    ("set twice", "invalid_params".to_owned()),
                    ("outside a Run", "plugin_call_back_refused".to_owned()),
                ],
                0,
                Vec::<Value>::new(),
            )
        );
    });
}
