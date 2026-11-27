//! Sink delivery under engine crashes, a resumed copy and a concurrent zombie
//! copy against a compare-and-set destination (ADRs 0003, 0008, 0026 and 0060;
//! issue 1173).
//!
//! Host `primary` commits batches and delivers them through [`Driver`]; when
//! crashed and restarted by the Simulation, it resumes under the next run
//! epoch. Host `zombie` retains an older epoch (`1`) and keeps delivering
//! beside the resumed copy. Client `verifier` drains any remaining Records
//! under the latest epoch once faults settle, records every visible row at the
//! destination, and checks the Observation log with `fencing` for the Sink's
//! [`GuaranteeClass`].

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation};
use goodfetch_sink::{
    Batch, DeliveryId, Driver, Failure, GuaranteeClass, Primitives, Probe, RunCopy, Status, Written,
};
use goodfetch_test_support::{
    FaultPlan, FaultPlanAction, FaultPlanGenerator, FaultPlanStep, ObservationLog, Operation,
    Record, Replay, TestEnv, ValueDigest, check_determinism, fencing, proptest_config_for,
};
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use rusqlite::Connection;
use serde::Serialize;
use serde_json::{Value, json};

use crate::run_files::{commit, run_file};

/// The scenario's name, which `just simulate delivery` takes.
const SCENARIO: &str = "delivery";
/// The version of its Fault plan generator.
const GENERATOR_VERSION: u32 = 1;
/// The primary engine host that commits and delivers, crashing and resuming.
const PRIMARY: &str = "primary";
/// A stale copy of the Run (epoch 1) that keeps attempting writes after resume.
const ZOMBIE: &str = "zombie";
/// The verifier client that finishes recovery and checks fencing invariants.
const VERIFIER: &str = "verifier";
/// The Sink name.
const SINK: &str = "cas-warehouse";
/// The Dataset name.
const DATASET: &str = "items";
/// The Run ID.
const RUN_ID: &str = "sim-run";
/// Total committed Records across all batches.
const TOTAL_RECORDS: u64 = 12;
/// Wait past every fault injection so recovery and verification run on settled state.
const SETTLED: Duration = Duration::from_millis(2000);

/// What a host observed beside the standard `Fence`/`Commit`/`Write`/`Visible`
/// operations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    Booted { epoch: u64 },
    ZombieSuperseded { by_epoch: u64 },
    ZombieDelivered,
}

/// One row stored by the compare-and-set destination.
#[derive(Clone, Debug)]
struct Row {
    run_epoch: u64,
    key: String,
    number: u64,
    value: Value,
}

/// Compare-and-set destination enforcing the semantics of a [`GuaranteeClass`].
struct CasStore {
    class: GuaranteeClass,
    fenced_epoch: u64,
    rows: Vec<Row>,
    zombie_writes_rejected: u64,
}

impl CasStore {
    fn new(class: GuaranteeClass) -> Self {
        Self {
            class,
            fenced_epoch: 0,
            rows: Vec::new(),
            zombie_writes_rejected: 0,
        }
    }

    fn visible(&self) -> Vec<(u64, String, Value)> {
        self.rows
            .iter()
            .map(|row| (row.run_epoch, row.key.clone(), row.value.clone()))
            .collect()
    }
}

/// Per-copy handle onto the shared [`CasStore`], logging each attempted write.
struct CasSink {
    store: Rc<RefCell<CasStore>>,
    log: Rc<RefCell<ObservationLog<Seen>>>,
    actor: String,
}

impl Primitives for CasSink {
    type Error = Failure;

    fn probe(&mut self, run: &RunCopy) -> Result<Probe, Failure> {
        let store = self.store.borrow();
        let mut delivered = BTreeMap::new();
        for row in store.rows.iter().filter(|r| r.run_epoch == run.run_epoch) {
            let last = delivered.entry(DATASET.to_owned()).or_insert(0);
            *last = row.number.max(*last);
        }
        Ok(Probe {
            class: store.class,
            delivered,
        })
    }

