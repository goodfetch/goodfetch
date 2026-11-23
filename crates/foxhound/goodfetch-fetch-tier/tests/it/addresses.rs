//! The address check of the HTTP tier's connections (ADRs 0017, 0067 and
//! 0118; spec #565's story 102), on reqwest and `wreq` against the Proving
//! ground: each transport judges every address its resolver answers before
//! a connection tries one, on each connection's own lookup, so a name
//! answering a public and a private address reaches neither and a name
//! whose answer turns private is refused on the connection after; through
//! the Session's upstream proxy, it resolves and judges the name itself
//! before sending, so the proxy, which would resolve it again, never sees a
//! request for a private name. The Proving ground's connection record is
//! the oracle, each refusal paired with a positive control.
//!
//! The transports resolve through the Proving ground's DNS fixture, in
//! process on every OS: a case's scripted lookups become the name of the
//! fixture's zone that answers them (`mixed.dns.test` for one lookup, its
//! answers at once; `alternating.dns.test` for a name answering each
//! lookup in turn). The Proving ground serves on loopback, so its watched
//! target's address stands for a public one, which the Session lists in its
//! private ranges, except where a case aims at that address itself.
//! Rebinding also runs end to end on Linux, through `DnsNamespace`, with
//! the system's resolver configuration.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::{IpAddr, SocketAddr};
use std::time::Duration;

use goodfetch_engine::dns::{HostMapping, ResolverConfiguration};
use goodfetch_engine::egress::Policy;
use goodfetch_fetch_tier::{
    Hop, HttpProfile, Reqwest, Reuse, Transport, TransportConfig, TransportFailure, Wreq,
};
use goodfetch_protocol::egress::EgressRule;
use goodfetch_test_support::{
    CaseDomains, CaseLookup, Family, Outcome, ProvingGround, Refusal, SsrfCase, SsrfTable, TestEnv,
    run_cases,
};
use rstest::rstest;
use serde_json::Value;
use url::Url;

use crate::{Stack, running_clock_runtime};

const SEED: u64 = 678;

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

/// The URL of the Proving ground's forward proxy `name`.
fn proxy(proving_ground: &ProvingGround, name: &str) -> Url {
    Url::parse(&format!(
        "http://{}",
        proving_ground.description().fixtures.proxies[name]
    ))
    .unwrap()
}

