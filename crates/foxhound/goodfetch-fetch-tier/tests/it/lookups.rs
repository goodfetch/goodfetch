//! The lookups of the HTTP tier's transports (ADR 0118), reqwest's and
//! `wreq`'s, against the Proving ground's DNS fixture, whose record lists
//! every query it answered: a transport looks a new host up once, whatever
//! families the Session holds, and keeps the answer for its TTL, but for
//! an answer whose every address failed to connect, which it forgets, so
//! the next connection asks again (RFC 8305 section 9).
//!
//! Each Session holds a local address, so its lookups take the engine's
//! own resolver on every OS (ADR 0044), and each name declares a TTL, so
//! an answer kept is not asked again.
#![expect(
    clippy::disallowed_types,
    reason = "a loopback listener bound and closed on the test's thread, as an address nothing answers on, never an async worker (ADR 0060)"
)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};

use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_fetch_tier::{
    Hop, HttpProfile, LocalSource, Reqwest, Reuse, Transport, TransportConfig, TransportFailure,
    Wreq,
};
use goodfetch_page::ProxyRoute;
use goodfetch_test_support::{ProvingGround, TestEnv};
use rstest::rstest;
use serde_json::Value;
use url::Url;

use crate::{Stack, running_clock_runtime};

const SEED: u64 = 691;

/// `stack`'s transport sending from `local`, through `proxy` when given,
/// resolving names through the Proving ground's DNS fixture, loopback
/// listed as private.
fn transport(
    stack: Stack,
    proving_ground: &ProvingGround,
    local: LocalSource,
    proxy: Option<ProxyRoute>,
) -> Box<dyn Transport> {
    let fixture: SocketAddr = proving_ground.description().fixtures.dns.parse().unwrap();
    let config = TransportConfig {
        egress: Policy::new(&["*"], &[] as &[&str])
            .unwrap()
            .with_private_ranges(&["127.0.0.0/8".to_owned()])
            .unwrap(),
        resolve: HostMapping::default(),
        resolver: ResolverConfiguration::name_servers([fixture]).into(),
        local: Some(local),
        proxy,
        ..crate::config(proving_ground, None, &[])
    };
    match stack {
        Stack::Reqwest => Box::new(Reqwest::new(config).unwrap()),
        Stack::Wreq => Box::new(Wreq::new(config, HttpProfile::default()).unwrap()),
    }
}

/// The DNS fixture's name answering `address` alone, kept for a minute.
fn kept_name(address: IpAddr) -> String {
    let IpAddr::V4(address) = address else {
        panic!("the Proving ground serves on IPv4 loopback: {address}");
    };
    format!(
        "{}.ttl-60.mixed.dns.test",
        address.octets().map(|octet| octet.to_string()).join("-")
    )
}

/// The record types of the queries the DNS fixture answered for `name`,
/// in order.
fn queries_of(observations: &Value, name: &str) -> Vec<String> {
    observations["dns_queries"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|query| query["name"] == name)
        .map(|query| query["record_type"].as_str().unwrap().to_owned())
        .collect()
}

/// Sends a GET of `url` on `transport`, on a new connection: its status.
fn send(transport: &dyn Transport, url: &Url) -> Result<u16, TransportFailure> {
    running_clock_runtime().block_on(async {
        transport
            .send(&Hop::get(url.clone()), Reuse::Never)
            .await
            .map(|sent| sent.status)
    })
}

/// A Session holding an IPv4 and an IPv6 address looks a new host up
/// once: one A query and one AAAA query.
#[rstest]
fn a_new_host_is_looked_up_once_by_a_session_holding_both_families(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let target: SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    let name = kept_name(target.ip());
    let transport = transport(
        stack,
        &proving_ground,
        LocalSource {
            ipv4: Some(Ipv4Addr::LOCALHOST),
            ipv6: Some(Ipv6Addr::LOCALHOST),
            interface: None,
        },
        None,
    );
    let url = Url::parse(&format!("http://{name}:{}/", target.port())).unwrap();

    let status = send(transport.as_ref(), &url);
    let mut queries = queries_of(&proving_ground.observations().unwrap(), &name);
    queries.sort();

    assert_eq!(
        (status, queries),
        (Ok(200), vec!["A".to_owned(), "AAAA".to_owned()])
    );
}

/// The refused sends of `url` on `stack`'s transport, through `proxy`
/// when given, on two new connections in turn, and the A queries the DNS
/// fixture had answered for `name` after each.
fn refused_twice(
    stack: Stack,
    proving_ground: &ProvingGround,
    proxy: Option<ProxyRoute>,
    url: &str,
    name: &str,
) -> (Vec<Result<u16, TransportFailure>>, Vec<usize>) {
    let transport = transport(
        stack,
        proving_ground,
        LocalSource {
            ipv4: Some(Ipv4Addr::LOCALHOST),
            ..LocalSource::default()
        },
        proxy,
    );
    let url = Url::parse(url).unwrap();
    let mut sends = Vec::new();
    let mut asked = Vec::new();
    for _ in 0..2 {
        sends.push(send(transport.as_ref(), &url));
        let queries = queries_of(&proving_ground.observations().unwrap(), name);
        asked.push(queries.iter().filter(|query| *query == "A").count());
    }
    (sends, asked)
}

/// A name answering only an address nothing listens on, kept for a
/// minute, and the URL of that address's port under it.
fn refusing(proving_ground: &ProvingGround) -> (String, String) {
    let refusing = {
        let listener = TcpListener::bind((proving_ground.address().ip(), 0)).unwrap();
        listener.local_addr().unwrap()
    };
    let name = kept_name(refusing.ip());
    let url = format!("http://{name}:{}/", refusing.port());
    (name, url)
}

/// A host whose every address refused the first connection is looked up
/// again on the next one, though its answer's TTL has not run out.
#[rstest]
fn a_host_whose_every_address_refused_is_looked_up_again_on_the_next_connection(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (name, url) = refusing(&proving_ground);

    let (sends, asked) = refused_twice(stack, &proving_ground, None, &url, &name);

    let refused = Err(TransportFailure::ConnectionRefused);
    assert_eq!((sends, asked), (vec![refused.clone(), refused], vec![1, 2]));
}

/// A proxy whose every address refused the first connection is looked up
/// again on the next one, though its answer's TTL has not run out: through
/// a tunnel, which both transports report the same way.
#[rstest]
fn a_proxy_whose_every_address_refused_is_looked_up_again_on_the_next_connection(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let (name, proxy) = refusing(&proving_ground);
    let route = ProxyRoute {
        url: proxy,
        credentials: None,
    };
    let site = format!("https://{}/", proving_ground.address());

    let (sends, asked) = refused_twice(stack, &proving_ground, Some(route), &site, &name);

    let refused = Err(TransportFailure::ProxyUnreachable);
    assert_eq!((sends, asked), (vec![refused.clone(), refused], vec![1, 2]));
}