    fn write(&mut self, batch: &Batch) -> Result<Written, Failure> {
        let run_epoch = batch.run.run_epoch;
        for record in &batch.records {
            self.log.borrow_mut().push(Record::new(
                self.actor.clone(),
                Operation::Write {
                    epoch: run_epoch,
                    key: record.delivery_id.as_str().to_owned(),
                    digest: digest(&record.value),
                },
            ));
        }
        let mut store = self.store.borrow_mut();
        match store.class {
            GuaranteeClass::Fenced => {
                // Compare-and-set on the destination's fenced epoch.
                if run_epoch < store.fenced_epoch {
                    store.zombie_writes_rejected += 1;
                    return Ok(Written::Superseded {
                        run_epoch: store.fenced_epoch,
                    });
                }
                if run_epoch > store.fenced_epoch {
                    store.fenced_epoch = run_epoch;
                    store.rows.retain(|row| row.run_epoch >= run_epoch);
                }
                for record in &batch.records {
                    let key = record.delivery_id.as_str().to_owned();
                    if store.rows.iter().any(|row| row.key == key) {
                        continue;
                    }
                    store.rows.push(Row {
                        run_epoch,
                        key,
                        number: record.number,
                        value: record.value.clone(),
                    });
                }
                Ok(Written::Delivered)
            }
            GuaranteeClass::RecordFenced => {
                for record in &batch.records {
                    let key = record.delivery_id.as_str().to_owned();
                    if let Some(existing) = store.rows.iter_mut().find(|row| row.key == key) {
                        if existing.run_epoch > run_epoch {
                            store.zombie_writes_rejected += 1;
                            continue;
                        }
                        *existing = Row {
                            run_epoch,
                            key,
                            number: record.number,
                            value: record.value.clone(),
                        };
                    } else {
                        store.rows.push(Row {
                            run_epoch,
                            key,
                            number: record.number,
                            value: record.value.clone(),
                        });
                    }
                }
                Ok(Written::Delivered)
            }
            GuaranteeClass::Keyed => {
                for record in &batch.records {
                    let key = record.delivery_id.as_str().to_owned();
                    let row = Row {
                        run_epoch,
                        key: key.clone(),
                        number: record.number,
                        value: record.value.clone(),
                    };
                    if let Some(existing) = store.rows.iter_mut().find(|r| r.key == key) {
                        *existing = row;
                    } else {
                        store.rows.push(row);
                    }
                }
                Ok(Written::Delivered)
            }
            GuaranteeClass::AtLeastOnce => {
                for record in &batch.records {
                    store.rows.push(Row {
                        run_epoch,
                        key: record.delivery_id.as_str().to_owned(),
                        number: record.number,
                        value: record.value.clone(),
                    });
                }
                Ok(Written::Delivered)
            }
        }
    }

    fn map_failure(&self, error: Failure) -> Failure {
        error
    }
}

/// Shared state across the simulated hosts of one run.
#[derive(Clone)]
struct World {
    class: GuaranteeClass,
    run_file: Rc<RefCell<Connection>>,
    committed: Rc<RefCell<u64>>,
    boots: Rc<RefCell<u64>>,
    store: Rc<RefCell<CasStore>>,
    log: Rc<RefCell<ObservationLog<Seen>>>,
}

impl World {
    fn new(class: GuaranteeClass) -> Self {
        Self {
            class,
            run_file: Rc::new(RefCell::new(run_file(&[]))),
            committed: Rc::new(RefCell::new(0)),
            boots: Rc::new(RefCell::new(0)),
            store: Rc::new(RefCell::new(CasStore::new(class))),
            log: Rc::new(RefCell::new(ObservationLog::new())),
        }
    }

    fn commit_next(&self, count: u64) {
        let mut committed = self.committed.borrow_mut();
        let remaining = TOTAL_RECORDS.saturating_sub(*committed);
        let to_add = count.min(remaining);
        if to_add == 0 {
            return;
        }
        let first = *committed + 1;
        let values: Vec<Value> = (first..first + to_add)
            .map(|n| json!({ "n": n, "sku": format!("sku-{n}") }))
            .collect();
        {
            let conn = self.run_file.borrow();
            commit(&conn, DATASET, i64::try_from(first).unwrap(), &values);
        }
        for (offset, value) in values.iter().enumerate() {
            let number = first + offset as u64;
            self.log.borrow_mut().push(Record::new(
                "run-file",
                Operation::Commit {
                    key: DeliveryId::new(RUN_ID, SINK, DATASET, number)
                        .as_str()
                        .to_owned(),
                    digest: digest(value),
                },
            ));
        }
        *committed += to_add;
    }

    fn driver(&self, epoch: u64, actor: &str) -> Driver<CasSink> {
        let sink = CasSink {
            store: Rc::clone(&self.store),
            log: Rc::clone(&self.log),
            actor: actor.to_owned(),
        };
        Driver::new(
            SINK,
            sink,
            RunCopy {
                run_id: RUN_ID.to_owned(),
                run_epoch: epoch,
            },
            vec![DATASET.to_owned()],
        )
        .with_batch_records(2)
    }
}

fn digest(value: &Value) -> ValueDigest {
    ValueDigest::of(value.to_string())
}

