//! A Plugin's own requests, its `net.fetch` (ADRs 0017 and 0033; ticket
//! #831), against the Proving ground, whose connection record and request
//! log are the oracle: a destination outside the Plugin's own endpoints is
//! checked as a page's, never under the run's `egress.private_ranges` or a
//! switched-off `security.resolved_ip_check`, so every case of the SSRF
//! table a Plugin's request meets is refused before it connects; and a
//! redirect reaches the Plugin unfollowed, its location judged as a new
//! `net.fetch`.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::plugin_gate::PluginFetch;
use goodfetch_engine::plugin_gate::proxy::{
    PluginProxy, RefusalObserver, StreamAccountant, StreamRecord, StreamTransport,
};
use goodfetch_engine::plugin_host::instance::{CallOptions, InMemoryPlugin, PluginInstance, Scope};
use goodfetch_engine::settings::{Layers, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::PluginManifest;
use goodfetch_protocol::plugin::net::{NetFetchParams, NetFetchResult};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_PLUGIN};
use goodfetch_test_support::{
    Answer, EgressRule, Family, Outcome, ProvingGround, ReceivedRequests, Redirect, Refusal,
    Runner, SsrfCase, SsrfTable, TestEnv,
};
use serde_json::{Map, Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use url::Url;

const SEED: u64 = 831;

/// The Plugin whose `net.fetch` each test sends.
const PLUGIN: &str = "kettle";

/// The Proving ground's watched target, which answers any plain HTTP
/// request and whose every connection the record lists.
fn target(proving_ground: &ProvingGround) -> SocketAddr {
    proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap()
}

/// How many connections reached the watched target.
fn target_connections(proving_ground: &ProvingGround) -> usize {
    proving_ground.observations().unwrap()["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == "target")
        .count()
}

/// `url` with `port` written after its host, keeping the host's spelling.
fn at_port(url: &str, port: u16) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    format!("{scheme}://{}:{port}{}", &rest[..end], &rest[end..])
}

fn table(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("settings are a table"),
    }
}

/// The settings of a Run whose user configuration is `configuration` and
/// whose own settings weaken its pages' egress as far as a run can: every
/// address in `egress.private_ranges` and `security.resolved_ip_check`
/// off, neither of which reaches a Plugin.
fn run_settings(configuration: Value) -> Resolved {
    let weakened = json!({
        "egress": { "private_ranges": ["0.0.0.0/0", "::/0"] },
        "security": { "resolved_ip_check": false },
    });
    let layers = Layers {
        user_configuration: Values::read(
            SettingsLayer::UserConfiguration,
            &table(configuration),
            Product::Collie,
        )
        .unwrap(),
        run: Values::read(SettingsLayer::Run, &table(weakened), Product::Collie).unwrap(),
        ..Layers::default()
    };
    resolve(&layers, Product::Collie, Product::Collie).unwrap()
}

/// The Plugin's `net.fetch` under `settings`.
fn plugin_fetch(settings: &Resolved) -> PluginFetch {
    PluginFetch::of(
        settings,
        PLUGIN,
        Clock::system(),
        TaskOwner::root(BlockingPolicy::Inline).blocking_threads(),
    )
    .unwrap()
}

/// The params of a `net.fetch` of `url` with `GET`.
fn get(url: &str) -> NetFetchParams {
    serde_json::from_value(json!({ "call_id": 1, "method": "GET", "url": url })).unwrap()
}

/// The Plugin's `net.fetch` of `url` with `GET` under `settings`.
fn fetch(settings: &Resolved, url: &str) -> Result<NetFetchResult, CallError> {
    goodfetch_test_support::runtime().block_on(plugin_fetch(settings).fetch(get(url)))
}

/// The table's form of an `egress_refused`, none for another failure.
fn refusal(failure: &CallError) -> Option<Refusal> {
    (failure.code() == "egress_refused").then(|| Refusal::EgressRefused {
        rule: serde_json::from_value::<EgressRule>(failure.details().unwrap()["rule"].clone())
            .unwrap(),
    })
}

