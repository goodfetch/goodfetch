//! A Plugin's own requests, its `net.fetch` (ADRs 0017 and 0033; ticket
//! #831), against the Proving ground, whose connection record and request
//! log are the oracle: a destination outside the Plugin's own endpoints is
//! checked as a page's, never under the run's `egress.private_ranges` or a
//! switched-off `security.resolved_ip_check`, so every case of the SSRF
//! table a Plugin's request meets is refused before it connects; and a
//! redirect reaches the Plugin unfollowed, its location judged as a new
//! `net.fetch`.

use std::net::SocketAddr;

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::plugin_gate::PluginFetch;
use goodfetch_engine::settings::{Layers, Resolved, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::plugin::net::{NetFetchParams, NetFetchResult};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{
    Answer, EgressRule, Family, Outcome, ProvingGround, ReceivedRequests, Redirect, Refusal,
    Runner, SsrfCase, SsrfTable, TestEnv,
};
use serde_json::{Map, Value, json};

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
