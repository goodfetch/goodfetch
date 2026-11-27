//! Group Client routing and `run_moved` retry-once under partitions and run
//! handoffs (ADRs 0055 and 0060, issue 244).
//!
//! Four simulated hosts run on `turmoil`:
//! - `lead`: authoritative group Run index (`run_id -> owner`) updated when
//!   a Run moves from `owner-a` to `owner-b` (with monotonic `run_epoch`);
//! - `owner-a`: initial owner of `run-moving-1`; once the Run moves off it,
//!   fences itself and answers `run_moved` to every subsequent forwarded call;
//! - `owner-b`: destination owner after `run-moving-1` moves; applies calls
//!   only once it holds the higher `run_epoch` ownership;
//! - `receiver`: Group member holding a Client connection and a cached
//!   `run_id -> owner` route. Forwards unary calls to the cached owner; on
//!   `run_moved`, refreshes its cache from `lead` and retries ONCE.
//!
//! Invariants judged on every run (ADR 0055):
//! - After a Run moves, the former owner answers `run_moved` and never applies
//!   any call for it;
//! - The receiving Group member refreshes its cache and retries at most once;
//! - **No call is ever applied by two owners** (for every `call_id`, the set of
//!   owners that applied it has size <= 1), even under partitions and delays.
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

use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation};
use goodfetch_test_support::{
    FaultPlan, FaultPlanAction, FaultPlanGenerator, FaultPlanStep, ObservationLog, Operation,
    Record, Replay, TestEnv, check_determinism, proptest_config_for,
};
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const SCENARIO: &str = "group_routing";
const GENERATOR_VERSION: u32 = 1;
const LEAD: &str = "lead";
const OWNER_A: &str = "owner-a";
const OWNER_B: &str = "owner-b";
const RECEIVER: &str = "receiver";
const RUN_ID: &str = "run-moving-1";
const PORT: u16 = 9666;
const RPC_TIMEOUT: Duration = Duration::from_millis(200);
const TOTAL_CALLS: u64 = 10;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireReq {
    Lookup { run_id: String },
    Forward { call_id: u64, run_id: String },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireResp {
    Route { owner: String },
    Applied { owner: String, call_id: u64 },
    RunMoved { new_owner: Option<String> },
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    RunMoved {
        call_id: u64,
        from: String,
    },
    Refreshed {
        owner: String,
    },
    Retried {
        call_id: u64,
        to: String,
    },
    Applied {
        call_id: u64,
        owner: String,
    },
    Failed {
        call_id: u64,
        reason: String,
    },
    MovedRun {
        from: String,
        to: String,
        epoch: u64,
    },
}

#[derive(Default)]
struct State {
    /// Authoritative owner in the Lead Daemon's group Run index.
    lead_owner: String,
    /// Monotonic run epoch incremented on every handoff.
    run_epoch: u64,
    /// Which host currently holds the Run's writer lease (at most one host).
    active_holder: Option<String>,
    /// Per `call_id`, every owner that applied it (invariant: len <= 1).
    applied_by: BTreeMap<u64, BTreeSet<String>>,
    /// Per `call_id`, how many times `receiver` retried after `run_moved` (invariant: <= 1).
    retries_per_call: BTreeMap<u64, u32>,
    /// Total `run_moved` answers observed.
    run_moved_count: u64,
    /// Total calls applied after a `run_moved` retry.
    retried_and_applied: u64,
}

#[derive(Clone)]
struct World {
    start: SystemTime,
    state: Rc<RefCell<State>>,
    log: Rc<RefCell<ObservationLog<Seen>>>,
}

impl World {
    fn new(start: SystemTime) -> Self {
        Self {
            start,
            state: Rc::new(RefCell::new(State {
                lead_owner: OWNER_A.to_owned(),
                run_epoch: 1,
                active_holder: Some(OWNER_A.to_owned()),
                applied_by: BTreeMap::new(),
                retries_per_call: BTreeMap::new(),
                run_moved_count: 0,
                retried_and_applied: 0,
            })),
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

    fn move_run(&self, clock: &goodfetch_engine::clock::Clock, to: &str) {
        let mut st = self.state.borrow_mut();
        let from = st.lead_owner.clone();
        if from == to {
            return;
        }
        st.active_holder = None;
        st.run_epoch += 1;
        let epoch = st.run_epoch;
        to.clone_into(&mut st.lead_owner);
        st.active_holder = Some(to.to_owned());
        drop(st);
        self.record(
            clock,
            LEAD,
            Seen::MovedRun {
                from,
                to: to.to_owned(),
                epoch,
            },
        );
    }
}

async fn send_rpc(target: &str, req: &WireReq) -> Result<WireResp, String> {
    let fut = async {
        let mut stream = turmoil::net::TcpStream::connect((target, PORT))
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
        serde_json::from_str::<WireResp>(&resp).map_err(|e| e.to_string())
    };
    tokio::time::timeout(RPC_TIMEOUT, fut)
        .await
        .map_err(|_| "timeout".to_owned())?
}

async fn lead_host(host: Host, world: World) -> turmoil::Result {
    let listener =
        turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT)).await?;
    let w = world.clone();
    let _server = tokio::task::spawn_local(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let w = w.clone();
            tokio::task::spawn_local(async move {
                let (r, mut wr) = stream.into_split();
                let mut reader = BufReader::new(r);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let resp = match serde_json::from_str::<WireReq>(&line) {
                    Ok(WireReq::Lookup { .. }) => WireResp::Route {
                        owner: w.state.borrow().lead_owner.clone(),
                    },
                    _ => WireResp::Unavailable,
                };
                if let Ok(mut out) = serde_json::to_vec(&resp) {
                    out.push(b'\n');
                    let _ = wr.write_all(&out).await;
                    let _ = wr.flush().await;
                }
            });
        }
    });
    host.clock().sleep(Duration::from_millis(350)).await;
    world.move_run(host.clock(), OWNER_B);
    host.clock().sleep(Duration::from_millis(2500)).await;
    Ok(())
}

