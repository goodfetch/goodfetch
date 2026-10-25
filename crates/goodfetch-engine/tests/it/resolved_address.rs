//! The resolved-address check on the engine's own HTTP client, against the
//! Proving ground (ADRs 0017 and 0067; spec #564): a request of the run
//! whose host stands for an address a page may not reach ends with
//! `egress_refused` (`private_address`) before any connection, which the
//! Proving ground's connection record shows, whatever the address's
//! encoding, IPv6 form or IANA block, and whatever public answer its name
//! also has. Each refusal pairs with a positive control that reaches the
//! Proving ground's watched target.
//!
//! The Proving ground serves on loopback, so its address stands for a
//! public one: a Session lists it in its private ranges, as every test that
//! reaches loopback does with the check on, except where a case aims at
//! that address itself. A case's lookups are given through the `resolve`
//! mapping, whose addresses are judged as any answer is. Rebinding runs end
//! to end on Linux, through `DnsNamespace`, against the Proving ground's
//! DNS fixture.

use std::net::{IpAddr, SocketAddr};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::http_client::{FetchError, PageTraffic, Resolve, Waiter};
use goodfetch_engine::mask::{Masks, Redaction};
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::tls::{CertificateExceptions, ClientCertificates, Trust};
use goodfetch_protocol::egress::{EgressRefusedDetails, EgressRule};
use goodfetch_test_support::{
    Answer, CaseDomains, Family, Outcome, ProvingGround, Refusal, SsrfCase, SsrfTable, TestEnv,
    run_cases,
};
use serde_json::json;

use crate::egress::{at_port, request_log_lines, table_rule, target_connections};

const SEED: u64 = 621;

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

/// What a run may reach: `allowed` hosts, the addresses `resolve` maps
/// them to, and the private `ranges`.
fn traffic(
    allowed: &[String],
    blocked: &[String],
    resolve: Resolve,
    ranges: &[String],
) -> PageTraffic {
    PageTraffic {
        egress: Policy::new(allowed, blocked)
            .unwrap()
            .with_private_ranges(ranges)
            .unwrap(),
        resolve,
        local: None,
        proxy: None,
        trust: Trust::default_roots(),
        exceptions: CertificateExceptions::default(),
        client_certificates: ClientCertificates::default(),
        cookies: Vec::new(),
        clock: Clock::system(),
        delays: Delays::BUILT_IN,
    }
}

/// Fetches `url`, its port that of the watched target, under `traffic`:
/// the refusal, and how many connections reached the target meanwhile.
fn fetch(
    proving_ground: &ProvingGround,
    traffic: &PageTraffic,
    url: &str,
) -> (Result<u16, FetchError>, usize) {
    let before = target_connections(&proving_ground.observations().unwrap());
    let fetched = goodfetch_test_support::runtime().block_on(async {
        traffic
            .get(&at_port(url, target(proving_ground).port()), Waiter::Call)
            .await
            .map(|response| response.status())
    });
    let after = target_connections(&proving_ground.observations().unwrap());
    (fetched, after - before)
}

/// The SSRF table's thin runner on the engine's HTTP client: the case's
/// lookups as the `resolve` mapping, `public` being the watched target's
/// address, which the Session lists when `public_listed`.
fn run_case(proving_ground: &ProvingGround, case: &SsrfCase, public_listed: bool) -> Outcome {
    let public = target(proving_ground).ip();
    let domains = case.domains.clone().unwrap_or(CaseDomains {
        allowed: vec!["*".to_owned()],
        blocked: Vec::new(),
    });
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
    let traffic = traffic(&domains.allowed, &domains.blocked, resolve, &ranges);
    let (fetched, connections) = fetch(proving_ground, &traffic, &case.url);
    Outcome {
        refusal: match fetched {
            Err(FetchError::Refused(refused)) => Some(Refusal::EgressRefused {
                rule: table_rule(refused.rule),
            }),
            _ => None,
        },
        connections: vec![public; connections],
    }
}

/// Whether a case aims at the address that stands for a public one, which
/// the Session then may not list.
fn aims_at(case: &SsrfCase, public: IpAddr) -> bool {
    case.target_address(public)
        .is_some_and(|target| target.to_canonical() == public.to_canonical())
}

