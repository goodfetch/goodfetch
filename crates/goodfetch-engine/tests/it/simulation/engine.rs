//! The engine's process (`EngineState`) and Connection (`ConnectionState`)
//! transition tables under crashes and lost connections (ADRs 0060 and 0128):
//! a real Daemon runs on `turmoil` through `transport`'s in-memory network
//! while a Client opens Connections, sends valid and invalid JSON-RPC frames,
//! loses Connections across partitions and abrupt closes, and rides through a
//! Daemon crash, restart and drain. Every transition emitted by the Daemon and
//! its Connections is checked step-by-step against
//! `goodfetch_engine::transition::engine::{engine, connection}`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use goodfetch_engine::daemon::{DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::ops::Ops;
use goodfetch_engine::politeness::PolitenessOptions;
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::secrets::Keychain;
use goodfetch_engine::tasks::BlockingPolicy;
use goodfetch_engine::transition::engine::{
    self, ConnectionEffect, ConnectionEvent, ConnectionState, EngineEvent, EngineState,
};
use goodfetch_engine::{Channel, Composition, Dependencies, Engine, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation, connect, listen};
use goodfetch_test_support::{
    FaultPlan, FaultPlanAction, FaultPlanGenerator, FaultPlanStep, ObservationLog, Operation,
    Record, Replay, TestEnv, check_determinism, proptest_config_for,
};
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio_util::sync::CancellationToken;
use tracing::field::{Field, Visit};
use tracing_subscriber::Layer;
use tracing_subscriber::layer::SubscriberExt as _;

/// The scenario's name, which `just simulate` takes.
const SCENARIO: &str = "engine";
/// The version of its Fault plan generator.
const GENERATOR_VERSION: u32 = 1;
const DAEMON: &str = "daemon";
const CLIENT: &str = "client";
const PORT: u16 = 7474;
const TOKEN: &str = "simulation-launch-token";
/// Timeout for a single client read/connect step so partitioned or crashed
/// links are observed as lost connections without stalling the simulation.
const RPC_TIMEOUT: Duration = Duration::from_millis(80);
/// Time after which fault plans finish injecting (at most 5 steps of 120 ms).
const SETTLED: Duration = Duration::from_millis(650);

/// A single transition emitted by the real Daemon or Connection under test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "machine", rename_all = "snake_case")]
enum TransitionStep {
    Engine {
        from: String,
        event: String,
        to: String,
    },
    Connection {
        from: String,
        event: String,
        to: String,
        effect: String,
    },
}

/// Deterministic observations recorded for replay and determinism checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    Boot(u64),
    Transition(TransitionStep),
    ConnectionLost,
    Drained,
}

/// Shared state captured by the custom tracing layer during one simulation run.
#[expect(
    clippy::disallowed_types,
    reason = "single-threaded simulation tracing layer collector; no lock order to test under loom (ADR 0060)"
)]
#[derive(Clone, Default)]
struct Capture {
    transitions: Arc<Mutex<Vec<TransitionStep>>>,
    bug_refusals: Arc<Mutex<Vec<String>>>,
}

struct FieldCollector {
    machine: Option<String>,
    from: Option<String>,
    event: Option<String>,
    to: Option<String>,
    effect: Option<String>,
    message: Option<String>,
}

impl FieldCollector {
    fn new() -> Self {
        Self {
            machine: None,
            from: None,
            event: None,
            to: None,
            effect: None,
            message: None,
        }
    }
}

impl Visit for FieldCollector {
    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        let formatted = format!("{value:?}");
        match field.name() {
            "from" => self.from = Some(formatted),
            "event" => self.event = Some(formatted),
            "to" => self.to = Some(formatted),
            "effect" => self.effect = Some(formatted),
            "message" => self.message = Some(formatted),
            _ => {}
        }
    }

    fn record_str(&mut self, field: &Field, value: &str) {
        match field.name() {
            "machine" => self.machine = Some(value.to_owned()),
            "message" => self.message = Some(value.to_owned()),
            _ => {}
        }
    }
}