async fn owner_host(name: &'static str, host: Host, world: World) -> turmoil::Result {
    let listener =
        turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT)).await?;
    let w = world.clone();
    let clock = *host.clock();
    let _server = tokio::task::spawn_local(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let w = w.clone();
            tokio::task::spawn_local(async move {
                let (r, mut wr) = stream.into_split();
                let mut reader = BufReader::new(r);
                let mut line = String::new();
                if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                let resp = match serde_json::from_str::<WireReq>(&line) {
                    Ok(WireReq::Forward { call_id, .. }) => {
                        let mut st = w.state.borrow_mut();
                        if st.active_holder.as_deref() == Some(name) {
                            st.applied_by
                                .entry(call_id)
                                .or_default()
                                .insert(name.to_owned());
                            drop(st);
                            w.record(
                                &clock,
                                name,
                                Seen::Applied {
                                    call_id,
                                    owner: name.to_owned(),
                                },
                            );
                            WireResp::Applied {
                                owner: name.to_owned(),
                                call_id,
                            }
                        } else {
                            let new_owner = Some(st.lead_owner.clone());
                            st.run_moved_count += 1;
                            drop(st);
                            w.record(
                                &clock,
                                name,
                                Seen::RunMoved {
                                    call_id,
                                    from: name.to_owned(),
                                },
                            );
                            WireResp::RunMoved { new_owner }
                        }
                    }
                    _ => WireResp::Unavailable,
                };
                if let Ok(mut out) = serde_json::to_vec(&resp) {
                    out.push(b'\n');
                    let _ = wr.write_all(&out).await;
                    let _ = wr.flush().await;
                }
            });
        }
    });
    host.clock().sleep(Duration::from_millis(2850)).await;
    Ok(())
}

async fn receiver_host(host: Host, world: World) -> turmoil::Result {
    host.clock().sleep(Duration::from_millis(20)).await;
    let mut cached_owner: Option<String> = None;

    for call_id in 1..=TOTAL_CALLS {
        let target = if let Some(ref owner) = cached_owner {
            owner.clone()
        } else if let Ok(WireResp::Route { owner }) = send_rpc(
            LEAD,
            &WireReq::Lookup {
                run_id: RUN_ID.to_owned(),
            },
        )
        .await
        {
            cached_owner = Some(owner.clone());
            owner
        } else {
            world.record(
                host.clock(),
                RECEIVER,
                Seen::Failed {
                    call_id,
                    reason: "lead_unreachable".to_owned(),
                },
            );
            host.clock().sleep(Duration::from_millis(70)).await;
            continue;
        };

        let req = WireReq::Forward {
            call_id,
            run_id: RUN_ID.to_owned(),
        };
        match send_rpc(&target, &req).await {
            Ok(WireResp::Applied { .. }) => {}
            Ok(WireResp::RunMoved { .. }) => {
                cached_owner = None;
                let refreshed = send_rpc(
                    LEAD,
                    &WireReq::Lookup {
                        run_id: RUN_ID.to_owned(),
                    },
                )
                .await;
                if let Ok(WireResp::Route { owner: new_owner }) = refreshed {
                    cached_owner = Some(new_owner.clone());
                    world.record(
                        host.clock(),
                        RECEIVER,
                        Seen::Refreshed {
                            owner: new_owner.clone(),
                        },
                    );
                    *world
                        .state
                        .borrow_mut()
                        .retries_per_call
                        .entry(call_id)
                        .or_default() += 1;
                    world.record(
                        host.clock(),
                        RECEIVER,
                        Seen::Retried {
                            call_id,
                            to: new_owner.clone(),
                        },
                    );
                    if let Ok(WireResp::Applied { .. }) = send_rpc(&new_owner, &req).await {
                        world.state.borrow_mut().retried_and_applied += 1;
                    }
                } else {
                    world.record(
                        host.clock(),
                        RECEIVER,
                        Seen::Failed {
                            call_id,
                            reason: "refresh_failed".to_owned(),
                        },
                    );
                }
            }
            Ok(WireResp::Unavailable | WireResp::Route { .. }) | Err(_) => {
                world.record(
                    host.clock(),
                    RECEIVER,
                    Seen::Failed {
                        call_id,
                        reason: "peer_unreachable".to_owned(),
                    },
                );
            }
        }
        host.clock().sleep(Duration::from_millis(70)).await;
    }
    Ok(())
}

