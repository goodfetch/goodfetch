//! The Usage ledger's Slices under engine crashes and restarts (ADRs 0041
//! and 0060): two engine hosts each run one run on one ledger, charging
//! requests out of the Slices it takes, journaling what it used of each
//! under its run epoch, and giving back what is left when it ends. A crash
//! leaves the run's Slices held; the next boot resumes the run under the
//! next epoch. Once every run has ended or died, an operator reclaims the
//! Slices of gone runs from their journals, as the next engine to open the
//! ledger does, and checks that the ledger charged exactly what the runs
//! used, holding nothing. The ledger's transitions, which the slices model
//! names, are the run's model trace: `just simulation` writes one per seed
//! and validates it with TLC (`test-support/models/slices/`).

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::ledger::{
    Charging, Holder, Ledger, Placement, Probed, SliceId, SliceSizing, TARGET,
};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::run::{LimitScope, LimitSetter};
use goodfetch_protocol::usage::{INSTALL, Limit, REQUESTS};
use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation};
use goodfetch_test_support::{
    CapturedLog, FaultPlan, FaultPlanAction, FaultPlanGenerator, FaultPlanStep, ObservationLog,
    Operation, Record, Replay, TestEnv, check_determinism, model_trace, model_traces,
    proptest_config_for,
};
use jiff::Timestamp;
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use serde::Serialize;
use tracing_subscriber::layer::SubscriberExt as _;

/// The scenario's name, which `just simulate` takes.
const SCENARIO: &str = "usage";
/// The version of its Fault plan generator.
const GENERATOR_VERSION: u32 = 1;
/// The model whose trace the scenario writes.
const MODEL: &str = "slices";
/// The engine hosts, each running the run of its name.
const ENGINES: [&str; 2] = ["engine-a", "engine-b"];
const OPERATOR: &str = "operator";
/// The Limit on the install's requests: its Slices hold 10 each.
const AMOUNT: u64 = 1000;
/// How many charges each run makes over all its epochs.
const CHARGES: u64 = 5;
/// When the operator reclaims: past every fault a plan injects (at most 7
/// steps of 150 ms) and every run a restart resumes (at most 5 charges of
/// 100 ms).
const SETTLED: Duration = Duration::from_millis(2500);

/// What a host saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    /// A run charged `amount` under `epoch`.
    Charged { epoch: u64, amount: u64 },
    /// A run gave back its Slices and ended.
    GaveBack { epoch: u64 },
    /// The operator reclaimed Slices.
    Reclaimed { charged: usize, for_review: usize },
    /// The Limit's Usage after the reclaim.
    Usage { charged: u64, held: u64 },
}

/// One run's state across its engine's boots: what its run state and its
/// writer lock would tell a reclaimer.
#[derive(Debug, Default)]
struct RunState {
    /// How many times its engine booted, the epoch of the latest boot.
    boots: u64,
    /// The epoch whose boot is running, which holds the run's writer lock.
    live: Option<u64>,
    /// Whether it gave back its Slices and ended.
    ended: bool,
    /// How many charges it made.
    charges: u64,
    /// What it used, in all.
    used: u64,
    /// What it used of each Slice, by the epoch it journaled it under.
    journal: BTreeMap<u64, BTreeMap<SliceId, u64>>,
}

type Runs = Rc<RefCell<BTreeMap<&'static str, RunState>>>;
type Log = Rc<RefCell<ObservationLog<Seen>>>;

/// The writer lock of a run's boot, let go when the boot ends or its
/// engine crashes.
struct WriterLock {
    runs: Runs,
    run: &'static str,
    epoch: u64,
}

impl Drop for WriterLock {
    fn drop(&mut self) {
        let mut runs = self.runs.borrow_mut();
        let state = runs.entry(self.run).or_default();
        if state.live == Some(self.epoch) {
            state.live = None;
        }
    }
}

/// One run of the scenario: its Observation log and its model trace.
struct Usage {
    log: ObservationLog<Seen>,
    trace: Vec<u8>,
}

/// What every host of one run shares.
#[derive(Clone)]
struct World {
    start: SystemTime,
    ledger: PathBuf,
    folder: PathBuf,
    runs: Runs,
    log: Log,
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
}

fn now(host: &Host) -> Timestamp {
    Timestamp::try_from(host.clock().wall()).unwrap()
}

fn install() -> LimitScope {
    LimitScope {
        kind: INSTALL.to_owned(),
        id: String::new(),
    }
}

