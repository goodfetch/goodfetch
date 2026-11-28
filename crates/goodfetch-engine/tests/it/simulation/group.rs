//! Daemon group lead-epoch confirmation and fencing under partitions, crashes
//! and restarts (ADRs 0055 and 0060): three Group members on `turmoil` exchange
//! `hello` and `group.lead_epoch` through [`PeerLink`], durably recording the
//! highest lead epoch they have seen in [`LeadEpochs`] and tracking confirmation
//! through [`Leadership`].
//!
//! `member-a` leads at epoch 1; mid-run, the operator promotes `member-b` to
//! epoch 2. When `member-b` reaches a strict majority it confirms automatically
//! and fences a returning `member-a` at stale epoch 1; when partitions prevent
//! a majority, the operator stops `member-a` and confirms `member-b` with
//! `group lead --confirm`. Every single-writer action is judged by
//! [`one_lead`], and every transition `goodfetch::group` emits is validated by
//! TLC against `test-support/models/lead-epoch/TraceLeadEpoch.tla`.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::group::{ExchangeOutcome, LeadEpochs, Leadership, PeerLink, TARGET};
use goodfetch_engine::{Channel, Identity};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::group::{
    HELLO, LEAD_EPOCH, LEAD_EPOCH_STALE, LeadEpoch, PEER_PROTOCOL_VERSION,
};
use goodfetch_simulation::turmoil;
use goodfetch_simulation::{Fault, Host, Injection, Simulation};
use goodfetch_test_support::{
    CapturedLog, FaultPlan, FaultPlanAction, FaultPlanGenerator, FaultPlanStep, ObservationLog,
    Operation, Record, Replay, TestEnv, check_determinism, model_trace, model_traces, one_lead,
    proptest_config_for,
};
use proptest::prelude::*;
use proptest::test_runner::Config;
use proptest_state_machine::ReferenceStateMachine;
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tracing_subscriber::layer::SubscriberExt as _;

/// The scenario's name, which `just simulate` takes.
const SCENARIO: &str = "group";
/// The version of its Fault plan generator.
const GENERATOR_VERSION: u32 = 1;
/// The model whose trace the scenario writes.
const MODEL: &str = "lead-epoch";
/// The three Group members on turmoil's network.
const MEMBERS: [&str; 3] = ["member-a", "member-b", "member-c"];
const OPERATOR: &str = "operator";
/// The simulated peer-link TCP port on each Group member.
const PORT: u16 = 9443;
/// When the operator promotes `member-b` to epoch 2.
const PROMOTE_AT: Duration = Duration::from_millis(450);
/// When the operator falls back to `group lead --confirm` if `member-b` still
/// cannot reach a majority under partitions or crashes.
const OPERATOR_CONFIRM_AT: Duration = Duration::from_millis(1200);
/// When the operator finishes and judges the Observation log.
const SETTLED: Duration = Duration::from_millis(2000);
/// Timeout on a single simulated peer RPC so partitioned links fail fast.
const RPC_TIMEOUT: Duration = Duration::from_millis(80);

/// Extra observations beside [`Operation::Lead`] for deterministic replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    /// A Lead Daemon confirmed by majority or `group lead --confirm`.
    Confirmed { epoch: u64, by_operator: bool },
    /// A Lead Daemon was fenced off by a higher lead epoch.
    Fenced { epoch: u64, higher: u64 },
}

/// Durable per-member state that survives crashes and restarts.
#[derive(Debug)]
struct Disk {
    highest: LeadEpoch,
    configured_lead: Option<LeadEpoch>,
}

type Disks = Rc<RefCell<BTreeMap<&'static str, Disk>>>;
type Live = Rc<RefCell<BTreeMap<&'static str, Rc<RefCell<Leadership>>>>>;
type Log = Rc<RefCell<ObservationLog<Seen>>>;

/// Removes a crashed host's live [`Leadership`] handle on drop so the operator
/// never confirms a down Daemon.
struct LiveGuard {
    live: Live,
    name: &'static str,
}