/// The URL a Plugin's `net.fetch` asks for in `case`: the case's own, or,
/// for a redirect, which `net.fetch` hands the Plugin unfollowed, the
/// location the Plugin asks for next.
fn requested(case: &SsrfCase) -> &str {
    match &case.redirect {
        None => &case.url,
        Some(
            Redirect::Status { location, .. }
            | Redirect::RefreshHeader { location }
            | Redirect::MetaRefresh { location }
            | Redirect::Script { location },
        ) => location,
    }
}

/// The SSRF table's thin runner on a Plugin's `net.fetch`: each case's URL
/// at the watched target's port, its lookups as the user configuration's
/// `network.resolve` mapping, a public answer the target's address; a
/// case's connections are those the target accepted. With `endpoint`, the
/// URL's origin is the Plugin's own endpoint in user configuration: the
/// positive control.
fn run_case(proving_ground: &ProvingGround, case: &SsrfCase, endpoint: bool) -> Outcome {
    let target = target(proving_ground);
    let url = at_port(requested(case), target.port());
    let resolve: Map<String, Value> = case
        .lookups
        .iter()
        .map(|lookup| {
            let answers: Vec<String> = lookup
                .answers
                .iter()
                .map(|answer| match answer {
                    Answer::Public => target.ip().to_string(),
                    Answer::Address(address) => address.to_string(),
                })
                .collect();
            (lookup.name.clone(), json!(answers))
        })
        .collect();
    let mut configuration = json!({ "network": { "resolve": resolve } });
    if endpoint {
        configuration["plugins"] = json!({ PLUGIN: { "settings": { "api": url } } });
    }
    let settings = run_settings(configuration);
    let before = target_connections(proving_ground);
    let fetched = fetch(&settings, &url);
    let after = target_connections(proving_ground);
    Outcome {
        refusal: fetched.err().as_ref().and_then(refusal),
        connections: vec![target.ip(); after - before],
    }
}

/// The table's Plugin traffic and its address cases, each URL a Plugin's
/// `net.fetch`: refused before any connection under a Run whose own
/// settings allow its pages every private address, while the positive
/// control, the same loopback target as the Plugin's endpoint, is reached.
///
/// Covers: story:7-57, story:7-58, error:egress_refused, setting:egress.private_ranges, setting:security.resolved_ip_check
#[test]
fn every_plugin_traffic_case_of_the_ssrf_table_is_refused_before_it_connects() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let runner = Runner::committed(
        "plugin_traffic",
        file!(),
        "every_plugin_traffic_case_of_the_ssrf_table_is_refused_before_it_connects",
    )
    .unwrap();
    assert!(
        runner
            .cases()
            .iter()
            .any(|case| case.family == Family::PluginTraffic),
        "the layer runs the Plugin traffic family"
    );
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "the_plugins_endpoint_on_loopback"
        family = "plugin_traffic"
        url = "http://127.0.0.1/"
        route = { via = "plugin" }
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();

    let result = runner.run(&controls.cases, target(&proving_ground).ip(), |case| {
        let control = case.name == "the_plugins_endpoint_on_loopback";
        run_case(&proving_ground, case, control)
    });

    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}

/// A `net.fetch` to the Plugin's own endpoint that gets no response, a
/// loopback listener that closes the connection it accepts, fails with
/// `plugin_fetch_failed`, which the Plugin may retry.
///
/// Covers: story:7-57, error:plugin_fetch_failed
#[test]
fn a_request_that_gets_no_response_fails_with_plugin_fetch_failed() {
    let runtime = goodfetch_test_support::runtime();
    let listener = runtime
        .block_on(tokio::net::TcpListener::bind("127.0.0.1:0"))
        .unwrap();
    let url = format!("http://{}/", listener.local_addr().unwrap());
    let settings = run_settings(json!({ "plugins": { PLUGIN: { "settings": { "api": url } } } }));
    let fetch = plugin_fetch(&settings);
    let closing = async {
        let (connection, _) = listener.accept().await.unwrap();
        drop(connection);
    };

    let ((), fetched) = runtime.block_on(async { tokio::join!(closing, fetch.fetch(get(&url))) });
    let failure = fetched.unwrap_err();

    assert_eq!(
        (failure.code(), failure.retryable()),
        ("plugin_fetch_failed", true),
        "{failure:?}"
    );
}

