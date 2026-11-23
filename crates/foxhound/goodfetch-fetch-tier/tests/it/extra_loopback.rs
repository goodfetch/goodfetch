//! Tests that need the extra loopback addresses `127.0.0.2` to `127.0.0.9`
//! (TESTING.md, Storage, network and OS services): a host without them skips
//! this module.
//!
//! A Session's local addresses on the HTTP tiers (spec #565's story 108;
//! ADR 0044), reqwest's and `wreq`'s, against the Proving ground: each
//! hop's connection leaves from the Session's address, which the Proving
//! ground's connection record names, and its DNS queries leave from it
//! through the engine's resolver, which the DNS fixture's record names; an
//! interface name binds the socket to its device where the OS allows.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};

use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_fetch_tier::{
    Hop, HttpProfile, LocalSource, Reqwest, Reuse, Transport, TransportConfig, TransportFailure,
    Wreq,
};
use goodfetch_protocol::dns::IpFamily;
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use rstest::rstest;
use serde_json::Value;
use url::Url;

use crate::{Stack, running_clock_runtime};

const SEED: u64 = 682;

/// The extra loopback address the Session holds.
const BOUND: Ipv4Addr = Ipv4Addr::new(127, 0, 0, 3);

/// `stack`'s transport under `config`.
fn built(stack: Stack, config: TransportConfig) -> Box<dyn Transport> {
    match stack {
        Stack::Reqwest => Box::new(Reqwest::new(config).unwrap()),
        Stack::Wreq => Box::new(Wreq::new(config, HttpProfile::default()).unwrap()),
    }
}

/// A transport sending from `local`, resolving names through the Proving
/// ground's DNS fixture alone and `resolve`, which reaches the Proving
/// ground on loopback.
fn bound(
    stack: Stack,
    proving_ground: &ProvingGround,
    local: LocalSource,
    resolve: Vec<(String, Vec<IpAddr>)>,
) -> Box<dyn Transport> {
    let fixture: SocketAddr = proving_ground.description().fixtures.dns.parse().unwrap();
    built(
        stack,
        TransportConfig {
            egress: Policy::new(&["*"], &[] as &[&str])
                .unwrap()
                .with_private_ranges(&["127.0.0.0/8".to_owned()])
                .unwrap(),
            resolve: HostMapping::new(resolve),
            resolver: ResolverConfiguration::name_servers([fixture]).into(),
            local: Some(local),
            ..crate::config(proving_ground, None, &[])
        },
    )
}

/// The source address of the connection each request of the case for the
/// target `path` came on.
fn sources_of(observations: &Value, path: &str) -> Vec<IpAddr> {
    let connections = observations["connections"].as_array().unwrap();
    ReceivedRequests::of(observations)
        .of_the_case()
        .filter(|request| request.target == path)
        .map(|request| {
            let connection = connections
                .iter()
                .find(|connection| connection["id"] == request.connection)
                .unwrap_or_else(|| panic!("no connection for {request:?}"));
            connection["source"]
                .as_str()
                .unwrap()
                .parse::<SocketAddr>()
                .unwrap()
                .ip()
        })
        .collect()
}

/// A Session holding `127.0.0.3` reaches the site from it, and asks the
/// name server for the site's name from it.
///
/// Covers: story:4-108, row:SA-25
#[rstest]
fn a_sessions_hop_and_its_dns_queries_leave_from_its_local_address(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = bound(
        stack,
        &proving_ground,
        LocalSource {
            ipv4: Some(BOUND),
            ..LocalSource::default()
        },
        Vec::new(),
    );
    let hop = Hop::get(
        Url::parse(&format!(
            "http://1.catalog.test:{}/catalog/1",
            proving_ground.address().port()
        ))
        .unwrap(),
    );

    let status = running_clock_runtime().block_on(async {
        transport
            .send(&hop, Reuse::Allowed)
            .await
            .map(|sent| sent.status)
    });
    let observations = proving_ground.observations().unwrap();
    let queries = ProvingGround::dns_query_sources(&observations, "1.catalog.test");

    assert_eq!(
        (
            status,
            sources_of(&observations, "/catalog/1"),
            queries.is_empty(),
            queries.iter().all(|source| *source == IpAddr::V4(BOUND)),
        ),
        (Ok(200), vec![IpAddr::V4(BOUND)], false, true)
    );
}

