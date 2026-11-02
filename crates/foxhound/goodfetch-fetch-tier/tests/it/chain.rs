//! A crawl request's redirect chain (ADR 0118; pitfalls U12 and U13), the
//! fetch driver following the Proving ground's `fetch-edge.test` redirects
//! and refresh stubs over reqwest: Fetch's statuses, method, body and
//! `Authorization` rules as the site sees them; a loop costing the site
//! exactly `fetch.redirect.max` + 1 requests; a 3xx's `Retry-After` delaying
//! the next hop or failing the request past the crawl's bound; refresh
//! stubs within `fetch.refresh.max_delay` followed as one more hop, others
//! not; and every hop after the first through the gate.
#![expect(
    clippy::disallowed_types,
    reason = "a test gate's record, written and read in one step each, never across an `.await`"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use bytes::Bytes;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::deadline::Deadline;
use goodfetch_engine::egress::Refused;
use goodfetch_engine::response_limits::Limits;
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_fetch_tier::{
    FetchBounds, Fetched, Following, Gating, Hop, HopGate, Skip, Stop, Transport, fetch, follow,
};
use goodfetch_page::page_value::Permit;
use goodfetch_protocol::egress::EgressRule;
use goodfetch_protocol::foxhound::outcomes::SkipReason;
use goodfetch_protocol::page::NAVIGATION_FAILED;
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_FIRST_BYTE, TIMEOUT_NAVIGATION};
use goodfetch_test_support::{ProvingGround, ReceivedRequests, TestEnv};
use rstest::rstest;
use serde_json::Value;
use url::Url;

use crate::{Stack, reqwest, running_clock_runtime, transport};

const SEED: u64 = 762;
const HOST: &str = "fetch-edge.test";
const HTTP1_HOST: &str = "h1.fetch-edge.test";

fn deadlines() -> FetchBounds {
    FetchBounds {
        first_byte: Deadline::given(&TIMEOUT_FIRST_BYTE, 30_000, SettingsLayer::Run),
        navigation: Deadline::given(&TIMEOUT_NAVIGATION, 30_000, SettingsLayer::Run),
        limits: Limits::default(),
    }
}

/// The defaults: 19 redirects, refreshes up to 100 s, no status handled,
/// a `Retry-After` waited up to 15 minutes.
fn defaults() -> Following {
    Following {
        max: 19,
        refresh: true,
        refresh_max_delay: Some(Duration::from_secs(100)),
        handled: BTreeSet::new(),
        retry_after_max: Some(Duration::from_mins(15)),
    }
}

/// A gate that admits every hop, or skips those whose path is `skipped`,
/// and records what it admitted, the permits it gave back and the waits.
#[derive(Default)]
struct Recording {
    skipped: Option<&'static str>,
    /// Whether the hop a chain ends on is a duplicate: another request's
    /// chain reached its key first.
    duplicate: bool,
    admitted: Mutex<Vec<String>>,
    released: Arc<Mutex<usize>>,
    waits: Mutex<Vec<(String, Duration)>>,
    reached: Mutex<Vec<String>>,
    /// Each hop as it was sent, with how many hops were admitted then and
    /// whether it was the chain's lead.
    sent: Mutex<Vec<(String, usize, bool)>>,
    /// Each hop as its answer arrived, with how long after its send.
    answered: Mutex<Vec<(String, Duration)>>,
}

impl HopGate for Recording {
    fn admit<'a>(&'a self, hop: &'a Hop) -> Gating<'a, Result<Permit, Stop>> {
        Box::pin(async move {
            if self.skipped == Some(hop.url.path()) {
                return Err(Stop::Skipped(Skip {
                    reason: SkipReason::RobotsTxt,
                    message: "disallowed".to_owned(),
                }));
            }
            self.admitted
                .lock()
                .unwrap()
                .push(hop.url.path().to_owned());
            let released = Arc::clone(&self.released);
            Ok(Permit::new(move || *released.lock().unwrap() += 1))
        })
    }