impl<S: tracing::Subscriber> Layer<S> for Capture {
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        let meta = event.metadata();
        let mut fields = FieldCollector::new();
        event.record(&mut fields);
        if meta.target() == "goodfetch::transition" {
            match fields.machine.as_deref() {
                Some("engine") => {
                    if let (Some(from), Some(event), Some(to)) =
                        (fields.from, fields.event, fields.to)
                    {
                        self.transitions
                            .lock()
                            .unwrap()
                            .push(TransitionStep::Engine { from, event, to });
                    }
                }
                Some("connection") => {
                    if let (Some(from), Some(event), Some(to), Some(effect)) =
                        (fields.from, fields.event, fields.to, fields.effect)
                    {
                        self.transitions
                            .lock()
                            .unwrap()
                            .push(TransitionStep::Connection {
                                from,
                                event,
                                to,
                                effect,
                            });
                    }
                }
                _ => {}
            }
        } else if *meta.level() == tracing::Level::ERROR
            && fields
                .message
                .as_deref()
                .is_some_and(|msg| msg.contains("an invalid transition was refused"))
        {
            self.bug_refusals
                .lock()
                .unwrap()
                .push(format!("{:?} -> {:?}", fields.from, fields.event));
        }
    }
}

struct AlwaysPick;

impl Succession for AlwaysPick {
    fn is_pick(&self) -> bool {
        true
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        Box::new(())
    }
}

type Log = Rc<RefCell<ObservationLog<Seen>>>;

