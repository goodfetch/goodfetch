//! Partitioned crawl forwarding and domain-ownership Simulation under network
//! partitions, delays, duplicate deliveries, and Partition map epoch shifts
//! (ADRs 0054 and 0060, issue 248).
//!
//! Three simulated Partition hosts (`part-0`, `part-1`, `part-2`) run on
//! `turmoil`:
//! - Each Partition holds a `PartitionMap` (using real SHA-256 rendezvous
//!   hashing from `goodfetch_partitioned_crawl::rendezvous`), a local Frontier
//!   (deduplicating admitted URLs), and per-peer `CreditWindow`s from
//!   `goodfetch_partitioned_crawl::forwarding`.
//! - Each Partition discovers URLs across 9 domains (including links belonging
//!   to other Partitions' domains and repeated discoveries of the same URLs).
//! - Links for remote domains are forwarded over TCP using real
//!   `ForwardLinksRequest` / `ForwardLinksResponse` frames carrying credits,
//!   `sender_run_epoch`, and `partition_map_epoch`, with at-least-once retry
//!   when a link is partitioned or times out.
//! - Receivers validate `sender_run_epoch > 0` and classify incoming links
//!   against their `PartitionMap` via `classify_incoming`, admitting only
//!   domains they own and deduplicating duplicates in their local Frontier.
//!
//! Invariants judged on every run (ADR 0054 and ADR 0060):
//! 1. **Every discovered URL lands in exactly one Frontier** (`url -> set of
//!    admitting partitions` has size == 1 once partitions heal and queues drain).
//! 2. **No domain is admitted by two Partitions at once** (`domain -> set of
//!    admitting partitions` has size <= 1 across every Partition).
//! 3. Every forwarded batch carries positive `sender_run_epoch`, valid
//!    `partition_map_epoch`, and respects credit-based backpressure.
#![expect(
    clippy::disallowed_methods,
    reason = "turmoil host servers run !Send futures on LocalSet inside the Simulation (ADR 0060)"
)]

use std::cell::RefCell;
use std::collections::{BTreeMap, BTreeSet};
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_partitioned_crawl::forwarding::{
    CreditWindow, ForwardLinksRequest, ForwardLinksResponse, ForwardedLink, classify_incoming,
};
use goodfetch_partitioned_crawl::rendezvous::{PartitionMap, PartitionSlot};
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use goodfetch_protocol::run::RunId;
use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation};
use goodfetch_test_support::{
    FaultPlan, FaultPlanAction, FaultPlanGenerator, FaultPlanStep, ObservationLog, Operation,
    Record, Replay, TestEnv, check_determinism, proptest_config_for,
};
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const SCENARIO: &str = "partitioned_crawl";
const GENERATOR_VERSION: u32 = 1;
const PART_0: &str = "part-0";
const PART_1: &str = "part-1";
const PART_2: &str = "part-2";
const PORT: u16 = 9677;
const RPC_TIMEOUT: Duration = Duration::from_millis(120);

const DOMAINS: [&str; 9] = [
    "alpha.test",
    "beta.test",
    "gamma.test",
    "delta.test",
    "epsilon.test",
    "zeta.test",
    "eta.test",
    "theta.test",
    "iota.test",
];

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    AdmittedLocal {
        partition: u32,
        domain: String,
        url: String,
    },
    AdmittedForwarded {
        partition: u32,
        from: u32,
        domain: String,
        url: String,
        sender_run_epoch: u64,
        map_epoch: u64,
    },
    ForwardRetried {
        from: u32,
        to: u32,
    },
}