    fn wait_until<'a>(&'a self, hop: &'a Hop, until: SystemTime) -> Gating<'a, ()> {
        Box::pin(async move {
            let wait = until
                .duration_since(Clock::system().wall())
                .unwrap_or_default();
            self.waits
                .lock()
                .unwrap()
                .push((hop.url.path().to_owned(), wait));
            tokio::time::sleep(wait).await;
        })
    }

    fn reached<'a>(&'a self, hop: &'a Hop) -> Gating<'a, Result<(), Stop>> {
        Box::pin(async move {
            self.reached.lock().unwrap().push(hop.url.path().to_owned());
            if self.duplicate {
                return Err(Stop::Skipped(Skip {
                    reason: SkipReason::Duplicate,
                    message: "reached first by another request".to_owned(),
                }));
            }
            Ok(())
        })
    }

    fn sending<'a>(&'a self, hop: &'a Hop, lead: bool) -> Gating<'a, ()> {
        Box::pin(async move {
            let admitted = self.admitted.lock().unwrap().len();
            self.sent
                .lock()
                .unwrap()
                .push((hop.url.path().to_owned(), admitted, lead));
        })
    }

    fn answered<'a>(&'a self, hop: &'a Hop, latency: Duration) -> Gating<'a, ()> {
        Box::pin(async move {
            self.answered
                .lock()
                .unwrap()
                .push((hop.url.path().to_owned(), latency));
        })
    }
}

/// The chain from `first` through `transport` under `following`.
fn followed(
    transport: &dyn Transport,
    first: Hop,
    following: &Following,
    gate: &Recording,
) -> Result<Fetched, Stop> {
    running_clock_runtime().block_on(follow(
        first,
        following,
        gate,
        (
            &Clock::system(),
            &TaskOwner::root(BlockingPolicy::Threads),
            &goodfetch_counted_work::CountedWork::default(),
        ),
        |hop| async move { fetch(transport, &hop, &deadlines(), &Clock::system()).await },
    ))
}

fn hop(proving_ground: &ProvingGround, path: &str) -> Hop {
    Hop::get(Url::parse(&proving_ground.url(HOST, path)).unwrap())
}

/// The requests the Proving ground received on `host` for `path`.
fn requests_for(proving_ground: &ProvingGround, host: &str, path: &str) -> Vec<Value> {
    proving_ground
        .received_requests()
        .unwrap()
        .of_the_case()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|seen| seen.split(':').next() == Some(host))
                && request["target"] == path
        })
        .cloned()
        .collect()
}

/// The value of a request's header `name`, if it has one.
fn header<'a>(request: &'a Value, name: &str) -> Option<&'a str> {
    request["headers"].as_array()?.iter().find_map(|pair| {
        (pair[0].as_str()?.eq_ignore_ascii_case(name))
            .then(|| pair[1].as_str())
            .flatten()
    })
}

fn failed(stop: Result<Fetched, Stop>) -> goodfetch_engine::CallError {
    match stop {
        Err(Stop::Failed(error)) => error,
        other => panic!("not failed: {:?}", other.map(|fetched| fetched.url)),
    }
}

/// Covers: story:5-124, setting:fetch.redirect.max, error:too_many_redirects
#[rstest]
#[case::one(1)]
#[case::five(5)]
#[case::nineteen(19)]
fn a_redirect_loop_costs_the_site_exactly_its_count_and_one_more_requests(#[case] max: u32) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording::default();
    let following = Following { max, ..defaults() };

    let error = failed(followed(
        &transport,
        hop(&proving_ground, "/redirect/loop"),
        &following,
        &gate,
    ));

    assert_eq!(error.code(), "too_many_redirects", "{error:?}");
    assert!(!error.retryable());
    let details = error.details().unwrap();
    assert_eq!(details["hops"].as_array().unwrap().len(), max as usize);
    assert_eq!(
        details["limit"],
        serde_json::json!({ "kind": "setting", "name": "fetch.redirect.max", "count": max })
    );
    assert_eq!(
        requests_for(&proving_ground, HOST, "/redirect/loop").len(),
        max as usize + 1
    );
    assert_eq!(gate.admitted.lock().unwrap().len(), max as usize);
    assert_eq!(*gate.released.lock().unwrap(), max as usize);
}