#[derive(Clone)]
struct World {
    seed: u64,
    start: SystemTime,
    home: PathBuf,
    boots: Rc<RefCell<u64>>,
    live_drain: Rc<RefCell<Option<CancellationToken>>>,
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

/// Drops the live drain handle when the Daemon host crashes or exits.
struct DrainGuard {
    live_drain: Rc<RefCell<Option<CancellationToken>>>,
}

impl Drop for DrainGuard {
    fn drop(&mut self) {
        *self.live_drain.borrow_mut() = None;
    }
}

struct EngineRun {
    log: ObservationLog<Seen>,
    transitions: Vec<TransitionStep>,
    boots: u64,
}

fn parse_engine_state(name: &str) -> Result<EngineState, String> {
    match name {
        "Attached" => Ok(EngineState::Attached),
        "Serving" => Ok(EngineState::Serving),
        "HandingOver" => Ok(EngineState::HandingOver),
        "Draining" => Ok(EngineState::Draining),
        "Stopping" => Ok(EngineState::Stopping),
        other => Err(format!("unknown EngineState `{other}`")),
    }
}

fn parse_engine_event(name: &str) -> Result<EngineEvent, String> {
    match name {
        "ClientLost" => Ok(EngineEvent::ClientLost),
        "StdioEnded" => Ok(EngineEvent::StdioEnded),
        "ParentGone" => Ok(EngineEvent::ParentGone),
        "Idle" => Ok(EngineEvent::Idle),
        "StopAsked" => Ok(EngineEvent::StopAsked),
        "HandOver" => Ok(EngineEvent::HandOver),
        "Drain" => Ok(EngineEvent::Drain),
        "Drained" => Ok(EngineEvent::Drained),
        "ListenerFailed" => Ok(EngineEvent::ListenerFailed),
        "PartPanicked" => Ok(EngineEvent::PartPanicked),
        "RegistryPanicked" => Ok(EngineEvent::RegistryPanicked),
        other => Err(format!("unknown EngineEvent `{other}`")),
    }
}

fn parse_connection_state(name: &str) -> Result<ConnectionState, String> {
    match name {
        "Opening" => Ok(ConnectionState::Opening),
        "Open" => Ok(ConnectionState::Open),
        "Closing" => Ok(ConnectionState::Closing),
        "Closed" => Ok(ConnectionState::Closed),
        other => Err(format!("unknown ConnectionState `{other}`")),
    }
}

fn parse_connection_event(name: &str) -> Result<ConnectionEvent, String> {
    match name {
        "Hello" => Ok(ConnectionEvent::Hello),
        "Request" => Ok(ConnectionEvent::Request),
        "Notification" => Ok(ConnectionEvent::Notification),
        "Reply" => Ok(ConnectionEvent::Reply),
        "Unreadable" => Ok(ConnectionEvent::Unreadable),
        "Accepted" => Ok(ConnectionEvent::Accepted),
        "Rejected" => Ok(ConnectionEvent::Rejected),
        "ClientLost" => Ok(ConnectionEvent::ClientLost),
        "CallsEnded" => Ok(ConnectionEvent::CallsEnded),
        other => Err(format!("unknown ConnectionEvent `{other}`")),
    }
}

fn parse_connection_effect(name: &str) -> Result<ConnectionEffect, String> {
    match name {
        "Handshake" => Ok(ConnectionEffect::Handshake),
        "Serve" => Ok(ConnectionEffect::Serve),
        "Answer" => Ok(ConnectionEffect::Answer),
        "Refuse" => Ok(ConnectionEffect::Refuse),
        "CancelCalls" => Ok(ConnectionEffect::CancelCalls),
        "Nothing" => Ok(ConnectionEffect::Nothing),
        other => Err(format!("unknown ConnectionEffect `{other}`")),
    }
}

/// Checks every emitted transition step against the engine's transition table.
fn verify_transitions(steps: &[TransitionStep]) -> Result<(), String> {
    for step in steps {
        match step {
            TransitionStep::Engine { from, event, to } => {
                let from = parse_engine_state(from)?;
                let event = parse_engine_event(event)?;
                let to = parse_engine_state(to)?;
                let expected = engine::engine(from, event)
                    .map_err(|refused| format!("engine table refused {step:?}: {refused}"))?;
                if expected != to {
                    return Err(format!(
                        "engine transition {from:?} + {event:?} gave {to:?}, table says {expected:?}"
                    ));
                }
            }
            TransitionStep::Connection {
                from,
                event,
                to,
                effect,
            } => {
                let from = parse_connection_state(from)?;
                let event = parse_connection_event(event)?;
                let to = parse_connection_state(to)?;
                let effect = parse_connection_effect(effect)?;
                let expected = engine::connection(from, event)
                    .map_err(|refused| format!("connection table refused {step:?}: {refused}"))?;
                if expected != (to, effect) {
                    return Err(format!(
                        "connection transition {from:?} + {event:?} gave ({to:?}, {effect:?}), table says {expected:?}"
                    ));
                }
            }
        }
    }
    Ok(())
}

fn daemon_engine() -> Engine {
    let identity = Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.1.0").unwrap(),
        channel: Channel::CratesIo,
    };
    let composition = Composition::default().with(goodfetch_engine::contribution());
    Engine::new(identity, &composition).unwrap()
}

/// The Daemon host: boots a real `Engine::run_daemon` listening on turmoil's
/// adapter of `transport`, ready to be crashed, restarted and drained.
async fn serve_daemon(host: Host, world: World) -> turmoil::Result {
    let boot = {
        let mut boots = world.boots.borrow_mut();
        *boots += 1;
        *boots
    };
    world.record(&host, DAEMON, Seen::Boot(boot));

    let network = listen(host.owner(), PORT).await?;
    let (listener, _) = network
        .listen_at(SocketAddr::from((Ipv4Addr::UNSPECIFIED, PORT)))
        .await?;

    let drain_token = CancellationToken::new();
    *world.live_drain.borrow_mut() = Some(drain_token.clone());
    let _guard = DrainGuard {
        live_drain: Rc::clone(&world.live_drain),
    };

    #[expect(
        clippy::disallowed_methods,
        reason = "the Simulation boots each Daemon from a deterministic seeded Entropy (ADR 0059)"
    )]
    let entropy = Entropy::seeded(world.seed.wrapping_add(boot));
    let dependencies = Dependencies {
        home: Home::new(world.home.clone()),
        clock: *host.clock(),
        entropy,
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: PolitenessOptions::default(),
        environment: BTreeMap::new(),
        keychain: Keychain::absent(),
    };
    let options = DaemonOptions {
        token: TOKEN.to_owned(),
        endpoint: DaemonEndpoint::Unix("daemon.sock".into()),
        pid: 4242,
        idle_timeout: Duration::from_secs(60),
        keep_alive: true,
        holds: Holds::new(),
        succession: Arc::new(AlwaysPick),
        drain: Drain {
            asked: Some(drain_token),
            grace: Duration::from_secs(5),
        },
        install: None,
        ops: Ops::default(),
        websocket: None,
        accounts: None,
        live_view: None,
        http_listener: None,
    };

    let engine = daemon_engine();
    let _ended = engine.run_daemon(listener, dependencies, options).await;
    Ok(())
}

