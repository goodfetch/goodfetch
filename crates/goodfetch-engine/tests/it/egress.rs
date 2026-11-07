//! The domain policy on the engine's own HTTP client, against the Proving
//! ground (ADRs 0014, 0017 and 0100): a request of the run the policy
//! allows reaches its host, and one it refuses ends with `egress_refused`
//! before any connection, which the Proving ground's connection record
//! shows; a refused request no call waits on is a `page.request_failed`
//! event in the Run trace. The `resolve` setting maps every host name to
//! the Proving ground's loopback address, whose certificate authority the
//! client trusts for `scope.test`, as `tls.trusted_cas` would, and which
//! the Session's private ranges list, as every test that reaches loopback
//! does with the resolved-address check on (spec #564): no test reaches
//! the internet.

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
    Answer, CaseDomains, Outcome, ProvingGround, ReceivedRequests, Refusal, Runner, SsrfCase,
    SsrfTable, TestEnv,
};
use rustls::pki_types::CertificateDer;
use rustls::pki_types::pem::PemObject as _;
use serde_json::{Value, json};

/// Any seed: the Crawl family's scope hosts are served on every one.
const SEED: u64 = 620;

/// What a run may reach under `allowed` and `blocked`: every host name the
/// Proving ground serves, at its address, under its certificate authority.
fn traffic(proving_ground: &ProvingGround, allowed: &[&str], blocked: &[&str]) -> PageTraffic {
    let description = proving_ground.description();
    let resolve = description
        .resolve
        .iter()
        .fold(Resolve::new(), |resolve, (host, address)| {
            resolve.with(host.clone(), vec![address.parse::<IpAddr>().unwrap()])
        });
    PageTraffic {
        egress: Policy::new(allowed, blocked)
            .unwrap()
            .with_private_ranges(&[proving_ground.address().ip().to_string()])
            .unwrap(),
        resolve,
        local: None,
        proxy: None,
        trust: Trust::default_roots()
            .with_trusted_cas(
                "scope.test",
                vec![CertificateDer::from_pem_file(&description.ca).unwrap()],
            )
            .unwrap(),
        exceptions: CertificateExceptions::default(),
        client_certificates: ClientCertificates::default(),
        cookies: Vec::new(),
        clock: Clock::system(),
        delays: Delays::default(),
    }
}

/// `path` on `host` over HTTPS, at the Proving ground's site.
fn site_url(proving_ground: &ProvingGround, host: &str, path: &str) -> String {
    format!("https://{host}:{}{path}", proving_ground.address().port())
}

/// `elsewhere.test` at the Proving ground's watched target, which answers
/// any plain HTTP request and whose every connection the record lists
/// under the listener `target`.
fn target_url(proving_ground: &ProvingGround) -> String {
    let target: SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    format!("http://elsewhere.test:{}/offsite", target.port())
}

/// The hosts and paths of the page requests the Proving ground received.
fn requests(observations: &Value) -> Vec<Value> {
    ReceivedRequests::of(observations)
        .every()
        .map(|request| json!([request["host"], request["target"]]))
        .collect()
}

/// How many connections reached the watched target.
pub(crate) fn target_connections(observations: &Value) -> usize {
    observations["connections"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|connection| connection["listener"] == "target")
        .count()
}

