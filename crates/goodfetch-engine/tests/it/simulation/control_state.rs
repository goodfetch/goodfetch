//! Replicated read-mostly control-state Simulation under partitions, crashes,
//! restarts and reordered pushes (ADRs 0055 and 0060, issue 243).
//!
//! Three Group members (`lead`, `worker-a`, `worker-b`) run on `turmoil`:
//! `lead` publishes strictly versioned snapshots of `operator_policy` and
//! `user_configuration` (including `[group_member.<name>]` scopes), pushes
//! them over simulated TCP, and also injects delayed stale pushes to exercise
//! out-of-order delivery. Each worker applies snapshots through
//! [`ReplicatedState`] backed by durable per-member disk state and syncs on
//! restart.
//!
//! Invariants judged on every run:
//! 1. Monotonicity: no Group member ever applies a snapshot whose version is
//!    less than or equal to a version it already applied.
//! 2. Convergence: once faults stop and every member is healed and running,
//!    every Group member converges to the exact latest version and content of
//!    both `operator_policy` and `user_configuration`.
//! 3. Group-member scope isolation: resolving the converged policy and
//!    configuration on `worker-a` vs `worker-b` applies only that member's
//!    `[group_member.<name>]` machine-specific settings.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::control_state::{
    ApplyOutcome, ControlStateRegistry, ReplicatedState, STORE_OPERATOR_POLICY,
    STORE_USER_CONFIGURATION,
};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::settings::{
    Layers, operator_policy, resolve, user_configuration_with_members,
};
use goodfetch_protocol::browser_cache::BROWSER_EXECUTABLE_PATH;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::group::{ControlStateSnapshot, StoredReplicatedStore};
use goodfetch_protocol::settings::{SettingsLayer, TIMEOUT_ACTION, TIMEOUT_NAVIGATION};
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
use serde_json::json;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

const SCENARIO: &str = "control_state";
const GENERATOR_VERSION: u32 = 1;
const LEAD: &str = "lead";
const WORKERS: [&str; 2] = ["worker-a", "worker-b"];
const MEMBERS: [&str; 3] = [LEAD, WORKERS[0], WORKERS[1]];
const OPERATOR: &str = "operator";
const PORT: u16 = 9553;
const TOTAL_REVISIONS: u64 = 5;
const HEAL_ALL_AT: Duration = Duration::from_millis(1200);
const SETTLED: Duration = Duration::from_millis(2200);
const RPC_TIMEOUT: Duration = Duration::from_millis(80);