impl Drop for LiveGuard {
    fn drop(&mut self) {
        self.live.borrow_mut().remove(self.name);
    }
}

/// One run of the scenario: its Observation log and its `goodfetch::group`
/// model trace.
struct GroupRun {
    log: ObservationLog<Seen>,
    trace: Vec<u8>,
}

/// Shared simulation state: durable disks, live handles, and the Observation log.
#[derive(Clone)]
struct World {
    start: SystemTime,
    disks: Disks,
    live: Live,
    log: Log,
}

impl World {
    fn new(start: SystemTime) -> Self {
        let mut disks = BTreeMap::new();
        for name in MEMBERS {
            disks.insert(
                name,
                Disk {
                    highest: LeadEpoch::new(1),
                    configured_lead: (name == MEMBERS[0]).then_some(LeadEpoch::new(1)),
                },
            );
        }
        Self {
            start,
            disks: Rc::new(RefCell::new(disks)),
            live: Rc::default(),
            log: Rc::new(RefCell::new(ObservationLog::new())),
        }
    }

    fn elapsed_ms(&self, clock: Clock) -> u64 {
        let at = clock.wall().duration_since(self.start).unwrap_or_default();
        u64::try_from(at.as_millis()).unwrap_or(u64::MAX)
    }

    fn record_lead(&self, clock: Clock, leader: &str, epoch: LeadEpoch) {
        self.log.borrow_mut().push(
            Record::new(leader, Operation::Lead { epoch: epoch.get() }).at(self.elapsed_ms(clock)),
        );
    }

    fn record_seen(&self, clock: Clock, who: &str, seen: Seen) {
        self.log
            .borrow_mut()
            .push(Record::new(who, Operation::Other(seen)).at(self.elapsed_ms(clock)));
    }
}

fn leader_of(epoch: LeadEpoch) -> Option<&'static str> {
    match epoch.get() {
        1 => Some(MEMBERS[0]),
        2 => Some(MEMBERS[1]),
        _ => None,
    }
}

fn collie_identity() -> Identity {
    Identity {
        product: Product::Collie,
        product_version: ProductVersion::parse("0.4.0").unwrap(),
        channel: Channel::Homebrew,
    }
}

fn all_member_names() -> Vec<String> {
    MEMBERS.iter().map(|name| (*name).to_owned()).collect()
}

/// Runs the scenario from `seed` with `injections`, verifying [`one_lead`].
fn group(seed: u64, start: SystemTime, injections: Vec<Injection>) -> Result<GroupRun, String> {
    let world = World::new(start);
    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber =
        tracing_subscriber::registry().with(model_trace(&[TARGET], move || writer.clone()));
    let outcome = tracing::subscriber::with_default(subscriber, || {
        let mut simulation = Simulation::new(seed, start);
        for name in MEMBERS {
            let world = world.clone();
            simulation.host(name, move |host| member(host, name, world.clone()));
        }
        let operator = world.clone();
        simulation.client(OPERATOR, move |host| operate(host, operator));
        simulation
            .run_injecting(injections)
            .map_err(|error| error.to_string())
    });
    outcome?;
    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("the hosts are done with the log");
    };
    let log = log.into_inner();
    if let Err(violation) = one_lead(&log) {
        return Err(format!("one_lead violated: {violation:?}"));
    }
    Ok(GroupRun {
        log,
        trace: captured.written(),
    })
}