/// Covers: story:5-124, setting:fetch.redirect.max
#[test]
fn a_chain_of_nineteen_redirects_reaches_its_document_each_hop_through_the_gate() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording::default();

    let fetched = followed(
        &transport,
        hop(&proving_ground, "/redirect/hops/19"),
        &defaults(),
        &gate,
    )
    .unwrap();

    assert_eq!(
        (fetched.status, fetched.url.path()),
        (200, "/redirect/hops/0")
    );
    let admitted: Vec<String> = (0..19)
        .rev()
        .map(|n| format!("/redirect/hops/{n}"))
        .collect();
    assert_eq!(*gate.admitted.lock().unwrap(), admitted);
    assert_eq!(*gate.reached.lock().unwrap(), ["/redirect/hops/0"]);
}

/// Each hop is told to the gate as it is sent, the first included, as
/// the chain's lead, and each after it once admitted, so its domain's
/// delay counts from its send, never from an admission whatever ran after
/// it held back (ADR 0086), and only the request's own send takes a token
/// of a rated run's sends.
///
/// Covers: story:5-124
#[test]
fn each_hop_is_told_to_the_gate_as_it_is_sent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording::default();

    followed(
        &transport,
        hop(&proving_ground, "/redirect/hops/2"),
        &defaults(),
        &gate,
    )
    .unwrap();

    assert_eq!(
        *gate.sent.lock().unwrap(),
        [
            ("/redirect/hops/2".to_owned(), 0, true),
            ("/redirect/hops/1".to_owned(), 1, false),
            ("/redirect/hops/0".to_owned(), 2, false),
        ]
    );
}

/// Each hop's latency, from its send to its answer's arrival, is told to
/// the gate, for the domain's adaptive delay (spec #582's Settings): the
/// `pacing` host held its answer as long as its `Server-Timing` names, and
/// the latency is no shorter.
///
/// Covers: story:6-247
#[test]
fn each_hops_latency_is_told_to_the_gate_once_its_answer_arrived() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    proving_ground.set_time_step(5).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording::default();
    let first = Hop::get(Url::parse(&proving_ground.url("pacing.test", "/")).unwrap());

    let fetched = followed(&transport, first, &defaults(), &gate).unwrap();

    let held = Duration::from_millis(
        fetched
            .headers
            .iter()
            .find(|(name, _)| name == "server-timing")
            .and_then(|(_, value)| value.strip_prefix("hold;dur=")?.parse().ok())
            .expect("a Server-Timing hold"),
    );
    let answered = gate.answered.lock().unwrap().clone();
    assert!(
        matches!(answered.as_slice(), [(path, latency)] if path == "/" && *latency >= held),
        "{answered:?} after a hold of {held:?}"
    );
}

/// Covers: story:5-124
#[rstest]
#[case::a_301_turns_a_post_into_a_get("/redirect/301", "POST", "GET", false)]
#[case::a_302_turns_a_post_into_a_get("/redirect/302", "POST", "GET", false)]
#[case::a_303_turns_a_put_into_a_get("/redirect/303", "PUT", "GET", false)]
#[case::a_307_keeps_a_post_and_its_body("/redirect/307", "POST", "POST", true)]
#[case::a_308_keeps_a_put_and_its_body("/redirect/308", "PUT", "PUT", true)]
fn the_site_gets_each_redirects_method_and_body_as_fetch_sends_them(
    #[case] path: &str,
    #[case] method: &str,
    #[case] arrives_as: &str,
    #[case] body_kept: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let mut first = hop(&proving_ground, path);
    first.method = http::Method::from_bytes(method.as_bytes()).unwrap();
    first.body = Some(Bytes::from_static(b"q=1"));
    first.headers.push((
        "content-type".to_owned(),
        "application/x-www-form-urlencoded".to_owned(),
    ));

    let fetched = followed(&transport, first, &defaults(), &Recording::default()).unwrap();

    assert_eq!(fetched.url.path(), "/target");
    let target = requests_for(&proving_ground, HOST, "/target");
    assert_eq!(target.len(), 1, "{target:?}");
    assert_eq!(target[0]["method"], arrives_as);
    assert_eq!(
        (
            header(&target[0], "content-length").is_some(),
            header(&target[0], "content-type").is_some()
        ),
        (body_kept, body_kept),
        "{:?}",
        target[0]
    );
}

