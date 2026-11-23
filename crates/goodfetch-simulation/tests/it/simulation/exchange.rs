//! Two hosts exchange messages through turmoil's adapter of `transport`:
//! the server listens as a Daemon does, on the in-memory network the
//! adapter feeds, and the client dials it and records each message it sends
//! and each acknowledgement it reads in an Observation log. The client
//! neither reconnects nor resends, so a fault that loses a message or the
//! server fails the exchange: the failure the Fault plan generator finds.

use std::cell::RefCell;
use std::collections::HashSet;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::entropy::Entropy;
use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation, connect, listen};
use goodfetch_test_support::{
    CapturedLog, FaultPlan, FaultPlanAction, FaultPlanFailure, FaultPlanGenerator, FaultPlanStep,
    ObservationLog, Operation, Record, Replay, ReplayOutput, TestEnv, check_determinism,
    proptest_config_for, subscriber_writing_to,
};
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use serde::Serialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

/// The scenario's name, which `just simulate` takes.
const SCENARIO: &str = "exchange";
/// The version of its Fault plan generator.
const GENERATOR_VERSION: u32 = 1;
const SERVER: &str = "server";
const CLIENT: &str = "client";
const PORT: u16 = 7070;
/// How many messages a run of a Fault plan sends.
const MESSAGES: u64 = 5;
/// How long the client waits for each acknowledgement.
const ACKNOWLEDGEMENT_WAIT: Duration = Duration::from_secs(2);

/// What the client saw.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Exchanged {
    Sent(u64),
    Acknowledged(u64),
    /// A value the client read from outside the seed, a planted leak.
    Leaked(String),
}

/// Where a run of the exchange reads from outside its seed.
#[derive(Debug, Clone, Copy)]
enum Leak {
    None,
    /// The client records the OS's wall time.
    RealTime,
    /// The client records the order of a `std` hash set, which each set
    /// keys anew.
    HashOrder,
}

/// One run of the exchange: its Observation log and its `tracing` output.
struct Exchange {
    log: ObservationLog<Exchanged>,
    tracing: Vec<u8>,
}

/// Runs the exchange of `messages` messages from `seed`, with each fault
/// of `injections` at its moment.
fn exchange(
    seed: u64,
    start: SystemTime,
    messages: u64,
    leak: Leak,
    injections: Vec<Injection>,
) -> Result<Exchange, String> {
    let log = Rc::new(RefCell::new(ObservationLog::new()));
    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber = subscriber_writing_to(
        move || start + turmoil::sim_elapsed().unwrap_or_default(),
        move || writer.clone(),
    );
    let outcome = tracing::subscriber::with_default(subscriber, || {
        let mut simulation = Simulation::new(seed, start);
        simulation.host(SERVER, serve);
        let client_log = Rc::clone(&log);
        simulation.client(CLIENT, move |host| {
            send(host, start, messages, leak, client_log)
        });
        simulation.run_injecting(injections)
    });
    outcome.map_err(|error| error.to_string())?;
    let Ok(log) = Rc::try_unwrap(log) else {
        panic!("the client is done with its log")
    };
    Ok(Exchange {
        log: log.into_inner(),
        tracing: captured.written(),
    })
}

/// [`exchange`] with no fault, which must succeed.
fn exchanged(seed: u64, start: SystemTime, messages: u64, leak: Leak) -> Exchange {
    exchange(seed, start, messages, leak, Vec::new())
        .unwrap_or_else(|error| panic!("the exchange of seed {seed} failed: {error}"))
}

/// The server: a Daemon's listener on the network the adapter feeds, which
/// sends back each line it reads.
async fn serve(host: Host) -> turmoil::Result {
    let network = listen(host.owner(), PORT).await?;
    let (mut listener, _) = network
        .listen_at(SocketAddr::from((Ipv4Addr::UNSPECIFIED, PORT)))
        .await?;
    loop {
        let (reader, mut writer) = listener.accept().await?.split();
        let mut lines = BufReader::new(reader).lines();
        while let Some(line) = lines.next_line().await? {
            tracing::info!(message = %line, "acknowledged");
            writer.write_all(format!("{line}\n").as_bytes()).await?;
        }
    }
}