/// Wire envelope exchanged between simulated Group members on turmoil.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Message {
    Push {
        snapshots: Vec<ControlStateSnapshot>,
    },
    Sync {
        versions: BTreeMap<String, u64>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Answer {
    Pushed {
        versions: BTreeMap<String, u64>,
    },
    Synced {
        snapshots: Vec<ControlStateSnapshot>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Seen {
    Published {
        store: String,
        version: u64,
    },
    Applied {
        store: String,
        version: u64,
    },
    IgnoredStale {
        store: String,
        incoming: u64,
        current: u64,
    },
    Converged {
        policy_version: u64,
        config_version: u64,
    },
}

/// Durable persisted replicated stores surviving member crashes and restarts.
#[derive(Debug, Default)]
struct Disk {
    stores: BTreeMap<String, StoredReplicatedStore>,
}

type Disks = Rc<RefCell<BTreeMap<&'static str, Disk>>>;
type Log = Rc<RefCell<ObservationLog<Seen>>>;

#[derive(Clone)]
struct World {
    start: SystemTime,
    registry: Arc<ControlStateRegistry>,
    disks: Disks,
    log: Log,
    regression: Rc<RefCell<Option<String>>>,
}

impl World {
    fn new(start: SystemTime) -> Self {
        let mut disks = BTreeMap::new();
        for name in MEMBERS {
            disks.insert(name, Disk::default());
        }
        Self {
            start,
            registry: Arc::new(ControlStateRegistry::with_defaults()),
            disks: Rc::new(RefCell::new(disks)),
            log: Rc::new(RefCell::new(ObservationLog::new())),
            regression: Rc::new(RefCell::new(None)),
        }
    }

    fn elapsed_ms(&self, clock: Clock) -> u64 {
        let at = clock.wall().duration_since(self.start).unwrap_or_default();
        u64::try_from(at.as_millis()).unwrap_or(u64::MAX)
    }

    fn record(&self, clock: Clock, host: &str, event: Seen) {
        self.log
            .borrow_mut()
            .push(Record::new(host, Operation::Other(event)).at(self.elapsed_ms(clock)));
    }

    fn hydrate_state(&self, host: &'static str) -> ReplicatedState {
        let mut state = ReplicatedState::new(Arc::clone(&self.registry));
        let stored: Vec<StoredReplicatedStore> = self
            .disks
            .borrow()
            .get(host)
            .map(|disk| disk.stores.values().cloned().collect())
            .unwrap_or_default();
        state.hydrate(stored);
        state
    }

    fn persist_snapshot(&self, host: &'static str, snapshot: &ControlStateSnapshot) {
        let mut disks = self.disks.borrow_mut();
        let disk = disks.get_mut(host).unwrap();
        if let Some(existing) = disk.stores.get(&snapshot.store)
            && snapshot.version <= existing.version
        {
            *self.regression.borrow_mut() = Some(format!(
                "{host} attempted non-monotonic persist on {}: {} <= {}",
                snapshot.store, snapshot.version, existing.version
            ));
            return;
        }
        disk.stores.insert(
            snapshot.store.clone(),
            StoredReplicatedStore {
                store: snapshot.store.clone(),
                version: snapshot.version,
                content: snapshot.content.clone(),
            },
        );
    }

    fn apply_all(
        &self,
        clock: Clock,
        host: &'static str,
        state: &mut ReplicatedState,
        snapshots: Vec<ControlStateSnapshot>,
    ) -> BTreeMap<String, u64> {
        for snapshot in snapshots {
            let before = state.version(&snapshot.store);
            let incoming_store = snapshot.store.clone();
            let incoming_version = snapshot.version;
            match state.apply(snapshot.clone()) {
                Ok(ApplyOutcome::Applied) => {
                    if incoming_version <= before {
                        *self.regression.borrow_mut() = Some(format!(
                            "{host} applied stale snapshot on {incoming_store}: {incoming_version} <= {before}"
                        ));
                    }
                    self.persist_snapshot(host, &snapshot);
                    self.record(
                        clock,
                        host,
                        Seen::Applied {
                            store: incoming_store,
                            version: incoming_version,
                        },
                    );
                }
                Ok(ApplyOutcome::Stale { current }) => {
                    if incoming_version > before {
                        *self.regression.borrow_mut() = Some(format!(
                            "{host} marked strictly newer snapshot stale on {incoming_store}: {incoming_version} > {before}"
                        ));
                    }
                    self.record(
                        clock,
                        host,
                        Seen::IgnoredStale {
                            store: incoming_store,
                            incoming: incoming_version,
                            current,
                        },
                    );
                }
                Err(error) => {
                    *self.regression.borrow_mut() = Some(format!(
                        "{host} rejected valid control-state snapshot: {error}"
                    ));
                }
            }
        }
        state.versions()
    }
}

fn policy_toml(revision: u64) -> String {
    let nav_ms = 30_000 + revision * 1_000;
    format!(
        r#"version = "0.1"

[settings.timeout]
navigation = {nav_ms}

[group_member.worker-a.browser]
executable_path = "/opt/browsers/worker-a-v{revision}"

[group_member.worker-b.browser]
executable_path = "/opt/browsers/worker-b-v{revision}"
"#
    )
}

fn config_toml(revision: u64) -> String {
    let action_ms = 10_000 + revision * 500;
    format!(
        r#"version = "0.1"

[settings.timeout]
action = {action_ms}
"#
    )
}

async fn send_message(peer: &str, message: &Message) -> Option<Answer> {
    let exchange = async {
        let mut stream = turmoil::net::TcpStream::connect((peer, PORT)).await.ok()?;
        let mut line = serde_json::to_vec(message).ok()?;
        line.push(b'\n');
        stream.write_all(&line).await.ok()?;
        stream.flush().await.ok()?;
        let mut reader = BufReader::new(stream);
        let mut reply = Vec::new();
        let read = reader.read_until(b'\n', &mut reply).await.ok()?;
        if read == 0 {
            return None;
        }
        serde_json::from_slice::<Answer>(&reply).ok()
    };
    tokio::time::timeout(RPC_TIMEOUT, exchange)
        .await
        .ok()
        .flatten()
}

async fn host_lead(host: Host, world: World) -> turmoil::Result {
    let clock = *host.clock();
    let state = Rc::new(RefCell::new(world.hydrate_state(LEAD)));
    let listener =
        turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT)).await?;

    // Accept sync requests from workers.
    let sync_state = Rc::clone(&state);
    #[expect(
        clippy::disallowed_methods,
        reason = "turmoil host tasks run inside turmoil's single-threaded LocalSet and are dropped when turmoil crashes the host"
    )]
    tokio::task::spawn_local(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let sync_state = Rc::clone(&sync_state);
            #[expect(
                clippy::disallowed_methods,
                reason = "turmoil host connection tasks run inside turmoil's single-threaded LocalSet"
            )]
            tokio::task::spawn_local(async move {
                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let mut line = Vec::new();
                if reader.read_until(b'\n', &mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                if let Ok(Message::Sync { versions }) = serde_json::from_slice(&line) {
                    let snapshots = sync_state.borrow().snapshots_newer_than(&versions);
                    let mut out = serde_json::to_vec(&Answer::Synced { snapshots }).unwrap();
                    out.push(b'\n');
                    let _ = write_half.write_all(&out).await;
                }
            });
        }
    });

    // Publish revisions v1..=TOTAL_REVISIONS and push to both workers,
    // including deliberate stale reordered pushes alongside fresh ones.
    let mut acked: BTreeMap<&'static str, BTreeMap<String, u64>> =
        WORKERS.into_iter().map(|w| (w, BTreeMap::new())).collect();
    let mut stale_history: Vec<ControlStateSnapshot> = Vec::new();
    let mut published_revisions = state.borrow().version(STORE_OPERATOR_POLICY);

    for tick in 0..45_u64 {
        if published_revisions < TOTAL_REVISIONS && tick.is_multiple_of(3) {
            published_revisions += 1;
            let pol = state
                .borrow_mut()
                .publish(
                    STORE_OPERATOR_POLICY,
                    Some(policy_toml(published_revisions)),
                )
                .unwrap();
            let cfg = state
                .borrow_mut()
                .publish(
                    STORE_USER_CONFIGURATION,
                    Some(config_toml(published_revisions)),
                )
                .unwrap();
            for snapshot in [pol, cfg].into_iter().flatten() {
                world.persist_snapshot(LEAD, &snapshot);
                world.record(
                    clock,
                    LEAD,
                    Seen::Published {
                        store: snapshot.store.clone(),
                        version: snapshot.version,
                    },
                );
                stale_history.push(snapshot);
            }
        }

        for worker in WORKERS {
            let mut batch = {
                let member_acked = acked.get(worker).unwrap();
                state.borrow().snapshots_newer_than(member_acked)
            };
            // Deliberately append a stale older snapshot to test out-of-order
            // and duplicate arrival on the receiving Group member.
            if let Some(old) = stale_history.first().cloned()
                && !batch.is_empty()
            {
                batch.push(old);
            }
            if !batch.is_empty()
                && let Some(Answer::Pushed { versions }) =
                    send_message(worker, &Message::Push { snapshots: batch }).await
            {
                let slot = acked.get_mut(worker).unwrap();
                for (store, version) in versions {
                    let entry = slot.entry(store).or_insert(0);
                    if version > *entry {
                        *entry = version;
                    }
                }
            }
        }

        clock.sleep(Duration::from_millis(50)).await;
    }
    Ok(())
}

