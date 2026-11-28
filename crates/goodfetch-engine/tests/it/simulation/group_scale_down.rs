//! Simulation of Daemon group scale-down Checkpoint handoff, `runs adopt` for
//! Runs missed by the drain grace, and a zombie copy of the removed Group
//! member attempting to write to a fenced destination, judged by ADR 0060's
//! `fencing` invariant (ADRs 0003, 0055 and 0060, issue 245).
//!
//! Five simulated hosts run on `turmoil`:
//! - `lead`: authoritative group Run index (`run_id -> (owner, epoch)`) and
//!   placement;
//! - `member-a`: Group member being scaled down; hands off `run-handed-off`
//!   during drain, leaves `run-leftover` checkpointed past the drain grace,
//!   and later comes back as a zombie copy attempting a stale-epoch write;
//! - `member-b`: remaining Group member receiving `run-handed-off` via
//!   Checkpoint handoff and `run-leftover` via `runs adopt`, taking `epoch + 1`
//!   and fencing the destination before writing;
//! - `adopter`: `runs adopt` job that reads `run-leftover`'s Checkpoint from
//!   the kept volume and hands it to `member-b`;
//! - `destination`: fenced destination that accepts `Fence(epoch)` and rejects
//!   any write with `epoch < highest_fence_epoch`, recording `Operation::Fence`,
//!   `Operation::Write`, `Operation::Commit`, and `Operation::Visible` into the
//!   `ObservationLog` judged by `goodfetch_test_support::invariants::fencing`.
#![expect(
    clippy::disallowed_methods,
    reason = "turmoil host servers run !Send futures on LocalSet inside the Simulation (ADR 0060)"
)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation};
use goodfetch_test_support::{
    FaultPlan, FaultPlanAction, FaultPlanGenerator, FaultPlanStep, GuaranteeClass, ObservationLog,
    Operation, Record, Replay, TestEnv, ValueDigest, check_determinism, fencing,
    proptest_config_for,
};
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const SCENARIO: &str = "group_scale_down";
const GENERATOR_VERSION: u32 = 1;
const LEAD: &str = "lead";
const MEMBER_A: &str = "member-a";
const MEMBER_B: &str = "member-b";
const ADOPTER: &str = "adopter";
const DESTINATION: &str = "destination";
const PORT: u16 = 9667;
const RPC_TIMEOUT: Duration = Duration::from_millis(200);
const RUN_HANDED_OFF: &str = "run-handed-off";
const RUN_LEFTOVER: &str = "run-leftover";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireReq {
    Handoff {
        run_id: String,
        checkpoint_epoch: u64,
        adopt: bool,
    },
    Fence {
        run_id: String,
        epoch: u64,
    },
    Write {
        run_id: String,
        epoch: u64,
        key: String,
        value: String,
    },
    ListAssigned {
        member: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum WireResp {
    HandedOff { new_epoch: u64 },
    Fenced { highest_epoch: u64 },
    Written,
    RejectedStale { highest_epoch: u64 },
    Assigned { runs: Vec<String> },
    Unavailable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Event {
    HandoffComplete {
        run_id: String,
        to: String,
        new_epoch: u64,
        adopt: bool,
    },
    ZombieRejected {
        run_id: String,
        stale_epoch: u64,
        highest_epoch: u64,
    },
    RejoinedAssigned {
        member: String,
        runs: Vec<String>,
    },
}

#[derive(Default)]
struct WorldState {
    owners: BTreeMap<String, (String, u64)>,
    fenced_epoch: BTreeMap<String, u64>,
    handoffs_done: u64,
    adoptions_done: u64,
    zombie_rejected: u64,
    rejoined_resumed: Vec<String>,
}

#[derive(Clone)]
struct World {
    start: SystemTime,
    state: Rc<RefCell<WorldState>>,
    /// Per-run observation logs judged independently by `invariants::fencing`.
    logs: Rc<RefCell<BTreeMap<String, ObservationLog<Event>>>>,
    /// Combined deterministic replay log.
    replay: Rc<RefCell<ObservationLog<Event>>>,
}

impl World {
    fn new(start: SystemTime) -> Self {
        let mut owners = BTreeMap::new();
        owners.insert(RUN_HANDED_OFF.to_owned(), (MEMBER_A.to_owned(), 1));
        owners.insert(RUN_LEFTOVER.to_owned(), (MEMBER_A.to_owned(), 1));
        let mut fenced_epoch = BTreeMap::new();
        fenced_epoch.insert(RUN_HANDED_OFF.to_owned(), 1);
        fenced_epoch.insert(RUN_LEFTOVER.to_owned(), 1);
        Self {
            start,
            state: Rc::new(RefCell::new(WorldState {
                owners,
                fenced_epoch,
                ..WorldState::default()
            })),
            logs: Rc::new(RefCell::new(BTreeMap::new())),
            replay: Rc::new(RefCell::new(ObservationLog::new())),
        }
    }

    fn at_ms(&self, clock: &goodfetch_engine::clock::Clock) -> u64 {
        let elapsed = clock.wall().duration_since(self.start).unwrap_or_default();
        u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX)
    }

    fn record_op(
        &self,
        clock: &goodfetch_engine::clock::Clock,
        run_id: &str,
        who: &str,
        op: Operation<Event>,
    ) {
        let at = self.at_ms(clock);
        let rec = Record::new(who, op).at(at);
        self.logs
            .borrow_mut()
            .entry(run_id.to_owned())
            .or_default()
            .push(rec.clone());
        self.replay.borrow_mut().push(rec);
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
                    Ok(WireReq::ListAssigned { member }) => {
                        let runs = w
                            .state
                            .borrow()
                            .owners
                            .iter()
                            .filter(|(_, (owner, _))| owner == &member)
                            .map(|(id, _)| id.clone())
                            .collect();
                        WireResp::Assigned { runs }
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
    host.clock().sleep(Duration::from_millis(2500)).await;
    Ok(())
}

async fn destination_host(host: Host, world: World) -> turmoil::Result {
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
                    Ok(WireReq::Fence { run_id, epoch }) => {
                        let highest = {
                            let mut st = w.state.borrow_mut();
                            let entry = st.fenced_epoch.entry(run_id.clone()).or_insert(0);
                            if epoch > *entry {
                                *entry = epoch;
                            }
                            *entry
                        };
                        w.record_op(
                            &clock,
                            &run_id,
                            DESTINATION,
                            Operation::Fence { epoch: highest },
                        );
                        WireResp::Fenced {
                            highest_epoch: highest,
                        }
                    }
                    Ok(WireReq::Write {
                        run_id,
                        epoch,
                        key,
                        value,
                    }) => {
                        let highest = *w.state.borrow().fenced_epoch.get(&run_id).unwrap_or(&0);
                        if epoch < highest {
                            w.state.borrow_mut().zombie_rejected += 1;
                            w.record_op(
                                &clock,
                                &run_id,
                                DESTINATION,
                                Operation::Other(Event::ZombieRejected {
                                    run_id: run_id.clone(),
                                    stale_epoch: epoch,
                                    highest_epoch: highest,
                                }),
                            );
                            WireResp::RejectedStale {
                                highest_epoch: highest,
                            }
                        } else {
                            let digest = ValueDigest::of(&value);
                            w.record_op(
                                &clock,
                                &run_id,
                                DESTINATION,
                                Operation::Commit {
                                    key: key.clone(),
                                    digest: digest.clone(),
                                },
                            );
                            w.record_op(
                                &clock,
                                &run_id,
                                DESTINATION,
                                Operation::Visible { epoch, key, digest },
                            );
                            WireResp::Written
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
    host.clock().sleep(Duration::from_millis(2500)).await;
    Ok(())
}

async fn member_b_host(host: Host, world: World) -> turmoil::Result {
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
                    Ok(WireReq::Handoff {
                        run_id,
                        checkpoint_epoch,
                        adopt,
                    }) => {
                        let new_epoch = checkpoint_epoch + 1;
                        // Fence destination first before accepting ownership & writing.
                        let fenced = send_rpc(
                            DESTINATION,
                            &WireReq::Fence {
                                run_id: run_id.clone(),
                                epoch: new_epoch,
                            },
                        )
                        .await;
                        if matches!(fenced, Ok(WireResp::Fenced { .. })) {
                            {
                                let mut st = w.state.borrow_mut();
                                st.owners
                                    .insert(run_id.clone(), (MEMBER_B.to_owned(), new_epoch));
                                if adopt {
                                    st.adoptions_done += 1;
                                } else {
                                    st.handoffs_done += 1;
                                }
                            }
                            w.record_op(
                                &clock,
                                &run_id,
                                MEMBER_B,
                                Operation::Other(Event::HandoffComplete {
                                    run_id: run_id.clone(),
                                    to: MEMBER_B.to_owned(),
                                    new_epoch,
                                    adopt,
                                }),
                            );
                            let key = format!("post-{run_id}");
                            let val = format!("by-member-b-epoch-{new_epoch}");
                            w.record_op(
                                &clock,
                                &run_id,
                                MEMBER_B,
                                Operation::Write {
                                    epoch: new_epoch,
                                    key: key.clone(),
                                    digest: ValueDigest::of(&val),
                                },
                            );
                            let _ = send_rpc(
                                DESTINATION,
                                &WireReq::Write {
                                    run_id,
                                    epoch: new_epoch,
                                    key,
                                    value: val,
                                },
                            )
                            .await;
                            WireResp::HandedOff { new_epoch }
                        } else {
                            WireResp::Unavailable
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
    host.clock().sleep(Duration::from_millis(2500)).await;
    Ok(())
}

async fn member_a_client(host: Host, world: World) -> turmoil::Result {
    // Epoch 1: member-a writes initial record for both Runs before scale-down.
    for run_id in [RUN_HANDED_OFF, RUN_LEFTOVER] {
        let key = format!("pre-{run_id}");
        let val = "by-member-a-epoch-1".to_owned();
        world.record_op(
            host.clock(),
            run_id,
            MEMBER_A,
            Operation::Write {
                epoch: 1,
                key: key.clone(),
                digest: ValueDigest::of(&val),
            },
        );
        let _ = send_rpc(
            DESTINATION,
            &WireReq::Write {
                run_id: run_id.to_owned(),
                epoch: 1,
                key,
                value: val,
            },
        )
        .await;
    }

    // Scale-down drain: hand off `run-handed-off` within grace; leave `run-leftover`
    // checkpointed past the drain grace for `runs adopt`.
    host.clock().sleep(Duration::from_millis(80)).await;
    let _ = send_rpc(
        MEMBER_B,
        &WireReq::Handoff {
            run_id: RUN_HANDED_OFF.to_owned(),
            checkpoint_epoch: 1,
            adopt: false,
        },
    )
    .await;

    // Later, a zombie copy of member-a (still holding stale epoch 1) wakes up
    // and tries to write to the fenced destination for both Runs.
    host.clock().sleep(Duration::from_millis(500)).await;
    for run_id in [RUN_HANDED_OFF, RUN_LEFTOVER] {
        let key = format!("zombie-{run_id}");
        let val = "stale-zombie-write".to_owned();
        world.record_op(
            host.clock(),
            run_id,
            MEMBER_A,
            Operation::Write {
                epoch: 1,
                key: key.clone(),
                digest: ValueDigest::of(&val),
            },
        );
        let _ = send_rpc(
            DESTINATION,
            &WireReq::Write {
                run_id: run_id.to_owned(),
                epoch: 1,
                key,
                value: val,
            },
        )
        .await;
    }

    // Finally, member-a is added back cleanly and queries Lead for Runs still
    // assigned to it (must NOT resume Runs already handed off or adopted).
    host.clock().sleep(Duration::from_millis(300)).await;
    if let Ok(WireResp::Assigned { runs }) = send_rpc(
        LEAD,
        &WireReq::ListAssigned {
            member: MEMBER_A.to_owned(),
        },
    )
    .await
    {
        world.state.borrow_mut().rejoined_resumed.clone_from(&runs);
        world.record_op(
            host.clock(),
            RUN_HANDED_OFF,
            MEMBER_A,
            Operation::Other(Event::RejoinedAssigned {
                member: MEMBER_A.to_owned(),
                runs,
            }),
        );
    }
    Ok(())
}

async fn adopter_client(host: Host, _world: World) -> turmoil::Result {
    // Runs after member-a's drain grace expired: adopts `run-leftover` into member-b.
    host.clock().sleep(Duration::from_millis(280)).await;
    let _ = send_rpc(
        MEMBER_B,
        &WireReq::Handoff {
            run_id: RUN_LEFTOVER.to_owned(),
            checkpoint_epoch: 1,
            adopt: true,
        },
    )
    .await;
    Ok(())
}

type ScaleDownOutcome = (ObservationLog<Event>, u64, u64, u64, Vec<String>);

fn run_scale_down(
    seed: u64,
    start: SystemTime,
    injections: Vec<Injection>,
) -> Result<ScaleDownOutcome, String> {
    let mut sim = Simulation::new(seed, start);
    let world = World::new(start);

    {
        let w = world.clone();
        sim.host(LEAD, move |host| lead_host(host, w.clone()));
    }
    {
        let w = world.clone();
        sim.host(DESTINATION, move |host| destination_host(host, w.clone()));
    }
    {
        let w = world.clone();
        sim.host(MEMBER_B, move |host| member_b_host(host, w.clone()));
    }
    {
        let w = world.clone();
        sim.client(MEMBER_A, move |host| member_a_client(host, w.clone()));
    }
    {
        let w = world.clone();
        sim.client(ADOPTER, move |host| adopter_client(host, w.clone()));
    }

    sim.run_injecting(injections)
        .map_err(|error| error.to_string())?;
    drop(sim);

    // Judge EVERY Run's ObservationLog with ADR 0060's fencing invariant!
    for (run_id, log) in world.logs.borrow().iter() {
        if let Err(violation) = fencing(log, GuaranteeClass::Fenced) {
            return Err(format!(
                "fencing invariant failed for {run_id}: {violation:?}"
            ));
        }
    }

    let st = world.state.borrow();
    let out = (
        st.handoffs_done,
        st.adoptions_done,
        st.zombie_rejected,
        st.rejoined_resumed.clone(),
    );
    drop(st);
    let Ok(replay) = Rc::try_unwrap(world.replay) else {
        panic!("the hosts are done with the replay log")
    };
    Ok((replay.into_inner(), out.0, out.1, out.2, out.3))
}

struct ScaleDownFaults;

impl ReferenceStateMachine for ScaleDownFaults {
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        const PAIRS: [(&str, &str); 3] = [
            (MEMBER_A, MEMBER_B),
            (MEMBER_A, DESTINATION),
            (ADOPTER, MEMBER_B),
        ];
        let now = *now;
        (
            0..180_u64,
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
                other => return Err(format!("group_scale_down injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Deterministic replay of scale-down Checkpoint handoff, `runs adopt`, zombie
/// stale-epoch write rejection, and non-duplication on rejoin — judged by
/// ADR 0060's `fencing` invariant.
///
/// Covers: story:3-111
#[test]
fn group_scale_down_handoff_adopt_and_zombie_fencing_are_deterministic() {
    let env = TestEnv::new();
    let checked = check_determinism([env.seed()], |seed| {
        let (log, handoffs, adoptions, zombie_rejected, rejoined) =
            run_scale_down(seed, env.start_time(), Vec::new()).unwrap();
        assert_eq!(handoffs, 1, "scale-down must hand off run-handed-off");
        assert_eq!(adoptions, 1, "runs adopt must adopt run-leftover");
        assert_eq!(
            zombie_rejected, 2,
            "fenced destination must reject both stale zombie writes"
        );
        assert!(
            rejoined.is_empty(),
            "re-added member-a must not be assigned Runs already handed off or adopted"
        );
        Replay::new(&log, &[])
    });
    assert_eq!(checked, Ok(()));
}

/// Generated partition schedules and committed fault plans over
/// `group_scale_down` never violate ADR 0060's `fencing` invariant.
///
/// Covers: story:3-111
#[test]
fn group_scale_down_fencing_holds_under_partitions() {
    let env = TestEnv::new();
    let checked = generator(&env).check::<ScaleDownFaults, _>(1..8, |plan| {
        run_scale_down(plan.seed, env.start_time(), injections(plan)?).map(drop)
    });
    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
