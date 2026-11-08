//! Operator-configured endpoints as their own egress class, on their first
//! consumer of the engine's HTTP client, the OTLP collector (ADRs 0017 and
//! 0042; spec #564): an endpoint on loopback is reached unless the Operator
//! policy sets `egress.no_private_endpoints`, which refuses every address
//! that is not globally reachable before any connection; and an endpoint
//! takes only its own `tls.trusted_cas` entry, never an exception of
//! `tls.accept_invalid_certs`. The Proving ground's connection record and
//! request log are the oracle; its address is loopback, the private
//! address the endpoint is refused.

use std::net::SocketAddr;
use std::path::Path;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::otlp::{Collector, ExportFailure};
use goodfetch_engine::settings::{Layers, Policy, Resolved, Values, resolve};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::egress::{EgressRefusedDetails, EgressRule};
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_protocol::tls::{CertificateFailure, KnownCertificateFailure};
use goodfetch_test_support::{
    Answer, Family, Outcome, ProvingGround, ReceivedRequests, Refusal, Route, Runner, SsrfCase,
    SsrfTable, TestEnv,
};
use hyper::Request;
use hyper::body::Bytes;
use rstest::rstest;
use serde_json::{Map, Value, json};

const SEED: u64 = 627;

/// How many connections reached the watched target.
fn target_connections(observations: &Value) -> usize {
    observations["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == "target")
        .count()
}

/// `url` with `port` written after its host, keeping the host's spelling,
/// so each case reaches the watched target, which serves any path.
fn at_port(url: &str, port: u16) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    format!("{scheme}://{}:{port}{}", &rest[..end], &rest[end..])
}

/// The table's form of the rule a refusal names.
fn table_rule(rule: EgressRule) -> goodfetch_test_support::EgressRule {
    match rule {
        EgressRule::DomainPolicy => goodfetch_test_support::EgressRule::DomainPolicy,
        EgressRule::PrivateAddress => goodfetch_test_support::EgressRule::PrivateAddress,
        EgressRule::Scheme => goodfetch_test_support::EgressRule::Scheme,
    }
}

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

fn table(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => panic!("settings are a table"),
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

/// The Operator policy of Goodfetch Cloud, which refuses private endpoints,
/// or one that does not.
fn no_private_endpoints(set: bool) -> Value {
    if set {
        json!({ "egress": { "no_private_endpoints": true } })
    } else {
        json!({})
    }
}

/// Exports an empty batch to the collector at `url`: the collector's
/// status, or why it gave none.
fn export(collector: &Collector, url: &str) -> Result<u16, ExportFailure> {
    let request = Request::post(url)
        .header("content-type", "application/x-protobuf")
        .body(Bytes::new())
        .unwrap();
    goodfetch_test_support::runtime().block_on(async {
        collector
            .send(request)
            .await
            .map(|answer| answer.status().as_u16())
    })
}

/// Story 4-16: a collector on loopback is reached by default, as a model
/// on localhost is; under an Operator policy that sets
/// `egress.no_private_endpoints` it is refused with `egress_refused`
/// naming the address and the setting, and no connection reaches it.
/// Covers: story:4-16, error:egress_refused, setting:egress.no_private_endpoints
#[rstest]
#[case::by_default(false)]
#[case::under_no_private_endpoints(true)]
fn a_collector_on_loopback_is_reached_unless_the_operator_policy_refuses_private_endpoints(
    #[case] refused: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let target = target(&proving_ground);
    let url = format!("http://{target}/v1/traces");
    let collector = Collector::of(
        &machine_settings(no_private_endpoints(refused), json!({})),
        Clock::system(),
    );

    let exported = export(&collector, &url);

    let connections = target_connections(&proving_ground.observations().unwrap());
    if refused {
        let Err(ExportFailure::Refused(refusal)) = exported else {
            panic!("the collector is refused: {exported:?}");
        };
        assert_eq!(
            (refusal.details(), connections),
            (
                EgressRefusedDetails {
                    url: url.clone(),
                    rule: EgressRule::PrivateAddress,
                    address: Some(target.ip().to_string()),
                    setting: Some("egress.no_private_endpoints".to_owned()),
                },
                0
            )
        );
    } else {
        assert_eq!((exported.unwrap(), connections), (200, 1));
    }
}

/// An endpoint's reply is a body the engine itself reads, so
/// `network.max_body_size` caps it as the endpoint's own limit does (ticket
/// #644): a collector whose answer passes a one-byte limit fails as too
/// long, and the same collector under the default limit answers.
/// Covers: setting:network.max_body_size
#[test]
fn an_endpoints_reply_past_network_max_body_size_is_too_long() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = format!("http://{}/v1/traces", target(&proving_ground));
    let limited = Collector::of(
        &machine_settings(json!({}), json!({ "network": { "max_body_size": 1 } })),
        Clock::system(),
    );
    let unlimited = Collector::of(&machine_settings(json!({}), json!({})), Clock::system());

    let cut = export(&limited, &url);
    let whole = export(&unlimited, &url);

    assert!(matches!(cut, Err(ExportFailure::TooLong)), "{cut:?}");
    assert_eq!(whole.unwrap(), 200);
}