/// Runs the scenario from `seed` in `folder`, with each fault of
/// `injections` at its moment.
fn usage(
    seed: u64,
    start: SystemTime,
    folder: &Path,
    injections: Vec<Injection>,
) -> Result<Usage, String> {
    let folder = tempfile::tempdir_in(folder).map_err(|error| error.to_string())?;
    let world = World {
        start,
        ledger: folder.path().join("usage.db"),
        folder: folder.path().to_owned(),
        runs: Rc::default(),
        log: Rc::new(RefCell::new(ObservationLog::new())),
    };
    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber =
        tracing_subscriber::registry().with(model_trace(&[TARGET], move || writer.clone()));
    let outcome = tracing::subscriber::with_default(subscriber, || {
        Ledger::at(world.ledger.clone(), None)
            .add_limit(
                &Limit {
                    scope: install(),
                    usage_kind: REQUESTS.to_owned(),
                    currency: None,
                    period: None,
                    amount: AMOUNT,
                    must_understand: true,
                    set_by: LimitSetter::Person,
                },
                Timestamp::try_from(start).unwrap(),
            )
            .map_err(|error| error.to_string())?;
        let mut simulation = Simulation::new(seed, start);
        for run in ENGINES {
            let world = world.clone();
            simulation.host(run, move |host| engine(host, run, world.clone()));
        }
        let operator = world.clone();
        simulation.client(OPERATOR, move |host| operate(host, operator));
        simulation
            .run_injecting(injections)
            .map_err(|error| error.to_string())
    });
    outcome?;
    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("the hosts are done with the log")
    };
    Ok(Usage {
        log: log.into_inner(),
        trace: captured.written(),
    })
}

/// An engine's boot: resumes its run under the next epoch unless the run
/// ended, charges what is left of its charges, each after a pause its
/// entropy draws, journaling what it used of each Slice, then gives back.
async fn engine(mut host: Host, run: &'static str, world: World) -> turmoil::Result {
    let epoch = {
        let mut runs = world.runs.borrow_mut();
        let state = runs.entry(run).or_default();
        if state.ended {
            return Ok(());
        }
        state.boots += 1;
        state.live = Some(state.boots);
        state.boots
    };
    let _lock = WriterLock {
        runs: Rc::clone(&world.runs),
        run,
        epoch,
    };
    let mut charging = Charging::new(
        Ledger::at(world.ledger.clone(), None),
        Holder {
            run_id: run.to_owned(),
            epoch,
            folder: world.folder.join(run),
        },
        Placement {
            product: Product::Foxhound,
            scopes: vec![install()],
        },
        SliceSizing::default(),
    );
    if epoch == 1 {
        charging.may_start(now(&host))?;
    } else {
        charging.resume(now(&host))?;
    }
    while world.runs.borrow()[run].charges < CHARGES {
        let pause = host.entropy().next_u64() % 100;
        tokio::time::sleep(Duration::from_millis(pause)).await;
        let amount = 1 + host.entropy().next_u64() % 3;
        let charged = charging.charge(REQUESTS, amount, now(&host))?;
        {
            let mut runs = world.runs.borrow_mut();
            let state = runs.entry(run).or_default();
            state.charges += 1;
            state.used += amount;
            state.journal.entry(epoch).or_default().extend(charged.used);
        }
        world.record(&host, run, Seen::Charged { epoch, amount });
    }
    charging.give_back()?;
    world.runs.borrow_mut().entry(run).or_default().ended = true;
    world.record(&host, run, Seen::GaveBack { epoch });
    Ok(())
}

/// What a reclaimer finds of the run that holds a Slice: live while the
/// boot of the Slice's epoch runs, else gone with what it journaled under
/// that epoch.
fn probe(runs: &BTreeMap<&'static str, RunState>, holder: &Holder) -> Probed {
    let Some(state) = runs.get(holder.run_id.as_str()) else {
        return Probed::Unreadable;
    };
    if state.live == Some(holder.epoch) {
        return Probed::Live;
    }
    Probed::Gone {
        used: state
            .journal
            .get(&holder.epoch)
            .cloned()
            .unwrap_or_default(),
        beyond: 0,
    }
}

/// The operator: once every run has ended or died, reclaims the Slices of
/// gone runs, then checks that the Limit holds nothing and charged what the
/// runs used.
async fn operate(host: Host, world: World) -> turmoil::Result {
    tokio::time::sleep(SETTLED).await;
    let ledger = Ledger::at(world.ledger.clone(), None);
    let reclaimed = ledger.reclaim(&|holder| probe(&world.runs.borrow(), holder))?;
    world.record(
        &host,
        OPERATOR,
        Seen::Reclaimed {
            charged: reclaimed.charged.len(),
            for_review: reclaimed.for_review.len(),
        },
    );
    let usage = ledger.usage(Product::Foxhound, now(&host))?;
    let [limit] = &usage[..] else {
        return Err(format!("the ledger holds one Limit, not {usage:?}").into());
    };
    world.record(
        &host,
        OPERATOR,
        Seen::Usage {
            charged: limit.charged,
            held: limit.held,
        },
    );
    if limit.held != 0 {
        return Err(format!("the Limit still holds {} after the reclaim", limit.held).into());
    }
    let used: u64 = world.runs.borrow().values().map(|state| state.used).sum();
    if limit.charged != used {
        return Err(format!(
            "the Limit charged {} for the {used} requests the runs used",
            limit.charged
        )
        .into());
    }
    Ok(())
}