/// The client: sends `messages` lines, each after a pause its entropy
/// draws, and records each send and each acknowledgement.
async fn send(
    mut host: Host,
    start: SystemTime,
    messages: u64,
    leak: Leak,
    log: Rc<RefCell<ObservationLog<Exchanged>>>,
) -> turmoil::Result {
    let clock = *host.clock();
    let record = |seen: Exchanged| {
        let at = clock.wall().duration_since(start).unwrap_or_default();
        let at = u64::try_from(at.as_millis()).unwrap_or(u64::MAX);
        log.borrow_mut()
            .push(Record::new(CLIENT, Operation::Other(seen)).at(at));
    };
    let (reader, mut writer) = connect(host.owner(), (SERVER, PORT)).await?.split();
    let mut lines = BufReader::new(reader).lines();
    match leak {
        Leak::None => {}
        Leak::RealTime => {
            #[expect(
                clippy::disallowed_methods,
                reason = "the leak of real time this test plants"
            )]
            let now = SystemTime::now();
            record(Exchanged::Leaked(format!("{now:?}")));
        }
        Leak::HashOrder => {
            let names: HashSet<String> = (0..16).map(|name| format!("host-{name}")).collect();
            record(Exchanged::Leaked(
                names.into_iter().collect::<Vec<_>>().join(","),
            ));
        }
    }
    for message in 0..messages {
        let pause = host.entropy().next_u64() % 50;
        tokio::time::sleep(Duration::from_millis(pause)).await;
        writer.write_all(format!("{message}\n").as_bytes()).await?;
        record(Exchanged::Sent(message));
        let line = tokio::time::timeout(ACKNOWLEDGEMENT_WAIT, lines.next_line())
            .await
            .map_err(|_| {
                format!(
                    "the server acknowledged no message {message} within {} s",
                    ACKNOWLEDGEMENT_WAIT.as_secs()
                )
            })??
            .ok_or("the server closed the connection")?;
        record(Exchanged::Acknowledged(line.parse()?));
    }
    Ok(())
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

fn replay(exchange: &Exchange) -> Replay {
    Replay::new(&exchange.log, &exchange.tracing)
}

#[test]
fn two_hosts_exchange_messages_through_the_adapter() {
    let env = TestEnv::new();

    let run = exchanged(env.seed(), env.start_time(), 3, Leak::None);

    let seen: Vec<_> = run
        .log
        .records()
        .iter()
        .map(|record| match &record.operation {
            Operation::Other(seen) => seen.clone(),
            other => panic!("the client records only what it exchanged, not {other:?}"),
        })
        .collect();
    assert_eq!(
        seen,
        [
            Exchanged::Sent(0),
            Exchanged::Acknowledged(0),
            Exchanged::Sent(1),
            Exchanged::Acknowledged(1),
            Exchanged::Sent(2),
            Exchanged::Acknowledged(2),
        ]
    );
    let traced = String::from_utf8_lossy(&run.tracing);
    assert_eq!(
        traced.matches("acknowledged").count(),
        3,
        "the server traces each line it sends back: {traced}"
    );
}

#[test]
fn the_same_seed_gives_the_same_observation_log_and_tracing_output() {
    let env = TestEnv::new();

    let checked = check_determinism(seeds(&env), |seed| {
        replay(&exchanged(seed, env.start_time(), MESSAGES, Leak::None))
    });

    assert_eq!(checked, Ok(()));
}

#[test]
fn another_seed_gives_another_run() {
    let env = TestEnv::new();
    let [one, other, ..] = seeds(&env)[..] else {
        unreachable!("four seeds")
    };

    let first = replay(&exchanged(one, env.start_time(), MESSAGES, Leak::None));
    let second = replay(&exchanged(other, env.start_time(), MESSAGES, Leak::None));

    assert_ne!(first, second, "the seed drives the pauses and the network");
}

#[test]
fn a_leak_of_real_time_is_found_at_the_record_that_read_it() {
    let env = TestEnv::new();

    let checked = check_determinism(seeds(&env), |seed| {
        replay(&exchanged(seed, env.start_time(), 2, Leak::RealTime))
    });

    let divergence = checked.expect_err("the OS's wall time differs between two runs");
    assert_eq!(
        (divergence.output, divergence.line),
        (ReplayOutput::ObservationLog, 1),
        "{divergence}"
    );
    assert!(divergence.to_string().contains("leaked"), "{divergence}");
}

#[test]
fn a_leak_of_hash_order_is_found_at_the_record_that_read_it() {
    let env = TestEnv::new();

    let checked = check_determinism(seeds(&env), |seed| {
        replay(&exchanged(seed, env.start_time(), 2, Leak::HashOrder))
    });

    let divergence = checked.expect_err("each hash set iterates in its own order");
    assert_eq!(
        (divergence.output, divergence.line),
        (ReplayOutput::ObservationLog, 1),
        "{divergence}"
    );
}