/// How many connections the listener `listener` accepted.
fn connections(observations: &Value, listener: &str) -> usize {
    observations["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == listener)
        .count()
}

/// The Session's egress policy: the domain policy `domains` and the
/// private `ranges`.
fn policy(domains: &CaseDomains, ranges: &[String]) -> Policy {
    Policy::new(&domains.allowed, &domains.blocked)
        .unwrap()
        .with_private_ranges(ranges)
        .unwrap()
}

/// `stack`'s transport resolving through the Proving ground's DNS fixture,
/// through `proxy` when given, under the domain policy `domains` and the
/// private `ranges`.
fn transport(
    stack: Stack,
    proving_ground: &ProvingGround,
    proxy: Option<Url>,
    domains: &CaseDomains,
    ranges: &[String],
) -> Box<dyn Transport> {
    let fixture: SocketAddr = proving_ground.description().fixtures.dns.parse().unwrap();
    built(
        stack,
        TransportConfig {
            egress: policy(domains, ranges),
            resolve: HostMapping::default(),
            resolver: ResolverConfiguration::name_servers([fixture]).into(),
            ..crate::config(proving_ground, proxy, &[])
        },
    )
}

/// `stack`'s transport under `config`.
fn built(stack: Stack, config: TransportConfig) -> Box<dyn Transport> {
    match stack {
        Stack::Reqwest => Box::new(Reqwest::new(config).unwrap()),
        Stack::Wreq => Box::new(Wreq::new(config, HttpProfile::default()).unwrap()),
    }
}

/// Every host allowed.
fn every_host() -> CaseDomains {
    CaseDomains {
        allowed: vec!["*".to_owned()],
        blocked: Vec::new(),
    }
}

/// The DNS fixture's label for `address`: its four numbers joined by `-`,
/// or its 32 hexadecimal digits.
fn label(address: IpAddr) -> String {
    match address {
        IpAddr::V4(address) => address.octets().map(|octet| octet.to_string()).join("-"),
        IpAddr::V6(address) => format!("{:032x}", u128::from(address)),
    }
}

/// The DNS fixture's name that answers `lookups`, all of one name, with
/// `public` for the public stand-in: its answers at once for one lookup,
/// each lookup's single answer in turn for several.
fn fixture_name(lookups: &[CaseLookup], public: IpAddr) -> String {
    let answers = |lookup: &CaseLookup| -> Vec<IpAddr> {
        lookup
            .answers
            .iter()
            .map(|answer| match answer {
                goodfetch_test_support::Answer::Public => public,
                goodfetch_test_support::Answer::Address(address) => *address,
            })
            .collect()
    };
    match lookups {
        [lookup] => format!(
            "{}.mixed.dns.test",
            answers(lookup)
                .into_iter()
                .map(label)
                .collect::<Vec<_>>()
                .join(".")
        ),
        _ => format!(
            "{}.alternating.dns.test",
            lookups
                .iter()
                .map(|lookup| {
                    let [address] = answers(lookup)[..] else {
                        panic!("an alternating lookup answers one address: {lookup:?}");
                    };
                    label(address)
                })
                .collect::<Vec<_>>()
                .join(".")
        ),
    }
}

/// `case`'s URL as the transport fetches it: at the watched target's port,
/// its scripted name replaced by the DNS fixture's name that answers it,
/// but for a domain policy case;
/// `None` for a URL the WHATWG parser refuses, which never reaches a tier
/// (the Frontier answers it `invalid_url`).
fn case_url(case: &SsrfCase, public: IpAddr, port: u16) -> Option<Url> {
    let mut url = case.url.clone();
    // A domain policy case's host is what the policy judges, before any
    // lookup.
    if let Some(lookup) = case
        .lookups
        .first()
        .filter(|_| case.family != Family::DomainPolicy)
    {
        assert!(
            case.lookups.iter().all(|other| other.name == lookup.name),
            "{}: one name",
            case.name
        );
        url = url.replacen(
            &format!("://{}", lookup.name),
            &format!("://{}", fixture_name(&case.lookups, public)),
            1,
        );
    }
    let mut url = Url::parse(&url).ok()?;
    url.set_port(Some(port)).ok()?;
    Some(url)
}

/// The table's form of the rule a refusal names.
fn table_rule(rule: EgressRule) -> goodfetch_test_support::EgressRule {
    match rule {
        EgressRule::DomainPolicy => goodfetch_test_support::EgressRule::DomainPolicy,
        EgressRule::PrivateAddress => goodfetch_test_support::EgressRule::PrivateAddress,
        EgressRule::Scheme => goodfetch_test_support::EgressRule::Scheme,
    }
}

/// How long a send that was let through may try to connect: a private
/// address nothing answers on would hold it until the OS gives up.
const LET_THROUGH: Duration = Duration::from_secs(3);

/// Sends `url` once on `transport`, on a new connection: its status, or
/// why it got none (`ConnectionTimedOut` past [`LET_THROUGH`]).
fn send(transport: &dyn Transport, url: &Url) -> Result<u16, TransportFailure> {
    running_clock_runtime().block_on(async {
        let hop = Hop::get(url.clone());
        match tokio::time::timeout(LET_THROUGH, transport.send(&hop, Reuse::Never)).await {
            Ok(sent) => sent.map(|sent| sent.status),
            Err(_) => Err(TransportFailure::ConnectionTimedOut),
        }
    })
}

/// The SSRF table's thin runner on `stack`'s transport, through the open
/// proxy when the case's route asks for it: each of the case's fetches up
/// to the first refusal, the watched target's address being `public`,
/// which the Session lists when `public_listed`.
fn run_case(
    stack: Stack,
    proving_ground: &ProvingGround,
    case: &SsrfCase,
    public_listed: bool,
) -> Outcome {
    let target = target(proving_ground);
    let public = target.ip();
    let ranges = if public_listed {
        vec![public.to_string()]
    } else {
        Vec::new()
    };
    let through = match case.route {
        goodfetch_test_support::Route::UpstreamProxy => Some(proxy(proving_ground, "open")),
        _ => None,
    };
    let transport = transport(
        stack,
        proving_ground,
        through,
        &case.domains.clone().unwrap_or_else(every_host),
        &ranges,
    );
    let url = case_url(case, public, target.port()).unwrap();
    let before = connections(&proving_ground.observations().unwrap(), "target");
    let mut refusal = None;
    for _ in 0..case.fetches {
        if let Err(TransportFailure::Egress(refused)) = send(transport.as_ref(), &url) {
            refusal = Some(Refusal::EgressRefused {
                rule: table_rule(refused.rule),
            });
            break;
        }
    }
    let after = connections(&proving_ground.observations().unwrap(), "target");
    Outcome {
        refusal,
        connections: vec![public; after - before],
    }
}

/// The positive controls: a name whose only answer is public reaches the
/// target directly and through the proxy, which the record shows.
const CONTROLS: &str = r#"
[[cases]]
name = "public_only"
family = "mixed_answers"
url = "http://public.ssrf.test/"
lookups = [{ name = "public.ssrf.test", answers = ["public"] }]
target = "public"
refusal = { code = "egress_refused", rule = "private_address" }

[[cases]]
name = "public_name_through_the_proxy"
family = "upstream_proxy"
url = "http://public.ssrf.test/"
route = { via = "upstream_proxy" }
lookups = [{ name = "public.ssrf.test", answers = ["public"] }]
target = "public"
refusal = { code = "egress_refused", rule = "private_address" }
"#;

/// Every case of the SSRF table a request of the HTTP tier meets, directly
/// or through the Session's upstream proxy, is refused before any
/// connection reaches its target: encodings, IPv6 forms, every non-global
/// IANA block, mixed answers (every address Happy Eyeballs would try),
/// rebinding, the domain policy, and the upstream proxy's cases, the proxy
/// itself seeing none of them.
///
/// Covers: story:4-102, error:egress_refused
#[rstest]
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn every_case_of_the_ssrf_table_the_http_tier_meets_is_refused_before_any_connection(
    #[case] stack: Stack,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let public = target(&proving_ground).ip();
    let table = SsrfTable::committed().unwrap();
    let families = [
        Family::Encoding,
        Family::Ipv6Form,
        Family::SpecialPurposeBlock,
        Family::MixedAnswers,
        Family::Rebinding,
        Family::DomainPolicy,
        Family::UpstreamProxy,
    ];
    let cases: Vec<&SsrfCase> = families
        .iter()
        .flat_map(|family| table.cases_of(*family))
        .collect();
    let (cases, unparsed): (Vec<&SsrfCase>, Vec<&SsrfCase>) = cases
        .into_iter()
        .partition(|case| case_url(case, public, 1).is_some());
    let controls = SsrfTable::parse(CONTROLS).unwrap();

    let controlled = run_cases(&controls.cases, public, |case| {
        run_case(stack, &proving_ground, case, true)
    });
    let proxied_before = connections(&proving_ground.observations().unwrap(), "open");
    let result = run_cases(cases.iter().copied(), public, |case| {
        let aims_at_public = case
            .target_address(public)
            .is_some_and(|aimed| aimed.to_canonical() == public.to_canonical());
        run_case(stack, &proving_ground, case, !aims_at_public)
    });
    let proxied = connections(&proving_ground.observations().unwrap(), "open") - proxied_before;

    assert_eq!(
        controlled.map_err(|let_through| {
            let_through
                .misses
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect::<Vec<_>>()
        }),
        Err(vec![
            ("public_only".to_owned(), None, true),
            ("public_name_through_the_proxy".to_owned(), None, true),
        ])
    );
    for family in families {
        assert!(
            cases.iter().any(|case| case.family == family),
            "the table has no {family:?} case the HTTP tier sends"
        );
    }
    // Only a zone ID's URL and the case named for it are URLs the WHATWG
    // parser refuses.
    assert_eq!(
        unparsed
            .iter()
            .filter(|case| !case.url.contains("%25"))
            .map(|case| case.name.as_str())
            .collect::<Vec<_>>(),
        ["url_that_does_not_parse"]
    );
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
    assert_eq!(proxied, 0, "the proxy saw a request of a refused case");
}

/// A name that answers a public address first and a private one after is
/// reached once, and refused on the next connection's own lookup, naming
/// the private address. (Through a proxy, the proxy's own lookup comes
/// after the transport's, so a name rebinding between the two is the
/// proxy's to judge; the table's upstream proxy cases hold the transport's
/// part.)
///
/// Covers: story:4-102, error:egress_refused
#[rstest]
#[case::reqwest(Stack::Reqwest)]
#[case::wreq(Stack::Wreq)]
fn a_name_whose_answer_turns_private_is_refused_on_the_next_connection(#[case] stack: Stack) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let target = target(&proving_ground);
    let private: IpAddr = "10.0.0.1".parse().unwrap();
    let transport = transport(
        stack,
        &proving_ground,
        None,
        &every_host(),
        &[target.ip().to_string()],
    );
    let url = Url::parse(&format!(
        "http://{}.{}.alternating.dns.test:{}/",
        label(target.ip()),
        label(private),
        target.port()
    ))
    .unwrap();

    let first = send(transport.as_ref(), &url);
    let second = send(transport.as_ref(), &url);

    let observations = proving_ground.observations().unwrap();
    let second = match second {
        Err(TransportFailure::Egress(refused)) => Ok((refused.rule, refused.address)),
        other => Err(other),
    };
    assert_eq!(
        (first, second, connections(&observations, "target")),
        (Ok(200), Ok((EgressRule::PrivateAddress, Some(private))), 1)
    );
}

/// A host the `network.resolve` mapping gives an address.
const MAPPED: &str = "mapped.test";

/// What the Session lists in `egress.private_ranges` for the mapped host's
/// answer, the watched target on loopback.
#[derive(Clone, Copy, Debug)]
enum Listing {
    /// Nothing.
    Nothing,
    /// The mapped host's name.
    Host,
    /// The target's address.
    Address,
}

/// The `network.resolve` mapping's answers are judged as the resolver's
/// are (ADR 0017, #622): a mapped host whose address is private is refused
/// before any connection, directly and, unsent to the proxy, through it,
/// unless the Session lists the host or a range holding its address in
/// `egress.private_ranges`, when it reaches the target.
///
/// Covers: story:4-102, error:egress_refused
#[rstest]
fn a_mapped_answer_to_a_private_address_is_refused_unless_its_host_or_range_is_listed(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
    #[values(Listing::Nothing, Listing::Host, Listing::Address)] listing: Listing,
    #[values(false, true)] proxied: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let target = target(&proving_ground);
    let ranges = match listing {
        Listing::Nothing => Vec::new(),
        Listing::Host => vec![MAPPED.to_owned()],
        Listing::Address => vec![target.ip().to_string()],
    };
    let through = proxied.then(|| proxy(&proving_ground, "open"));
    let transport = built(
        stack,
        TransportConfig {
            egress: policy(&every_host(), &ranges),
            resolve: HostMapping::new([(MAPPED, vec![target.ip()])]),
            ..crate::config(&proving_ground, through, &[])
        },
    );
    let url = Url::parse(&format!("http://{MAPPED}:{}/", target.port())).unwrap();
    let before = proving_ground.observations().unwrap();

    let sent = match send(transport.as_ref(), &url) {
        Err(TransportFailure::Egress(refused)) => Err((refused.rule, refused.address)),
        other => Ok(other.is_ok()),
    };

    let after = proving_ground.observations().unwrap();
    let opened = |listener: &str| connections(&after, listener) - connections(&before, listener);
    match listing {
        Listing::Nothing => assert_eq!(
            (sent, opened("target"), opened("open")),
            (Err((EgressRule::PrivateAddress, Some(target.ip()))), 0, 0)
        ),
        // Through the proxy, which knows no mapping, a listed host is
        // sent on; the proxy's own lookup is its own to answer.
        Listing::Host | Listing::Address if proxied => {
            assert_eq!((sent.is_ok(), opened("open")), (true, 1));
        }
        Listing::Host | Listing::Address => {
            assert_eq!((sent, opened("target")), (Ok(true), 1));
        }
    }
}

/// Rebinding end to end (ADR 0067): this test binary runs again in a
/// `DnsNamespace` whose resolver is the Proving ground's DNS fixture, and
/// there sends, on each transport with the system's resolver
/// configuration, a name that answers a loopback address other than
/// `127.0.0.1`, which stands for a public address its Session lists, and
/// then `127.0.0.1`. The first reaches its server; the second is refused,
/// naming `127.0.0.1`, and nothing connects to the server listening there.
/// The transports run at once, each on a name of its own: the fixture
/// keeps one turn per name, so two cases sharing one took each other's
/// turns and a first send went to `127.0.0.1` (#2108).
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_types,
    reason = "the test's two servers bind their own sockets on loopback, as sites outside the engine do (ADR 0060 covers the engine's sockets)"
)]
mod linux {
    use std::io::ErrorKind;
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener};

    use goodfetch_engine::clock::Clock;
    use goodfetch_engine::dns::{HostMapping, ResolverSource};
    use goodfetch_engine::tls::{CertificateExceptions, Trust};
    use goodfetch_fetch_tier::{Hop, Reuse, Sent, Transport, TransportConfig, TransportFailure};
    use goodfetch_test_support::{DnsNamespace, ProvingGround, TestEnv};
    use rstest::rstest;
    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
    use url::Url;

    use super::{SEED, built, every_host, label, policy};
    use crate::{Stack, running_clock_runtime};

    const CHILD: &str = "addresses::linux::in_the_namespace_a_name_that_rebinds_to_loopback_is_refused_on_its_second_connection";
    const OTHER_TARGETS_CHILD: &str =
        "addresses::linux::in_the_namespace_a_name_that_rebinds_past_loopback_is_refused";

    /// Runs the tests `child` names inside a DNS namespace on the Proving
    /// ground's DNS fixture, and asserts that some ran and all passed.
    fn passes_in_the_namespace(child: &str) {
        let env = TestEnv::new();
        let proving_ground = ProvingGround::start(&env, SEED).unwrap();
        let fixture = proving_ground.description().fixtures.dns.parse().unwrap();
        let namespace = DnsNamespace::enter(&env, fixture).unwrap();

        let output = namespace
            .output(
                &env,
                std::env::current_exe().unwrap(),
                [child, "--include-ignored"],
            )
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && !stdout.contains("running 0 tests"),
            "{output:?}"
        );
    }

    /// Covers: story:4-102
    #[test]
    fn on_linux_a_name_that_rebinds_to_loopback_is_refused_on_its_second_connection() {
        passes_in_the_namespace(CHILD);
    }

    /// The SSRF table's other two rebinding targets, through a real resolver,
    /// which the in-process runner's scripted answers do not go through.
    /// Covers: story:4-102
    #[test]
    fn on_linux_a_name_that_rebinds_past_loopback_is_refused() {
        passes_in_the_namespace(OTHER_TARGETS_CHILD);
    }

    /// The child, which runs only inside the namespace. The fixture takes
    /// turns per record type, so a name with an IPv6 entry answers it beside
    /// the public stand-in from its first lookup: that case is refused on
    /// either connection, and the metadata address on its second. No address
    /// of the namespace may answer the metadata one, so only `::1` is watched.
    #[rstest]
    #[case::reqwest_to_metadata(
        Stack::Reqwest,
        Ipv4Addr::new(127, 0, 0, 4),
        IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))
    )]
    #[case::wreq_to_metadata(
        Stack::Wreq,
        Ipv4Addr::new(127, 0, 0, 5),
        IpAddr::V4(Ipv4Addr::new(169, 254, 169, 254))
    )]
    #[case::reqwest_to_ipv6_loopback(
        Stack::Reqwest,
        Ipv4Addr::new(127, 0, 0, 6),
        IpAddr::V6(Ipv6Addr::LOCALHOST)
    )]
    #[case::wreq_to_ipv6_loopback(
        Stack::Wreq,
        Ipv4Addr::new(127, 0, 0, 7),
        IpAddr::V6(Ipv6Addr::LOCALHOST)
    )]
    #[ignore = "run as a child by addresses::linux::on_linux_a_name_that_rebinds_past_loopback_is_refused"]
    fn in_the_namespace_a_name_that_rebinds_past_loopback_is_refused(
        #[case] stack: Stack,
        #[case] public: Ipv4Addr,
        #[case] rebound: IpAddr,
    ) {
        let public = IpAddr::V4(public);
        let server = TcpListener::bind(SocketAddr::new(public, 0)).unwrap();
        server.set_nonblocking(true).unwrap();
        let port = server.local_addr().unwrap().port();
        let watched = rebound
            .is_loopback()
            .then(|| TcpListener::bind(SocketAddr::new(rebound, port)).unwrap());
        if let Some(watched) = &watched {
            watched.set_nonblocking(true).unwrap();
        }
        let transport = built(
            stack,
            TransportConfig {
                trust: Trust::default_roots(),
                exceptions: CertificateExceptions::default(),
                client_certificates: None,
                proxy: None,
                resolve: HostMapping::default(),
                egress: policy(&every_host(), &[public.to_string()]),
                clock: Clock::system(),
                resolver: ResolverSource::system(),
                local: None,
                delays: goodfetch_engine::happy_eyeballs::Delays::default(),
            },
        );
        let name = format!("{}.{}.alternating.dns.test", label(public), label(rebound));
        let hop = Hop::get(Url::parse(&format!("http://{name}:{port}/")).unwrap());

        let refused = running_clock_runtime().block_on(async {
            let server = tokio::net::TcpListener::from_std(server).unwrap();
            for _ in 0..2 {
                if let Err(failure) = served_send(&server, &*transport, &hop).await {
                    return Some(failure);
                }
            }
            None
        });

        let Some(TransportFailure::Egress(refused)) = refused else {
            panic!("a send is refused: {refused:?}");
        };
        assert_eq!(refused.address, Some(rebound));
        if let Some(watched) = watched {
            assert_eq!(
                watched.accept().map(|_| ()).unwrap_err().kind(),
                ErrorKind::WouldBlock
            );
        }
    }

    /// Answers one plain HTTP request on `listener` with an empty 200.
    async fn answer_once(listener: &tokio::net::TcpListener) {
        let (mut stream, _) = listener.accept().await.unwrap();
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(stream.read_u8().await.unwrap());
        }
        stream
            .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 0\r\nconnection: close\r\n\r\n")
            .await
            .unwrap();
    }

    /// What `transport` answers for `hop` while `listener` answers every
    /// request it gets, so the send ends whichever address its lookup gives.
    async fn served_send(
        listener: &tokio::net::TcpListener,
        transport: &dyn Transport,
        hop: &Hop,
    ) -> Result<Sent, TransportFailure> {
        let serving = async {
            loop {
                answer_once(listener).await;
            }
        };
        tokio::select! {
            sent = transport.send(hop, Reuse::Never) => sent,
            () = serving => unreachable!("the server serves until the send ends"),
        }
    }

    /// The child, which runs only inside the namespace, on each transport
    /// with its own stand-in for a public address.
    #[rstest]
    #[case::reqwest(Stack::Reqwest, Ipv4Addr::new(127, 0, 0, 2))]
    #[case::wreq(Stack::Wreq, Ipv4Addr::new(127, 0, 0, 3))]
    #[ignore = "run as a child by addresses::linux::on_linux_a_name_that_rebinds_to_loopback_is_refused_on_its_second_connection"]
    fn in_the_namespace_a_name_that_rebinds_to_loopback_is_refused_on_its_second_connection(
        #[case] stack: Stack,
        #[case] public: Ipv4Addr,
    ) {
        let public = IpAddr::V4(public);
        let server = TcpListener::bind(SocketAddr::new(public, 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        let private =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).unwrap();
        private.set_nonblocking(true).unwrap();
        let transport = built(
            stack,
            TransportConfig {
                trust: Trust::default_roots(),
                exceptions: CertificateExceptions::default(),
                client_certificates: None,
                proxy: None,
                resolve: HostMapping::default(),
                egress: policy(&every_host(), &[public.to_string()]),
                clock: Clock::system(),
                resolver: ResolverSource::system(),
                local: None,
                delays: goodfetch_engine::happy_eyeballs::Delays::default(),
            },
        );
        let name = format!("{}.127-0-0-1.alternating.dns.test", label(public));
        let hop = Hop::get(Url::parse(&format!("http://{name}:{port}/")).unwrap());

        let (first, second) = running_clock_runtime().block_on(async {
            server.set_nonblocking(true).unwrap();
            let server = tokio::net::TcpListener::from_std(server).unwrap();
            // A turn of the name out of step fails the case on its
            // assertions instead of a wait: the server answers every send.
            let first = served_send(&server, &*transport, &hop)
                .await
                .map(|sent| sent.status);
            let second = served_send(&server, &*transport, &hop).await.map(|_| ());
            (first, second)
        });

        assert_eq!(first.unwrap(), 200);
        let Err(TransportFailure::Egress(refused)) = second else {
            panic!("the second send is refused: {second:?}");
        };
        assert_eq!(refused.address, Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert_eq!(
            private.accept().map(|_| ()).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
    }
}