/// Covers: story:5-124
#[rstest]
#[case::to_another_origin("/redirect/cross-origin", HTTP1_HOST, false)]
#[case::on_its_own_origin("/redirect/302", HOST, true)]
fn authorization_is_absent_once_the_chain_reaches_another_origin(
    #[case] path: &str,
    #[case] target_host: &str,
    #[case] kept: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let port = proving_ground.address().port();
    let mut first = Hop::get(Url::parse(&format!("https://{HOST}:{port}{path}")).unwrap());
    first
        .headers
        .push(("authorization".to_owned(), "Bearer test-token".to_owned()));

    let fetched = followed(&transport, first, &defaults(), &Recording::default()).unwrap();

    assert_eq!(fetched.url.path(), "/target");
    let target = requests_for(&proving_ground, target_host, "/target");
    assert_eq!(target.len(), 1, "{target:?}");
    assert_eq!(header(&target[0], "authorization").is_some(), kept);
}

/// Covers: story:5-124, error:response_not_ok
#[rstest]
#[case::a_300("/redirect/300")]
#[case::a_redirect_with_no_location("/redirect/no-location")]
#[case::a_redirect_to_ftp("/redirect/ftp")]
fn any_other_3xx_fails_at_once_with_response_not_ok(#[case] path: &str) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording::default();

    let error = failed(followed(
        &transport,
        hop(&proving_ground, path),
        &defaults(),
        &gate,
    ));

    assert_eq!(error.code(), "response_not_ok", "{error:?}");
    assert!(!error.retryable());
    let details = error.details().unwrap();
    assert!(
        details["status"]
            .as_u64()
            .is_some_and(|status| (300..400).contains(&status))
    );
    assert!(
        details["fix"]
            .as_str()
            .unwrap()
            .contains("crawl.handle_statuses")
    );
    assert_eq!(requests_for(&proving_ground, HOST, "/target").len(), 0);
    assert!(gate.admitted.lock().unwrap().is_empty());
}

/// Covers: story:5-124, setting:crawl.handle_statuses
#[test]
fn a_redirect_status_the_crawl_handles_is_its_document_unfollowed() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let following = Following {
        handled: BTreeSet::from([301]),
        ..defaults()
    };

    let fetched = followed(
        &transport,
        hop(&proving_ground, "/redirect/301"),
        &following,
        &Recording::default(),
    )
    .unwrap();

    assert_eq!((fetched.status, fetched.url.path()), (301, "/redirect/301"));
    assert_eq!(requests_for(&proving_ground, HOST, "/target").len(), 0);
}

/// Covers: story:5-124, setting:politeness.retry_after.max
#[test]
fn a_redirects_retry_after_delays_its_next_hop() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording::default();

    let fetched = followed(
        &transport,
        hop(&proving_ground, "/retry-after/302/seconds"),
        &defaults(),
        &gate,
    )
    .unwrap();

    assert_eq!(fetched.url.path(), "/target");
    let waits = gate.waits.lock().unwrap().clone();
    assert_eq!(waits.len(), 1, "{waits:?}");
    assert_eq!(waits[0].0, "/target");
    assert!(
        waits[0].1 > Duration::from_secs(1) && waits[0].1 <= Duration::from_secs(2),
        "{waits:?}"
    );
    let first = &requests_for(&proving_ground, HOST, "/retry-after/302/seconds")[0];
    let gap = first["next_request"]["after_microseconds"]
        .as_u64()
        .unwrap();
    assert!(gap >= 1_500_000, "the next hop came {gap} µs after the 302");
}