/// A redirect reaches the Plugin unfollowed, and its location, which the
/// Plugin asks for as a new `net.fetch`, is judged as any other: the
/// Proving ground's page that redirects to its own loopback address, the
/// Plugin's endpoint, answers `302`, and the landing it names is refused
/// with `egress_refused` before any request reaches it.
///
/// Covers: story:7-57, story:7-58, error:egress_refused
#[test]
fn a_redirect_reaches_the_plugin_unfollowed_and_its_location_is_judged_anew() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let description = proving_ground.description();
    let page = format!(
        "http://16.redirects.test:{}/to_own_loopback",
        proving_ground.address().port()
    );
    let resolve: Map<String, Value> = description
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    let settings = run_settings(json!({
        "network": { "resolve": resolve },
        "plugins": { PLUGIN: { "settings": { "site": page } } },
    }));

    let redirected = fetch(&settings, &page).unwrap();
    let location = redirected
        .headers
        .iter()
        .find(|header| header.name == "location")
        .map(|header| header.value.clone())
        .unwrap();
    let landing = fetch(&settings, &location);

    let requested: Vec<String> = ReceivedRequests::of(&proving_ground.observations().unwrap())
        .every()
        .map(|request| request.target.clone())
        .collect();
    assert_eq!(
        (
            redirected.status,
            landing.as_ref().err().and_then(refusal),
            requested
        ),
        (
            302,
            Some(Refusal::EgressRefused {
                rule: EgressRule::PrivateAddress
            }),
            vec!["/to_own_loopback".to_owned()]
        ),
        "{landing:?}"
    );
}

/// Captures the last [`CallError`] observed by a [`PluginProxy`] so the SSRF
/// runner can verify the exact `egress_refused` rule through `proxy_tcp`.
#[derive(Debug, Default)]
struct LastRefusal(Mutex<Option<Refusal>>);

impl RefusalObserver for LastRefusal {
    fn observed(&self, error: &CallError) {
        if let Ok(mut guard) = self.0.lock() {
            *guard = refusal(error);
        }
    }
}

/// Extracts `(host, port)` from a case URL at the watched target's port.
fn host_and_port(case: &SsrfCase, target: SocketAddr) -> Option<(String, u16)> {
    let url = Url::parse(&at_port(requested(case), target.port())).ok()?;
    let host = url.host_str()?.to_owned();
    let port = url.port_or_known_default().unwrap_or(target.port());
    Some((host, port))
}

fn case_settings(proving_ground: &ProvingGround, case: &SsrfCase, endpoint: bool) -> Resolved {
    let target = target(proving_ground);
    let url = at_port(requested(case), target.port());
    let resolve: Map<String, Value> = case
        .lookups
        .iter()
        .map(|lookup| {
            let answers: Vec<String> = lookup
                .answers
                .iter()
                .map(|answer| match answer {
                    Answer::Public => target.ip().to_string(),
                    Answer::Address(address) => address.to_string(),
                })
                .collect();
            (lookup.name.clone(), json!(answers))
        })
        .collect();
    let mut configuration = json!({ "network": { "resolve": resolve } });
    if endpoint {
        configuration["plugins"] = json!({ PLUGIN: { "settings": { "api": url } } });
    }
    run_settings(configuration)
}

fn plugin_traffic_controls() -> SsrfTable {
    SsrfTable::parse(
        r#"
        [[cases]]
        name = "the_plugins_endpoint_on_loopback"
        family = "plugin_traffic"
        url = "http://127.0.0.1/"
        route = { via = "plugin" }
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap()
}

/// The SSRF table's Plugin traffic family through `net.connect`: every
/// disallowed destination fails with its exact `egress_refused` rule before
/// opening a TCP connection, while the positive control (the Plugin's own
/// configured endpoint) connects to the watched target.
///
/// Covers: story:7-55, story:7-61, error:egress_refused
#[test]
fn every_plugin_traffic_case_of_the_ssrf_table_is_refused_by_net_connect_before_it_connects() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let runner = Runner::committed(
        "plugin_traffic",
        file!(),
        "every_plugin_traffic_case_of_the_ssrf_table_is_refused_by_net_connect_before_it_connects",
    )
    .unwrap();
    let controls = plugin_traffic_controls();
    let watched = target(&proving_ground);

    let result = runner.run(&controls.cases, watched.ip(), |case| {
        let control = case.name == "the_plugins_endpoint_on_loopback";
        let settings = case_settings(&proving_ground, case, control);
        let before = target_connections(&proving_ground);
        let outcome = match host_and_port(case, watched) {
            Some((host, port)) => goodfetch_test_support::runtime()
                .block_on(plugin_fetch(&settings).connect(&host, port, None))
                .err()
                .as_ref()
                .and_then(refusal),
            None => Some(Refusal::EgressRefused {
                rule: EgressRule::Scheme,
            }),
        };
        let after = target_connections(&proving_ground);
        Outcome {
            refusal: outcome,
            connections: vec![watched.ip(); after - before],
        }
    });

    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}