async fn host_worker(host: Host, name: &'static str, world: World) -> turmoil::Result {
    let clock = *host.clock();
    let state = Rc::new(RefCell::new(world.hydrate_state(name)));
    let listener =
        turmoil::net::TcpListener::bind((IpAddr::V4(Ipv4Addr::UNSPECIFIED), PORT)).await?;

    // Accept incoming control-state pushes from Lead Daemon.
    let push_state = Rc::clone(&state);
    let push_world = world.clone();
    #[expect(
        clippy::disallowed_methods,
        reason = "turmoil host tasks run inside turmoil's single-threaded LocalSet and are dropped when turmoil crashes the host"
    )]
    tokio::task::spawn_local(async move {
        while let Ok((stream, _)) = listener.accept().await {
            let push_state = Rc::clone(&push_state);
            let push_world = push_world.clone();
            #[expect(
                clippy::disallowed_methods,
                reason = "turmoil host connection tasks run inside turmoil's single-threaded LocalSet"
            )]
            tokio::task::spawn_local(async move {
                let (read_half, mut write_half) = stream.into_split();
                let mut reader = BufReader::new(read_half);
                let mut line = Vec::new();
                if reader.read_until(b'\n', &mut line).await.unwrap_or(0) == 0 {
                    return;
                }
                if let Ok(Message::Push { snapshots }) = serde_json::from_slice(&line) {
                    let versions =
                        push_world.apply_all(clock, name, &mut push_state.borrow_mut(), snapshots);
                    let mut out = serde_json::to_vec(&Answer::Pushed { versions }).unwrap();
                    out.push(b'\n');
                    let _ = write_half.write_all(&out).await;
                }
            });
        }
    });

    // Sync from Lead Daemon on startup/restart until caught up.
    loop {
        let versions = state.borrow().versions();
        if let Some(Answer::Synced { snapshots }) =
            send_message(LEAD, &Message::Sync { versions }).await
            && !snapshots.is_empty()
        {
            let _ = world.apply_all(clock, name, &mut state.borrow_mut(), snapshots);
        }
        clock.sleep(Duration::from_millis(80)).await;
    }
}