/// One boot of a Group member: serves incoming peer links through [`PeerLink`]
/// and, when configured as Lead Daemon, exchanges epochs with peers to confirm
/// by majority before performing peer-checked single-writer actions.
async fn member(mut host: Host, name: &'static str, world: World) -> turmoil::Result {
    let (highest, configured) = {
        let mut disks = world.disks.borrow_mut();
        let disk = disks.get_mut(name).unwrap();
        if let Some(lead) = disk.configured_lead
            && lead > disk.highest
        {
            disk.highest = lead;
        }
        (disk.highest, disk.configured_lead)
    };
    let epochs = Rc::new(RefCell::new(LeadEpochs::for_member(name, highest)));
    let leadership = Rc::new(RefCell::new(match configured {
        Some(epoch) if highest > epoch => Leadership::fenced(name, epoch, highest),
        Some(epoch) => Leadership::starting(name, epoch, all_member_names()),
        None => Leadership::not_lead(name),
    }));
    world.live.borrow_mut().insert(name, Rc::clone(&leadership));
    let _guard = LiveGuard {
        live: Rc::clone(&world.live),
        name,
    };

    let listener =
        turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT)).await?;
    let serve_epochs = Rc::clone(&epochs);
    let serve_leadership = Rc::clone(&leadership);
    let serve_world = world.clone();
    let clock = *host.clock();
    #[expect(
        clippy::disallowed_methods,
        reason = "turmoil host tasks run inside turmoil's single-threaded LocalSet and are dropped when turmoil crashes the host"
    )]
    tokio::task::spawn_local(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let epochs = Rc::clone(&serve_epochs);
            let leadership = Rc::clone(&serve_leadership);
            let world = serve_world.clone();
            #[expect(
                clippy::disallowed_methods,
                reason = "turmoil host connection tasks run inside turmoil's single-threaded LocalSet"
            )]
            tokio::task::spawn_local(async move {
                serve_peer(stream, name, epochs, leadership, world, clock).await;
            });
        }
    });

    let mut state = MemberLoop {
        name,
        clock,
        configured,
        announced_confirmed: false,
        announced_fenced: false,
        epochs,
        leadership,
        world,
    };
    loop {
        let pause = 25 + host.entropy().next_u64() % 20;
        tokio::time::sleep(Duration::from_millis(pause)).await;
        state.tick().await;
    }
}

struct MemberLoop {
    name: &'static str,
    clock: Clock,
    configured: Option<LeadEpoch>,
    announced_confirmed: bool,
    announced_fenced: bool,
    epochs: Rc<RefCell<LeadEpochs>>,
    leadership: Rc<RefCell<Leadership>>,
    world: World,
}

impl MemberLoop {
    async fn tick(&mut self) {
        let latest_configured = self.world.disks.borrow()[self.name].configured_lead;
        if latest_configured != self.configured {
            self.configured = latest_configured;
            self.announced_confirmed = false;
            self.announced_fenced = false;
            match self.configured {
                Some(epoch) => {
                    let mut disks = self.world.disks.borrow_mut();
                    let disk = disks.get_mut(self.name).unwrap();
                    if epoch > disk.highest {
                        disk.highest = epoch;
                    }
                    *self.epochs.borrow_mut() = LeadEpochs::for_member(self.name, disk.highest);
                    *self.leadership.borrow_mut() = if disk.highest > epoch {
                        Leadership::fenced(self.name, epoch, disk.highest)
                    } else {
                        Leadership::starting(self.name, epoch, all_member_names())
                    };
                }
                None => {
                    *self.leadership.borrow_mut() = Leadership::not_lead(self.name);
                }
            }
        }

        let Some(lead_epoch) = self.configured else {
            return;
        };

        let unconfirmed = self.leadership.borrow().unconfirmed_peers();
        for peer in unconfirmed {
            if !self.leadership.borrow().is_unconfirmed() {
                break;
            }
            if let Some(reply) = send_hello(&peer, lead_epoch).await {
                handle_peer_reply(
                    self.clock,
                    self.name,
                    &peer,
                    lead_epoch,
                    &reply,
                    &self.epochs,
                    &self.leadership,
                    &self.world,
                    &mut self.announced_fenced,
                );
            }
        }

        let status = self.leadership.borrow().status();
        if let Ok(snapshot) = status
            && snapshot.confirmed
        {
            if !self.announced_confirmed {
                self.announced_confirmed = true;
                self.world.record_seen(
                    self.clock,
                    self.name,
                    Seen::Confirmed {
                        epoch: snapshot.lead_epoch.get(),
                        by_operator: snapshot.confirmed_by_operator,
                    },
                );
            }
            if snapshot.confirmed_by_operator {
                self.world
                    .record_lead(self.clock, self.name, snapshot.lead_epoch);
            } else {
                for peer in MEMBERS {
                    if peer == self.name {
                        continue;
                    }
                    if self.leadership.borrow().can_act().is_err() {
                        break;
                    }
                    if let Some(reply) = send_lead_epoch(peer, snapshot.lead_epoch).await {
                        handle_peer_reply(
                            self.clock,
                            self.name,
                            peer,
                            snapshot.lead_epoch,
                            &reply,
                            &self.epochs,
                            &self.leadership,
                            &self.world,
                            &mut self.announced_fenced,
                        );
                        if self.leadership.borrow().can_act().is_err() {
                            break;
                        }
                    }
                }
            }
        }
    }
}