/// The SSRF table's thin runner on the engine's HTTP client for
/// Operator-configured endpoints: each case's collector under the Operator
/// policy of `egress.no_private_endpoints`, its lookups as the
/// `network.resolve` mapping, at the watched target's port; a case's
/// connections are those the target accepted. With `refusing` false, the
/// Operator policy leaves the switch off: the positive control.
fn run_case(proving_ground: &ProvingGround, case: &SsrfCase, refusing: bool) -> Outcome {
    let target = target(proving_ground);
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
    let collector = Collector::of(
        &machine_settings(
            no_private_endpoints(refusing),
            json!({ "network": { "resolve": resolve } }),
        ),
        Clock::system(),
    );
    let url = at_port(&case.url, target.port());
    // A URL no URI holds is no endpoint a collector can be given: nothing
    // is sent.
    if url.parse::<hyper::Uri>().is_err() {
        return Outcome::default();
    }
    let before = target_connections(&proving_ground.observations().unwrap());
    let exported = export(&collector, &url);
    let after = target_connections(&proving_ground.observations().unwrap());
    Outcome {
        refusal: match exported {
            Err(ExportFailure::Refused(refused)) => Some(Refusal::EgressRefused {
                rule: table_rule(refused.rule),
            }),
            _ => None,
        },
        connections: vec![target.ip(); after - before],
    }
}

/// The table's Operator endpoint cases and its address cases, each URL
/// the collector's: refused under `egress.no_private_endpoints` before any
/// connection, while the positive control, the loopback collector with the
/// switch off, reaches the target.
///
/// Covers: story:4-16, story:4-89, error:egress_refused, setting:egress.no_private_endpoints
#[test]
fn every_operator_endpoint_case_of_the_ssrf_table_is_refused_before_it_connects() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let runner = Runner::committed(
        "operator_endpoints",
        file!(),
        "every_operator_endpoint_case_of_the_ssrf_table_is_refused_before_it_connects",
    )
    .unwrap();
    let kinds: Vec<&Route> = runner
        .cases()
        .iter()
        .filter(|case| case.family == Family::OperatorEndpoint)
        .map(|case| &case.route)
        .collect();
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "loopback_collector_with_the_switch_off"
        family = "operator_endpoint"
        url = "http://127.0.0.1/v1/traces"
        route = { via = "operator_endpoint", kind = "otlp" }
        target = "127.0.0.1"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();

    let result = runner.run(&controls.cases, target(&proving_ground).ip(), |case| {
        let control = case.name == "loopback_collector_with_the_switch_off";
        run_case(&proving_ground, case, !control)
    });

    assert!(kinds.len() >= 7, "only {} cases", kinds.len());
    assert!(
        runner.cases().len() >= 70,
        "only {} cases",
        runner.cases().len()
    );
    // A proxy's row is an Operator-configured endpoint too, sent here as
    // the collector's URL: the `session_proxy` layer runs it as a proxy's.
    assert!(
        kinds.iter().all(|route| matches!(
            route,
            Route::OperatorEndpoint { kind } if kind == "otlp" || kind == "proxy"
        )),
        "{kinds:?}"
    );
    // An endpoint is the Operator's text, sent as an HTTP URI: the
    // spellings only the WHATWG parser reads are no URI, so nothing is
    // sent, and a zone ID's is a URL the policy cannot judge, refused as
    // such. Neither reaches anything.
    let unsent = |case: &str| (case.to_owned(), None, false);
    assert_eq!(
        result.map_err(|failed| {
            let let_through: Vec<_> = failed
                .let_through
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect();
            (failed.no_control, failed.held, let_through)
        }),
        Err((
            false,
            Vec::new(),
            vec![
                unsent("percent_encoded_host"),
                unsent("full_width_digits"),
                unsent("ideographic_full_stops"),
                unsent("tab_inside_the_host"),
                unsent("backslashes"),
                (
                    "zone_id".to_owned(),
                    Some(Refusal::EgressRefused {
                        rule: goodfetch_test_support::EgressRule::DomainPolicy
                    }),
                    false
                ),
            ]
        ))
    );
}