async fn operate(host: Host, world: World) -> turmoil::Result {
    let clock = *host.clock();
    clock.sleep(SETTLED).await;
    let disks = world.disks.borrow();
    let lead_policy = disks[LEAD]
        .stores
        .get(STORE_OPERATOR_POLICY)
        .map_or(0, |s| s.version);
    let lead_config = disks[LEAD]
        .stores
        .get(STORE_USER_CONFIGURATION)
        .map_or(0, |s| s.version);
    world.record(
        clock,
        OPERATOR,
        Seen::Converged {
            policy_version: lead_policy,
            config_version: lead_config,
        },
    );
    Ok(())
}

/// Runs the 3-member control-state simulation with `injections` (plus a final
/// heal of every partition and restart of any crashed member at `HEAL_ALL_AT`
/// so convergence after faults stop is always verified).
fn run_control_state(
    seed: u64,
    start: SystemTime,
    mut injections: Vec<Injection>,
) -> Result<ObservationLog<Seen>, String> {
    // Ensure every fault heals at HEAL_ALL_AT so convergence after faults stop
    // is checked on every generated Fault plan.
    for worker in WORKERS {
        injections.push(Injection {
            at: HEAL_ALL_AT,
            host: LEAD.to_owned(),
            fault: Fault::Heal {
                peer: worker.to_owned(),
            },
        });
        injections.push(Injection {
            at: HEAL_ALL_AT,
            host: worker.to_owned(),
            fault: Fault::Restart,
        });
    }
    injections.sort_by_key(|step| step.at);

    let world = World::new(start);
    let mut simulation = Simulation::new(seed, start);
    {
        let world = world.clone();
        simulation.host(LEAD, move |host| host_lead(host, world.clone()));
    }
    for worker in WORKERS {
        let world = world.clone();
        simulation.host(worker, move |host| host_worker(host, worker, world.clone()));
    }
    {
        let operator = world.clone();
        simulation.client(OPERATOR, move |host| operate(host, operator));
    }

    simulation
        .run_injecting(injections)
        .map_err(|error| format!("simulation failed: {error}"))?;
    drop(simulation);

    if let Some(regression) = world.regression.borrow().clone() {
        return Err(regression);
    }

    verify_convergence(&world)?;

    let Ok(log) = Rc::try_unwrap(world.log) else {
        panic!("the hosts are done with the log");
    };
    Ok(log.into_inner())
}