/// The SSRF table's Plugin traffic family through `proxy_tcp`: every
/// disallowed destination fails with its exact `egress_refused` rule through
/// the per-instance proxy before opening an upstream TCP connection, while the
/// positive control (the Plugin's own configured endpoint) connects.
///
/// Covers: story:7-55, story:7-61, error:egress_refused
#[test]
fn every_plugin_traffic_case_of_the_ssrf_table_is_refused_by_proxy_tcp_before_it_connects() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let runner = Runner::committed(
        "plugin_traffic",
        file!(),
        "every_plugin_traffic_case_of_the_ssrf_table_is_refused_by_proxy_tcp_before_it_connects",
    )
    .unwrap();
    let controls = plugin_traffic_controls();
    let watched = target(&proving_ground);

    let result = runner.run(&controls.cases, watched.ip(), |case| {
        let control = case.name == "the_plugins_endpoint_on_loopback";
        let settings = case_settings(&proving_ground, case, control);
        let before = target_connections(&proving_ground);
        let observed = Arc::new(LastRefusal::default());
        goodfetch_test_support::runtime().block_on(async {
            let owner = TaskOwner::root(BlockingPolicy::Inline);
            let proxy = PluginProxy::start_loopback(
                &owner,
                PLUGIN,
                Arc::new(plugin_fetch(&settings)),
                None,
                Some(Arc::clone(&observed) as Arc<dyn RefusalObserver>),
            )
            .await
            .unwrap();
            if let Some((host, port)) = host_and_port(case, watched) {
                let auth = data_encoding::BASE64.encode(
                    format!(
                        "{}:{}",
                        proxy.credentials().username,
                        proxy.credentials().password
                    )
                    .as_bytes(),
                );
                let authority = if host.contains(':') {
                    format!("[{host}]:{port}")
                } else {
                    format!("{host}:{port}")
                };
                let mut client = tokio::net::TcpStream::connect(proxy.local_addr().unwrap())
                    .await
                    .unwrap();
                let req = format!(
                    "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
                );
                client.write_all(req.as_bytes()).await.unwrap();
                let mut resp = [0_u8; 128];
                let _ = client.read(&mut resp).await;
            }
            proxy.stop();
            owner.stop().await;
        });
        let after = target_connections(&proving_ground);
        Outcome {
            refusal: observed.0.lock().unwrap().clone(),
            connections: vec![watched.ip(); after - before],
        }
    });

    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}

/// A per-instance `proxy_tcp` proxy serves both HTTP `CONNECT` and SOCKS5 for
/// an allowed destination, refuses an unallowed destination and wrong/missing
/// credentials, and generates distinct credentials for every instance.
///
#[derive(Debug)]
struct CaptureStreams(Arc<Mutex<Vec<StreamRecord>>>);

impl StreamAccountant for CaptureStreams {
    fn record(&self, record: StreamRecord) -> futures_util::future::BoxFuture<'static, ()> {
        let sink = Arc::clone(&self.0);
        Box::pin(async move {
            sink.lock().unwrap().push(record);
        })
    }
}