/// Faults the seed draws: each engine may crash within its charging, and
/// a crashed one may restart, so runs die, resume and are reclaimed.
fn drawn_faults(seed: u64) -> Vec<Injection> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test draws its Simulation's faults from its seed (ADR 0060)"
    )]
    let mut entropy = Entropy::seeded(seed);
    let mut injections = Vec::new();
    for engine in ENGINES {
        if entropy.next_u64().is_multiple_of(2) {
            continue;
        }
        let crash = Duration::from_millis(entropy.next_u64() % 400);
        injections.push(Injection {
            at: crash,
            host: engine.to_owned(),
            fault: Fault::Crash,
        });
        if entropy.next_u64().is_multiple_of(2) {
            injections.push(Injection {
                at: crash + Duration::from_millis(entropy.next_u64() % 300),
                host: engine.to_owned(),
                fault: Fault::Restart,
            });
        }
    }
    injections
}

/// The scenario from `seed` under the faults it draws, which must succeed.
fn usage_of_seed(seed: u64, env: &TestEnv) -> Usage {
    usage(seed, env.start_time(), env.folder(), drawn_faults(seed))
        .unwrap_or_else(|error| panic!("the scenario of seed {seed} failed: {error}"))
}

/// Four seeds drawn from the test's own.
fn seeds(env: &TestEnv) -> Vec<u64> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test draws its Simulations' seeds from its own (ADR 0060)"
    )]
    let mut entropy = Entropy::seeded(env.seed());
    (0..4).map(|_| entropy.next_u64()).collect()
}

#[test]
fn the_same_seed_gives_the_same_observation_log_and_model_trace() {
    let env = TestEnv::new();

    let checked = check_determinism(seeds(&env), |seed| {
        let run = usage_of_seed(seed, &env);
        Replay::new(&run.log, &run.trace)
    });

    assert_eq!(checked, Ok(()));
}

/// The trace `just simulation` validates against the slices model: the
/// seed's run, kept in `GOODFETCH_MODEL_TRACES` when it names a folder.
#[test]
fn the_run_of_the_seed_writes_its_slices_trace() {
    let env = TestEnv::new();

    let run = usage_of_seed(env.seed(), &env);

    let trace = String::from_utf8(run.trace).unwrap();
    let transitions: Vec<String> = trace
        .lines()
        .map(|line| {
            let line: serde_json::Value = serde_json::from_str(line).unwrap();
            line["transition"].as_str().unwrap().to_owned()
        })
        .collect();
    assert_eq!(
        transitions.first().map(String::as_str),
        Some("set_limit"),
        "the model takes its Limit from the first line: {trace}"
    );
    assert!(
        transitions
            .iter()
            .any(|transition| transition == "take_slice"),
        "the runs take Slices: {trace}"
    );
    if let Some(folder) = model_traces() {
        keep(
            &folder.join(MODEL),
            &format!("{SCENARIO}-seed-{}.ndjson", env.seed()),
            &trace,
        );
    }
}

/// Writes `trace` as `name` in `folder`, for `just simulation` to validate.
#[expect(
    clippy::disallowed_methods,
    reason = "the test's own thread writes the trace once its Simulation has ended"
)]
fn keep(folder: &Path, name: &str, trace: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(folder.join(name), trace).unwrap();
}

/// Crashes and restarts of the engines, each at or after the one before.
struct EngineFaults;

impl ReferenceStateMachine for EngineFaults {
    /// The simulated moment of the last step, in milliseconds.
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        let now = *now;
        (
            0..150_u64,
            prop::sample::select(ENGINES.as_slice()),
            prop_oneof![Just(FaultPlanAction::Crash), Just(FaultPlanAction::Restart)],
        )
            .prop_map(move |(after, engine, action)| FaultPlanStep {
                at: now + after,
                host: engine.to_owned(),
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

/// The scenario's generator, which draws from the Simulation's Fault plan
/// stream of the test's seed, after the scenario's committed plans.
fn generator(env: &TestEnv) -> FaultPlanGenerator {
    let committed = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/it/simulation/fixtures/fault_plans")
        .join(SCENARIO);
    FaultPlanGenerator::new(env, SCENARIO, GENERATOR_VERSION)
        .with_committed_plans(&committed)
        .with_config(Config {
            // The written Fault plan is the regression to commit.
            failure_persistence: None,
            ..proptest_config_for(env.suite(), Simulation::fault_plan_seed(env.seed()))
        })
}

/// The plan's faults, as the Simulation injects them.
fn injections(plan: &FaultPlan) -> Result<Vec<Injection>, String> {
    plan.steps
        .iter()
        .map(|step| {
            let fault = match &step.action {
                FaultPlanAction::Crash => Fault::Crash,
                FaultPlanAction::Restart => Fault::Restart,
                other => return Err(format!("the usage scenario injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

#[test]
fn engine_crashes_and_restarts_fault_plans() {
    let env = TestEnv::new();

    let checked = generator(&env).check::<EngineFaults, _>(1..8, |plan| {
        usage(plan.seed, env.start_time(), env.folder(), injections(plan)?).map(drop)
    });

    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