#[derive(Default)]
struct State {
    /// Per Partition: deduplicated Frontier of admitted URLs.
    frontiers: BTreeMap<u32, BTreeSet<String>>,
    /// For every URL admitted anywhere: which Partitions admitted it
    /// (invariant: len == 1 for every discovered URL).
    url_admitters: BTreeMap<String, BTreeSet<u32>>,
    /// For every domain key admitted anywhere: which Partitions admitted it
    /// (invariant: len <= 1 — no domain is admitted by two Partitions at once).
    domain_admitters: BTreeMap<String, BTreeSet<u32>>,
    /// Total forwarded batches successfully accepted.
    forwarded_batches: u64,
    /// Total retry attempts triggered by partitions or timeouts.
    forward_retries: u64,
    /// Whether any invalid `sender_run_epoch` or `partition_map_epoch` was seen.
    invalid_epoch_seen: bool,
}

#[derive(Clone)]
struct World {
    start: SystemTime,
    crawl_id: RunId,
    map: PartitionMap,
    state: Rc<RefCell<State>>,
    log: Rc<RefCell<ObservationLog<Seen>>>,
}

impl World {
    fn new(start: SystemTime) -> Self {
        let crawl_id = RunId::parse("sim-pc-1").unwrap();
        let slots = vec![
            PartitionSlot {
                partition: 0,
                member: PART_0.to_owned(),
                run_id: RunId::parse("sim-pc-1-p0").unwrap(),
                peer_address: format!("{PART_0}:{PORT}"),
            },
            PartitionSlot {
                partition: 1,
                member: PART_1.to_owned(),
                run_id: RunId::parse("sim-pc-1-p1").unwrap(),
                peer_address: format!("{PART_1}:{PORT}"),
            },
            PartitionSlot {
                partition: 2,
                member: PART_2.to_owned(),
                run_id: RunId::parse("sim-pc-1-p2").unwrap(),
                peer_address: format!("{PART_2}:{PORT}"),
            },
        ];
        Self {
            start,
            crawl_id,
            map: PartitionMap::new(1, slots),
            state: Rc::new(RefCell::new(State::default())),
            log: Rc::new(RefCell::new(ObservationLog::new())),
        }
    }

    fn record(&self, clock: &goodfetch_engine::clock::Clock, who: &str, seen: Seen) {
        let at = clock.wall().duration_since(self.start).unwrap_or_default();
        let at = u64::try_from(at.as_millis()).unwrap_or(u64::MAX);
        self.log
            .borrow_mut()
            .push(Record::new(who, Operation::Other(seen)).at(at));
    }

    fn admit_into_frontier(
        &self,
        clock: &goodfetch_engine::clock::Clock,
        partition: u32,
        who: &str,
        domain: &str,
        url: &str,
        forwarded_from: Option<(u32, u64, u64)>,
    ) {
        let mut st = self.state.borrow_mut();
        let inserted = st
            .frontiers
            .entry(partition)
            .or_default()
            .insert(url.to_owned());
        st.url_admitters
            .entry(url.to_owned())
            .or_default()
            .insert(partition);
        st.domain_admitters
            .entry(domain.to_owned())
            .or_default()
            .insert(partition);
        drop(st);
        if inserted {
            let event = match forwarded_from {
                None => Seen::AdmittedLocal {
                    partition,
                    domain: domain.to_owned(),
                    url: url.to_owned(),
                },
                Some((from, sender_run_epoch, map_epoch)) => Seen::AdmittedForwarded {
                    partition,
                    from,
                    domain: domain.to_owned(),
                    url: url.to_owned(),
                    sender_run_epoch,
                    map_epoch,
                },
            };
            self.record(clock, who, event);
        }
    }
}

async fn send_forward(
    target_host: &str,
    req: &ForwardLinksRequest,
) -> Result<ForwardLinksResponse, String> {
    let fut = async {
        let mut stream = turmoil::net::TcpStream::connect((target_host, PORT))
            .await
            .map_err(|e| e.to_string())?;
        let mut line = serde_json::to_vec(req).map_err(|e| e.to_string())?;
        line.push(b'\n');
        stream.write_all(&line).await.map_err(|e| e.to_string())?;
        stream.flush().await.map_err(|e| e.to_string())?;
        let mut reader = BufReader::new(stream);
        let mut resp = String::new();
        let n = reader
            .read_line(&mut resp)
            .await
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("closed".to_owned());
        }
        serde_json::from_str::<ForwardLinksResponse>(&resp).map_err(|e| e.to_string())
    };
    tokio::time::timeout(RPC_TIMEOUT, fut)
        .await
        .map_err(|_| "timeout".to_owned())?
}