async fn assert_http_proxy_flows(
    proxy_addr: std::net::SocketAddr,
    allowed_addr: std::net::SocketAddr,
    forbidden_addr: std::net::SocketAddr,
    auth: &str,
) {
    // 1. Missing credentials -> 407 Proxy Authentication Required.
    {
        let mut client = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
        client
            .write_all(
                format!("CONNECT {allowed_addr} HTTP/1.1\r\nHost: {allowed_addr}\r\n\r\n")
                    .as_bytes(),
            )
            .await
            .unwrap();
        let mut buf = [0_u8; 128];
        let n = client.read(&mut buf).await.unwrap();
        assert!(
            String::from_utf8_lossy(&buf[..n]).starts_with("HTTP/1.1 407"),
            "expected 407 without credentials"
        );
    }

    // 2. HTTP CONNECT to allowed endpoint succeeds and echoes payload.
    {
        let mut client = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "CONNECT {allowed_addr} HTTP/1.1\r\nHost: {allowed_addr}\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut head = [0_u8; 64];
        let n = client.read(&mut head).await.unwrap();
        assert!(
            String::from_utf8_lossy(&head[..n]).starts_with("HTTP/1.1 200"),
            "expected 200 for allowed endpoint"
        );
        client.write_all(b"ping-http").await.unwrap();
        let mut echo = [0_u8; 9];
        client.read_exact(&mut echo).await.unwrap();
        assert_eq!(&echo, b"ping-http");
    }

    // 3. HTTP CONNECT to unallowed destination -> 403 Forbidden.
    {
        let mut client = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
        let req = format!(
            "CONNECT {forbidden_addr} HTTP/1.1\r\nHost: {forbidden_addr}\r\nProxy-Authorization: Basic {auth}\r\n\r\n"
        );
        client.write_all(req.as_bytes()).await.unwrap();
        let mut head = [0_u8; 64];
        let n = client.read(&mut head).await.unwrap();
        assert!(
            String::from_utf8_lossy(&head[..n]).starts_with("HTTP/1.1 403"),
            "expected 403 for unallowed destination"
        );
    }
}

async fn assert_socks5_proxy_flows(
    proxy_addr: std::net::SocketAddr,
    allowed_addr: std::net::SocketAddr,
    username: &str,
    password: &str,
) {
    // 4. SOCKS5 with valid credentials to allowed endpoint succeeds.
    {
        let mut client = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
        let mut method_resp = [0_u8; 2];
        client.read_exact(&mut method_resp).await.unwrap();
        assert_eq!(method_resp, [0x05, 0x02]);

        let u = username.as_bytes();
        let p = password.as_bytes();
        let mut auth_msg = vec![0x01, u8::try_from(u.len()).unwrap()];
        auth_msg.extend_from_slice(u);
        auth_msg.push(u8::try_from(p.len()).unwrap());
        auth_msg.extend_from_slice(p);
        client.write_all(&auth_msg).await.unwrap();
        let mut auth_resp = [0_u8; 2];
        client.read_exact(&mut auth_resp).await.unwrap();
        assert_eq!(auth_resp, [0x01, 0x00]);

        let port_be = allowed_addr.port().to_be_bytes();
        client
            .write_all(&[0x05, 0x01, 0x00, 0x01, 127, 0, 0, 1, port_be[0], port_be[1]])
            .await
            .unwrap();
        let mut conn_resp = [0_u8; 10];
        client.read_exact(&mut conn_resp).await.unwrap();
        assert_eq!(conn_resp[1], 0x00, "SOCKS5 CONNECT should succeed");

        client.write_all(b"ping-socks").await.unwrap();
        let mut echo = [0_u8; 10];
        client.read_exact(&mut echo).await.unwrap();
        assert_eq!(&echo, b"ping-socks");
    }

    // 5. SOCKS5 with wrong credentials is rejected (0x01).
    {
        let mut client = tokio::net::TcpStream::connect(proxy_addr).await.unwrap();
        client.write_all(&[0x05, 0x01, 0x02]).await.unwrap();
        let mut method_resp = [0_u8; 2];
        client.read_exact(&mut method_resp).await.unwrap();
        client
            .write_all(&[0x01, 0x03, b'b', b'a', b'd', 0x03, b'b', b'a', b'd'])
            .await
            .unwrap();
        let mut auth_resp = [0_u8; 2];
        client.read_exact(&mut auth_resp).await.unwrap();
        assert_eq!(auth_resp, [0x01, 0x01]);
    }
}