fn judged(class: GuaranteeClass) -> goodfetch_test_support::GuaranteeClass {
    match class {
        GuaranteeClass::Fenced => goodfetch_test_support::GuaranteeClass::Fenced,
        GuaranteeClass::RecordFenced => goodfetch_test_support::GuaranteeClass::RecordFenced,
        GuaranteeClass::Keyed => goodfetch_test_support::GuaranteeClass::Keyed,
        GuaranteeClass::AtLeastOnce => goodfetch_test_support::GuaranteeClass::AtLeastOnce,
    }
}

/// Primary engine host: each boot increments the run epoch, logs a Fence, and
/// interleaves commits and deliveries until all Records are delivered.
async fn primary(mut host: Host, world: World) -> turmoil::Result {
    let epoch = {
        let mut boots = world.boots.borrow_mut();
        *boots += 1;
        *boots
    };
    let actor = format!("copy-{epoch}");
    world
        .log
        .borrow_mut()
        .push(Record::new(actor.clone(), Operation::Fence { epoch }));
    world.log.borrow_mut().push(Record::new(
        actor.clone(),
        Operation::Other(Seen::Booted { epoch }),
    ));
    let mut driver = world.driver(epoch, &actor);
    while *world.committed.borrow() < TOTAL_RECORDS {
        let pause = 20 + (host.entropy().next_u64() % 40);
        tokio::time::sleep(Duration::from_millis(pause)).await;
        world.commit_next(3);
        let status = driver.deliver(&world.run_file.borrow())?;
        if matches!(status, Status::Superseded { .. }) {
            return Ok(());
        }
    }
    let _ = driver.deliver(&world.run_file.borrow())?;
    Ok(())
}