/// The hosts of the page requests the Proving ground received.
fn requested_hosts(observations: &Value) -> Vec<String> {
    ReceivedRequests::of(observations)
        .every()
        .map(|request| request.host.as_str().to_owned())
        .collect()
}

/// The user configuration of a collector at `catalog.test`, a host whose
/// certificate the Proving ground's own authority signed, with `tls`.
fn at_catalog(proving_ground: &ProvingGround, tls: &Value) -> Value {
    let description = proving_ground.description();
    let resolve: Map<String, Value> = description
        .resolve
        .iter()
        .map(|(host, address)| (host.clone(), json!([address])))
        .collect();
    json!({ "network": { "resolve": resolve }, "tls": tls })
}

/// The Proving ground's certificate authority, as `tls.trusted_cas` names
/// a file.
fn authority(proving_ground: &ProvingGround) -> String {
    Path::new(&proving_ground.description().ca)
        .display()
        .to_string()
}

/// Story 4-41 (ADR 0042): a domain `tls.accept_invalid_certs` lists, for
/// trust on first use or with `any`, makes no exception for the
/// Operator-configured endpoint at that domain, whose unknown issuer is
/// refused with `certificate_invalid` before anything is sent; the
/// endpoint's own `tls.trusted_cas` entry trusts its authority (positive
/// control).
/// Covers: story:4-41, error:certificate_invalid, setting:tls.accept_invalid_certs, setting:tls.trusted_cas
#[rstest]
#[case::listed_for_trust_on_first_use(false, json!({ "accept_invalid_certs": { "catalog.test": null } }))]
#[case::listed_with_any(false, json!({ "accept_invalid_certs": { "catalog.test": "any" } }))]
#[case::its_own_trusted_authority(true, json!(null))]
fn an_endpoint_takes_its_own_trusted_authority_and_no_certificate_exception(
    #[case] trusted: bool,
    #[case] tls: Value,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let tls = if trusted {
        json!({ "trusted_cas": { "catalog.test": [authority(&proving_ground)] } })
    } else {
        tls
    };
    let collector = Collector::of(
        &machine_settings(json!({}), at_catalog(&proving_ground, &tls)),
        Clock::system(),
    );
    let url = format!(
        "https://catalog.test:{}/v1/traces",
        proving_ground.address().port()
    );

    let exported = export(&collector, &url);

    let hosts = requested_hosts(&proving_ground.observations().unwrap());
    if trusted {
        assert_eq!(
            (exported.is_ok(), hosts),
            (true, vec!["catalog.test".to_owned()]),
            "{exported:?}"
        );
    } else {
        let Err(ExportFailure::CertificateInvalid(invalid)) = exported else {
            panic!("the endpoint's certificate is refused: {exported:?}");
        };
        assert_eq!(
            (invalid.host.as_str(), invalid.failure, hosts),
            (
                "catalog.test",
                CertificateFailure::Known(KnownCertificateFailure::UnknownIssuer),
                Vec::<String>::new()
            )
        );
    }
}