fn verify_convergence(world: &World) -> Result<(), String> {
    let disks = world.disks.borrow();
    let expected_policy = policy_toml(TOTAL_REVISIONS);
    let expected_config = config_toml(TOTAL_REVISIONS);
    for member in MEMBERS {
        let pol = disks[member]
            .stores
            .get(STORE_OPERATOR_POLICY)
            .ok_or_else(|| format!("{member} never converged on operator_policy"))?;
        let cfg = disks[member]
            .stores
            .get(STORE_USER_CONFIGURATION)
            .ok_or_else(|| format!("{member} never converged on user_configuration"))?;
        if pol.version != TOTAL_REVISIONS || pol.content.as_deref() != Some(&expected_policy) {
            return Err(format!(
                "{member} operator_policy did not converge: version={}, expected={TOTAL_REVISIONS}",
                pol.version
            ));
        }
        if cfg.version != TOTAL_REVISIONS || cfg.content.as_deref() != Some(&expected_config) {
            return Err(format!(
                "{member} user_configuration did not converge: version={}, expected={TOTAL_REVISIONS}",
                cfg.version
            ));
        }
    }

    // Verify per-member `[group_member.<name>]` settings isolation on the
    // converged cached TOML files.
    let version = ProductVersion::parse("0.1.0").unwrap();
    for worker in WORKERS {
        let pol_text = disks[worker].stores[STORE_OPERATOR_POLICY]
            .content
            .as_deref()
            .unwrap();
        let cfg_text = disks[worker].stores[STORE_USER_CONFIGURATION]
            .content
            .as_deref()
            .unwrap();
        let parsed_policy = operator_policy(pol_text, Product::Collie, &version)
            .map_err(|error| format!("policy parse failed on {worker}: {error}"))?;
        let (parsed_user, user_members) = user_configuration_with_members(
            SettingsLayer::UserConfiguration,
            cfg_text,
            Product::Collie,
            &version,
        )
        .map_err(|error| format!("config parse failed on {worker}: {error}"))?;
        let mut group_member = parsed_policy
            .group_members
            .get(worker)
            .cloned()
            .unwrap_or_default();
        if let Some(user_member) = user_members.get(worker) {
            group_member.merge_from(user_member);
        }
        let layers = Layers {
            operator_policy: parsed_policy,
            user_configuration: parsed_user,
            group_member,
            ..Layers::default()
        };
        let resolved = resolve(&layers, Product::Collie, Product::Collie)
            .map_err(|error| format!("resolve failed on {worker}: {error}"))?;
        let expected_path = format!("/opt/browsers/{worker}-v{TOTAL_REVISIONS}");
        let got_path = resolved
            .effective(&BROWSER_EXECUTABLE_PATH)
            .ok_or_else(|| format!("{worker} missing browser.executable_path"))?;
        if got_path.layer != SettingsLayer::GroupMember || got_path.value != json!(expected_path) {
            return Err(format!(
                "{worker} resolved wrong browser.executable_path: {got_path:?}"
            ));
        }
        let nav = resolved.effective(&TIMEOUT_NAVIGATION).unwrap();
        if nav.value != json!(30_000 + TOTAL_REVISIONS * 1_000) {
            return Err(format!(
                "{worker} resolved wrong timeout.navigation: {nav:?}"
            ));
        }
        let action = resolved.effective(&TIMEOUT_ACTION).unwrap();
        if action.value != json!(10_000 + TOTAL_REVISIONS * 500) {
            return Err(format!(
                "{worker} resolved wrong timeout.action: {action:?}"
            ));
        }
    }
    Ok(())
}

