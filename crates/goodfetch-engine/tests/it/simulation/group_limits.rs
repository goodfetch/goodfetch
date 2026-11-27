//! Group-wide Usage ledger Slices under partitions, lost Lead Daemon and
//! crashed Group members (ADRs 0041, 0055 and 0060, issue 258).
//!
//! Four simulated hosts run on `turmoil`:
//! - `lead`: owns the group's real [`Ledger`] SQLite file and serves
//!   Slice takes and give-backs over simulated TCP, while also running a
//!   local run of its own;
//! - `worker-a` and `worker-b`: Group members running runs that spend
//!   held Slices in memory and request new Slices from `lead` over
//!   simulated TCP;
//! - `attached`: an Attached engine beside `worker-a` that takes its
//!   Slices through `worker-a`'s local redirect hop (`attached -> worker-a
//!   -> lead`), failing closed with `lead_unreachable` whenever `worker-a`
//!   or `lead` is unreachable.
//!
//! Invariant judged on every run (ADR 0055):
//! - `charged + held <= AMOUNT` on the Lead Daemon's ledger;
//! - total completed charges across all 4 engines never exceed `AMOUNT`
//!   and never exceed `charged + held` on the Lead Daemon's ledger.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::ledger::{
    Holder, Ledger, SliceId, SliceSizing, StoredLimit, from_wire_slice,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::group::WireSlice;
use goodfetch_protocol::run::{LimitScope, LimitSetter, Period, PeriodLength};
use goodfetch_protocol::usage::{Limit, PERSON, REQUESTS};
use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation};
use goodfetch_test_support::{
    FaultPlan, FaultPlanAction, FaultPlanGenerator, FaultPlanStep, ObservationLog, Operation,
    Record, Replay, TestEnv, check_determinism, proptest_config_for,
};
use jiff::Timestamp;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const SCENARIO: &str = "group_limits";
const GENERATOR_VERSION: u32 = 1;
const LEAD: &str = "lead";
const WORKERS: [&str; 2] = ["worker-a", "worker-b"];
const ATTACHED: &str = "attached";
const ALL_RUNNERS: [&str; 4] = [LEAD, WORKERS[0], WORKERS[1], ATTACHED];
const OPERATOR: &str = "operator";
const LEAD_PORT: u16 = 9555;
const REDIRECT_PORT: u16 = 9556;