/// A Session holding only an IPv6 address fails a name that answers only
/// IPv4 with `address_family_unavailable`, and opens no connection from
/// an address the OS chose.
///
/// Covers: story:4-46, story:4-108, error:address_family_unavailable
#[rstest]
fn a_name_answering_only_the_family_a_session_lacks_fails_with_the_family_held(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = bound(
        stack,
        &proving_ground,
        LocalSource {
            ipv6: Some(Ipv6Addr::LOCALHOST),
            ..LocalSource::default()
        },
        vec![(
            "catalog.test".to_owned(),
            vec![proving_ground.address().ip()],
        )],
    );
    let hop = Hop::get(
        Url::parse(&format!(
            "http://catalog.test:{}/catalog/2",
            proving_ground.address().port()
        ))
        .unwrap(),
    );

    let failure = running_clock_runtime().block_on(async {
        transport
            .send(&hop, Reuse::Allowed)
            .await
            .map(|sent| sent.status)
    });
    let observations = proving_ground.observations().unwrap();

    let code = failure
        .as_ref()
        .err()
        .map(|failure| failure.error(hop.url.as_str()).code());
    assert_eq!(
        (failure, code, sources_of(&observations, "/catalog/2"),),
        (
            Err(TransportFailure::AddressFamily {
                name: "catalog.test".to_owned(),
                held: IpFamily::Ipv6,
            }),
            Some("address_family_unavailable"),
            Vec::new(),
        )
    );
}

/// A Session holding only an IPv6 address fails a URL naming an IPv4
/// address as it fails a name answering only IPv4, rather than opening a
/// connection from an address the OS chose.
///
/// Covers: story:4-108, error:address_family_unavailable
#[rstest]
fn an_address_of_the_family_a_session_lacks_fails_with_the_family_held(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = bound(
        stack,
        &proving_ground,
        LocalSource {
            ipv6: Some(Ipv6Addr::LOCALHOST),
            ..LocalSource::default()
        },
        Vec::new(),
    );
    let address = proving_ground.address();
    let hop = Hop::get(Url::parse(&format!("http://{address}/catalog/3")).unwrap());

    let failure = running_clock_runtime().block_on(async {
        transport
            .send(&hop, Reuse::Allowed)
            .await
            .map(|sent| sent.status)
    });
    let observations = proving_ground.observations().unwrap();

    assert_eq!(
        (failure, sources_of(&observations, "/catalog/3")),
        (
            Err(TransportFailure::AddressFamily {
                name: address.ip().to_string(),
                held: IpFamily::Ipv6,
            }),
            Vec::new(),
        )
    );
}

/// The first network interface of this host that is not loopback.
#[cfg(target_os = "linux")]
fn other_interface() -> String {
    #[expect(
        clippy::disallowed_methods,
        reason = "the test reads the host's interfaces before any runtime starts"
    )]
    let names = std::fs::read_dir("/sys/class/net").unwrap();
    names
        .map(|entry| entry.unwrap().file_name().into_string().unwrap())
        .find(|name| name != "lo")
        .expect("a Linux host with an interface beside `lo`")
}

/// An interface name binds the socket to its device on Linux: a Session
/// of `lo`'s address bound to `lo` reaches the site from that address,
/// and the same address bound to another device cannot reach loopback at
/// all, which only the device binding explains.
///
/// Covers: story:4-108, row:SA-25
#[cfg(target_os = "linux")]
#[rstest]
fn an_interface_binds_its_device(#[values(Stack::Reqwest, Stack::Wreq)] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let address = Ipv4Addr::new(127, 0, 0, 4);
    let on = |interface: String, path: &str| {
        let transport = bound(
            stack,
            &proving_ground,
            LocalSource {
                ipv4: Some(address),
                ipv6: None,
                interface: Some(interface),
            },
            vec![(
                "catalog.test".to_owned(),
                vec![proving_ground.address().ip()],
            )],
        );
        let hop = Hop::get(
            Url::parse(&format!(
                "http://catalog.test:{}{path}",
                proving_ground.address().port()
            ))
            .unwrap(),
        );
        running_clock_runtime().block_on(async {
            transport
                .send(&hop, Reuse::Never)
                .await
                .map(|sent| sent.status)
        })
    };

    let on_loopback = on("lo".to_owned(), "/catalog/3");
    let elsewhere = on(other_interface(), "/catalog/4");
    let observations = proving_ground.observations().unwrap();

    assert_eq!(
        (
            on_loopback,
            sources_of(&observations, "/catalog/3"),
            elsewhere.is_err(),
            sources_of(&observations, "/catalog/4"),
        ),
        (Ok(200), vec![IpAddr::V4(address)], true, Vec::new())
    );
}