fn drawn_faults(seed: u64) -> Vec<Injection> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test draws its Simulation's faults from its seed (ADR 0060)"
    )]
    let mut entropy = Entropy::seeded(seed);
    let mut injections = Vec::new();
    for worker in WORKERS {
        if entropy.next_u64().is_multiple_of(2) {
            let cut = Duration::from_millis(80 + entropy.next_u64() % 400);
            injections.push(Injection {
                at: cut,
                host: LEAD.to_owned(),
                fault: Fault::Partition {
                    peer: worker.to_owned(),
                },
            });
            injections.push(Injection {
                at: cut + Duration::from_millis(150 + entropy.next_u64() % 300),
                host: LEAD.to_owned(),
                fault: Fault::Heal {
                    peer: worker.to_owned(),
                },
            });
        }
        if entropy.next_u64().is_multiple_of(2) {
            let crash = Duration::from_millis(100 + entropy.next_u64() % 500);
            injections.push(Injection {
                at: crash,
                host: worker.to_owned(),
                fault: Fault::Crash,
            });
            injections.push(Injection {
                at: crash + Duration::from_millis(120 + entropy.next_u64() % 250),
                host: worker.to_owned(),
                fault: Fault::Restart,
            });
        }
    }
    injections
}

fn seeds(env: &TestEnv) -> Vec<u64> {
    #[expect(
        clippy::disallowed_methods,
        reason = "a test draws its Simulations' seeds from its own (ADR 0060)"
    )]
    let mut entropy = Entropy::seeded(env.seed());
    (0..4).map(|_| entropy.next_u64()).collect()
}

/// Covers: story:3-93, story:3-107
#[test]
fn the_same_seed_gives_the_same_observation_log() {
    let env = TestEnv::new();
    let checked = check_determinism(seeds(&env), |seed| {
        let log = run_control_state(seed, env.start_time(), drawn_faults(seed)).unwrap();
        Replay::new(&log, &[])
    });
    assert_eq!(checked, Ok(()));
}

/// Explicit scenario: reordered stale pushes, partitions and worker crashes
/// never regress versions and every Group member converges once faults stop.
///
/// Covers: story:3-93, story:3-107
#[test]
fn reordered_and_lost_pushes_never_regress_and_all_members_converge() {
    let env = TestEnv::new();
    let injections = vec![
        Injection {
            at: Duration::from_millis(100),
            host: LEAD.to_owned(),
            fault: Fault::Partition {
                peer: WORKERS[0].to_owned(),
            },
        },
        Injection {
            at: Duration::from_millis(220),
            host: WORKERS[1].to_owned(),
            fault: Fault::Crash,
        },
        Injection {
            at: Duration::from_millis(500),
            host: WORKERS[1].to_owned(),
            fault: Fault::Restart,
        },
        Injection {
            at: Duration::from_millis(650),
            host: LEAD.to_owned(),
            fault: Fault::Heal {
                peer: WORKERS[0].to_owned(),
            },
        },
    ];

    let log = run_control_state(env.seed(), env.start_time(), injections).unwrap();
    let saw_stale_ignored = log.records().iter().any(|record| {
        matches!(
            record.operation,
            Operation::Other(Seen::IgnoredStale { .. })
        )
    });
    assert!(
        saw_stale_ignored,
        "expected at least one reordered stale push to be ignored"
    );
}

struct ControlStateFaults;

impl ReferenceStateMachine for ControlStateFaults {
    type State = u64;
    type Transition = FaultPlanStep;

    fn init_state() -> BoxedStrategy<u64> {
        Just(0).boxed()
    }

    fn transitions(now: &u64) -> BoxedStrategy<FaultPlanStep> {
        const PAIRS: [(&str, &str); 2] = [(LEAD, WORKERS[0]), (LEAD, WORKERS[1])];
        let now = *now;
        prop_oneof![
            (
                0..150_u64,
                prop::sample::select(WORKERS.as_slice()),
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
                other => return Err(format!("control_state scenario injects no {other:?}")),
            };
            Ok(Injection {
                at: Duration::from_millis(step.at),
                host: step.host.clone(),
                fault,
            })
        })
        .collect()
}

/// Covers: story:3-93, story:3-107
#[test]
fn control_state_partitions_and_reordering_fault_plans() {
    let env = TestEnv::new();
    let checked = generator(&env).check::<ControlStateFaults, _>(1..8, |plan| {
        run_control_state(plan.seed, env.start_time(), injections(plan)?).map(drop)
    });
    if let Err(failure) = checked {
        panic!("{failure}");
    }
}