/// Covers: story:7-55, story:7-61
#[test]
fn proxy_tcp_allows_configured_endpoint_over_http_and_socks5_refuses_others_and_uses_distinct_credentials()
 {
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let allowed_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let allowed_addr = allowed_listener.local_addr().unwrap();
        let forbidden_listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let forbidden_addr = forbidden_listener.local_addr().unwrap();

        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let echo_stopping = owner.token().child_token();
        owner.spawn(async move {
            while let Some(Ok((mut sock, _))) = echo_stopping
                .run_until_cancelled(allowed_listener.accept())
                .await
            {
                let mut buf = [0_u8; 64];
                if let Ok(n) = sock.read(&mut buf).await {
                    let _ = sock.write_all(&buf[..n]).await;
                }
            }
        });

        let endpoint_url = format!("http://{allowed_addr}/");
        let settings = run_settings(json!({
            "plugins": { PLUGIN: { "settings": { "api": endpoint_url } } }
        }));
        let recorded: Arc<Mutex<Vec<StreamRecord>>> = Arc::new(Mutex::new(Vec::new()));
        let proxy1 = PluginProxy::start_loopback(
            &owner,
            PLUGIN,
            Arc::new(plugin_fetch(&settings)),
            Some(Arc::new(CaptureStreams(Arc::clone(&recorded)))),
            None,
        )
        .await
        .unwrap();
        let proxy2 = PluginProxy::start_loopback(
            &owner,
            PLUGIN,
            Arc::new(plugin_fetch(&settings)),
            None,
            None,
        )
        .await
        .unwrap();

        assert_ne!(
            proxy1.credentials(),
            proxy2.credentials(),
            "credentials must differ per instance"
        );

        let proxy_addr = proxy1.local_addr().unwrap();
        let auth = data_encoding::BASE64.encode(
            format!(
                "{}:{}",
                proxy1.credentials().username,
                proxy1.credentials().password
            )
            .as_bytes(),
        );
        assert_http_proxy_flows(proxy_addr, allowed_addr, forbidden_addr, &auth).await;
        assert_socks5_proxy_flows(
            proxy_addr,
            allowed_addr,
            &proxy1.credentials().username,
            &proxy1.credentials().password,
        )
        .await;

        proxy1.stop();
        proxy2.stop();
        owner.stop().await;

        let records = recorded.lock().unwrap();
        assert_eq!(records.len(), 2, "expected HTTP and SOCKS5 stream records");
        assert!(
            records
                .iter()
                .all(|r| r.destination == allowed_addr.to_string()
                    && r.transport == StreamTransport::ProxyTcp
                    && r.bytes >= 18),
            "unexpected records: {records:?}"
        );
    });
}

async fn exchange_net_connect_frames(
    from_engine: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
    to_engine: &mut tokio::io::DuplexStream,
    call_id: Value,
    upstream_port: u16,
) {
    let connect_req = json!({
        "jsonrpc": "2.0",
        "id": "cb-connect",
        "method": "net.connect",
        "params": {
            "call_id": call_id,
            "host": "127.0.0.1",
            "port": upstream_port,
        }
    });
    to_engine
        .write_all(format!("{connect_req}\n").as_bytes())
        .await
        .unwrap();

    let connect_reply: Value =
        serde_json::from_str(&from_engine.next_line().await.unwrap().unwrap()).unwrap();
    let stream_id = connect_reply["result"]["stream_id"]
        .as_str()
        .expect("net.connect must return stream_id")
        .to_owned();
    assert!(connect_reply["result"]["credits"].as_u64().unwrap() > 0);

    let send_frame = json!({
        "jsonrpc": "2.0",
        "method": "net.stream.data",
        "params": {
            "stream_id": stream_id,
            "data": data_encoding::BASE64.encode(b"hello"),
        }
    });
    to_engine
        .write_all(format!("{send_frame}\n").as_bytes())
        .await
        .unwrap();

    let mut saw_echo = false;
    let mut saw_credits = false;
    let mut saw_lease_close = false;
    for _ in 0..10 {
        let Some(line) = from_engine.next_line().await.unwrap() else {
            break;
        };
        let msg: Value = serde_json::from_str(&line).unwrap();
        match msg["method"].as_str() {
            Some("net.stream.data") => {
                let data = data_encoding::BASE64
                    .decode(msg["params"]["data"].as_str().unwrap().as_bytes())
                    .unwrap();
                assert_eq!(data, b"hello");
                saw_echo = true;
            }
            Some("net.stream.credits") => {
                assert_eq!(msg["params"]["bytes"].as_u64(), Some(5));
                saw_credits = true;
            }
            Some("net.stream.close") => {
                assert_eq!(msg["params"]["reason"].as_str(), Some("lease_ended"));
                saw_lease_close = true;
                break;
            }
            _ => {}
        }
    }
    assert!(saw_echo, "expected echoed net.stream.data frame");
    assert!(saw_credits, "expected returned net.stream.credits frame");
    assert!(
        saw_lease_close,
        "expected net.stream.close when lease ended"
    );
}