fn run_group_routing(
    seed: u64,
    start: SystemTime,
    injections: Vec<Injection>,
) -> Result<(ObservationLog<Seen>, u64, u64), String> {
    let mut sim = Simulation::new(seed, start);
    let world = World::new(start);

    {
        let w = world.clone();
        sim.host(LEAD, move |host| lead_host(host, w.clone()));
    }
    {
        let w = world.clone();
        sim.host(OWNER_A, move |host| owner_host(OWNER_A, host, w.clone()));
    }
    {
        let w = world.clone();
        sim.host(OWNER_B, move |host| owner_host(OWNER_B, host, w.clone()));
    }
    {
        let w = world.clone();
        sim.client(RECEIVER, move |host| receiver_host(host, w.clone()));
    }

    sim.run_injecting(injections)
        .map_err(|error| error.to_string())?;
    drop(sim);

    let st = world.state.borrow();
    for (call_id, owners) in &st.applied_by {
        if owners.len() > 1 {
            return Err(format!(
                "call {call_id} was applied by multiple owners: {owners:?}"
            ));
        }
    }
    for (call_id, retries) in &st.retries_per_call {
        if *retries > 1 {
            return Err(format!(
                "call {call_id} retried {retries} times after run_moved (must retry at most once)"
            ));
        }
    }
    let moved = st.run_moved_count;
    let retried_applied = st.retried_and_applied;
    drop(st);
    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("the hosts are done with the log")
    };
    Ok((log.into_inner(), moved, retried_applied))
}

struct GroupRoutingFaults;

impl ReferenceStateMachine for GroupRoutingFaults {
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        const PAIRS: [(&str, &str); 3] =
            [(RECEIVER, LEAD), (RECEIVER, OWNER_A), (RECEIVER, OWNER_B)];
        let now = *now;
        (
            0..150_u64,
            prop::sample::select(PAIRS.as_slice()),
            prop::bool::ANY,
        )
            .prop_map(move |(after, (host, peer), partition)| FaultPlanStep {
                at: now + after,
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
                other => return Err(format!("group_routing scenario injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Deterministic replay of a run moved under partitions: former owner answers
/// `run_moved`, receiver refreshes and retries once, and no call is applied by
/// two owners.
///
/// Covers: story:3-109, story:3-110
#[test]
fn group_routing_simulation_is_deterministic_and_retries_once() {
    let env = TestEnv::new();
    let sample = || {
        vec![
            Injection {
                at: Duration::from_millis(180),
                host: RECEIVER.to_owned(),
                fault: Fault::Partition {
                    peer: LEAD.to_owned(),
                },
            },
            Injection {
                at: Duration::from_millis(280),
                host: RECEIVER.to_owned(),
                fault: Fault::Heal {
                    peer: LEAD.to_owned(),
                },
            },
        ]
    };
    let checked = check_determinism([env.seed()], |seed| {
        let (log, moved, retried_applied) =
            run_group_routing(seed, env.start_time(), sample()).unwrap();
        assert!(moved >= 1, "must observe run_moved after handoff");
        assert!(
            retried_applied >= 1,
            "must refresh cache and retry once onto new owner"
        );
        Replay::new(&log, &[])
    });
    assert_eq!(checked, Ok(()));
}

/// Generated partition schedules and committed fault plans over `group_routing`
/// never apply any call on two owners and never retry more than once.
///
/// Covers: story:3-109, story:3-110
#[test]
fn group_routing_holds_under_partitions_and_moves() {
    let env = TestEnv::new();
    let checked = generator(&env).check::<GroupRoutingFaults, _>(1..8, |plan| {
        run_group_routing(plan.seed, env.start_time(), injections(plan)?).map(drop)
    });
    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