/// Covers: story:4-8, story:4-9, error:egress_refused
#[test]
fn every_address_case_of_the_ssrf_table_is_refused_before_any_connection() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let public = target(&proving_ground).ip();
    let table = SsrfTable::committed().unwrap();
    let cases: Vec<&SsrfCase> = [
        Family::Encoding,
        Family::Ipv6Form,
        Family::SpecialPurposeBlock,
        Family::MixedAnswers,
    ]
    .iter()
    .flat_map(|family| table.cases_of(*family))
    .collect();
    // The positive controls: the loopback address of the encodings, once
    // the Session lists it, and a name whose only answer is public, each
    // reach the target, which the record shows.
    let controls = SsrfTable::parse(
        r#"
        [[cases]]
        name = "listed_loopback"
        family = "encoding"
        url = "http://127.0.0.1/"
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }

        [[cases]]
        name = "public_only"
        family = "mixed_answers"
        url = "http://public.ssrf.test/"
        lookups = [{ name = "public.ssrf.test", answers = ["public"] }]
        target = "public"
        refusal = { code = "egress_refused", rule = "private_address" }
        "#,
    )
    .unwrap();

    let controlled = run_cases(&controls.cases, public, |case| {
        run_case(&proving_ground, case, true)
    });
    let result = run_cases(cases.iter().copied(), public, |case| {
        run_case(&proving_ground, case, !aims_at(case, public))
    });

    assert_eq!(
        controlled.map_err(|let_through| {
            let_through
                .misses
                .iter()
                .map(|miss| (miss.case.clone(), miss.refusal.clone(), miss.reached_target))
                .collect::<Vec<_>>()
        }),
        Err(vec![
            ("listed_loopback".to_owned(), None, true),
            ("public_only".to_owned(), None, true),
        ])
    );
    assert!(cases.len() >= 70, "only {} cases", cases.len());
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}

/// A regression test for Firecrawl's CVE-2024-56800 (GHSA-vjp8-2wgg-p734):
/// a scrape target whose name resolves to the cloud-metadata address or the
/// intranet reaches neither, while the same name resolving publicly is
/// fetched.
///
/// Covers: story:4-8, error:egress_refused
#[test]
fn a_page_whose_name_resolves_to_the_metadata_address_or_the_intranet_is_refused() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let public = target(&proving_ground).ip();
    let every_host = vec!["*".to_owned()];
    let at = |address: IpAddr| {
        traffic(
            &every_host,
            &[],
            Resolve::new().with("attacker.test", vec![address]),
            &[public.to_string()],
        )
    };
    let url = "http://attacker.test/latest/meta-data/";
    let fetched_url = at_port(url, target(&proving_ground).port());

    let refused: Vec<(Option<EgressRefusedDetails>, usize)> = ["169.254.169.254", "10.0.0.1"]
        .map(|address| {
            let (fetched, connections) = fetch(&proving_ground, &at(address.parse().unwrap()), url);
            let details = match fetched {
                Err(FetchError::Refused(refused)) => {
                    assert_eq!(refused.error().code(), "egress_refused");
                    Some(refused.details())
                }
                _ => None,
            };
            (details, connections)
        })
        .into();
    let (control, control_connections) = fetch(&proving_ground, &at(public), url);

    assert_eq!(
        refused,
        ["169.254.169.254", "10.0.0.1"].map(|address| (
            Some(EgressRefusedDetails {
                url: fetched_url.clone(),
                rule: EgressRule::PrivateAddress,
                address: Some(address.to_owned()),
                setting: Some("egress.private_ranges".to_owned()),
            }),
            0
        ))
    );
    assert_eq!((control.unwrap(), control_connections), (200, 1));
}