fn hello_message(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "hello",
        "params": {
            "protocol_version": "0.1",
            "surface": "cli",
            "needs": "collie",
            "product_version": "0.1.0",
            "token": TOKEN,
        }
    })
}

fn status_message(id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "daemon.status",
        "params": {}
    })
}

async fn send_json(
    writer: &mut (impl tokio::io::AsyncWrite + Unpin),
    value: &Value,
) -> std::io::Result<()> {
    let mut line = serde_json::to_string(value).unwrap();
    line.push('\n');
    writer.write_all(line.as_bytes()).await?;
    writer.flush().await
}

async fn read_reply(
    lines: &mut tokio::io::Lines<BufReader<impl tokio::io::AsyncRead + Unpin>>,
) -> Option<Value> {
    let line = tokio::time::timeout(RPC_TIMEOUT, lines.next_line())
        .await
        .ok()?
        .ok()??;
    serde_json::from_str(&line).ok()
}

/// The Client host: exercises opening refusal, open RPC calls, a lost
/// Connection, reconnection across Daemon faults, and a clean Drain.
async fn drive_client(host: Host, world: World) -> turmoil::Result {
    // Allow the Daemon host to bind its listener before dialing.
    tokio::time::sleep(Duration::from_millis(20)).await;

    // 1. Refuse a non-hello request on an Opening connection (Opening -> Closed).
    if let Ok(Ok(conn)) =
        tokio::time::timeout(RPC_TIMEOUT, connect(host.owner(), (DAEMON, PORT))).await
    {
        let (reader, mut writer) = conn.split();
        let mut lines = BufReader::new(reader).lines();
        let _ = send_json(&mut writer, &status_message(1)).await;
        let _ = read_reply(&mut lines).await;
    }

    // 2. Open a connection, complete hello + daemon.status, then drop abruptly
    //    so the Daemon drives ConnectionState through ClientLost -> Closing -> Closed.
    if let Ok(Ok(conn)) =
        tokio::time::timeout(RPC_TIMEOUT, connect(host.owner(), (DAEMON, PORT))).await
    {
        let (reader, mut writer) = conn.split();
        let mut lines = BufReader::new(reader).lines();
        if send_json(&mut writer, &hello_message(1)).await.is_ok() {
            let _ = read_reply(&mut lines).await;
            let _ = send_json(&mut writer, &status_message(2)).await;
            let _ = read_reply(&mut lines).await;
            // Duplicate hello in Open state triggers a program refusal while staying Open.
            let _ = send_json(&mut writer, &hello_message(3)).await;
            let _ = read_reply(&mut lines).await;
        }
        drop(lines);
        drop(writer);
        world.record(&host, CLIENT, Seen::ConnectionLost);
    }

    // 3. Hold a connection open across the fault-injection window (partitions,
    //    heals, crashes and restarts), observing lost connections and probing.
    let mut persistent = tokio::time::timeout(RPC_TIMEOUT, connect(host.owner(), (DAEMON, PORT)))
        .await
        .ok()
        .and_then(Result::ok)
        .map(|conn| {
            let (reader, writer) = conn.split();
            (BufReader::new(reader).lines(), writer)
        });
    if let Some((lines, writer)) = &mut persistent
        && send_json(writer, &hello_message(10)).await.is_ok()
    {
        let _ = read_reply(lines).await;
    }

    let mut next_id = 20_u64;
    while turmoil::sim_elapsed().unwrap_or_default() < SETTLED {
        tokio::time::sleep(Duration::from_millis(60)).await;
        next_id += 1;
        let lost = if let Some((lines, writer)) = &mut persistent {
            send_json(writer, &status_message(next_id)).await.is_err()
                || read_reply(lines).await.is_none()
        } else {
            true
        };
        if lost {
            if persistent.take().is_some() {
                world.record(&host, CLIENT, Seen::ConnectionLost);
            }
            if let Ok(Ok(conn)) =
                tokio::time::timeout(RPC_TIMEOUT, connect(host.owner(), (DAEMON, PORT))).await
            {
                let (reader, mut writer) = conn.split();
                let mut lines = BufReader::new(reader).lines();
                if send_json(&mut writer, &hello_message(next_id))
                    .await
                    .is_ok()
                    && read_reply(&mut lines).await.is_some()
                {
                    persistent = Some((lines, writer));
                }
            }
        }
    }
    drop(persistent);

    // 4. After faults settle, if the Daemon is running, verify it serves a
    //    fresh Connection and then drain it cleanly (Serving -> Draining -> Stopping).
    let drain_token = world.live_drain.borrow().clone();
    if let Some(drain_token) = drain_token {
        for attempt in 0..4_u64 {
            if let Ok(Ok(conn)) =
                tokio::time::timeout(RPC_TIMEOUT, connect(host.owner(), (DAEMON, PORT))).await
            {
                let (reader, mut writer) = conn.split();
                let mut lines = BufReader::new(reader).lines();
                if send_json(&mut writer, &hello_message(100 + attempt))
                    .await
                    .is_ok()
                    && read_reply(&mut lines).await.is_some()
                    && send_json(&mut writer, &status_message(200 + attempt))
                        .await
                        .is_ok()
                    && read_reply(&mut lines).await.is_some()
                {
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
        drain_token.cancel();
        world.record(&host, CLIENT, Seen::Drained);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }

    Ok(())
}

/// Runs the scenario from `seed` in `folder` with `injections`.
fn run_engine(
    seed: u64,
    start: SystemTime,
    folder: &Path,
    injections: Vec<Injection>,
) -> Result<EngineRun, String> {
    let home = tempfile::tempdir_in(folder).map_err(|error| error.to_string())?;
    let world = World {
        seed,
        start,
        home: home.path().to_owned(),
        boots: Rc::new(RefCell::new(0)),
        live_drain: Rc::new(RefCell::new(None)),
        log: Rc::new(RefCell::new(ObservationLog::new())),
    };
    let capture = Capture::default();
    let subscriber = tracing_subscriber::registry().with(capture.clone());

    let outcome = tracing::subscriber::with_default(subscriber, || {
        let mut simulation = Simulation::new(seed, start);
        let daemon_world = world.clone();
        simulation.host(DAEMON, move |host| serve_daemon(host, daemon_world.clone()));
        let client_world = world.clone();
        simulation.client(CLIENT, move |host| drive_client(host, client_world));
        simulation
            .run_injecting(injections)
            .map_err(|error| error.to_string())
    });
    outcome?;

    let bug_refusals = capture.bug_refusals.lock().unwrap().clone();
    if !bug_refusals.is_empty() {
        return Err(format!(
            "engine or connection logged bug-refused transitions: {bug_refusals:?}"
        ));
    }

    let transitions = capture.transitions.lock().unwrap().clone();
    verify_transitions(&transitions)?;

    for step in &transitions {
        world.log.borrow_mut().push(Record::new(
            DAEMON,
            Operation::Other(Seen::Transition(step.clone())),
        ));
    }

    let boots = *world.boots.borrow();
    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("the simulation hosts are done with the log")
    };
    Ok(EngineRun {
        log: log.into_inner(),
        transitions,
        boots,
    })
}

/// Faults on the Daemon and its link to the Client: partitions, heals, crashes
/// and restarts, ordered in simulated time.
struct EngineAndConnectionFaults;

impl ReferenceStateMachine for EngineAndConnectionFaults {
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        let now = *now;
        let action = prop_oneof![
            Just(FaultPlanAction::Crash),
            Just(FaultPlanAction::Restart),
            Just(FaultPlanAction::Partition {
                peer: CLIENT.to_owned()
            }),
            Just(FaultPlanAction::Heal {
                peer: CLIENT.to_owned()
            }),
        ];
        (10..120_u64, action)
            .prop_map(move |(after, action)| FaultPlanStep {
                at: now + after,
                host: DAEMON.to_owned(),
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
                FaultPlanAction::Partition { peer } => Fault::Partition { peer: peer.clone() },
                FaultPlanAction::Heal { peer } => Fault::Heal { peer: peer.clone() },
                other => return Err(format!("the engine scenario injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Covers: story:3-152
#[test]
fn the_engine_simulation_is_deterministic() {
    let env = TestEnv::new();
    let plan = vec![
        Injection {
            at: Duration::from_millis(150),
            host: DAEMON.to_owned(),
            fault: Fault::Partition {
                peer: CLIENT.to_owned(),
            },
        },
        Injection {
            at: Duration::from_millis(250),
            host: DAEMON.to_owned(),
            fault: Fault::Heal {
                peer: CLIENT.to_owned(),
            },
        },
        Injection {
            at: Duration::from_millis(350),
            host: DAEMON.to_owned(),
            fault: Fault::Crash,
        },
        Injection {
            at: Duration::from_millis(500),
            host: DAEMON.to_owned(),
            fault: Fault::Restart,
        },
    ];

    let checked = check_determinism([env.seed()], |seed| {
        let run = run_engine(seed, env.start_time(), env.folder(), plan.clone())
            .expect("committed engine crash and lost-connection plan passes");
        assert!(
            run.boots >= 2,
            "the Daemon must boot again after its crash: boots={}",
            run.boots
        );
        let saw_lost_connection = run.transitions.iter().any(|step| {
            matches!(
                step,
                TransitionStep::Connection { event, to, .. }
                    if event == "ClientLost" && to == "Closing"
            )
        });
        let saw_closed = run.transitions.iter().any(|step| {
            matches!(
                step,
                TransitionStep::Connection { event, to, .. }
                    if event == "CallsEnded" && to == "Closed"
            )
        });
        let saw_engine_drain = run.transitions.iter().any(|step| {
            matches!(
                step,
                TransitionStep::Engine { from, event, to }
                    if from == "Serving" && event == "Drain" && to == "Draining"
            )
        });
        let saw_engine_stopped = run.transitions.iter().any(|step| {
            matches!(
                step,
                TransitionStep::Engine { from, event, to }
                    if from == "Draining" && event == "Drained" && to == "Stopping"
            )
        });
        assert!(
            saw_lost_connection && saw_closed,
            "expected ConnectionState ClientLost -> Closing -> CallsEnded -> Closed in {:?}",
            run.transitions
        );
        assert!(
            saw_engine_drain && saw_engine_stopped,
            "expected EngineState Serving -> Drain -> Draining -> Drained -> Stopping in {:?}",
            run.transitions
        );
        Replay::new(&run.log, &[])
    });

    assert_eq!(checked, Ok(()));
}

/// Covers: story:3-151, story:3-152
#[test]
fn engine_crashes_and_lost_connections_fault_plans() {
    let env = TestEnv::new();

    let checked = generator(&env).check::<EngineAndConnectionFaults, _>(1..6, |plan| {
        run_engine(plan.seed, env.start_time(), env.folder(), injections(plan)?).map(drop)
    });

    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