/// Monthly person Limit shared across all 4 engines (total demand = 4 * 12 = 48 > 24).
const AMOUNT: u64 = 24;
/// How many 1-request charges each host attempts.
const TARGET_CHARGES: u64 = 12;
const RPC_TIMEOUT: Duration = Duration::from_millis(80);
const SETTLED: Duration = Duration::from_millis(2400);

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LeadRequest {
    TakeSlice {
        run_id: String,
        epoch: u64,
        needing: u64,
    },
    GiveBack {
        used: Vec<(SliceId, u64)>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum LeadAnswer {
    Slice { slice: Option<WireSlice> },
    GivenBack,
    Unavailable { message: String },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    Charged {
        from_held: bool,
        total: u64,
    },
    SliceTaken {
        size: u64,
    },
    LimitReached,
    LeadUnreachable,
    Checked {
        charged: u64,
        held: u64,
        completed: u64,
    },
}

#[derive(Debug, Default)]
struct RunnerState {
    boots: u64,
    completed: u64,
    held_slice: Option<(SliceId, u64, u64)>,
    stopped_reached: bool,
}

#[derive(Clone)]
struct World {
    start: SystemTime,
    ledger_path: PathBuf,
    limit: StoredLimit,
    runners: Rc<RefCell<BTreeMap<&'static str, RunnerState>>>,
    log: Rc<RefCell<ObservationLog<Seen>>>,
}

impl World {
    fn record(&self, host: &Host, who: &str, seen: Seen) {
        let at = host
            .clock()
            .wall()
            .duration_since(self.start)
            .unwrap_or_default();
        let at = u64::try_from(at.as_millis()).unwrap_or(u64::MAX);
        self.log
            .borrow_mut()
            .push(Record::new(who, Operation::Other(seen)).at(at));
    }

    fn ledger(&self) -> Ledger {
        Ledger::at(self.ledger_path.clone(), None)
    }

    fn now_ts(&self, host: &Host) -> Timestamp {
        Timestamp::try_from(host.clock().wall()).unwrap()
    }
}

async fn send_request(
    target: &str,
    port: u16,
    request: &LeadRequest,
) -> Result<LeadAnswer, String> {
    let future = async {
        let mut stream = turmoil::net::TcpStream::connect((target, port))
            .await
            .map_err(|error| error.to_string())?;
        let mut encoded = serde_json::to_vec(request).map_err(|error| error.to_string())?;
        encoded.push(b'\n');
        stream
            .write_all(&encoded)
            .await
            .map_err(|error| error.to_string())?;
        stream.flush().await.map_err(|error| error.to_string())?;
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        let read = reader
            .read_line(&mut line)
            .await
            .map_err(|error| error.to_string())?;
        if read == 0 {
            return Err("peer closed connection".to_owned());
        }
        serde_json::from_str::<LeadAnswer>(&line).map_err(|error| error.to_string())
    };
    tokio::time::timeout(RPC_TIMEOUT, future)
        .await
        .map_err(|_| "peer request timed out".to_owned())?
}

fn execute_on_lead(world: &World, ts: Timestamp, request: LeadRequest) -> LeadAnswer {
    let ledger = world.ledger();
    match request {
        LeadRequest::TakeSlice {
            run_id,
            epoch,
            needing,
        } => {
            let holder = Holder {
                run_id,
                epoch,
                folder: world.ledger_path.parent().unwrap().to_path_buf(),
            };
            match ledger.take_slice_direct(
                &world.limit,
                &holder,
                ts,
                SliceSizing::default(),
                needing,
            ) {
                Ok(slice) => LeadAnswer::Slice {
                    slice: slice.map(|s| WireSlice {
                        id: s.id,
                        size: s.size,
                    }),
                },
                Err(error) => LeadAnswer::Unavailable {
                    message: error.to_string(),
                },
            }
        }
        LeadRequest::GiveBack { used } => match ledger.give_back_direct(&used) {
            Ok(()) => LeadAnswer::GivenBack,
            Err(error) => LeadAnswer::Unavailable {
                message: error.to_string(),
            },
        },
    }
}

async fn lead_host(host: Host, world: World) -> turmoil::Result {
    let listener =
        turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), LEAD_PORT)).await?;
    let server_world = world.clone();
    let server_clock = goodfetch_engine::clock::Clock::paused(world.start);
    let _server = tokio::task::spawn_local(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let world = server_world.clone();
            let clock = server_clock;
            tokio::task::spawn_local(async move {
                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let mut line = String::new();
                let Ok(read) = reader.read_line(&mut line).await else {
                    return;
                };
                if read == 0 {
                    return;
                }
                let ts = clock.timestamp();
                let answer = match serde_json::from_str::<LeadRequest>(&line) {
                    Ok(req) => execute_on_lead(&world, ts, req),
                    Err(error) => LeadAnswer::Unavailable {
                        message: error.to_string(),
                    },
                };
                if let Ok(mut out) = serde_json::to_vec(&answer) {
                    out.push(b'\n');
                    let _ = write_half.write_all(&out).await;
                    let _ = write_half.flush().await;
                }
            });
        }
    });

    drive_runner(host, LEAD, None, world).await;
    Ok(())
}

async fn worker_host(
    host: Host,
    name: &'static str,
    serve_redirect: bool,
    world: World,
) -> turmoil::Result {
    if serve_redirect {
        let listener =
            turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), REDIRECT_PORT))
                .await?;
        tokio::task::spawn_local(async move {
            while let Ok((stream, _)) = listener.accept().await {
                tokio::task::spawn_local(async move {
                    let (read_half, mut write_half) = stream.into_split();
                    let mut reader = BufReader::new(read_half);
                    let mut line = String::new();
                    let Ok(read) = reader.read_line(&mut line).await else {
                        return;
                    };
                    if read == 0 {
                        return;
                    }
                    let answer = match serde_json::from_str::<LeadRequest>(&line) {
                        Ok(req) => match send_request(LEAD, LEAD_PORT, &req).await {
                            Ok(reply) => reply,
                            Err(message) => LeadAnswer::Unavailable { message },
                        },
                        Err(error) => LeadAnswer::Unavailable {
                            message: error.to_string(),
                        },
                    };
                    if let Ok(mut out) = serde_json::to_vec(&answer) {
                        out.push(b'\n');
                        let _ = write_half.write_all(&out).await;
                        let _ = write_half.flush().await;
                    }
                });
            }
        });
    }

    drive_runner(host, name, Some((LEAD, LEAD_PORT)), world).await;
    Ok(())
}

async fn attached_host(host: Host, world: World) -> turmoil::Result {
    // Takes slices strictly through worker-a's redirect endpoint (never directly from lead).
    drive_runner(host, ATTACHED, Some((WORKERS[0], REDIRECT_PORT)), world).await;
    Ok(())
}