/// Covers: story:4-2, story:4-3, error:egress_refused, setting:egress.allowed_domains
#[test]
fn a_fetch_reaches_a_listed_host_and_its_subdomain_and_another_host_is_refused_before_it_connects()
{
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let scope_only = traffic(&proving_ground, &["scope.test"], &[]);
    let every_host = traffic(&proving_ground, &["*"], &[]);
    let refused_url = target_url(&proving_ground);
    let runtime = goodfetch_test_support::runtime();

    let (host, subdomain, refused, refused_connections, allowed_elsewhere) =
        runtime.block_on(async {
            let host = scope_only
                .get(&site_url(&proving_ground, "scope.test", "/"), Waiter::Call)
                .await
                .map(|response| response.status());
            let subdomain = scope_only
                .get(
                    &site_url(&proving_ground, "www.scope.test", "/news"),
                    Waiter::Call,
                )
                .await
                .map(|response| response.status());
            let refused = scope_only.get(&refused_url, Waiter::Call).await;
            let refused_connections = target_connections(&proving_ground.observations().unwrap());
            let allowed_elsewhere = every_host
                .get(&refused_url, Waiter::Call)
                .await
                .map(|response| response.status());
            (
                host,
                subdomain,
                refused,
                refused_connections,
                allowed_elsewhere,
            )
        });

    assert_eq!(host.unwrap(), 200);
    assert_eq!(subdomain.unwrap(), 200);
    let Err(FetchError::Refused(refused)) = refused else {
        panic!("the fetch to another host is refused: {refused:?}");
    };
    assert_eq!(refused.error().code(), "egress_refused");
    assert_eq!(
        refused.details(),
        EgressRefusedDetails {
            url: refused_url.clone(),
            rule: EgressRule::DomainPolicy,
            address: None,
            setting: Some("egress.allowed_domains".to_owned()),
        }
    );
    assert_eq!(refused_connections, 0);
    // The positive control: the same URL, allowed, reaches the target, and
    // the record shows that connection.
    assert_eq!(allowed_elsewhere.unwrap(), 200);
    let observations = proving_ground.observations().unwrap();
    assert_eq!(target_connections(&observations), 1);
    assert_eq!(
        requests(&observations),
        [
            json!(["scope.test", "/"]),
            json!(["www.scope.test", "/news"])
        ]
    );
}

/// The Run trace's request log, as lines of JSON.
#[expect(
    clippy::disallowed_methods,
    reason = "the test reads the Run trace the engine wrote, outside any async worker"
)]
pub(crate) fn request_log_lines(run_folder: &std::path::Path) -> Vec<Value> {
    match std::fs::read_to_string(run_folder.join("trace/requests.jsonl")) {
        Ok(text) => text
            .lines()
            .map(|line| serde_json::from_str(line).unwrap())
            .collect(),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(error) => panic!("the request log cannot be read: {error}"),
    }
}

/// Covers: story:4-3, event:page.request_failed, error:egress_refused
#[test]
fn a_refused_request_no_call_waits_on_is_a_request_failed_event_in_the_run_trace() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let scope_only = traffic(&proving_ground, &["scope.test"], &[]);
    let refused_url = target_url(&proving_ground);
    let run_folder = env.folder().join("run");
    let log = RequestLog::new(
        TaskOwner::root(BlockingPolicy::Inline),
        &run_folder,
        Masks::new().for_run(Redaction::On),
        goodfetch_counted_work::CountedWork::default(),
    );
    let runtime = goodfetch_test_support::runtime();

    let (allowed, refused) = runtime.block_on(async {
        let allowed = scope_only
            .get(
                &site_url(&proving_ground, "scope.test", "/"),
                Waiter::Nobody(&log),
            )
            .await
            .map(|response| response.status());
        let refused = scope_only.get(&refused_url, Waiter::Nobody(&log)).await;
        (allowed, refused)
    });

    assert_eq!(allowed.unwrap(), 200);
    assert!(matches!(refused, Err(FetchError::Refused(_))));
    assert_eq!(
        request_log_lines(&run_folder),
        [json!({
            "event": "page.request_failed",
            "data": {
                "url": refused_url,
                "method": "GET",
                "failure": {
                    "code": "egress_refused",
                    "retryable": false,
                    "url": refused_url,
                    "rule": "domain_policy",
                    "setting": "egress.allowed_domains",
                },
            },
        })]
    );
    assert_eq!(
        target_connections(&proving_ground.observations().unwrap()),
        0
    );
}