/// Covers: story:4-8, event:page.request_failed
#[test]
fn a_private_address_no_call_waits_on_is_a_request_failed_event_in_the_run_trace() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let url = at_port("http://intranet.test/", target(&proving_ground).port());
    let traffic = traffic(
        &["*".to_owned()],
        &[],
        Resolve::new().with("intranet.test", vec!["10.0.0.1".parse().unwrap()]),
        &[],
    );
    let run_folder = env.folder().join("run");
    let log = RequestLog::new(
        TaskOwner::root(BlockingPolicy::Inline),
        &run_folder,
        Masks::new().for_run(Redaction::On),
    );

    let refused = goodfetch_test_support::runtime()
        .block_on(traffic.get(&url, Waiter::Nobody(&log)))
        .map(|response| response.status());

    assert!(
        matches!(refused, Err(FetchError::Refused(_))),
        "{refused:?}"
    );
    assert_eq!(
        request_log_lines(&run_folder),
        [json!({
            "event": "page.request_failed",
            "data": {
                "url": url,
                "method": "GET",
                "failure": {
                    "code": "egress_refused",
                    "retryable": false,
                    "url": url,
                    "rule": "private_address",
                    "address": "10.0.0.1",
                    "setting": "egress.private_ranges",
                },
            },
        })]
    );
}

/// Rebinding end to end (ADR 0067): this test binary runs again in a
/// `DnsNamespace` whose resolver is the Proving ground's DNS fixture, and
/// there fetches a name that answers `127.0.0.2`, which stands for a public
/// address its Session lists, and then `127.0.0.1`. The first fetch reaches
/// its server; the second is refused, naming `127.0.0.1`, and nothing
/// connects to the server listening there.
#[cfg(target_os = "linux")]
mod linux {
    use std::io::ErrorKind;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpListener};

    use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};

    use goodfetch_engine::http_client::{FetchError, Resolve, Waiter};
    use goodfetch_test_support::{DnsNamespace, ProvingGround, TestEnv};

    use super::{SEED, traffic};

    const NAME: &str = "127-0-0-2.127-0-0-1.alternating.dns.test";
    const CHILD: &str = "resolved_address::linux::in_the_namespace_a_name_that_rebinds_to_loopback_is_refused_on_its_second_connection";

    /// Covers: story:4-8
    #[test]
    fn on_linux_a_name_that_rebinds_to_loopback_is_refused_on_its_second_connection() {
        let env = TestEnv::new();
        let proving_ground = ProvingGround::start(&env, SEED).unwrap();
        let fixture = proving_ground.description().fixtures.dns.parse().unwrap();
        let namespace = DnsNamespace::enter(&env, fixture).unwrap();

        let output = namespace
            .output(
                &env,
                std::env::current_exe().unwrap(),
                [CHILD, "--exact", "--include-ignored"],
            )
            .unwrap();

        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(
            output.status.success() && stdout.contains("1 passed"),
            "{output:?}"
        );
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

    /// The child, which runs only inside the namespace.
    #[test]
    #[ignore = "run as a child by resolved_address::linux::on_linux_a_name_that_rebinds_to_loopback_is_refused_on_its_second_connection"]
    fn in_the_namespace_a_name_that_rebinds_to_loopback_is_refused_on_its_second_connection() {
        let public = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
        let server = TcpListener::bind(SocketAddr::new(public, 0)).unwrap();
        let port = server.local_addr().unwrap().port();
        let private =
            TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port)).unwrap();
        private.set_nonblocking(true).unwrap();
        let traffic = traffic(
            &["*".to_owned()],
            &[],
            Resolve::new(),
            &[public.to_string()],
        );
        let url = format!("http://{NAME}:{port}/");

        let (first, second) = goodfetch_test_support::runtime().block_on(async {
            server.set_nonblocking(true).unwrap();
            let server = tokio::net::TcpListener::from_std(server).unwrap();
            let (first, ()) = tokio::join!(
                async {
                    traffic
                        .get(&url, Waiter::Call)
                        .await
                        .map(|response| response.status())
                },
                answer_once(&server),
            );
            let second = traffic.get(&url, Waiter::Call).await.map(|_| ());
            (first, second)
        });

        assert_eq!(first.unwrap(), 200);
        let Err(FetchError::Refused(refused)) = second else {
            panic!("the second fetch is refused: {second:?}");
        };
        assert_eq!(refused.address, Some(IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert_eq!(
            private.accept().map(|_| ()).unwrap_err().kind(),
            ErrorKind::WouldBlock
        );
    }
}