/// Covers: story:5-124, setting:politeness.retry_after.max, error:response_unavailable
#[test]
fn a_redirects_retry_after_past_the_crawls_bound_fails_with_its_status() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording::default();
    let following = Following {
        retry_after_max: Some(Duration::from_secs(3)),
        ..defaults()
    };

    let error = failed(followed(
        &transport,
        hop(&proving_ground, "/retry-after/302/seconds/long"),
        &following,
        &gate,
    ));

    assert_eq!(error.code(), "response_unavailable", "{error:?}");
    let details = error.details().unwrap();
    assert_eq!(details["status"], 302);
    assert!(details["retry_after"].is_string(), "{details:?}");
    assert_eq!(requests_for(&proving_ground, HOST, "/target").len(), 0);
    assert!(gate.waits.lock().unwrap().is_empty());
}

/// Covers: story:5-125, setting:fetch.refresh.max_delay
#[rstest]
#[case::a_refresh_header("/refresh/header", Some(100), true)]
#[case::an_instant_meta("/refresh/meta/0", Some(100), true)]
#[case::a_five_second_meta_at_the_default("/refresh/meta/5", Some(100), true)]
#[case::a_200_second_meta_at_the_default("/refresh/meta/200", Some(100), false)]
#[case::a_200_second_meta_at_null("/refresh/meta/200", None, true)]
#[case::a_five_second_meta_at_zero("/refresh/meta/5", Some(0), false)]
#[case::an_instant_meta_at_zero("/refresh/meta/0", Some(0), true)]
#[case::a_meta_inside_noscript("/refresh/noscript", None, false)]
#[case::a_refresh_with_no_url("/refresh/no-url", None, false)]
#[case::a_refresh_to_itself("/refresh/self", None, false)]
#[case::a_script_redirect("/refresh/script", None, false)]
fn a_refresh_stub_within_the_crawls_delay_is_one_more_hop_through_the_gate(
    #[case] path: &str,
    #[case] max_delay: Option<u64>,
    #[case] refreshed: bool,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording::default();
    let following = Following {
        refresh_max_delay: max_delay.map(Duration::from_secs),
        ..defaults()
    };

    let fetched = followed(&transport, hop(&proving_ground, path), &following, &gate).unwrap();

    let (ended_on, admitted) = if refreshed {
        ("/target", vec!["/target".to_owned()])
    } else {
        (path, Vec::new())
    };
    assert_eq!(
        (fetched.status, fetched.url.path()),
        (200, ended_on),
        "{:?}",
        String::from_utf8_lossy(&fetched.body)
    );
    assert_eq!(*gate.admitted.lock().unwrap(), admitted);
    assert_eq!(requests_for(&proving_ground, HOST, path).len(), 1);
}

/// A hop to a private address, the Proving ground's loopback address
/// written dotted or in hex, is refused by the address check at that hop
/// on either transport and ends its request `skipped` with
/// `domain_policy`, never connecting (ADRs 0017 and 0118): the target is
/// never requested and every connection the Proving ground accepted
/// carried a request; a hop on the request's own host, through its
/// mapped name, reaches its target (the positive control).
///
/// Covers: story:4-103
#[rstest]
fn a_hop_to_a_private_address_ends_skipped_with_domain_policy_and_never_connects(
    #[values(Stack::Reqwest, Stack::Wreq)] stack: Stack,
    #[values("/redirect/address", "/redirect/address/hex")] path: &str,
) {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = transport(stack, &proving_ground, None);
    let gate = Recording::default();

    let stopped = followed(
        transport.as_ref(),
        hop(&proving_ground, path),
        &defaults(),
        &gate,
    );
    let control = followed(
        transport.as_ref(),
        hop(&proving_ground, "/redirect/302"),
        &defaults(),
        &gate,
    );

    let observations = proving_ground.observations().unwrap();
    let requests = ReceivedRequests::of(&observations);
    let addressed = requests
        .of_the_case()
        .filter(|request| request["target"] == "/address/target")
        .count();
    // The observations' own read is the one connection with no request.
    let connections = observations["connections"].as_array().unwrap().len() - 1;
    assert!(
        matches!(
            &stopped,
            Err(Stop::Skipped(Skip {
                reason: SkipReason::DomainPolicy,
                ..
            }))
        ),
        "{:?}",
        stopped.map(|fetched| fetched.url)
    );
    assert_eq!(
        (
            control.ok().map(|fetched| fetched.url.path().to_owned()),
            addressed,
            connections <= requests.every().count(),
        ),
        (Some("/target".to_owned()), 0, true)
    );
}