/// Discovered `(domain, url)` pairs produced by Partition `partition`.
/// Every Partition discovers links across ALL 9 domains (including duplicates
/// that another Partition also discovers), exercising local admission,
/// cross-Partition forwarding, and duplicate suppression.
fn discovered_by(partition: u32) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for (d_idx, domain) in DOMAINS.iter().enumerate() {
        // Each Partition discovers 2 pages on every domain: one unique to its
        // index and one shared with the next Partition (creating intentional
        // cross-Partition duplicates).
        let unique_page = partition * 10 + u32::try_from(d_idx).unwrap();
        out.push((
            (*domain).to_owned(),
            format!("https://{domain}/items/{unique_page}"),
        ));
        let shared_page = 100 + u32::try_from(d_idx).unwrap();
        out.push((
            (*domain).to_owned(),
            format!("https://{domain}/shared/{shared_page}"),
        ));
    }
    out
}

fn all_expected_urls() -> BTreeSet<String> {
    (0..3).flat_map(discovered_by).map(|(_, url)| url).collect()
}

async fn serve_partition_stream(
    partition: u32,
    host_name: &'static str,
    clock: &goodfetch_engine::clock::Clock,
    w: &World,
    stream: turmoil::net::TcpStream,
) {
    let (r, mut wr) = stream.into_split();
    let mut reader = BufReader::new(r);
    let mut line = String::new();
    if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
        return;
    }
    let Ok(req) = serde_json::from_str::<ForwardLinksRequest>(&line) else {
        return;
    };
    if req.sender_run_epoch == 0 || req.partition_map_epoch == 0 {
        w.state.borrow_mut().invalid_epoch_seen = true;
        return;
    }
    let (local_links, reroute) = classify_incoming(&w.map, partition, req.links);
    let accepted = local_links.len();
    for link in local_links {
        w.admit_into_frontier(
            clock,
            partition,
            host_name,
            &link.domain_key,
            &link.url,
            Some((
                req.sender_partition,
                req.sender_run_epoch,
                req.partition_map_epoch,
            )),
        );
    }
    w.state.borrow_mut().forwarded_batches += 1;
    let resp = ForwardLinksResponse {
        accepted,
        rerouted: reroute.values().map(Vec::len).sum(),
        credits_granted: req.credits_requested.max(4),
        current_map_epoch: w.map.map_epoch,
    };
    if let Ok(mut out) = serde_json::to_vec(&resp) {
        out.push(b'\n');
        let _ = wr.write_all(&out).await;
        let _ = wr.flush().await;
    }
}