/// Handles one incoming peer TCP stream with [`PeerLink::accepting`], updating
/// durable `highest` whenever [`LeadEpochs::admit`] raises it and recording the
/// linearization point of accepted `group.lead_epoch` actions.
async fn serve_peer(
    stream: turmoil::net::TcpStream,
    name: &'static str,
    epochs: Rc<RefCell<LeadEpochs>>,
    leadership: Rc<RefCell<Leadership>>,
    world: World,
    clock: Clock,
) {
    let mut link = PeerLink::accepting(collie_identity());
    let (read_half, mut write_half) = stream.into_split();
    let mut reader = BufReader::new(read_half);
    let mut frame = Vec::new();
    while let Ok(read) = reader.read_until(b'\n', &mut frame).await {
        if read == 0 {
            break;
        }
        let parsed: Option<Value> = serde_json::from_slice(&frame).ok();
        let is_action = parsed
            .as_ref()
            .and_then(|v| v.get("method").and_then(Value::as_str))
            == Some(LEAD_EPOCH);
        let action_epoch = parsed
            .as_ref()
            .and_then(|v| v.pointer("/params/lead_epoch").and_then(Value::as_u64))
            .map(LeadEpoch::new);

        let answer = {
            let mut guard = epochs.borrow_mut();
            let before = guard.highest();
            let answer = link.receive(&mut guard, std::mem::take(&mut frame));
            let after = guard.highest();
            if after > before {
                world.disks.borrow_mut().get_mut(name).unwrap().highest = after;
                leadership.borrow_mut().fence_if_higher(after);
            }
            answer
        };

        // Record the linearization point of an accepted single-writer action
        // while this peer still holds `highest == action_epoch`.
        if is_action
            && let (Some(epoch), Some(reply_bytes)) = (action_epoch, &answer.reply)
            && let Ok(reply_json) = serde_json::from_slice::<Value>(reply_bytes)
            && reply_json.get("result").is_some()
            && let Some(leader) = leader_of(epoch)
        {
            world.record_lead(clock, leader, epoch);
        }

        if let Some(reply) = answer.reply
            && write_half.write_all(&reply).await.is_err()
        {
            break;
        }
        if answer.close {
            break;
        }
    }
}