async fn answer_in_memory_hello(
    from_engine: &mut tokio::io::Lines<BufReader<tokio::io::DuplexStream>>,
    to_engine: &mut tokio::io::DuplexStream,
    kind: &str,
) {
    let hello_msg: Value =
        serde_json::from_str(&from_engine.next_line().await.unwrap().unwrap()).unwrap();
    let hello_reply = json!({
        "jsonrpc": "2.0",
        "id": hello_msg["id"],
        "result": {
            "protocol_version": "0.1",
            "name": PLUGIN,
            "version": "1.0.0",
            "kinds": [{ "kind": kind, "capabilities": [] }],
            "network_level": "none",
        }
    });
    to_engine
        .write_all(format!("{hello_reply}\n").as_bytes())
        .await
        .unwrap();
}

/// `net.connect` carries opaque bytes as `net.stream.*` frames under credits,
/// closes the stream when its call's lease ends, and reports bytes, time and
/// destination once through [`StreamAccountant`].
///
/// Covers: story:7-55, story:7-61
#[test]
fn net_connect_stream_carries_frames_under_credits_closes_with_lease_and_records_usage_and_trace() {
    let runtime = goodfetch_test_support::runtime();
    runtime.block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let clock = Clock::system();
        let recorded: Arc<Mutex<Vec<StreamRecord>>> = Arc::new(Mutex::new(Vec::new()));
        let accountant = Arc::new(CaptureStreams(Arc::clone(&recorded)));

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let upstream_addr = listener.local_addr().unwrap();
        let (eof_tx, eof_rx) = tokio::sync::oneshot::channel::<()>();
        owner.spawn(async move {
            if let Ok((mut sock, _)) = listener.accept().await {
                let mut buf = [0_u8; 5];
                if sock.read_exact(&mut buf).await.is_ok() {
                    let _ = sock.write_all(&buf).await;
                }
                let mut tail = [0_u8; 16];
                let _ = sock.read(&mut tail).await;
                let _ = eof_tx.send(());
            }
        });

        let endpoint_url = format!("http://{upstream_addr}/");
        let settings = run_settings(json!({
            "plugins": { PLUGIN: { "settings": { "api": endpoint_url } } }
        }));
        let fetch = Arc::new(plugin_fetch(&settings).with_streams(accountant));
        let manifest = PluginManifest::read(
            json!({
                "manifest_version": "0.1",
                "name": PLUGIN,
                "version": "1.0.0",
                "kinds": ["proxy"],
                "protocol": { "min": "0.1", "max": "0.1" },
                "entrypoint": "plugin",
                "network_level": "none",
            })
            .to_string()
            .as_bytes(),
        )
        .unwrap();

        let (instance, mut peer) = PluginInstance::start_in_memory(
            &owner,
            InMemoryPlugin {
                manifest,
                product: Product::Collie,
                fetch,
                clock,
                spend: None,
            },
            Scope::Task(None),
        );

        let mut from_engine = BufReader::new(peer.from_engine).lines();
        answer_in_memory_hello(&mut from_engine, &mut peer.to_engine, "proxy").await;

        let call_instance = instance.clone();
        let (call_done_tx, call_done_rx) = tokio::sync::oneshot::channel();
        owner.spawn(async move {
            let outcome = call_instance
                .call_under(
                    "proxy.acquire",
                    &json!({}),
                    CallOptions {
                        deadline: Some(Deadline::given(
                            &TIMEOUT_PLUGIN,
                            250,
                            SettingsLayer::Default,
                        )),
                        ..CallOptions::default()
                    },
                )
                .await;
            let _ = call_done_tx.send(outcome);
        });

        let call_msg: Value =
            serde_json::from_str(&from_engine.next_line().await.unwrap().unwrap()).unwrap();
        exchange_net_connect_frames(
            &mut from_engine,
            &mut peer.to_engine,
            call_msg["id"].clone(),
            upstream_addr.port(),
        )
        .await;

        tokio::time::timeout(Duration::from_secs(2), eof_rx)
            .await
            .expect("upstream socket must close on lease end")
            .unwrap();

        let _ = call_done_rx.await.unwrap();
        instance.stop().await;
        owner.stop().await;

        let records = recorded.lock().unwrap();
        assert_eq!(records.len(), 1, "{records:?}");
        assert_eq!(records[0].destination, upstream_addr.to_string());
        assert_eq!(records[0].bytes, 10);
        assert_eq!(records[0].transport, StreamTransport::NetConnect);
    });
}