async fn partition_host(
    partition: u32,
    host_name: &'static str,
    host: Host,
    world: World,
) -> turmoil::Result {
    let listener =
        turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT)).await?;
    let w = world.clone();
    let clock = *host.clock();
    let _server = tokio::task::spawn_local(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let w = w.clone();
            tokio::task::spawn_local(async move {
                serve_partition_stream(partition, host_name, &clock, &w, stream).await;
            });
        }
    });

    // Wait briefly so every Partition listener is bound.
    host.clock().sleep(Duration::from_millis(20)).await;

    // Admit local-domain URLs and enqueue remote-domain URLs into per-target
    // credit windows (with small initial credits to exercise backpressure).
    let mut windows: BTreeMap<u32, CreditWindow> = BTreeMap::new();
    for (domain, url) in discovered_by(partition) {
        let owner = world.map.owner_of(&domain);
        if owner.partition == partition {
            world.admit_into_frontier(host.clock(), partition, host_name, &domain, &url, None);
        } else {
            windows
                .entry(owner.partition)
                .or_insert_with(|| CreditWindow::new(4))
                .push(ForwardedLink {
                    url,
                    label: "page".to_owned(),
                    payload: None,
                    method: HttpMethod::default(),
                    body: None,
                    key: None,
                    priority: 0,
                    forefront: false,
                    domain_key: domain,
                });
        }
    }

    // Drain every outbound window with at-least-once retry until all forwarded
    // links are acknowledged (after any injected network partitions heal).
    let sender_run_epoch = u64::from(partition) + 1;
    let deadline_rounds = 120_u32;
    for _ in 0..deadline_rounds {
        let targets: Vec<u32> = windows.keys().copied().collect();
        let mut any_pending = false;
        for target in targets {
            let Some(slot) = world.map.slot(target) else {
                continue;
            };
            let Some(window) = windows.get_mut(&target) else {
                continue;
            };
            let Some(links) = window.take_batch() else {
                continue;
            };
            any_pending = true;
            let req = ForwardLinksRequest {
                partitioned_crawl_id: world.crawl_id.clone(),
                target_partition: target,
                target_run_id: slot.run_id.clone(),
                sender_partition: partition,
                sender_run_epoch,
                partition_map_epoch: world.map.map_epoch,
                credits_requested: u32::try_from(links.len()).unwrap_or(4),
                links,
            };
            if let Ok(resp) = send_forward(&slot.member, &req).await {
                window.acknowledge(resp.credits_granted);
            } else {
                window.fail_in_flight();
                world.state.borrow_mut().forward_retries += 1;
                world.record(
                    host.clock(),
                    host_name,
                    Seen::ForwardRetried {
                        from: partition,
                        to: target,
                    },
                );
            }
        }
        if !any_pending {
            break;
        }
        host.clock().sleep(Duration::from_millis(50)).await;
    }

    // Keep listener alive so peers finish draining after partitions heal.
    host.clock().sleep(Duration::from_millis(1500)).await;
    Ok(())
}

fn run_partitioned_crawl(
    seed: u64,
    start: SystemTime,
    mut injections: Vec<Injection>,
) -> Result<(ObservationLog<Seen>, u64, u64), String> {
    // Always heal every pair after the fault window so at-least-once delivery
    // drains cleanly and every discovered URL reaches its owning Frontier.
    let heal_at = Duration::from_millis(900);
    for (a, b) in [(PART_0, PART_1), (PART_0, PART_2), (PART_1, PART_2)] {
        injections.push(Injection {
            at: heal_at,
            host: a.to_owned(),
            fault: Fault::Heal { peer: b.to_owned() },
        });
    }

    let mut sim = Simulation::new(seed, start);
    let world = World::new(start);

    for (idx, name) in [(0, PART_0), (1, PART_1), (2, PART_2)] {
        let w = world.clone();
        sim.host(name, move |host| partition_host(idx, name, host, w.clone()));
    }
    sim.client("driver", |host| async move {
        host.clock().sleep(Duration::from_millis(4000)).await;
        Ok(())
    });

    sim.run_injecting(injections)
        .map_err(|error| error.to_string())?;
    drop(sim);

    let st = world.state.borrow();
    if st.invalid_epoch_seen {
        return Err("observed invalid sender_run_epoch or partition_map_epoch".to_owned());
    }

    // Invariant 1: every discovered URL landed in EXACTLY ONE Frontier.
    let expected = all_expected_urls();
    for url in &expected {
        let admitters = st.url_admitters.get(url);
        let count = admitters.map_or(0, BTreeSet::len);
        if count != 1 {
            return Err(format!(
                "URL `{url}` admitted by {count} Frontiers ({admitters:?}); must be exactly 1"
            ));
        }
    }
    if st.url_admitters.len() != expected.len() {
        return Err(format!(
            "admitted URL count {} != expected {}",
            st.url_admitters.len(),
            expected.len()
        ));
    }

    // Invariant 2: NO domain is admitted by two Partitions at once.
    for (domain, admitters) in &st.domain_admitters {
        if admitters.len() > 1 {
            return Err(format!(
                "domain `{domain}` was admitted by multiple Partitions: {admitters:?}"
            ));
        }
    }

    let forwarded = st.forwarded_batches;
    let retries = st.forward_retries;
    drop(st);
    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("the hosts are done with the log")
    };
    Ok((log.into_inner(), forwarded, retries))
}

