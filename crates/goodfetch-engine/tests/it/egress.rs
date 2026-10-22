//! The domain policy on the engine's own HTTP client, against the Proving
//! ground (ADRs 0014, 0017 and 0100): a request of the run the policy
//! allows reaches its host, and one it refuses ends with `egress_refused`
//! before any connection, which the Proving ground's connection record
//! shows; a refused request no call waits on is a `page.request_failed`
//! event in the Run trace. The `resolve` setting maps every host name to
//! the Proving ground's loopback address, whose certificate authority the
//! client trusts: no test reaches the internet.

use std::net::{IpAddr, SocketAddr};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::happy_eyeballs::Delays;
use goodfetch_engine::http_client::{FetchError, PageTraffic, Resolve, Waiter};
use goodfetch_engine::run_trace::RequestLog;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::egress::{EgressRefusedDetails, EgressRule};
use goodfetch_test_support::{ProvingGround, TestEnv};
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
        egress: Policy::new(allowed, blocked).unwrap(),
        resolve,
        trusted_certificates: vec![CertificateDer::from_pem_file(&description.ca).unwrap()],
        clock: Clock::system(),
        delays: Delays::BUILT_IN,
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
    observations["requests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|request| json!([request["host"], request["target"]]))
        .collect()
}

/// How many connections reached the watched target.
fn target_connections(observations: &Value) -> usize {
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
fn request_log_lines(run_folder: &std::path::Path) -> Vec<Value> {
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
    let log = RequestLog::new(TaskOwner::root(BlockingPolicy::Inline), &run_folder);
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