/// Covers: story:5-124
#[test]
fn a_hop_the_gate_skips_is_never_sent() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording {
        skipped: Some("/private/target"),
        ..Recording::default()
    };

    let stopped = followed(
        &transport,
        hop(&proving_ground, "/redirect/robots"),
        &defaults(),
        &gate,
    );

    assert!(
        matches!(
            &stopped,
            Err(Stop::Skipped(Skip {
                reason: SkipReason::RobotsTxt,
                ..
            }))
        ),
        "{:?}",
        stopped.map(|fetched| fetched.url)
    );
    assert_eq!(
        requests_for(&proving_ground, HOST, "/private/target").len(),
        0
    );
}

/// A chain whose document another request's chain reached first, its key
/// recorded meanwhile, ends `skipped` with `duplicate` once that document
/// arrived, never answered as its own (ADR 0120); a chain with no redirect
/// records nothing.
///
/// Covers: story:6-220
#[test]
fn a_chain_ending_on_a_key_another_request_reached_first_ends_skipped_duplicate() {
    let env = TestEnv::new();
    let proving_ground = ProvingGround::start(&env, SEED).unwrap();
    let transport = reqwest(&proving_ground, None);
    let gate = Recording {
        duplicate: true,
        ..Recording::default()
    };

    let redirected = followed(
        &transport,
        hop(&proving_ground, "/redirect/hops/1"),
        &defaults(),
        &gate,
    );
    let direct = followed(
        &transport,
        hop(&proving_ground, "/redirect/hops/0"),
        &defaults(),
        &gate,
    );

    assert!(
        matches!(
            &redirected,
            Err(Stop::Skipped(Skip {
                reason: SkipReason::Duplicate,
                ..
            }))
        ),
        "{:?}",
        redirected.map(|fetched| fetched.url)
    );
    assert_eq!(
        (
            direct.map(|fetched| fetched.status).ok(),
            gate.reached.lock().unwrap().clone()
        ),
        (Some(200), vec!["/redirect/hops/0".to_owned()])
    );
}

/// A chain whose first hop the transport refuses at its connection, its
/// name having turned private since the address check let it through,
/// ends `skipped` with `domain_policy`, the policy's refusal, as a later
/// hop's does; another failure of the first hop stays a failure.
///
/// Covers: story:4-98
#[test]
fn a_first_hop_refused_at_its_connection_ends_skipped_with_domain_policy() {
    let refused = Refused {
        url: "http://rebound.test/".to_owned(),
        rule: EgressRule::PrivateAddress,
        address: Some("10.0.0.1".parse().unwrap()),
        setting: Some("egress.private_ranges"),
    };
    let follow_failing = |error: CallError| {
        running_clock_runtime().block_on(follow(
            Hop::get(Url::parse("http://rebound.test/").unwrap()),
            &defaults(),
            &Recording::default(),
            (
                &Clock::system(),
                &TaskOwner::root(BlockingPolicy::Inline),
                &goodfetch_counted_work::CountedWork::default(),
            ),
            |_| {
                let error = error.clone();
                async move { Err::<Fetched, _>(error) }
            },
        ))
    };

    let skipped = follow_failing(refused.error());
    let failed = follow_failing(CallError::new(
        &NAVIGATION_FAILED,
        "unreachable".to_owned(),
        &serde_json::json!({ "url": "http://rebound.test/" }),
    ));

    assert!(
        matches!(
            &skipped,
            Err(Stop::Skipped(Skip {
                reason: SkipReason::DomainPolicy,
                ..
            }))
        ),
        "{:?}",
        skipped.map(|fetched| fetched.url)
    );
    assert!(
        matches!(&failed, Err(Stop::Failed(error)) if error.code() == NAVIGATION_FAILED.name),
        "{:?}",
        failed.map(|fetched| fetched.url)
    );
}