async fn drive_runner(
    mut host: Host,
    who: &'static str,
    remote: Option<(&'static str, u16)>,
    world: World,
) {
    let epoch = {
        let mut runners = world.runners.borrow_mut();
        let entry = runners.entry(who).or_default();
        entry.boots += 1;
        // A crash loses in-memory held Slices; their server-side reservation stays held.
        entry.held_slice = None;
        entry.boots
    };

    for _ in 0..40 {
        let pause_ms = 20 + (host.entropy().next_u64() % 40);
        host.clock().sleep(Duration::from_millis(pause_ms)).await;

        let (done, spent_from_held) = {
            let mut runners = world.runners.borrow_mut();
            let state = runners.entry(who).or_default();
            if state.stopped_reached || state.completed >= TARGET_CHARGES {
                (true, None)
            } else if let Some((id, used, size)) = state.held_slice {
                if used < size {
                    let next_used = used + 1;
                    state.held_slice = if next_used < size {
                        Some((id, next_used, size))
                    } else {
                        None
                    };
                    state.completed += 1;
                    (false, Some(state.completed))
                } else {
                    (false, None)
                }
            } else {
                (false, None)
            }
        };
        if done {
            break;
        }
        if let Some(total) = spent_from_held {
            world.record(
                &host,
                who,
                Seen::Charged {
                    from_held: true,
                    total,
                },
            );
            continue;
        }

        // Held slice empty: request a new Slice from Lead Daemon (or redirect host).
        let req = LeadRequest::TakeSlice {
            run_id: who.to_owned(),
            epoch,
            needing: 1,
        };
        let answer = match remote {
            None => Ok(execute_on_lead(&world, world.now_ts(&host), req)),
            Some((target, port)) => send_request(target, port, &req).await,
        };
        match answer {
            Ok(LeadAnswer::Slice { slice: Some(wire) }) => {
                let slice = from_wire_slice(wire);
                world.record(&host, who, Seen::SliceTaken { size: slice.size });
                let total = {
                    let mut runners = world.runners.borrow_mut();
                    let state = runners.entry(who).or_default();
                    state.completed += 1;
                    if slice.size > 1 {
                        state.held_slice = Some((slice.id, 1, slice.size));
                    }
                    state.completed
                };
                world.record(
                    &host,
                    who,
                    Seen::Charged {
                        from_held: false,
                        total,
                    },
                );
            }
            Ok(LeadAnswer::Slice { slice: None }) => {
                world
                    .runners
                    .borrow_mut()
                    .entry(who)
                    .or_default()
                    .stopped_reached = true;
                world.record(&host, who, Seen::LimitReached);
                break;
            }
            Ok(LeadAnswer::Unavailable { .. } | LeadAnswer::GivenBack) | Err(_) => {
                world.record(&host, who, Seen::LeadUnreachable);
                host.clock().sleep(Duration::from_millis(60)).await;
            }
        }
    }

    // Give back any remaining unused Slice if Lead Daemon is reachable.
    let leftover = world
        .runners
        .borrow_mut()
        .entry(who)
        .or_default()
        .held_slice
        .take();
    if let Some((slice_id, used, _size)) = leftover {
        let req = LeadRequest::GiveBack {
            used: vec![(slice_id, used)],
        };
        match remote {
            None => {
                let _ = execute_on_lead(&world, world.now_ts(&host), req);
            }
            Some((target, port)) => {
                let _ = send_request(target, port, &req).await;
            }
        }
    }
}

async fn operator_host(host: Host, world: World) -> turmoil::Result {
    host.clock().sleep(SETTLED).await;
    let usage = world
        .ledger()
        .usage(Product::Collie, world.now_ts(&host))
        .map_err(|error| error.to_string())?;
    let entry = usage
        .iter()
        .find(|u| u.limit.scope.kind == PERSON && u.limit.scope.id == "alice")
        .ok_or_else(|| "person monthly limit missing on Lead Daemon".to_owned())?;
    let total_completed: u64 = ALL_RUNNERS
        .iter()
        .map(|name| {
            world
                .runners
                .borrow()
                .get(name)
                .map_or(0, |state| state.completed)
        })
        .sum();
    let total_committed = entry.charged + entry.held;
    world.record(
        &host,
        OPERATOR,
        Seen::Checked {
            charged: entry.charged,
            held: entry.held,
            completed: total_completed,
        },
    );
    if total_committed > AMOUNT {
        return Err(format!(
            "Limits invariant violated: charged ({}) + held ({}) = {} > limit ({AMOUNT})",
            entry.charged, entry.held, total_committed
        )
        .into());
    }
    if total_completed > AMOUNT {
        return Err(format!(
            "Limits invariant violated: completed charges ({total_completed}) > limit ({AMOUNT})"
        )
        .into());
    }
    if total_completed > total_committed {
        return Err(format!(
            "Limits invariant violated: completed charges ({total_completed}) exceeded ledger charged+held ({total_committed})"
        )
        .into());
    }
    Ok(())
}

fn run_group_limits(
    seed: u64,
    start: SystemTime,
    folder: &Path,
    injections: Vec<Injection>,
) -> Result<ObservationLog<Seen>, String> {
    let dir = tempfile::tempdir_in(folder).map_err(|error| error.to_string())?;
    let ledger_path = dir.path().join("usage-ledger.sqlite");
    let ledger = Ledger::at(ledger_path.clone(), None);
    let ts = Timestamp::try_from(start).map_err(|error| error.to_string())?;
    let limit = Limit {
        scope: LimitScope {
            kind: PERSON.to_owned(),
            id: "alice".to_owned(),
        },
        usage_kind: REQUESTS.to_owned(),
        currency: None,
        period: Some(Period {
            every: PeriodLength::Month,
            time_zone: "UTC".to_owned(),
        }),
        amount: AMOUNT,
        must_understand: true,
        set_by: LimitSetter::Person,
    };
    ledger
        .add_limit(&limit, ts)
        .map_err(|error| error.to_string())?;
    let stored = ledger
        .non_redirect_limits()
        .map_err(|error| error.to_string())?
        .into_iter()
        .next()
        .ok_or_else(|| "inserted limit missing".to_owned())?;

    let world = World {
        start,
        ledger_path,
        limit: stored,
        runners: Rc::default(),
        log: Rc::new(RefCell::new(ObservationLog::new())),
    };

    let mut simulation = Simulation::new(seed, start);
    {
        let world = world.clone();
        simulation.host(LEAD, move |host| lead_host(host, world.clone()));
    }
    {
        let world = world.clone();
        simulation.host(WORKERS[0], move |host| {
            worker_host(host, WORKERS[0], true, world.clone())
        });
    }
    {
        let world = world.clone();
        simulation.host(WORKERS[1], move |host| {
            worker_host(host, WORKERS[1], false, world.clone())
        });
    }
    {
        let world = world.clone();
        simulation.host(ATTACHED, move |host| attached_host(host, world.clone()));
    }
    {
        let world = world.clone();
        simulation.client(OPERATOR, move |host| operator_host(host, world.clone()));
    }

    simulation
        .run_injecting(injections)
        .map_err(|error| error.to_string())?;
    drop(simulation);
    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("the hosts are done with the log")
    };
    Ok(log.into_inner())
}