/// Zombie copy holding epoch 1 that keeps attempting deliveries after the
/// primary crashes and resumes under a higher epoch.
async fn zombie(mut host: Host, world: World) -> turmoil::Result {
    let mut driver = world.driver(1, "zombie-1");
    for _ in 0..6 {
        let pause = 60 + (host.entropy().next_u64() % 80);
        tokio::time::sleep(Duration::from_millis(pause)).await;
        if *world.committed.borrow() == 0 {
            continue;
        }
        match driver.deliver(&world.run_file.borrow())? {
            Status::Superseded { run_epoch } => {
                world.log.borrow_mut().push(Record::new(
                    "zombie-1",
                    Operation::Other(Seen::ZombieSuperseded {
                        by_epoch: run_epoch,
                    }),
                ));
                return Ok(());
            }
            Status::Complete | Status::CompleteWithRejections => {
                world.log.borrow_mut().push(Record::new(
                    "zombie-1",
                    Operation::Other(Seen::ZombieDelivered),
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

/// Verifier client: waits for faults to settle, finishes any remaining commits
/// and delivery under a fresh recovery epoch, records visible rows and runs
/// the fencing checker for the Sink's Guarantee class.
async fn verifier(_host: Host, world: World) -> turmoil::Result {
    tokio::time::sleep(SETTLED).await;
    world.commit_next(TOTAL_RECORDS);
    let final_epoch = {
        let mut boots = world.boots.borrow_mut();
        *boots += 1;
        *boots
    };
    let actor = format!("copy-{final_epoch}");
    world.log.borrow_mut().push(Record::new(
        actor.clone(),
        Operation::Fence { epoch: final_epoch },
    ));
    let mut recovery = world.driver(final_epoch, &actor);
    let status = recovery.deliver(&world.run_file.borrow())?;
    if status != Status::Complete {
        return Err(format!("final recovery did not complete: {status:?}").into());
    }
    // One last zombie attempt after final recovery to confirm fencing holds.
    let mut late_zombie = world.driver(1, "zombie-late");
    let zombie_status = late_zombie.deliver(&world.run_file.borrow())?;
    if world.class == GuaranteeClass::Fenced
        && zombie_status
            != (Status::Superseded {
                run_epoch: final_epoch,
            })
    {
        return Err(
            format!("Fenced CAS destination allowed late zombie write: {zombie_status:?}").into(),
        );
    }
    let visible = world.store.borrow().visible();
    if world.class == GuaranteeClass::Fenced {
        if visible.len() != usize::try_from(TOTAL_RECORDS).unwrap() {
            return Err(format!(
                "Fenced destination held {} rows, expected {TOTAL_RECORDS}",
                visible.len()
            )
            .into());
        }
        if let Some((epoch, key, _)) = visible.iter().find(|(e, _, _)| *e != final_epoch) {
            return Err(format!(
                "zombie write from epoch {epoch} landed for {key} under Fenced epoch {final_epoch}"
            )
            .into());
        }
    }
    for (run_epoch, key, value) in visible {
        world.log.borrow_mut().push(Record::new(
            "destination",
            Operation::Visible {
                epoch: run_epoch,
                key,
                digest: digest(&value),
            },
        ));
    }
    fencing(&world.log.borrow(), judged(world.class)).map_err(|error| error.to_string())?;
    Ok(())
}

/// Runs one seeded delivery Simulation for `class` with `injections`.
fn run_delivery(
    seed: u64,
    start: SystemTime,
    class: GuaranteeClass,
    injections: Vec<Injection>,
) -> Result<ObservationLog<Seen>, String> {
    let world = World::new(class);
    let mut simulation = Simulation::new(seed, start);
    let p_world = world.clone();
    simulation.host(PRIMARY, move |host| primary(host, p_world.clone()));
    let z_world = world.clone();
    simulation.host(ZOMBIE, move |host| zombie(host, z_world.clone()));
    let v_world = world.clone();
    simulation.client(VERIFIER, move |host| verifier(host, v_world));
    simulation
        .run_injecting(injections)
        .map_err(|error| error.to_string())?;
    drop(simulation);
    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("simulation hosts finished with the ObservationLog");
    };
    Ok(log.into_inner())
}

/// A seeded Simulation kills the Run mid-delivery and resumes it while a
/// zombie copy keeps delivering to a compare-and-set destination; judged by
/// the fencing checker for every Guarantee class (ADRs 0026 and 0060).
///
/// Covers: story:6-285
#[rstest::rstest]
fn crash_resume_and_zombie_pass_fencing_checker(
    #[values(
        GuaranteeClass::Fenced,
        GuaranteeClass::RecordFenced,
        GuaranteeClass::Keyed,
        GuaranteeClass::AtLeastOnce
    )]
    class: GuaranteeClass,
) {
    let env = TestEnv::new();
    let injections = vec![
        Injection {
            at: Duration::from_millis(75),
            host: PRIMARY.to_owned(),
            fault: Fault::Crash,
        },
        Injection {
            at: Duration::from_millis(140),
            host: PRIMARY.to_owned(),
            fault: Fault::Restart,
        },
        Injection {
            at: Duration::from_millis(260),
            host: PRIMARY.to_owned(),
            fault: Fault::Crash,
        },
        Injection {
            at: Duration::from_millis(340),
            host: PRIMARY.to_owned(),
            fault: Fault::Restart,
        },
    ];
    let log = run_delivery(env.seed(), env.start_time(), class, injections).unwrap();
    assert!(
        log.records()
            .iter()
            .any(|r| matches!(r.operation, Operation::Fence { epoch } if epoch >= 2)),
        "expected resume under epoch >= 2"
    );
}

/// Running the delivery scenario twice from the same seed replays the exact
/// same Observation log (ADR 0060).
///
/// Covers: story:6-285
#[test]
fn seed_replay_is_deterministic() {
    let env = TestEnv::new();
    let injections = vec![
        Injection {
            at: Duration::from_millis(80),
            host: PRIMARY.to_owned(),
            fault: Fault::Crash,
        },
        Injection {
            at: Duration::from_millis(160),
            host: PRIMARY.to_owned(),
            fault: Fault::Restart,
        },
    ];
    let seeds = [env.seed(), env.seed().wrapping_add(1)];
    let checked = check_determinism(seeds, |seed| {
        let log = run_delivery(
            seed,
            env.start_time(),
            GuaranteeClass::Fenced,
            injections.clone(),
        )
        .unwrap();
        Replay::new(&log, &[])
    });
    assert_eq!(checked, Ok(()));
}

/// Crash/restart fault plans against `primary` while `zombie` keeps writing.
struct DeliveryFaults;

impl ReferenceStateMachine for DeliveryFaults {
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        let now = *now;
        (
            10..150_u64,
            prop_oneof![Just(FaultPlanAction::Crash), Just(FaultPlanAction::Restart)],
        )
            .prop_map(move |(after, action)| FaultPlanStep {
                at: now + after,
                host: PRIMARY.to_owned(),
                action,
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
                FaultPlanAction::Crash => Fault::Crash,
                FaultPlanAction::Restart => Fault::Restart,
                other => return Err(format!("the delivery scenario injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Generated crash/restart fault plans keep the compare-and-set destination
/// fenced for every Guarantee class; any failing plan replays from its seed or
/// `--plan` file (ADR 0060).
///
/// Covers: story:6-285
#[test]
fn delivery_crashes_and_zombie_fault_plans() {
    let env = TestEnv::new();
    let checked = generator(&env).check::<DeliveryFaults, String>(1..6, |plan| {
        let inj = injections(plan)?;
        for class in [
            GuaranteeClass::Fenced,
            GuaranteeClass::RecordFenced,
            GuaranteeClass::Keyed,
            GuaranteeClass::AtLeastOnce,
        ] {
            run_delivery(plan.seed, env.start_time(), class, inj.clone())?;
        }
        Ok(())
    });
    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