/// Applies one peer JSON-RPC reply to this Lead Daemon's [`LeadEpochs`] and
/// [`Leadership`].
#[expect(
    clippy::too_many_arguments,
    reason = "local simulation helper wiring host state, peer reply and observation log"
)]
fn handle_peer_reply(
    clock: Clock,
    name: &'static str,
    peer: &str,
    lead_epoch: LeadEpoch,
    reply: &Value,
    epochs: &Rc<RefCell<LeadEpochs>>,
    leadership: &Rc<RefCell<Leadership>>,
    world: &World,
    announced_fenced: &mut bool,
) {
    if let Some(peer_epoch) = reply
        .pointer("/result/lead_epoch")
        .and_then(Value::as_u64)
        .map(LeadEpoch::new)
    {
        if peer_epoch > lead_epoch {
            let _ = epochs.borrow_mut().admit(peer_epoch);
            let highest = epochs.borrow().highest();
            world.disks.borrow_mut().get_mut(name).unwrap().highest = highest;
        }
        if let ExchangeOutcome::Fenced(higher) = leadership
            .borrow_mut()
            .record_peer_exchange(peer, peer_epoch)
            && !*announced_fenced
        {
            *announced_fenced = true;
            world.record_seen(
                clock,
                name,
                Seen::Fenced {
                    epoch: lead_epoch.get(),
                    higher: higher.get(),
                },
            );
        }
    } else if reply.pointer("/error/code").and_then(Value::as_str) == Some(LEAD_EPOCH_STALE.name) {
        let higher = reply
            .pointer("/error/details/highest_lead_epoch")
            .and_then(Value::as_u64)
            .map_or_else(|| LeadEpoch::new(lead_epoch.get() + 1), LeadEpoch::new);
        let _ = epochs.borrow_mut().admit(higher);
        let highest = epochs.borrow().highest();
        world.disks.borrow_mut().get_mut(name).unwrap().highest = highest;
        if let ExchangeOutcome::Fenced(higher) =
            leadership.borrow_mut().record_peer_exchange(peer, higher)
            && !*announced_fenced
        {
            *announced_fenced = true;
            world.record_seen(
                clock,
                name,
                Seen::Fenced {
                    epoch: lead_epoch.get(),
                    higher: higher.get(),
                },
            );
        }
    }
}

/// Sends one `hello` line to `peer` over `turmoil::net::TcpStream`.
async fn send_hello(peer: &str, lead_epoch: LeadEpoch) -> Option<Value> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": HELLO,
        "params": {
            "protocol_version": PEER_PROTOCOL_VERSION,
            "product": Product::Collie,
            "product_version": "0.4.0",
            "lead_epoch": lead_epoch.get(),
        },
    });
    send_one_round(peer, &[request]).await
}

/// Opens a peer link with `hello`, then sends one `group.lead_epoch` line.
async fn send_lead_epoch(peer: &str, lead_epoch: LeadEpoch) -> Option<Value> {
    let hello = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": HELLO,
        "params": {
            "protocol_version": PEER_PROTOCOL_VERSION,
            "product": Product::Collie,
            "product_version": "0.4.0",
            "lead_epoch": lead_epoch.get(),
        },
    });
    let action = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": LEAD_EPOCH,
        "params": {
            "lead_epoch": lead_epoch.get(),
        },
    });
    send_one_round(peer, &[hello, action]).await
}

async fn send_one_round(peer: &str, requests: &[Value]) -> Option<Value> {
    tokio::time::timeout(RPC_TIMEOUT, async move {
        let stream = turmoil::net::TcpStream::connect((peer, PORT)).await.ok()?;
        let (read_half, mut write_half) = stream.into_split();
        let mut reader = BufReader::new(read_half);
        let mut last_reply = None;
        for request in requests {
            let mut bytes = serde_json::to_vec(request).ok()?;
            bytes.push(b'\n');
            write_half.write_all(&bytes).await.ok()?;
            let mut line = Vec::new();
            let read = reader.read_until(b'\n', &mut line).await.ok()?;
            if read == 0 {
                return last_reply;
            }
            let reply: Value = serde_json::from_slice(&line).ok()?;
            let has_error = reply.get("error").is_some();
            last_reply = Some(reply);
            if has_error {
                break;
            }
        }
        last_reply
    })
    .await
    .ok()?
}