struct GroupLimitsFaults;

impl ReferenceStateMachine for GroupLimitsFaults {
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        const CRASHABLE: [&str; 3] = [LEAD, WORKERS[0], WORKERS[1]];
        const PAIRS: [(&str, &str); 3] = [
            (LEAD, WORKERS[0]),
            (LEAD, WORKERS[1]),
            (WORKERS[0], ATTACHED),
        ];
        let now = *now;
        prop_oneof![
            (
                0..150_u64,
                prop::sample::select(CRASHABLE.as_slice()),
                prop_oneof![Just(FaultPlanAction::Crash), Just(FaultPlanAction::Restart)],
            )
                .prop_map(move |(after, host, action)| FaultPlanStep {
                    at: now + after,
                    host: host.to_owned(),
                    action,
                }),
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
                }),
        ]
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
                FaultPlanAction::Crash => Fault::Crash,
                FaultPlanAction::Restart => Fault::Restart,
                FaultPlanAction::Partition { peer } => Fault::Partition { peer: peer.clone() },
                FaultPlanAction::Heal { peer } => Fault::Heal { peer: peer.clone() },
                other => return Err(format!("group_limits scenario injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Covers: story:3-112, story:3-115
#[test]
fn group_limits_simulation_is_deterministic() {
    let env = TestEnv::new();
    let sample = || {
        vec![
            Injection {
                at: Duration::from_millis(120),
                host: LEAD.to_owned(),
                fault: Fault::Partition {
                    peer: WORKERS[0].to_owned(),
                },
            },
            Injection {
                at: Duration::from_millis(240),
                host: LEAD.to_owned(),
                fault: Fault::Crash,
            },
            Injection {
                at: Duration::from_millis(480),
                host: LEAD.to_owned(),
                fault: Fault::Restart,
            },
            Injection {
                at: Duration::from_millis(550),
                host: LEAD.to_owned(),
                fault: Fault::Heal {
                    peer: WORKERS[0].to_owned(),
                },
            },
        ]
    };
    let checked = check_determinism([env.seed()], |seed| {
        let log = run_group_limits(seed, env.start_time(), env.folder(), sample()).unwrap();
        Replay::new(&log, &[])
    });
    assert_eq!(checked, Ok(()));
}

/// Covers: story:3-112, story:3-115
#[test]
fn group_limits_hold_under_partitions_and_lost_lead() {
    let env = TestEnv::new();
    let checked = generator(&env).check::<GroupLimitsFaults, _>(1..8, |plan| {
        run_group_limits(plan.seed, env.start_time(), env.folder(), injections(plan)?).map(drop)
    });
    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