/// `url` with `port` written after its host, keeping the host's spelling,
/// so each case reaches the watched target, which serves any path.
pub(crate) fn at_port(url: &str, port: u16) -> String {
    let Some((scheme, rest)) = url.split_once("://") else {
        return url.to_owned();
    };
    let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
    format!("{scheme}://{}:{port}{}", &rest[..end], &rest[end..])
}

/// The table's form of the rule a refusal names.
pub(crate) fn table_rule(rule: EgressRule) -> goodfetch_test_support::EgressRule {
    match rule {
        EgressRule::DomainPolicy => goodfetch_test_support::EgressRule::DomainPolicy,
        EgressRule::PrivateAddress => goodfetch_test_support::EgressRule::PrivateAddress,
        EgressRule::Scheme => goodfetch_test_support::EgressRule::Scheme,
    }
}

/// The SSRF case table's thin runner on the engine's HTTP client: each case
/// fetched under its domain policy, its names at the Proving ground's
/// watched target, the connection record the oracle.
fn run_case(proving_ground: &ProvingGround, case: &SsrfCase) -> Outcome {
    let target: SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    let domains = case.domains.clone().unwrap_or(CaseDomains {
        allowed: vec!["*".to_owned()],
        blocked: Vec::new(),
    });
    let resolve = case.lookups.iter().fold(Resolve::new(), |resolve, lookup| {
        let addresses = lookup
            .answers
            .iter()
            .map(|answer| match answer {
                Answer::Public => target.ip(),
                Answer::Address(address) => *address,
            })
            .collect();
        resolve.with(lookup.name.clone(), addresses)
    });
    let traffic = PageTraffic {
        egress: Policy::new(&domains.allowed, &domains.blocked)
            .unwrap()
            .with_private_ranges(&[target.ip().to_string()])
            .unwrap(),
        resolve,
        local: None,
        proxy: None,
        trust: Trust::default_roots(),
        exceptions: CertificateExceptions::default(),
        client_certificates: ClientCertificates::default(),
        cookies: Vec::new(),
        clock: Clock::system(),
        delays: Delays::default(),
    };
    let before = target_connections(&proving_ground.observations().unwrap());
    let fetched = goodfetch_test_support::runtime()
        .block_on(traffic.get(&at_port(&case.url, target.port()), Waiter::Call));
    let after = target_connections(&proving_ground.observations().unwrap());
    Outcome {
        refusal: match fetched {
            Err(FetchError::Refused(refused)) => Some(Refusal::EgressRefused {
                rule: table_rule(refused.rule),
            }),
            _ => None,
        },
        connections: vec![target.ip(); after - before],
    }
}

/// Covers: story:4-2, story:4-89
#[test]
fn every_domain_policy_case_of_the_ssrf_table_is_refused_before_any_connection() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let runner = Runner::committed(
        "http_client",
        file!(),
        "every_domain_policy_case_of_the_ssrf_table_is_refused_before_any_connection",
    )
    .unwrap();
    let target: SocketAddr = proving_ground
        .description()
        .fixtures
        .target
        .parse()
        .unwrap();
    // The positive control: a case's URL under a policy that allows it
    // reaches the target, and the record shows that connection.
    let control = SsrfTable::parse(
        r#"
        [[cases]]
        name = "allowed"
        family = "domain_policy"
        url = "http://elsewhere.test/offsite"
        domains = { allowed = ["elsewhere.test"] }
        lookups = [{ name = "elsewhere.test", answers = ["public"] }]
        target = "public"
        refusal = { code = "egress_refused", rule = "domain_policy" }
        "#,
    )
    .unwrap();

    let result = runner.run(&control.cases, target.ip(), |case| {
        run_case(&proving_ground, case)
    });

    assert!(
        runner.cases().len() >= 15,
        "only {} cases",
        runner.cases().len()
    );
    assert_eq!(result, Ok(()), "{}", result.clone().unwrap_err());
}
