//! The upstream proxy path of the engine's own HTTP client, against the
//! Proving ground's forward proxies (ADRs 0017, 0053, 0059 and 0127; spec
//! #564): before it sends a request of the run through the Session's
//! proxy, the engine judges the host and any literal address, resolves the
//! name itself, the `resolve` mapping first, and refuses a private answer,
//! so the proxy, which resolves the name again itself, never sees a request
//! for an internal address. The proxy's connection record is the oracle:
//! the connections its listener accepted and the relays it made.
//!
//! The Proving ground serves on loopback, so its address stands for a
//! public one, which a Session lists in its private ranges, as every test
//! that reaches loopback does with the check on, except where a case aims
//! at that address itself.

use std::net::SocketAddr;

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::http_client::{
    Failure, FetchError, PageTraffic, Resolve, UpstreamProxy, Waiter,
};
use goodfetch_engine::tls::{CertificateExceptions, Trust};
use goodfetch_protocol::egress::{EgressRefusedDetails, EgressRule};
use goodfetch_test_support::{
    Answer, Family, Outcome, ProvingGround, Refusal, SsrfCase, SsrfTable, TestEnv, run_cases,
};
use rstest::rstest;
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use serde_json::Value;

use crate::egress::{at_port, table_rule, target_connections};

const SEED: u64 = 623;

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

/// The Proving ground's forward proxy `name`.
fn proxy(proving_ground: &ProvingGround, name: &str) -> UpstreamProxy {
    let address: SocketAddr = proving_ground.description().fixtures.proxies[name]
        .parse()
        .unwrap();
    UpstreamProxy {
        host: address.ip().to_string(),
        port: address.port(),
    }
}

/// What a run may reach through `proxy`, or directly without one: every
/// host, the addresses `resolve` maps names to, the private `ranges`, and
/// the Proving ground's certificate authority for `scope.test`.
fn traffic(
    proving_ground: &ProvingGround,
    resolve: Resolve,
    ranges: &[String],
    proxy: Option<UpstreamProxy>,
) -> PageTraffic {
    PageTraffic {
        egress: Policy::new(&["*"], &[] as &[&str])
            .unwrap()
            .with_private_ranges(ranges)
            .unwrap(),
        resolve,
        proxy,
        trust: Trust::default_roots()
            .with_trusted_cas(
                "scope.test",
                vec![CertificateDer::from_pem_file(&proving_ground.description().ca).unwrap()],
            )
            .unwrap(),
        exceptions: CertificateExceptions::default(),
        cookies: Vec::new(),
        clock: Clock::system(),
        delays: Delays::BUILT_IN,
    }
}

/// How many connections the proxy `name`'s listener accepted.
fn proxy_connections(observations: &Value, name: &str) -> usize {
    observations["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == name)
        .count()
}

/// The targets the proxies' relays were for.
fn relay_targets(observations: &Value) -> Vec<String> {
    observations["relays"]
        .as_array()
        .unwrap()
        .iter()
        .map(|relay| relay["target"].as_str().unwrap().to_owned())
        .collect()
}

/// Fetches `url` under `traffic`: its status or why it got none.
fn fetch(traffic: &PageTraffic, url: &str) -> Result<u16, FetchError> {
    goodfetch_test_support::runtime().block_on(async {
        traffic
            .get(url, Waiter::Call)
            .await
            .map(|response| response.status())
    })
}

/// Covers: story:4-12, error:egress_refused
#[test]
fn through_the_proxy_a_name_that_resolves_privately_is_refused_before_the_proxy_sees_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let public = target(&proving_ground);
    let traffic = traffic(
        &proving_ground,
        Resolve::new()
            .with("internal.test", vec!["10.0.0.1".parse().unwrap()])
            .with("scope.test", vec![public.ip()]),
        &[public.ip().to_string()],
        Some(proxy(&proving_ground, "open")),
    );
    let refused_url = format!("http://internal.test:{}/", public.port());
    let control_url = format!("http://scope.test:{}/reached", public.port());

    let refused = fetch(&traffic, &refused_url);
    let after_refusal = proving_ground.observations().unwrap();
    let control = fetch(&traffic, &control_url);

    let Err(FetchError::Refused(refused)) = refused else {
        panic!("the private name is refused: {refused:?}");
    };
    assert_eq!(
        refused.details(),
        EgressRefusedDetails {
            url: refused_url,
            rule: EgressRule::PrivateAddress,
            address: Some("10.0.0.1".to_owned()),
            setting: Some("egress.private_ranges".to_owned()),
        }
    );
    assert_eq!(proxy_connections(&after_refusal, "open"), 0);
    assert_eq!(relay_targets(&after_refusal), Vec::<String>::new());
    // The positive control: a name whose answer is public goes through the
    // proxy, which relays it to the target.
    assert_eq!(control.unwrap(), 200);
    let observations = proving_ground.observations().unwrap();
    assert_eq!(proxy_connections(&observations, "open"), 1);
    assert_eq!(relay_targets(&observations), [control_url]);
    assert_eq!(target_connections(&observations), 1);
}