/// Faults on the server, each at or after the one before it: only links
/// slowed when `SLOW_ONLY`, else crashes, restarts, partitions, heals and
/// slowed links.
struct ServerFaults<const SLOW_ONLY: bool>;

impl<const SLOW_ONLY: bool> ReferenceStateMachine for ServerFaults<SLOW_ONLY> {
    /// The simulated moment of the last step, in milliseconds.
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        let now = *now;
        let peer = || CLIENT.to_owned();
        let slow = (0..500_u64).prop_map(move |milliseconds| FaultPlanAction::Latency {
            peer: peer(),
            milliseconds,
        });
        let action = if SLOW_ONLY {
            slow.boxed()
        } else {
            prop_oneof![
                Just(FaultPlanAction::Crash),
                Just(FaultPlanAction::Restart),
                Just(FaultPlanAction::Partition { peer: peer() }),
                Just(FaultPlanAction::Heal { peer: peer() }),
                slow,
            ]
            .boxed()
        };
        (0..100_u64, action)
            .prop_map(move |(after, action)| FaultPlanStep {
                at: now + after,
                host: SERVER.to_owned(),
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
/// stream of the test's seed.
fn generator(env: &TestEnv) -> FaultPlanGenerator {
    FaultPlanGenerator::new(env, SCENARIO, GENERATOR_VERSION).with_config(Config {
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
                FaultPlanAction::Partition { peer } => Fault::Partition { peer: peer.clone() },
                FaultPlanAction::Heal { peer } => Fault::Heal { peer: peer.clone() },
                FaultPlanAction::Latency { peer, milliseconds } => Fault::Latency {
                    peer: peer.clone(),
                    delay: Duration::from_millis(*milliseconds),
                },
                other => return Err(format!("the exchange injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Runs the exchange under `plan`, from the plan's seed.
fn run_plan(plan: &FaultPlan, start: SystemTime) -> Result<(), String> {
    exchange(plan.seed, start, MESSAGES, Leak::None, injections(plan)?).map(|_| ())
}

#[test]
fn slowed_links_fault_plans() {
    let env = TestEnv::new();

    let checked = generator(&env)
        .check::<ServerFaults<true>, _>(1..8, |plan| run_plan(plan, env.start_time()));

    if let Err(failure) = checked {
        panic!("{failure}");
    }
}

#[test]
fn a_lost_message_or_server_shrinks_to_one_fault_written_with_its_replay_command() {
    let env = TestEnv::new();

    let failure = generator(&env)
        .check::<ServerFaults<false>, _>(1..8, |plan| run_plan(plan, env.start_time()))
        .expect_err("a crash or a partition fails the exchange");

    let FaultPlanFailure::Shrunk {
        plan,
        reason,
        file,
        command,
        ..
    } = failure
    else {
        panic!("the shrunk plan is not written: {failure}");
    };
    let [step] = &plan.steps[..] else {
        panic!("one fault fails the exchange: {plan:?}");
    };
    assert!(
        matches!(
            step.action,
            FaultPlanAction::Crash | FaultPlanAction::Restart | FaultPlanAction::Partition { .. }
        ),
        "a slowed link or a heal alone fails nothing: {step:?}"
    );
    assert_eq!(
        command,
        format!("just simulate {SCENARIO} --plan '{}'", file.display())
    );
    let replayed = generator(&env)
        .replaying(&file)
        .check::<ServerFaults<false>, _>(1..8, |plan| run_plan(plan, env.start_time()))
        .expect_err("the written plan fails again");
    assert_eq!(
        replayed.to_string(),
        format!(
            "the scenario {SCENARIO} fails on the Fault plan {}: {reason}",
            file.display()
        )
    );
}

#[test]
fn a_committed_plan_replays_the_same_failure() {
    let env = TestEnv::new();
    let committed = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/it/simulation/fixtures/fault_plans/server_crash.json");
    let replay = || {
        generator(&env)
            .replaying(&committed)
            .check::<ServerFaults<false>, _>(1..8, |plan| run_plan(plan, env.start_time()))
            .expect_err("the server's crash fails the exchange")
            .to_string()
    };

    let first = replay();
    let second = replay();

    assert_eq!(first, second, "a plan replays its failure exactly");
    assert_eq!(
        first,
        format!(
            "the scenario {SCENARIO} fails on the Fault plan {}: the server closed the connection",
            committed.display()
        )
    );
}