/// The operator: promotes `member-b` to epoch 2 mid-run, falls back to
/// `group lead --confirm` (after stopping `member-a`) if partitions block
/// majority confirmation, and waits until the group settles.
async fn operate(host: Host, world: World) -> turmoil::Result {
    tokio::time::sleep(PROMOTE_AT).await;
    // Promote member-b to epoch 2. member-a keeps its stale epoch-1 lead config
    // so majority confirmation + peer-link fencing are exercised under partitions
    // and restarts!
    world
        .disks
        .borrow_mut()
        .get_mut(MEMBERS[1])
        .unwrap()
        .configured_lead = Some(LeadEpoch::new(2));

    tokio::time::sleep(OPERATOR_CONFIRM_AT.saturating_sub(PROMOTE_AT)).await;
    // If member-b is alive and still unconfirmed (e.g. partitioned from both
    // peers), stop member-a per ADR 0055's `--confirm` contract and confirm
    // member-b by operator.
    let needs_operator_confirm = world
        .live
        .borrow()
        .get(MEMBERS[1])
        .is_some_and(|lead| lead.borrow().is_unconfirmed());
    if needs_operator_confirm {
        world
            .disks
            .borrow_mut()
            .get_mut(MEMBERS[0])
            .unwrap()
            .configured_lead = None;
        if let Some(old_lead) = world.live.borrow().get(MEMBERS[0]).cloned() {
            *old_lead.borrow_mut() = Leadership::not_lead(MEMBERS[0]);
        }
        // Let any in-flight epoch-1 RPC from member-a finish before the
        // operator confirms member-b (ADR 0055: confirm only once the former
        // Lead Daemon has stopped).
        tokio::time::sleep(RPC_TIMEOUT + Duration::from_millis(20)).await;
        if let Some(new_lead) = world.live.borrow().get(MEMBERS[1]).cloned()
            && let Ok(snapshot) = new_lead.borrow_mut().confirm_by_operator()
        {
            world.record_seen(
                *host.clock(),
                MEMBERS[1],
                Seen::Confirmed {
                    epoch: snapshot.lead_epoch.get(),
                    by_operator: snapshot.confirmed_by_operator,
                },
            );
        }
    }

    tokio::time::sleep(
        SETTLED
            .saturating_sub(OPERATOR_CONFIRM_AT)
            .saturating_sub(RPC_TIMEOUT)
            .saturating_sub(Duration::from_millis(20)),
    )
    .await;
    Ok(())
}

/// Faults the seed draws: partitions, heals, crashes and restarts across the
/// three Group members.
fn drawn_faults(seed: u64) -> Vec<Injection> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test draws its Simulation's faults from its seed (ADR 0060)"
    )]
    let mut entropy = Entropy::seeded(seed);
    let mut injections = Vec::new();
    let pairs = [
        (MEMBERS[0], MEMBERS[1]),
        (MEMBERS[0], MEMBERS[2]),
        (MEMBERS[1], MEMBERS[2]),
    ];
    for (left, right) in pairs {
        if entropy.next_u64().is_multiple_of(2) {
            let cut = Duration::from_millis(100 + entropy.next_u64() % 600);
            injections.push(Injection {
                at: cut,
                host: left.to_owned(),
                fault: Fault::Partition {
                    peer: right.to_owned(),
                },
            });
            if entropy.next_u64().is_multiple_of(2) {
                injections.push(Injection {
                    at: cut + Duration::from_millis(200 + entropy.next_u64() % 400),
                    host: left.to_owned(),
                    fault: Fault::Heal {
                        peer: right.to_owned(),
                    },
                });
            }
        }
    }
    for member in MEMBERS {
        if entropy.next_u64().is_multiple_of(2) {
            let crash = Duration::from_millis(150 + entropy.next_u64() % 700);
            injections.push(Injection {
                at: crash,
                host: member.to_owned(),
                fault: Fault::Crash,
            });
            injections.push(Injection {
                at: crash + Duration::from_millis(150 + entropy.next_u64() % 300),
                host: member.to_owned(),
                fault: Fault::Restart,
            });
        }
    }
    injections
}

fn group_of_seed(seed: u64, env: &TestEnv) -> GroupRun {
    group(seed, env.start_time(), drawn_faults(seed))
        .unwrap_or_else(|error| panic!("the group scenario of seed {seed} failed: {error}"))
}