/// The SSRF table's thin runner on the engine's HTTP client through the
/// Proving ground's open proxy: the case's lookups as the `resolve`
/// mapping, `public` being the watched target's address, which the Session
/// lists when `public_listed`; a case's connections are those the target
/// accepted.
fn run_case(proving_ground: &ProvingGround, case: &SsrfCase, public_listed: bool) -> Outcome {
    let public = target(proving_ground).ip();
    let resolve = case.lookups.iter().fold(Resolve::new(), |resolve, lookup| {
        let addresses = lookup
            .answers
            .iter()
            .map(|answer| match answer {
                Answer::Public => public,
                Answer::Address(address) => *address,
            })
            .collect();
        resolve.with(lookup.name.clone(), addresses)
    });
    let ranges = if public_listed {
        vec![public.to_string()]
    } else {
        Vec::new()
    };
    let traffic = traffic(
        proving_ground,
        resolve,
        &ranges,
        Some(proxy(proving_ground, "open")),
    );
    let before = target_connections(&proving_ground.observations().unwrap());
    let fetched = fetch(&traffic, &at_port(&case.url, target(proving_ground).port()));
    let after = target_connections(&proving_ground.observations().unwrap());
    Outcome {
        refusal: match fetched {
            Err(FetchError::Refused(refused)) => Some(Refusal::EgressRefused {
                rule: table_rule(refused.rule),
            }),
            _ => None,
        },
        connections: vec![public; after - before],
    }
}

/// Covers: story:4-12, error:egress_refused
#[test]
fn every_upstream_proxy_case_of_the_ssrf_table_is_refused_before_the_proxy_sees_it() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let public = target(&proving_ground).ip();
    let table = SsrfTable::committed().unwrap();
    let cases: Vec<&SsrfCase> = table.cases_of(Family::UpstreamProxy).collect();
    // The positive control: a name whose only answer is public reaches the
    // target through the proxy, which the record shows.
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "public_name_through_the_proxy"
        family = "upstream_proxy"
        url = "http://scope.test/"
        route = { via = "upstream_proxy" }
        lookups = [{ name = "scope.test", answers = ["public"] }]
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();

    let controlled = run_cases(&controls.cases, public, |case| {
        run_case(&proving_ground, case, true)
    });
    let before = proxy_connections(&proving_ground.observations().unwrap(), "open");
    let result = run_cases(cases.iter().copied(), public, |case| {
        let aims_at_public = case
            .target_address(public)
            .is_some_and(|target| target.to_canonical() == public.to_canonical());
        run_case(&proving_ground, case, !aims_at_public)
    });
    let after = proxy_connections(&proving_ground.observations().unwrap(), "open");

    assert_eq!(
        controlled.map_err(|let_through| {
            let_through
                .misses
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect::<Vec<_>>()
        }),
        Err(vec![(
            "public_name_through_the_proxy".to_owned(),
            None,
            true
        )])
    );
    assert!(cases.len() >= 6, "only {} cases", cases.len());
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
    assert_eq!(after - before, 0);
}

/// Covers: story:4-13, error:egress_refused, setting:network.resolve
#[rstest]
#[case::direct_to_an_unlisted_address(false, false, Err(EgressRule::PrivateAddress))]
#[case::direct_to_a_listed_address(false, true, Ok(200))]
#[case::through_the_proxy_to_an_unlisted_address(true, false, Err(EgressRule::PrivateAddress))]
#[case::through_the_proxy_to_a_listed_address(true, true, Ok(200))]
fn a_resolve_mapping_to_a_private_address_is_refused_unless_a_private_range_lists_it(
    #[case] through_the_proxy: bool,
    #[case] listed: bool,
    #[case] expected: Result<u16, EgressRule>,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let mapped = target(&proving_ground);
    let ranges: Vec<String> = if listed {
        vec![mapped.ip().to_string()]
    } else {
        Vec::new()
    };
    let traffic = traffic(
        &proving_ground,
        Resolve::new().with("scope.test", vec![mapped.ip()]),
        &ranges,
        through_the_proxy.then(|| proxy(&proving_ground, "open")),
    );

    let fetched = fetch(&traffic, &format!("http://scope.test:{}/", mapped.port()));

    let observations = proving_ground.observations().unwrap();
    let outcome = fetched.map_err(|error| match error {
        FetchError::Refused(refused) => refused.rule,
        other => panic!("the fetch is refused or answered: {other:?}"),
    });
    assert_eq!(outcome, expected);
    assert_eq!(
        target_connections(&observations),
        usize::from(expected.is_ok())
    );
    assert_eq!(
        proxy_connections(&observations, "open"),
        usize::from(through_the_proxy && expected.is_ok())
    );
}

/// What the proxy's answer to the tunnel request is, as the call that
/// waited for it fails: its Error code.
fn tunnel_failure(error: FetchError, proxy: &UpstreamProxy, url: &str) -> String {
    match error {
        FetchError::Failed(Failure::Proxy(failure)) => failure.error(proxy, url).code().to_owned(),
        other => panic!("the proxy's own failure: {other:?}"),
    }
}

/// Covers: story:4-12, error:proxy_unavailable, error:proxy_auth_failed
#[rstest]
#[case::an_open_proxy_tunnels_to_the_site("open", Ok(200))]
#[case::a_proxy_that_refuses_every_tunnel("refusing_tunnels", Err("proxy_unavailable"))]
#[case::a_proxy_that_wants_credentials("datacenter", Err("proxy_auth_failed"))]
#[case::a_proxy_nothing_answers_on("down", Err("proxy_unavailable"))]
fn a_refused_tunnel_request_is_a_failure_of_the_proxy_itself(
    #[case] name: &str,
    #[case] expected: Result<u16, &str>,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let site = proving_ground.address();
    let proxy = proxy(&proving_ground, name);
    let traffic = traffic(
        &proving_ground,
        Resolve::new().with("scope.test", vec![site.ip()]),
        &[site.ip().to_string()],
        Some(proxy.clone()),
    );
    let url = format!("https://scope.test:{}/", site.port());

    let fetched = fetch(&traffic, &url);

    assert_eq!(
        fetched.map_err(|error| tunnel_failure(error, &proxy, &url)),
        expected.map_err(str::to_owned)
    );
}