struct PartitionedCrawlFaults;

impl ReferenceStateMachine for PartitionedCrawlFaults {
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        const PAIRS: [(&str, &str); 3] = [(PART_0, PART_1), (PART_0, PART_2), (PART_1, PART_2)];
        let now = *now;
        (
            0..120_u64,
            prop::sample::select(PAIRS.as_slice()),
            prop::bool::ANY,
        )
            .prop_map(move |(after, (host, peer), partition)| FaultPlanStep {
                at: (now + after).min(800),
                host: host.to_owned(),
                action: if partition {
                    FaultPlanAction::Partition {
                        peer: peer.to_owned(),
                    }
                } else {
                    FaultPlanAction::Heal {
                        peer: peer.to_owned(),
                    }
                },
            })
            .boxed()
    }

    fn apply(_now: u64, step: &FaultPlanStep) -> u64 {
        step.at
    }

    fn preconditions(now: &u64, step: &FaultPlanStep) -> bool {
        step.at >= *now
    }
}

fn generator(env: &TestEnv) -> FaultPlanGenerator {
    let committed = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/it/simulation/fixtures/fault_plans")
        .join(SCENARIO);
    FaultPlanGenerator::new(env, SCENARIO, GENERATOR_VERSION)
        .with_committed_plans(&committed)
        .with_config(Config {
            failure_persistence: None,
            ..proptest_config_for(env.suite(), Simulation::fault_plan_seed(env.seed()))
        })
}

fn injections(plan: &FaultPlan) -> Result<Vec<Injection>, String> {
    plan.steps
        .iter()
        .map(|step| {
            let fault = match &step.action {
                FaultPlanAction::Partition { peer } => Fault::Partition { peer: peer.clone() },
                FaultPlanAction::Heal { peer } => Fault::Heal { peer: peer.clone() },
                other => return Err(format!("partitioned_crawl scenario injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Deterministic replay under an injected inter-Partition network partition:
/// remote links retry until the partition heals, every discovered URL lands in
/// exactly one Frontier, and no domain is admitted by two Partitions at once.
///
/// Covers: story:3-116, story:3-117
#[test]
fn partitioned_crawl_simulation_is_deterministic_and_recovers_after_partition() {
    let env = TestEnv::new();
    let sample = || {
        vec![
            Injection {
                at: Duration::from_millis(10),
                host: PART_0.to_owned(),
                fault: Fault::Partition {
                    peer: PART_1.to_owned(),
                },
            },
            Injection {
                at: Duration::from_millis(300),
                host: PART_0.to_owned(),
                fault: Fault::Heal {
                    peer: PART_1.to_owned(),
                },
            },
        ]
    };
    let checked = check_determinism([env.seed()], |seed| {
        let (log, forwarded, retries) =
            run_partitioned_crawl(seed, env.start_time(), sample()).unwrap();
        assert!(
            forwarded >= 1,
            "must forward remote-domain links over peer link"
        );
        assert!(
            retries >= 1,
            "partitioned peer link must trigger at-least-once retry"
        );
        Replay::new(&log, &[])
    });
    assert_eq!(checked, Ok(()));
}

/// Generated partition schedules over `partitioned_crawl` always deliver every
/// discovered URL to exactly one Frontier and never admit any domain on two
/// Partitions at once.
///
/// Covers: story:3-116, story:3-117
#[test]
fn partitioned_crawl_invariants_hold_under_fault_plans() {
    let env = TestEnv::new();
    let checked = generator(&env).check::<PartitionedCrawlFaults, _>(1..8, |plan| {
        run_partitioned_crawl(plan.seed, env.start_time(), injections(plan)?).map(drop)
    });
    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