fn seeds(env: &TestEnv) -> Vec<u64> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test draws its Simulations' seeds from its own (ADR 0060)"
    )]
    let mut entropy = Entropy::seeded(env.seed());
    (0..4).map(|_| entropy.next_u64()).collect()
}

/// Covers: story:3-106, story:3-108
#[test]
fn the_same_seed_gives_the_same_observation_log_and_model_trace() {
    let env = TestEnv::new();

    let checked = check_determinism(seeds(&env), |seed| {
        let run = group_of_seed(seed, &env);
        Replay::new(&run.log, &run.trace)
    });

    assert_eq!(checked, Ok(()));
}

/// The trace `just simulation` validates against the lead-epoch model: the
/// seed's run, kept in `GOODFETCH_MODEL_TRACES` when it names a folder.
///
/// Covers: story:3-106, story:3-108
#[test]
fn the_run_of_the_seed_writes_its_lead_epoch_trace() {
    let env = TestEnv::new();

    let run = group_of_seed(env.seed(), &env);

    let trace = String::from_utf8(run.trace).unwrap();
    let transitions: Vec<String> = trace
        .lines()
        .map(|line| {
            let line: Value = serde_json::from_str(line).unwrap();
            line["transition"].as_str().unwrap().to_owned()
        })
        .collect();
    assert!(
        transitions
            .iter()
            .any(|transition| transition == "confirm_majority" || transition == "confirm_operator"),
        "a Lead Daemon confirms by majority or operator: {trace}"
    );
    if let Some(folder) = model_traces() {
        keep(
            &folder.join(MODEL),
            &format!("{SCENARIO}-seed-{}.ndjson", env.seed()),
            &trace,
        );
    }
}

/// Explicit scenario: when partitions isolate `member-b` from both peers,
/// `group lead --confirm` confirms epoch 2 and `one_lead` still holds.
///
/// Covers: story:3-106, story:3-108
#[test]
fn operator_confirm_lets_a_partitioned_lead_act_without_violating_one_lead() {
    let env = TestEnv::new();
    let injections = vec![
        Injection {
            at: Duration::from_millis(300),
            host: MEMBERS[1].to_owned(),
            fault: Fault::Partition {
                peer: MEMBERS[0].to_owned(),
            },
        },
        Injection {
            at: Duration::from_millis(300),
            host: MEMBERS[1].to_owned(),
            fault: Fault::Partition {
                peer: MEMBERS[2].to_owned(),
            },
        },
    ];

    let run = group(env.seed(), env.start_time(), injections).unwrap();
    let trace = String::from_utf8(run.trace).unwrap();
    assert!(
        trace.contains("\"transition\":\"confirm_operator\""),
        "expected operator confirmation under total partition: {trace}"
    );
}

#[expect(
    clippy::disallowed_methods,
    reason = "the test's own thread writes the trace once its Simulation has ended"
)]
fn keep(folder: &Path, name: &str, trace: &str) {
    std::fs::create_dir_all(folder).unwrap();
    std::fs::write(folder.join(name), trace).unwrap();
}

/// Partitions, heals, crashes and restarts of the three Group members, each at
/// or after the one before.
struct GroupFaults;

impl ReferenceStateMachine for GroupFaults {
    /// The simulated moment of the last step, in milliseconds.
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        const PAIRS: [(&str, &str); 3] = [
            (MEMBERS[0], MEMBERS[1]),
            (MEMBERS[0], MEMBERS[2]),
            (MEMBERS[1], MEMBERS[2]),
        ];
        let now = *now;
        prop_oneof![
            (
                0..150_u64,
                prop::sample::select(MEMBERS.as_slice()),
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
                other => return Err(format!("the group scenario injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Covers: story:3-106, story:3-108
#[test]
fn group_partitions_and_restarts_fault_plans() {
    let env = TestEnv::new();

    let checked = generator(&env).check::<GroupFaults, _>(1..8, |plan| {
        group(plan.seed, env.start_time(), injections(plan)?).map(drop)
    });

    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