/// Hooks and Extractors may declare only `none` and are always refused
/// `net.connect` with `plugin_call_back_refused`.
///
/// Covers: story:7-55, story:7-61, error:plugin_call_back_refused
#[test]
fn hook_and_extractor_plugins_are_refused_net_connect() {
    let runtime = goodfetch_test_support::runtime();
    for kind in ["hook", "extract"] {
        runtime.block_on(async move {
            let owner = TaskOwner::root(BlockingPolicy::Inline);
            let clock = Clock::system();
            let settings = run_settings(json!({
                "plugins": { PLUGIN: { "settings": { "api": "http://127.0.0.1:80/" } } }
            }));
            let manifest = PluginManifest::read(
                json!({
                    "manifest_version": "0.1",
                    "name": PLUGIN,
                    "version": "1.0.0",
                    "kinds": [kind],
                    "protocol": { "min": "0.1", "max": "0.1" },
                    "entrypoint": "plugin",
                    "network_level": "none",
                })
                .to_string()
                .as_bytes(),
            )
            .unwrap();
            let (instance, mut peer) = PluginInstance::start_in_memory(
                &owner,
                InMemoryPlugin {
                    manifest,
                    product: Product::Collie,
                    fetch: Arc::new(plugin_fetch(&settings)),
                    clock,
                    spend: None,
                },
                Scope::Task(None),
            );
            let mut from_engine = BufReader::new(peer.from_engine).lines();
            let hello: Value =
                serde_json::from_str(&from_engine.next_line().await.unwrap().unwrap()).unwrap();
            let hello_reply = json!({
                "jsonrpc": "2.0",
                "id": hello["id"],
                "result": {
                    "protocol_version": "0.1",
                    "name": PLUGIN,
                    "version": "1.0.0",
                    "kinds": [{ "kind": kind, "capabilities": [] }],
                    "network_level": "none",
                }
            });
            peer.to_engine
                .write_all(format!("{hello_reply}\n").as_bytes())
                .await
                .unwrap();

            let caller = instance.clone();
            let (call_done_tx, call_done_rx) = tokio::sync::oneshot::channel();
            owner.spawn(async move {
                let outcome = caller.call("test.op", &json!({})).await;
                let _ = call_done_tx.send(outcome);
            });
            let call_msg: Value =
                serde_json::from_str(&from_engine.next_line().await.unwrap().unwrap()).unwrap();
            let req = json!({
                "jsonrpc": "2.0",
                "id": "cb-1",
                "method": "net.connect",
                "params": { "call_id": call_msg["id"], "host": "127.0.0.1", "port": 80 }
            });
            peer.to_engine
                .write_all(format!("{req}\n").as_bytes())
                .await
                .unwrap();
            let reply: Value =
                serde_json::from_str(&from_engine.next_line().await.unwrap().unwrap()).unwrap();
            assert_eq!(
                reply["error"]["data"]["code"].as_str(),
                Some("plugin_call_back_refused"),
                "{kind} Plugin must be refused net.connect: {reply}"
            );
            let done = json!({ "jsonrpc": "2.0", "id": call_msg["id"], "result": {} });
            peer.to_engine
                .write_all(format!("{done}\n").as_bytes())
                .await
                .unwrap();
            let _ = call_done_rx.await.unwrap();
            instance.stop().await;
            owner.stop().await;
        });
    }
}
