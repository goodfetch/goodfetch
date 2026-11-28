//! The Politeness store against a real Valkey (ADRs 0053 and 0059, Linux):
//! its Permits are leases that expire unless renewed, its rates are GCRA,
//! a Permit given back wakes the key's waiters through pub/sub, and every
//! time its scripts read is the server's. Each test runs its own server,
//! under a namespace of its own.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::dns::HostMapping;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::{Home, RunStateLocation};
use goodfetch_engine::metrics::{METER, Metrics};
use goodfetch_engine::politeness::store::{
    HeartbeatParams, Lease, Pace, PolitenessStore, StoreError, TakeShared,
};
use goodfetch_engine::politeness::{
    Ending, Holder, KeyState, MemoryTables, PermitRule, Politeness, PolitenessOptions,
    RunPoliteness, RunPolitenessParts, SharedPoliteness, SharedPolitenessParts, SharedRule, Step,
    Taken, Verdict,
};
use goodfetch_engine::run_state::RunStateOptions;
use goodfetch_engine::secrets::{Carried, Keychain, Secrets};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{
    CallContext, CallError, Channel, Composition, Contribution, Dependencies, Engine, Handler,
    Identity,
};
use goodfetch_protocol::catalog::{
    Operation, OperationClass, OperationKind, PROTOCOL_VERSION, Product, Reach, Stability,
    schema_of,
};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::politeness::store::PolitenessStoreConfiguration;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
#[cfg(target_os = "linux")]
use goodfetch_test_support::ValkeyCluster;
use goodfetch_test_support::{CapturedLog, TestEnv, Toxiproxy, Valkey, model_trace};
use opentelemetry::metrics::MeterProvider as _;
use opentelemetry_sdk::metrics::SdkMeterProvider;
use prometheus::Encoder as _;
use proptest::prelude::*;
use rstest::rstest;
use rusqlite::Connection;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncReadExt as _, AsyncWriteExt as _, BufReader};
use tracing_subscriber::layer::SubscriberExt as _;

const LONG_LEASE: Duration = Duration::from_secs(30);

/// The store at `valkey`'s server, under `namespace`.
fn server(valkey: &Valkey, namespace: &str) -> PolitenessStoreConfiguration {
    PolitenessStoreConfiguration::read(&json!({
        "namespace": namespace,
        "addressing": { "kind": "server", "url": valkey.url() },
    }))
    .unwrap()
}

/// Makes `valkey` ask every client for `password`, as its operator would.
#[expect(
    clippy::disallowed_methods,
    reason = "the test sets the server's password through a client of its own, outside the Simulation (ADR 0060)"
)]
async fn require_password(valkey: &Valkey, password: &str) {
    let client = redis::Client::open(valkey.url()).unwrap();
    let mut connection = client.get_multiplexed_async_connection().await.unwrap();
    redis::cmd("CONFIG")
        .arg("SET")
        .arg("requirepass")
        .arg(password)
        .exec_async(&mut connection)
        .await
        .unwrap();
}

/// A store that asks for a password is reached with the password its
/// Secret reference resolves to; without it, or with another, the store
/// refuses the engine, which is told apart from a store out of reach.
///
/// Covers: setting:politeness.store
#[test]
fn a_store_asking_for_a_password_is_reached_with_it_only() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        require_password(&valkey, "the-store-password").await;
        let configuration = server(&valkey, "test");

        let without = PolitenessStore::connect(&configuration, None).await;
        let another =
            PolitenessStore::connect(&configuration, Some(&SecretString::from("another"))).await;
        let store = PolitenessStore::connect(
            &configuration,
            Some(&SecretString::from("the-store-password")),
        )
        .await
        .unwrap();
        let lease = store.take("example.com", "a", 1, LONG_LEASE).await.unwrap();

        assert!(
            matches!(without, Err(StoreError::Refused(_))),
            "{without:?}"
        );
        assert!(
            matches!(another, Err(StoreError::Refused(_))),
            "{another:?}"
        );
        assert_eq!(lease, Lease::Granted { in_flight: 1 });
    });
}

/// A `rediss://` store is spoken to in TLS only: the engine's first bytes
/// are a TLS handshake, never a command or its password in clear, and a
/// server that does not answer the handshake leaves the store out of reach.
///
/// Covers: setting:politeness.store
#[test]
fn a_tls_store_is_spoken_to_in_tls_only() {
    goodfetch_test_support::real_time_runtime().block_on(async {
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = listener.local_addr().unwrap().port();
        let configuration = PolitenessStoreConfiguration::read(&json!({
            "namespace": "test",
            "addressing": { "kind": "server", "url": format!("rediss://127.0.0.1:{port}") },
        }))
        .unwrap();
        let password = SecretString::from("the-store-password");
        let first_bytes = async {
            let (mut socket, _peer) = listener.accept().await.unwrap();
            let mut bytes = [0; 3];
            socket.read_exact(&mut bytes).await.unwrap();
            bytes
        };

        let (connected, bytes) = tokio::join!(
            PolitenessStore::connect(&configuration, Some(&password)),
            first_bytes,
        );

        assert_eq!(bytes[0], 0x16, "a TLS handshake record, read {bytes:?}");
        assert!(
            matches!(connected, Err(StoreError::Unreachable(_))),
            "{connected:?}"
        );
    });
}

/// A key admits Permits up to its cap across every holder, and room given
/// back is room another holder takes.
/// Covers: story:3-83
#[test]
fn a_full_key_refuses_a_permit_until_one_is_given_back() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let store = PolitenessStore::connect(&server(&valkey, "test"), None)
            .await
            .unwrap();

        let first = store.take("example.com", "a", 1, LONG_LEASE).await.unwrap();
        let second = store.take("example.com", "b", 1, LONG_LEASE).await.unwrap();
        store.give_back("example.com", "a").await.unwrap();
        let third = store.take("example.com", "b", 1, LONG_LEASE).await.unwrap();

        assert_eq!(first, Lease::Granted { in_flight: 1 });
        assert_eq!(second, Lease::Full { in_flight: 1 });
        assert_eq!(third, Lease::Granted { in_flight: 1 });
    });
}

/// A lease renewed while its request lasts keeps its room; one left to
/// expire frees it for another holder, and can no longer be renewed.
/// Covers: story:3-83
#[test]
fn a_lease_holds_while_renewed_and_frees_its_room_when_it_expires() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let store = PolitenessStore::connect(&server(&valkey, "test"), None)
            .await
            .unwrap();
        let lease = Duration::from_millis(400);

        store.take("example.com", "a", 1, lease).await.unwrap();
        let mut renewed = Vec::new();
        for _ in 0..3 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            renewed.push(store.renew("example.com", "a", lease).await.unwrap());
        }
        let while_renewed = store.take("example.com", "b", 1, lease).await.unwrap();
        tokio::time::sleep(Duration::from_millis(600)).await;
        let once_expired = store.take("example.com", "b", 1, lease).await.unwrap();
        let renewed_late = store.renew("example.com", "a", lease).await.unwrap();

        assert_eq!(renewed, [true, true, true]);
        assert_eq!(while_renewed, Lease::Full { in_flight: 1 });
        assert_eq!(once_expired, Lease::Granted { in_flight: 1 });
        assert!(!renewed_late);
    });
}

/// GCRA spaces a key's starts by its interval, every engine's together, and
/// tells a refused start how long to sleep; a burst admits that many more
/// at once.
/// Covers: story:3-83
#[test]
fn a_paced_key_admits_one_start_per_interval_and_names_the_wait() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let store = PolitenessStore::connect(&server(&valkey, "test"), None)
            .await
            .unwrap();
        let interval = Duration::from_millis(500);

        let first = store.pace("example.com", interval, 0).await.unwrap();
        let second = store.pace("example.com", interval, 0).await.unwrap();
        let Pace::Wait(wait) = second else {
            panic!("a second start within the interval waits, got {second:?}");
        };
        tokio::time::sleep(wait).await;
        let after_the_wait = store.pace("example.com", interval, 0).await.unwrap();
        let burst = [
            store.pace("example.org", interval, 1).await.unwrap(),
            store.pace("example.org", interval, 1).await.unwrap(),
            store.pace("example.org", interval, 1).await.unwrap(),
        ];

        assert_eq!(first, Pace::Now);
        assert!(
            wait > Duration::from_millis(300) && wait <= interval,
            "the wait is what is left of the interval, got {wait:?}"
        );
        assert_eq!(after_the_wait, Pace::Now);
        assert!(matches!(burst, [Pace::Now, Pace::Now, Pace::Wait(_)]));
    });
}

/// A Permit given back wakes whoever listens on its key, and only its key.
/// Covers: story:3-83
#[test]
fn a_permit_given_back_wakes_the_keys_waiters() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let store = PolitenessStore::connect(&server(&valkey, "test"), None)
            .await
            .unwrap();
        let mut wakes = store.wakes("example.com").await.unwrap();
        let mut other_wakes = store.wakes("example.org").await.unwrap();

        store.take("example.com", "a", 1, LONG_LEASE).await.unwrap();
        store.give_back("example.com", "a").await.unwrap();
        let woken = tokio::time::timeout(Duration::from_secs(5), wakes.next()).await;
        let other_woken =
            tokio::time::timeout(Duration::from_millis(200), other_wakes.next()).await;

        assert_eq!(woken.ok(), Some(Some(())));
        assert!(other_woken.is_err(), "another key's waiters sleep on");
    });
}

/// Two namespaces on one server share nothing.
/// Covers: story:3-83
#[test]
fn a_namespace_shares_nothing_with_another() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let first = PolitenessStore::connect(&server(&valkey, "first"), None)
            .await
            .unwrap();
        let second = PolitenessStore::connect(&server(&valkey, "second"), None)
            .await
            .unwrap();

        first.take("example.com", "a", 1, LONG_LEASE).await.unwrap();
        let elsewhere = second
            .take("example.com", "b", 1, LONG_LEASE)
            .await
            .unwrap();

        assert_eq!(elsewhere, Lease::Granted { in_flight: 1 });
    });
}

/// Permits send no script: each script is loaded once, when the store
/// connects, and every call after is an `EVALSHA` (ADR 0140's
/// `E6.script_sends`).
/// Covers: story:3-83
#[test]
fn permits_call_scripts_loaded_once_by_their_digest() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    let loaded = goodfetch_test_support::real_time_runtime().block_on(async {
        let store = PolitenessStore::connect(&server(&valkey, "test"), None)
            .await
            .unwrap();
        let loaded = calls(&valkey, "script");
        for holder in ["a", "b", "c"] {
            store
                .take("example.com", holder, 8, LONG_LEASE)
                .await
                .unwrap();
            store
                .pace("example.com", Duration::from_millis(1), 8)
                .await
                .unwrap();
            store
                .renew("example.com", holder, LONG_LEASE)
                .await
                .unwrap();
            store.give_back("example.com", holder).await.unwrap();
        }
        loaded
    });

    assert_eq!(calls(&valkey, "script"), loaded);
    assert_eq!(calls(&valkey, "eval"), 0);
    assert_eq!(calls(&valkey, "evalsha"), 12);
}

/// How many times the server ran `command`, from its `INFO commandstats`.
fn calls(valkey: &Valkey, command: &str) -> u64 {
    let stats = valkey.info("commandstats");
    let prefix = format!("cmdstat_{command}:calls=");
    stats
        .lines()
        .find_map(|line| line.strip_prefix(&prefix))
        .map_or(0, |rest| rest.split(',').next().unwrap().parse().unwrap())
}

/// Delegating cap rule with shared learned ceiling state, modeling both
/// domain and cloud-browser provider limits (stories 3-66 and 3-83).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct SharedCeiling {
    ceiling: Option<u64>,
}

#[derive(Default)]
struct CeilingMap(HashMap<String, SharedCeiling>);

impl KeyState for SharedCeiling {
    type Key = str;
    type Reads = ();
    type Writes = ();

    const DELEGATES: bool = true;
    const STORE_PART: Option<&'static str> = Some("test_ceiling");

    fn load(_: &Connection, _: &str, (): ()) -> rusqlite::Result<Self> {
        Ok(Self::default())
    }

    fn save(&self, _: &Connection, _: &str, (): &()) -> rusqlite::Result<()> {
        Ok(())
    }

    fn load_from(tables: &MemoryTables, key: &str, (): ()) -> Self {
        tables
            .get::<CeilingMap>()
            .and_then(|map| map.0.get(key).copied())
            .unwrap_or_default()
    }

    fn save_to(&self, tables: &mut MemoryTables, key: &str, (): &()) {
        tables
            .get_mut::<CeilingMap>()
            .0
            .insert(key.to_owned(), *self);
    }

    fn forget(tables: &mut MemoryTables, key: &str) {
        tables.get_mut::<CeilingMap>().0.remove(key);
    }

    fn store_key(key: &str) -> Option<&str> {
        Some(key)
    }

    fn encode_store(&self) -> Option<Vec<u8>> {
        serde_json::to_vec(self).ok()
    }

    fn load_store(bytes: Option<&[u8]>, (): ()) -> Option<Self> {
        Some(
            bytes
                .and_then(|bytes| serde_json::from_slice(bytes).ok())
                .unwrap_or_default(),
        )
    }

    fn encode_store_from(tables: &MemoryTables, key: &str) -> Option<Vec<u8>> {
        tables
            .get::<CeilingMap>()
            .and_then(|map| map.0.get(key))
            .and_then(|state| serde_json::to_vec(state).ok())
    }

    fn decode_store_into(tables: &mut MemoryTables, key: &str, bytes: &[u8]) {
        if let Ok(state) = serde_json::from_slice::<Self>(bytes) {
            tables
                .get_mut::<CeilingMap>()
                .0
                .insert(key.to_owned(), state);
        }
    }
}

struct CeilingRule {
    cap: u64,
    rate: Option<(Duration, u32)>,
}

impl PermitRule for CeilingRule {
    type State = SharedCeiling;

    fn concurrency(&self) -> Option<u64> {
        Some(self.cap)
    }

    fn effective_cap(&self, state: &SharedCeiling) -> Option<u64> {
        Some(state.ceiling.map_or(self.cap, |c| self.cap.min(c)))
    }

    fn store_pace(&self) -> Option<(Duration, u32)> {
        self.rate
    }

    fn reads(&self, _: Step) {}

    fn admit(&self, state: &SharedCeiling, in_flight: u64, _: SystemTime) -> Verdict {
        let cap = state.ceiling.map_or(self.cap, |c| self.cap.min(c));
        if in_flight < cap {
            Verdict::Go
        } else {
            Verdict::Busy
        }
    }

    fn started(&self, _: &mut SharedCeiling, _: SystemTime) {}

    fn ended(
        &self,
        state: &mut SharedCeiling,
        in_flight: u64,
        ending: Ending,
        _: SystemTime,
    ) -> (Option<SystemTime>, ()) {
        if matches!(ending, Ending::TooManyRequests(_)) {
            let tried = in_flight.saturating_add(1);
            let base = state.ceiling.map_or(tried, |c| c.min(tried));
            state.ceiling = Some((base / 2).max(1));
        }
        (None, ())
    }
}

fn ceiling_rule(cap: u64) -> SharedRule {
    SharedRule::new(CeilingRule { cap, rate: None })
}

fn test_holder(env: &TestEnv, run_id: &str, delegates: bool) -> Holder {
    Holder {
        run_id: run_id.to_owned(),
        epoch: 1,
        run_folder: env.folder().join(run_id),
        delegates,
    }
}

fn layer_values(layer: SettingsLayer, value: Value) -> Values {
    let Value::Object(map) = value else {
        panic!("settings layer must be a JSON object");
    };
    Values::read(layer, &map, Product::Collie).unwrap()
}

/// With a store configured, a `shared` key one engine uses is served from
/// that engine's Delegation in memory: 0 store round trips per attempt
/// (`E6.transactions = 0`), 0 script sends (`E6.script_sends = 0`), and
/// 0 opens (`E6.opens = 0`).
///
/// Covers: story:3-83, format:politeness-store
#[test]
fn delegated_key_is_served_from_memory_with_zero_store_round_trips() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let work = CountedWork::default();
        let options = PolitenessOptions {
            work: work.clone(),
            ..PolitenessOptions::default()
        };
        let store = PolitenessStore::connect(&server(&valkey, "deleg"), None)
            .await
            .unwrap();
        let politeness = Politeness::open_store(
            store,
            &options,
            "engine-a".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let holder = test_holder(&env, "run-1", true);
        let rule = ceiling_rule(4);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let evalsha_before = calls(&valkey, "evalsha");
        let scripts_before = calls(&valkey, "script");
        for _ in 0..10 {
            let Taken::Granted(permit) = politeness
                .take("example.com", &holder, rule.clone(), now)
                .await
                .unwrap()
            else {
                panic!("delegated take should grant");
            };
            politeness
                .give_back(permit, rule.clone(), Ending::Done, now)
                .await
                .unwrap();
        }
        let evalsha_after = calls(&valkey, "evalsha");

        // Only 1 EVALSHA for taking the Delegation itself; all 10 Permits
        // (take + give_back) cost 0 store round trips.
        assert_eq!(evalsha_after - evalsha_before, 1);
        assert_eq!(calls(&valkey, "script"), scripts_before);
        assert_eq!(calls(&valkey, "eval"), 0);
        assert_eq!(work.read(Row::PolitenessTransactions), 0);
        assert_eq!(work.read(Row::PolitenessDelegations), 1);
        assert_eq!(work.read(Row::PolitenessOpens), 0);
        assert_eq!(work.read(Row::PolitenessScriptSends), 0);

        owner.cancel();
        owner.wait().await;
    });
}

async fn open_store_pair(
    valkey: &Valkey,
    namespace: &str,
    work_a: &CountedWork,
    work_b: &CountedWork,
    owner: &TaskOwner,
) -> (Politeness, Politeness) {
    let store_a = PolitenessStore::connect(&server(valkey, namespace), None)
        .await
        .unwrap();
    let store_b = PolitenessStore::connect(&server(valkey, namespace), None)
        .await
        .unwrap();
    let a = Politeness::open_store(
        store_a,
        &PolitenessOptions {
            work: work_a.clone(),
            ..PolitenessOptions::default()
        },
        "engine-a".to_owned(),
        Clock::system(),
        owner,
    )
    .await
    .unwrap();
    let b = Politeness::open_store(
        store_b,
        &PolitenessOptions {
            work: work_b.clone(),
            ..PolitenessOptions::default()
        },
        "engine-b".to_owned(),
        Clock::system(),
        owner,
    )
    .await
    .unwrap();
    (a, b)
}

/// When a second engine needs a key another engine holds a Delegation for,
/// the store recalls the Delegation through the pub/sub wakes channel;
/// both engines then share Permits through the store within the cap and
/// within 3 store round trips per attempt, and return to Delegation once
/// one engine uses the key alone.
///
/// Covers: story:3-83
#[test]
fn second_engine_recalls_delegation_through_pubsub_and_shares_permits() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let work_a = CountedWork::default();
        let work_b = CountedWork::default();
        let (a, b) = open_store_pair(&valkey, "recall", &work_a, &work_b, &owner).await;
        let holder_a = test_holder(&env, "run-a", true);
        let holder_b = test_holder(&env, "run-b", true);
        let rule = ceiling_rule(1);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let Taken::Granted(permit_a) = a
            .take("example.com", &holder_a, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("engine A should take delegated permit");
        };
        assert_eq!(work_a.read(Row::PolitenessTransactions), 0);

        let mut wakes_b = b.wakes();
        assert_eq!(
            b.take("example.com", &holder_b, rule.clone(), now)
                .await
                .unwrap(),
            Taken::Wait(None)
        );
        let woken = tokio::time::timeout(Duration::from_secs(5), wakes_b.recv())
            .await
            .expect("recall wake arrives")
            .unwrap();
        assert_eq!(woken.key, "example.com");
        assert_eq!(
            b.take("example.com", &holder_b, rule.clone(), now)
                .await
                .unwrap(),
            Taken::Wait(None)
        );

        a.give_back(permit_a, rule.clone(), Ending::Done, now)
            .await
            .unwrap();
        let woken_after_return = tokio::time::timeout(Duration::from_secs(5), wakes_b.recv())
            .await
            .expect("return wake arrives")
            .unwrap();
        assert_eq!(woken_after_return.key, "example.com");

        let tx_before = work_b.read(Row::PolitenessTransactions);
        let Taken::Granted(permit_b) = b
            .take("example.com", &holder_b, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("engine B should take permit after A returned it");
        };
        b.give_back(permit_b, rule.clone(), Ending::Done, now)
            .await
            .unwrap();
        let tx_for_shared_attempt = work_b.read(Row::PolitenessTransactions) - tx_before;
        assert!(
            tx_for_shared_attempt <= 3,
            "shared attempt must stay within E6.shared_transactions cap of 3, got {tx_for_shared_attempt}"
        );

        let delegs_before = work_b.read(Row::PolitenessDelegations);
        let Taken::Granted(permit_b2) = b
            .take("example.com", &holder_b, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("engine B should re-acquire Delegation when alone");
        };
        b.give_back(permit_b2, rule.clone(), Ending::Done, now)
            .await
            .unwrap();
        assert_eq!(work_b.read(Row::PolitenessDelegations), delegs_before + 1);

        owner.cancel();
        owner.wait().await;
    });
}

/// With `politeness.delegation: false`, every Permit goes straight to the
/// store and no Delegation is acquired.
///
/// Covers: story:3-83
#[test]
fn politeness_delegation_false_keeps_per_permit_store_calls() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let work = CountedWork::default();
        let store = PolitenessStore::connect(&server(&valkey, "nodeleg"), None)
            .await
            .unwrap();
        let politeness = Politeness::open_store(
            store,
            &PolitenessOptions {
                work: work.clone(),
                ..PolitenessOptions::default()
            },
            "engine-a".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let holder = test_holder(&env, "run-1", false);
        let rule = ceiling_rule(2);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let Taken::Granted(permit) = politeness
            .take("example.com", &holder, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("undelegated take should grant");
        };
        politeness
            .give_back(permit, rule.clone(), Ending::Done, now)
            .await
            .unwrap();

        assert_eq!(work.read(Row::PolitenessDelegations), 0);
        assert!(work.read(Row::PolitenessTransactions) >= 2);
        assert!(work.read(Row::PolitenessTransactions) <= 3);

        owner.cancel();
        owner.wait().await;
    });
}

/// A Delegation in the store expires unless the engine's heartbeat renews
/// it: a live engine keeps renewing its Delegation past the TTL, while an
/// engine that stops heartbeating loses it so another engine can take over.
///
/// Covers: story:3-83
#[test]
fn delegation_expires_without_heartbeat_and_another_engine_takes_over() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let short_ttl = Duration::from_millis(450);
        let options = PolitenessOptions {
            store_delegation_ttl: short_ttl,
            store_permit_lease: Duration::from_secs(5),
            ..PolitenessOptions::default()
        };
        let store_a = PolitenessStore::connect(&server(&valkey, "expiry"), None)
            .await
            .unwrap();
        let store_b = PolitenessStore::connect(&server(&valkey, "expiry"), None)
            .await
            .unwrap();

        let acquired = store_a
            .delegate("example.com", "dead-engine", short_ttl)
            .await
            .unwrap();
        assert!(matches!(
            acquired,
            goodfetch_engine::politeness::store::DelegationAcquire::Granted { .. }
        ));

        let blocked = store_b
            .delegate("example.com", "engine-b", short_ttl)
            .await
            .unwrap();
        assert!(matches!(
            blocked,
            goodfetch_engine::politeness::store::DelegationAcquire::HeldByOther { .. }
        ));

        tokio::time::sleep(Duration::from_millis(600)).await;
        let b = Politeness::open_store(
            store_b.clone(),
            &options,
            "engine-b".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let holder_b = test_holder(&env, "run-b", true);
        let rule = ceiling_rule(2);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let Taken::Granted(permit) = b
            .take("example.com", &holder_b, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("engine B should take over after expiry");
        };

        tokio::time::sleep(Duration::from_millis(950)).await;
        let still_held = store_a
            .delegate("example.com", "other-engine", short_ttl)
            .await
            .unwrap();
        assert!(
            matches!(
                still_held,
                goodfetch_engine::politeness::store::DelegationAcquire::HeldByOther { ref holder }
                    if holder == "engine-b"
            ),
            "expected heartbeat to keep engine-b's Delegation live, got {still_held:?}"
        );

        b.give_back(permit, rule, Ending::Done, now).await.unwrap();
        owner.cancel();
        owner.wait().await;
    });
}

fn shared_politeness_for(
    env: &TestEnv,
    file_name: &str,
    engine: &'static str,
    secrets: Secrets,
    owner: &TaskOwner,
) -> Arc<SharedPoliteness> {
    Arc::new(SharedPoliteness::new(SharedPolitenessParts {
        path: env.home().join(file_name),
        options: PolitenessOptions::default(),
        tables: Vec::new(),
        holder_gone: |_, _| false,
        engine: engine.to_owned(),
        clock: Clock::system(),
        secrets,
        owner: owner.part(engine),
    }))
}

fn run_politeness_for(env: &TestEnv, shared: Arc<SharedPoliteness>, id: &str) -> RunPoliteness {
    RunPoliteness::new(RunPolitenessParts {
        shared,
        run_id: id.to_owned(),
        epoch: 1,
        folder: env.folder().join(id),
        admits: false,
        on_storage_failures: Box::new(|_, _| {}),
        on_store_degraded: None,
    })
}

fn precedence_settings(url: &str, sharing: Option<&str>) -> goodfetch_engine::settings::Resolved {
    let user_configuration = layer_values(
        SettingsLayer::UserConfiguration,
        json!({
            "politeness": {
                "store": {
                    "namespace": "precedence",
                    "addressing": { "kind": "server", "url": url },
                    "password": "env:TEST_VALKEY_PW",
                },
                "delegation": false,
            }
        }),
    );
    let run = sharing.map_or_else(Values::default, |sharing| {
        layer_values(
            SettingsLayer::Run,
            json!({ "politeness": { "sharing": sharing } }),
        )
    });
    resolve(
        &Layers {
            user_configuration,
            run,
            ..Layers::default()
        },
        Product::Collie,
        Product::Collie,
    )
    .unwrap()
}

fn secrets_with_env(env: &TestEnv, owner: &Arc<TaskOwner>, name: &str, value: &str) -> Secrets {
    let mut variables = BTreeMap::new();
    variables.insert(OsString::from(name), OsString::from(value));
    let home = Home::new(env.home().to_path_buf());
    let audit = Arc::new(AuditLog::of(&home, None, Arc::clone(owner)));
    Secrets::new(
        Carried {
            environment: variables,
            keychain: Keychain::absent(),
            home,
            entropy: Entropy::from_os().unwrap(),
        },
        audit,
        Arc::clone(owner),
        Clock::system(),
    )
}

/// With `politeness.store` configured (including a password resolved from a
/// Secret reference), `politeness.sharing: shared` uses the store,
/// `per_daemon` uses the per-user file, and `run` stays in memory.
///
/// Covers: story:3-83, setting:politeness.store
#[test]
fn store_precedence_routes_shared_to_store_per_daemon_to_file_and_run_to_memory() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        require_password(&valkey, "secret-store-pw").await;
        let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let secrets = secrets_with_env(&env, &owner, "TEST_VALKEY_PW", "secret-store-pw");
        let shared_a = shared_politeness_for(
            &env,
            "politeness-a.sqlite",
            "eng-a",
            secrets.clone(),
            &owner,
        );
        let shared_b = shared_politeness_for(&env, "politeness-b.sqlite", "eng-b", secrets, &owner);
        let shared_settings = precedence_settings(&valkey.url(), None);
        let per_daemon_settings = precedence_settings(&valkey.url(), Some("per_daemon"));
        let run_settings = precedence_settings(&valkey.url(), Some("run"));

        let (pol_a, holder_a) = run_politeness_for(&env, Arc::clone(&shared_a), "r-a")
            .open(&shared_settings)
            .await
            .unwrap();
        let (pol_b, holder_b) = run_politeness_for(&env, Arc::clone(&shared_b), "r-b")
            .open(&shared_settings)
            .await
            .unwrap();
        let rule = ceiling_rule(1);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let Taken::Granted(permit_a) = pol_a
            .take("shared.example", &holder_a, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("engine A should take shared permit");
        };
        assert_eq!(
            pol_b
                .take("shared.example", &holder_b, rule.clone(), now)
                .await
                .unwrap(),
            Taken::Wait(None)
        );
        assert!(!env.home().join("politeness-a.sqlite").exists());
        assert!(!env.home().join("politeness-b.sqlite").exists());

        let (pol_pd, holder_pd) = run_politeness_for(&env, Arc::clone(&shared_b), "r-pd")
            .open(&per_daemon_settings)
            .await
            .unwrap();
        let Taken::Granted(permit_pd) = pol_pd
            .take("shared.example", &holder_pd, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("per_daemon should ignore the store and use its own file");
        };
        assert!(env.home().join("politeness-b.sqlite").exists());

        let (pol_run, holder_run) = run_politeness_for(&env, shared_a, "r-run")
            .open(&run_settings)
            .await
            .unwrap();
        assert!(pol_run.held_in_memory());
        let Taken::Granted(permit_run) = pol_run
            .take("shared.example", &holder_run, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("run sharing should be isolated in memory");
        };
        assert!(!env.home().join("politeness-a.sqlite").exists());

        pol_a
            .give_back(permit_a, rule.clone(), Ending::Done, now)
            .await
            .unwrap();
        pol_pd
            .give_back(permit_pd, rule.clone(), Ending::Done, now)
            .await
            .unwrap();
        pol_run
            .give_back(permit_run, rule, Ending::Done, now)
            .await
            .unwrap();
        drop((pol_a, pol_b, pol_pd, pol_run, shared_b));
        owner.cancel();
        owner.wait().await;
    });
}

/// A cloud-browser provider's learned ceiling after a vendor refusal (429)
/// is written to the Politeness store and respected account-wide by another
/// engine on another machine.
///
/// Covers: story:3-66, story:3-83
#[test]
fn provider_ceiling_learned_from_refusal_is_shared_across_engines_through_the_store() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let store_a = PolitenessStore::connect(&server(&valkey, "provider"), None)
            .await
            .unwrap();
        let store_b = PolitenessStore::connect(&server(&valkey, "provider"), None)
            .await
            .unwrap();
        let a = Politeness::open_store(
            store_a,
            &PolitenessOptions::default(),
            "engine-a".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let b = Politeness::open_store(
            store_b,
            &PolitenessOptions::default(),
            "engine-b".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let holder_a = test_holder(&env, "run-a", false);
        let holder_b = test_holder(&env, "run-b", false);
        let rule = ceiling_rule(4);
        let provider_key = "provider cloud-vendor acct-1";
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let Taken::Granted(p1) = a
            .take(provider_key, &holder_a, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("p1 granted");
        };
        let Taken::Granted(p2) = a
            .take(provider_key, &holder_a, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("p2 granted");
        };
        a.give_back(p2, rule.clone(), Ending::TooManyRequests(None), now)
            .await
            .unwrap();

        let b_wait = b
            .take(provider_key, &holder_b, rule.clone(), now)
            .await
            .unwrap();
        assert_eq!(b_wait, Taken::Wait(None));

        a.give_back(p1, rule.clone(), Ending::Done, now)
            .await
            .unwrap();
        let Taken::Granted(pb1) = b
            .take(provider_key, &holder_b, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("pb1 granted");
        };
        let pb2 = b
            .take(provider_key, &holder_b, rule.clone(), now)
            .await
            .unwrap();
        assert_eq!(pb2, Taken::Wait(None));

        b.give_back(pb1, rule, Ending::Done, now).await.unwrap();
        owner.cancel();
        owner.wait().await;
    });
}

/// Against a real 3-primary Valkey Cluster container on Linux, every
/// Politeness key carries one `{<key>}` hash tag so scripts run on one
/// Cluster slot across keys hashing to different slots, with leases, GCRA
/// pacing and Delegations.
///
/// Covers: story:3-83
#[cfg(target_os = "linux")]
#[test]
fn cluster_store_shares_permits_rates_and_delegations_across_slots() {
    let env = TestEnv::new();
    let cluster = ValkeyCluster::start(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let config = PolitenessStoreConfiguration::read(&json!({
            "namespace": "cluster-test",
            "addressing": { "kind": "cluster", "nodes": cluster.urls() },
        }))
        .unwrap();
        let store_a = PolitenessStore::connect(&config, None).await.unwrap();
        let store_b = PolitenessStore::connect(&config, None).await.unwrap();

        for key in [
            "alpha.example",
            "beta.example",
            "gamma.example",
            "delta.example",
        ] {
            let g1 = store_a.take(key, "h1", 1, LONG_LEASE).await.unwrap();
            let g2 = store_b.take(key, "h2", 1, LONG_LEASE).await.unwrap();
            assert_eq!(g1, Lease::Granted { in_flight: 1 });
            assert_eq!(g2, Lease::Full { in_flight: 1 });
            store_a.give_back(key, "h1").await.unwrap();
            let p1 = store_a
                .pace(key, Duration::from_millis(200), 0)
                .await
                .unwrap();
            let p2 = store_b
                .pace(key, Duration::from_millis(200), 0)
                .await
                .unwrap();
            assert_eq!(p1, Pace::Now);
            assert!(matches!(p2, Pace::Wait(_)));
        }

        let a = Politeness::open_store(
            store_a,
            &PolitenessOptions::default(),
            "cluster-a".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let b = Politeness::open_store(
            store_b,
            &PolitenessOptions::default(),
            "cluster-b".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let holder_a = test_holder(&env, "cr-a", true);
        let holder_b = test_holder(&env, "cr-b", true);
        let rule = ceiling_rule(1);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let Taken::Granted(pa) = a
            .take("cluster-site.example", &holder_a, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("cluster delegated take granted");
        };
        let mut wakes_b = b.wakes();
        assert_eq!(
            b.take("cluster-site.example", &holder_b, rule.clone(), now)
                .await
                .unwrap(),
            Taken::Wait(None)
        );
        let woken = tokio::time::timeout(Duration::from_secs(5), wakes_b.recv())
            .await
            .expect("cluster recall wake arrives")
            .unwrap();
        assert_eq!(woken.key, "cluster-site.example");

        a.give_back(pa, rule.clone(), Ending::Done, now)
            .await
            .unwrap();
        let _ = tokio::time::timeout(Duration::from_secs(5), wakes_b.recv()).await;
        let Taken::Granted(pb) = b
            .take("cluster-site.example", &holder_b, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("cluster shared take granted after give_back");
        };
        b.give_back(pb, rule, Ending::Done, now).await.unwrap();

        owner.cancel();
        owner.wait().await;
    });
}

fn test_metering() -> (Metrics, prometheus::Registry, SdkMeterProvider) {
    let registry = prometheus::Registry::new();
    let exporter = opentelemetry_prometheus::exporter()
        .with_registry(registry.clone())
        .without_target_info()
        .scope_info_enabled(false)
        .build()
        .unwrap();
    let provider = SdkMeterProvider::builder().with_reader(exporter).build();
    let metrics = Metrics::new(&provider.meter(METER));
    (metrics, registry, provider)
}

fn render_prometheus(registry: &prometheus::Registry) -> String {
    let mut bytes = Vec::new();
    prometheus::TextEncoder::new()
        .encode(&registry.gather(), &mut bytes)
        .unwrap();
    String::from_utf8(bytes).unwrap()
}

fn store_degraded_gauge(registry: &prometheus::Registry) -> Option<f64> {
    render_prometheus(registry).lines().find_map(|line| {
        line.strip_prefix("goodfetch_politeness_store_degraded ")
            .and_then(|value| value.trim().parse::<f64>().ok())
    })
}

#[expect(
    clippy::disallowed_types,
    reason = "integration test reserves one free loopback port so two Toxiproxy proxies share a port on distinct loopback addresses (ADR 0060)"
)]
fn free_loopback_port_for_proxy() -> u16 {
    let listener = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap();
    listener.local_addr().unwrap().port()
}

/// Granted degraded shares are computed solely by the Valkey Lua script
/// (`heartbeat.lua`), never a Rust copy (ADR 0053): property-tested against a
/// real Valkey container over random engine counts, caps, heartbeats, shrinks,
/// leaves, and stale-engine reservations.
#[derive(Clone, Debug)]
enum ShareOp {
    Heartbeat {
        engine: usize,
        timeout_ms: u64,
        cap: u64,
    },
    Leave {
        engine: usize,
    },
}

struct ObservedShare {
    cap: u64,
    timeout_ms: u64,
    granted: u64,
    counted_reserved: u64,
}

async fn run_granted_shares_prop_case(
    store: &PolitenessStore,
    key: &str,
    engine_count: usize,
    base_cap: u64,
    ops: Vec<ShareOp>,
) {
    let mut active: BTreeMap<usize, ObservedShare> = BTreeMap::new();
    let mut high_water_cap: u64 = 0;
    for op in ops {
        match op {
            ShareOp::Heartbeat {
                engine,
                timeout_ms,
                cap,
            } => {
                let idx = engine % engine_count;
                let eng_id = format!("e-{idx}");
                let reported_cap = cap.max(base_cap);
                let acked_share = active.get(&idx).map_or(0, |o| o.granted);
                let ack = store
                    .heartbeat(&HeartbeatParams {
                        key,
                        engine_id: &eng_id,
                        heartbeat_ttl: Duration::from_secs(30),
                        degraded_max: Duration::from_secs(60),
                        timeout: Duration::from_millis(timeout_ms),
                        cap: Some(reported_cap),
                        acked_share,
                        permit_lease: Duration::from_secs(30),
                        in_flight_tokens: &[],
                    })
                    .await
                    .unwrap();

                let other_reserved: u64 = active
                    .iter()
                    .filter(|(k, _)| **k != idx)
                    .map(|(_, o)| o.counted_reserved)
                    .sum();
                let expected_max_cap = active
                    .iter()
                    .filter(|(k, _)| **k != idx)
                    .map(|(_, o)| o.cap)
                    .fold(reported_cap, u64::max);
                let expected_max_timeout = active
                    .iter()
                    .filter(|(k, _)| **k != idx)
                    .map(|(_, o)| o.timeout_ms)
                    .fold(timeout_ms, u64::max);
                let expected_live =
                    u64::try_from(active.keys().filter(|k| **k != idx).count() + 1).unwrap();

                assert_eq!(ack.max_cap, expected_max_cap);
                assert_eq!(ack.max_timeout_ms, expected_max_timeout);
                assert_eq!(ack.live_engines, expected_live);
                assert!(ack.granted_share >= 1, "share must be >= 1, got {ack:?}");
                if ack.granted_share > 1 {
                    assert!(
                        ack.granted_share + other_reserved <= ack.max_cap,
                        "non-minimum share {} + other {} exceeded cap {}",
                        ack.granted_share,
                        other_reserved,
                        ack.max_cap
                    );
                    assert!(
                        ack.granted_share <= ack.max_cap / ack.live_engines,
                        "share {} exceeded cap {} / live {}",
                        ack.granted_share,
                        ack.max_cap,
                        ack.live_engines
                    );
                }
                let counted_reserved = ack.granted_share.max(acked_share);
                high_water_cap = high_water_cap.max(ack.max_cap);
                active.insert(
                    idx,
                    ObservedShare {
                        cap: reported_cap,
                        timeout_ms,
                        granted: ack.granted_share,
                        counted_reserved,
                    },
                );
                let sum_granted: u64 = active.values().map(|o| o.granted).sum();
                let min_ones =
                    u64::try_from(active.values().filter(|o| o.granted == 1).count()).unwrap();
                assert!(
                    sum_granted <= high_water_cap + min_ones,
                    "sum of granted shares {sum_granted} exceeded peak cap {high_water_cap} + {min_ones} min-1 engines"
                );
            }
            ShareOp::Leave { engine } => {
                let idx = engine % engine_count;
                store.leave_engine(key, &format!("e-{idx}")).await.unwrap();
                active.remove(&idx);
            }
        }
    }
    settle_granted_shares(store, key, &mut active).await;
}

async fn settle_granted_shares(
    store: &PolitenessStore,
    key: &str,
    active: &mut BTreeMap<usize, ObservedShare>,
) {
    if active.is_empty() {
        return;
    }
    let ids: Vec<usize> = active.keys().copied().collect();
    let mut final_cap = 0;
    for _round in 0..2 {
        for &idx in &ids {
            let eng_id = format!("e-{idx}");
            let obs = active.get(&idx).unwrap();
            let ack = store
                .heartbeat(&HeartbeatParams {
                    key,
                    engine_id: &eng_id,
                    heartbeat_ttl: Duration::from_secs(30),
                    degraded_max: Duration::from_secs(60),
                    timeout: Duration::from_millis(obs.timeout_ms),
                    cap: Some(obs.cap),
                    acked_share: obs.granted,
                    permit_lease: Duration::from_secs(30),
                    in_flight_tokens: &[],
                })
                .await
                .unwrap();
            final_cap = ack.max_cap;
            let entry = active.get_mut(&idx).unwrap();
            entry.counted_reserved = ack.granted_share.max(entry.granted);
            entry.granted = ack.granted_share;
        }
    }
    let sum_granted: u64 = active.values().map(|o| o.granted).sum();
    let live = u64::try_from(active.len()).unwrap();
    if final_cap >= live {
        assert!(
            sum_granted <= final_cap,
            "settled shares {sum_granted} exceeded final cap {final_cap} with {live} live engines"
        );
    } else {
        assert_eq!(sum_granted, live);
    }
}

async fn verify_stale_reservation_expiry(store: &PolitenessStore) {
    let key = "stale-reservation.example";
    let hb = |engine_id: &'static str, ttl_ms: u64, deg_ms: u64, acked: u64| HeartbeatParams {
        key,
        engine_id,
        heartbeat_ttl: Duration::from_millis(ttl_ms),
        degraded_max: Duration::from_millis(deg_ms),
        timeout: Duration::from_millis(200),
        cap: Some(4),
        acked_share: acked,
        permit_lease: Duration::from_secs(5),
        in_flight_tokens: &[],
    };
    let a1 = store.heartbeat(&hb("stale-a", 250, 900, 0)).await.unwrap();
    assert_eq!(a1.granted_share, 4);

    let b1 = store
        .heartbeat(&hb("live-b", 5_000, 5_000, 0))
        .await
        .unwrap();
    assert_eq!(b1.granted_share, 1, "B waits for A's shrink ack");

    let a2 = store
        .heartbeat(&hb("stale-a", 250, 900, a1.granted_share))
        .await
        .unwrap();
    assert_eq!(a2.granted_share, 2);

    let b_before_ack = store
        .heartbeat(&hb("live-b", 5_000, 5_000, b1.granted_share))
        .await
        .unwrap();
    assert_eq!(b_before_ack.granted_share, 1);

    let a3 = store
        .heartbeat(&hb("stale-a", 250, 900, a2.granted_share))
        .await
        .unwrap();
    assert_eq!(a3.granted_share, 2);

    let b2 = store
        .heartbeat(&hb("live-b", 5_000, 5_000, b_before_ack.granted_share))
        .await
        .unwrap();
    assert_eq!(b2.granted_share, 2);

    tokio::time::sleep(Duration::from_millis(350)).await;
    let take_one = |tok: &'static str| {
        store.take_shared(
            key,
            "live-b",
            tok,
            Some(4),
            Duration::from_secs(5),
            None,
            Duration::from_secs(60),
            None,
        )
    };
    let t1 = take_one("live-b:1").await.unwrap();
    let t2 = take_one("live-b:2").await.unwrap();
    let t3 = take_one("live-b:3").await.unwrap();
    assert!(matches!(t1, TakeShared::Granted { .. }), "{t1:?}");
    assert!(matches!(t2, TakeShared::Granted { .. }), "{t2:?}");
    assert!(
        matches!(t3, TakeShared::Full { in_flight: 4 }),
        "stale A's share of 2 must stay reserved, got {t3:?}"
    );

    tokio::time::sleep(Duration::from_millis(900)).await;
    let t3_after = take_one("live-b:3").await.unwrap();
    assert!(
        matches!(t3_after, TakeShared::Granted { in_flight: 3 }),
        "once degraded_max + timeout elapses, reservation frees: {t3_after:?}"
    );
}

///
/// Covers: story:3-84
#[test]
fn granted_shares_property_tested_against_valkey_lua() {
    let env = TestEnv::new();
    let valkey = Valkey::start(&env);
    let runtime = goodfetch_test_support::real_time_runtime();
    let store = runtime.block_on(async {
        PolitenessStore::connect(&server(&valkey, "prop"), None)
            .await
            .unwrap()
    });

    let op_strategy = prop_oneof![
        4 => (0usize..5, 100u64..2000, 1u64..=10).prop_map(|(engine, timeout_ms, cap)| {
            ShareOp::Heartbeat {
                engine,
                timeout_ms,
                cap,
            }
        }),
        1 => (0usize..5).prop_map(|engine| ShareOp::Leave { engine }),
    ];

    let mut runner =
        proptest::test_runner::TestRunner::new(proptest::test_runner::Config::with_cases(16));
    let case_id = std::sync::atomic::AtomicU64::new(0);
    runner
        .run(
            &(
                2usize..=5,
                1u64..=10,
                prop::collection::vec(op_strategy, 8..24),
            ),
            |(engine_count, base_cap, ops)| {
                let id = case_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                let key = format!("prop-case-{id}.example");
                runtime.block_on(run_granted_shares_prop_case(
                    &store,
                    &key,
                    engine_count,
                    base_cap,
                    ops,
                ));
                Ok(())
            },
        )
        .unwrap();

    runtime.block_on(verify_stale_reservation_expiry(&store));
}

/// Stopping the Valkey container mid-run and starting it again exercises
/// Delegation survival until expiry, degraded-share admission, stopping at
/// `politeness.degraded_max` while keeping Runs live, ops metric updates, and
/// automatic reconnection with backoff (ADRs 0053, 0060 and 0142).
///
/// Covers: story:3-84, setting:politeness.degraded_max
#[rstest]
#[case::delegation_on(true)]
#[case::delegation_off(false)]
fn mid_run_valkey_container_stop_and_restart_degrades_stops_and_reconnects(
    #[case] delegation: bool,
) {
    let env = TestEnv::new();
    let mut valkey = Valkey::start_tcp(&env);
    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let (metrics, registry, _provider) = test_metering();
        let options = PolitenessOptions {
            store_delegation_ttl: Duration::from_millis(1500),
            store_permit_lease: Duration::from_millis(600),
            store_heartbeat_ttl: Duration::from_millis(500),
            store_request_timeout: Duration::from_millis(200),
            store_degraded_max: Duration::from_millis(3800),
            store_reconnect_initial: Duration::from_millis(60),
            metrics: Some(metrics),
            ..PolitenessOptions::default()
        };
        let store = PolitenessStore::connect(&server(&valkey, "restart"), None)
            .await
            .unwrap()
            .with_command_timeout(Duration::from_millis(250));
        let politeness = Politeness::open_store(
            store,
            &options,
            "engine-restart".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let mut degraded_rx = politeness.store_degraded();
        let holder = test_holder(&env, "run-restart", delegation);
        let rule = ceiling_rule(2);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let Taken::Granted(p1) = politeness
            .take("outage.example", &holder, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("initial take granted");
        };
        assert!(!degraded_rx.borrow().degraded);

        valkey.stop();

        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if degraded_rx.borrow().degraded {
                    break;
                }
                degraded_rx.changed().await.unwrap();
            }
        })
        .await
        .expect("store enters degraded mode after Valkey stop");
        assert_eq!(store_degraded_gauge(&registry), Some(1.0));

        if delegation {
            let Taken::Granted(p_del) = politeness
                .take("outage.example", &holder, rule.clone(), now)
                .await
                .unwrap()
            else {
                panic!("cut-off Delegation holder must keep serving until Delegation expires");
            };
            politeness
                .give_back(p_del, rule.clone(), Ending::Done, now)
                .await
                .unwrap();
        }
        politeness
            .give_back(p1, rule.clone(), Ending::Done, now)
            .await
            .unwrap();

        // Wait past Delegation/lease expiry + drain window (`timeout`), then
        // verify degraded admission within the granted share succeeds before
        // `politeness.degraded_max`.
        let wait_expiry = if delegation { 1600 } else { 850 };
        tokio::time::sleep(Duration::from_millis(wait_expiry)).await;
        let d1 = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if let Taken::Granted(p) = politeness
                    .take("outage.example", &holder, rule.clone(), now)
                    .await
                    .unwrap()
                {
                    break p;
                }
                assert!(
                    !degraded_rx.borrow().admission_stopped,
                    "degraded_max elapsed before degraded admission opened"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("degraded admission within granted share succeeds before degraded_max");
        let Taken::Granted(d2) = politeness
            .take("outage.example", &holder, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("second degraded permit within share=2 succeeds");
        };
        let over_share = politeness
            .take("outage.example", &holder, rule.clone(), now)
            .await
            .unwrap();
        assert_eq!(over_share, Taken::Wait(None));
        politeness
            .give_back(d1, rule.clone(), Ending::Done, now)
            .await
            .unwrap();
        politeness
            .give_back(d2, rule.clone(), Ending::Done, now)
            .await
            .unwrap();

        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if degraded_rx.borrow().admission_stopped {
                    break;
                }
                degraded_rx.changed().await.unwrap();
            }
        })
        .await
        .expect("degraded admission stops at politeness.degraded_max");
        let stopped_take = politeness
            .take("outage.example", &holder, rule.clone(), now)
            .await
            .unwrap();
        assert_eq!(stopped_take, Taken::Wait(None));

        let mut wakes = politeness.wakes();
        valkey.restart();
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if !degraded_rx.borrow().degraded {
                    break;
                }
                degraded_rx.changed().await.unwrap();
            }
        })
        .await
        .expect("engine reconnects automatically once Valkey restarts");
        assert_eq!(store_degraded_gauge(&registry), Some(0.0));
        let _ = tokio::time::timeout(Duration::from_secs(2), wakes.recv()).await;

        let Taken::Granted(after_reconnect) = politeness
            .take("outage.example", &holder, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("admission resumes after reconnection");
        };
        politeness
            .give_back(after_reconnect, rule, Ending::Done, now)
            .await
            .unwrap();

        owner.cancel();
        owner.wait().await;
    });
}

/// Toxiproxy cuts off one of two engines reached through `network.resolve`
/// (each engine's proxy on its own loopback address `127.0.0.2` and `127.0.0.3`
/// sharing one port), tested with `politeness.delegation` both `true` and
/// `false`, covering Delegation expiry under outage, stale reservation on
/// connected engines, combined in-flight bound, `politeness.degraded_max`
/// stop, and atomic registration of in-flight permits on reconnect BEFORE the
/// store frees the reservation.
///
/// Covers: story:3-84, setting:politeness.degraded_max
#[rstest]
#[case::delegation_on(true)]
#[case::delegation_off(false)]
fn toxiproxy_cut_off_one_engine_respects_shares_delegation_expiry_and_reconnect(
    #[case] delegation: bool,
) {
    let env = TestEnv::new();
    let valkey = Valkey::start_tcp(&env);
    let valkey_addr = valkey.addr().unwrap();
    let toxiproxy = Toxiproxy::start(&env).unwrap();

    let port = free_loopback_port_for_proxy();
    let addr_a = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)), port);
    let addr_b = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 3)), port);
    toxiproxy
        .proxy_on("politeness-a", valkey_addr, addr_a)
        .unwrap();
    toxiproxy
        .proxy_on("politeness-b", valkey_addr, addr_b)
        .unwrap();

    let shared_config = PolitenessStoreConfiguration::read(&json!({
        "namespace": format!("toxi-{}", if delegation { "on" } else { "off" }),
        "addressing": { "kind": "server", "url": format!("redis://localhost:{port}") },
    }))
    .unwrap();
    let resolve_a = HostMapping::new([("localhost", vec![addr_a.ip()])]);
    let resolve_b = HostMapping::new([("localhost", vec![addr_b.ip()])]);

    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let options = PolitenessOptions {
            store_delegation_ttl: Duration::from_millis(500),
            store_permit_lease: Duration::from_millis(450),
            store_heartbeat_ttl: Duration::from_millis(350),
            store_request_timeout: Duration::from_millis(150),
            store_degraded_max: Duration::from_millis(1800),
            store_reconnect_initial: Duration::from_millis(50),
            ..PolitenessOptions::default()
        };
        let store_a = PolitenessStore::connect_with_resolve(&shared_config, None, &resolve_a)
            .await
            .unwrap()
            .with_command_timeout(Duration::from_millis(200));
        let store_b = PolitenessStore::connect_with_resolve(&shared_config, None, &resolve_b)
            .await
            .unwrap()
            .with_command_timeout(Duration::from_millis(200));

        let a = Politeness::open_store(
            store_a,
            &options,
            "engine-a".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let b = Politeness::open_store(
            store_b,
            &options,
            "engine-b".to_owned(),
            Clock::system(),
            &owner,
        )
        .await
        .unwrap();
        let mut degraded_a = a.store_degraded();
        let holder_a = test_holder(&env, "run-a", delegation);
        let holder_b = test_holder(&env, "run-b", delegation);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

        let rule4 = ceiling_rule(4);
        let Taken::Granted(init_a) = a
            .take("shared.example", &holder_a, rule4.clone(), now)
            .await
            .unwrap()
        else {
            panic!("init_a granted");
        };
        let mut wakes_b = b.wakes();
        let init_b = loop {
            match b
                .take("shared.example", &holder_b, rule4.clone(), now)
                .await
                .unwrap()
            {
                Taken::Granted(p) => break p,
                Taken::Wait(_) => {
                    let _ = tokio::time::timeout(Duration::from_millis(300), wakes_b.recv()).await;
                }
                other @ Taken::Refused(_) => panic!("unexpected init_b {other:?}"),
            }
        };
        a.give_back(init_a, rule4.clone(), Ending::Done, now)
            .await
            .unwrap();
        b.give_back(init_b, rule4.clone(), Ending::Done, now)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(550)).await;

        let rule2 = ceiling_rule(2);
        if delegation {
            let Taken::Granted(p_only) = a
                .take("delegated.example", &holder_a, rule2.clone(), now)
                .await
                .unwrap()
            else {
                panic!("A acquires delegated.example");
            };
            a.give_back(p_only, rule2.clone(), Ending::Done, now)
                .await
                .unwrap();
        }

        toxiproxy.cut("politeness-a").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if degraded_a.borrow().degraded {
                    break;
                }
                degraded_a.changed().await.unwrap();
            }
        })
        .await
        .expect("engine A detects Toxiproxy cut-off");
        assert!(!b.store_degraded().borrow().degraded);

        if delegation {
            let Taken::Granted(p_del) = a
                .take("delegated.example", &holder_a, rule2.clone(), now)
                .await
                .unwrap()
            else {
                panic!("A keeps serving delegated.example until Delegation expires");
            };
            a.give_back(p_del, rule2.clone(), Ending::Done, now)
                .await
                .unwrap();
        }

        tokio::time::sleep(Duration::from_millis(700)).await;

        let Taken::Granted(b1) = b
            .take("shared.example", &holder_b, rule4.clone(), now)
            .await
            .unwrap()
        else {
            panic!("B takes 1st permit within unreserved cap");
        };
        let Taken::Granted(b2) = b
            .take("shared.example", &holder_b, rule4.clone(), now)
            .await
            .unwrap()
        else {
            panic!("B takes 2nd permit within unreserved cap");
        };
        assert_eq!(
            b.take("shared.example", &holder_b, rule4.clone(), now)
                .await
                .unwrap(),
            Taken::Wait(None),
            "connected B must respect stale A's reserved share of 2"
        );

        let Taken::Granted(a1) = a
            .take("shared.example", &holder_a, rule4.clone(), now)
            .await
            .unwrap()
        else {
            panic!("cut-off A takes 1st degraded permit");
        };
        let Taken::Granted(a2) = a
            .take("shared.example", &holder_a, rule4.clone(), now)
            .await
            .unwrap()
        else {
            panic!("cut-off A takes 2nd degraded permit");
        };
        assert_eq!(
            a.take("shared.example", &holder_a, rule4.clone(), now)
                .await
                .unwrap(),
            Taken::Wait(None),
            "cut-off A must not exceed its granted share of 2"
        );

        toxiproxy.heal("politeness-a").unwrap();
        tokio::time::timeout(Duration::from_secs(3), async {
            loop {
                if !degraded_a.borrow().degraded {
                    break;
                }
                degraded_a.changed().await.unwrap();
            }
        })
        .await
        .expect("engine A reconnects after Toxiproxy heal");

        assert_eq!(
            b.take("shared.example", &holder_b, rule4.clone(), now)
                .await
                .unwrap(),
            Taken::Wait(None),
            "reconnecting A must register its in-flight permits before freeing its reservation"
        );

        a.give_back(a1, rule4.clone(), Ending::Done, now)
            .await
            .unwrap();
        a.give_back(a2, rule4.clone(), Ending::Done, now)
            .await
            .unwrap();
        b.give_back(b1, rule4.clone(), Ending::Done, now)
            .await
            .unwrap();
        b.give_back(b2, rule4.clone(), Ending::Done, now)
            .await
            .unwrap();

        tokio::time::sleep(Duration::from_millis(200)).await;
        toxiproxy.cut("politeness-a").unwrap();
        tokio::time::timeout(Duration::from_secs(4), async {
            loop {
                if degraded_a.borrow().admission_stopped {
                    break;
                }
                degraded_a.changed().await.unwrap();
            }
        })
        .await
        .expect("cut-off A stops degraded admission at politeness.degraded_max");
        assert_eq!(
            a.take("shared.example", &holder_a, rule4.clone(), now)
                .await
                .unwrap(),
            Taken::Wait(None)
        );

        tokio::time::sleep(Duration::from_millis(400)).await;
        let mut full_b = Vec::new();
        for _ in 0..4 {
            let Taken::Granted(p) = b
                .take("shared.example", &holder_b, rule4.clone(), now)
                .await
                .unwrap()
            else {
                panic!("connected B should reach full cap=4 once A's reservation frees");
            };
            full_b.push(p);
        }
        for p in full_b {
            b.give_back(p, rule4.clone(), Ending::Done, now)
                .await
                .unwrap();
        }

        owner.cancel();
        owner.wait().await;
    });
}

/// `politeness.degraded_max` is machine-scoped (user configuration or Operator
/// policy only, ADR 0086), and `SharedPoliteness` reads both
/// `politeness.degraded_max` and `network.resolve` when opening a store.
///
/// Covers: story:3-84, setting:politeness.degraded_max
#[test]
fn politeness_degraded_max_is_machine_scoped_and_read_by_shared_politeness() {
    let env = TestEnv::new();
    let valkey = Valkey::start_tcp(&env);
    let valkey_addr = valkey.addr().unwrap();
    let toxiproxy = Toxiproxy::start(&env).unwrap();
    let port = free_loopback_port_for_proxy();
    let proxy_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)), port);
    toxiproxy
        .proxy_on("settings-proxy", valkey_addr, proxy_addr)
        .unwrap();

    assert!(
        goodfetch_protocol::setting_scopes::of(
            &goodfetch_protocol::politeness::POLITENESS_DEGRADED_MAX
        )
        .is_empty(),
        "politeness.degraded_max must be machine-scoped (no program scopes)"
    );

    goodfetch_test_support::real_time_runtime().block_on(async {
        let owner = Arc::new(TaskOwner::root(BlockingPolicy::Inline));
        let secrets = secrets_with_env(&env, &owner, "UNUSED_PW", "unused");
        let shared = shared_politeness_for(&env, "degraded-max.sqlite", "eng-cfg", secrets, &owner);
        let run = run_politeness_for(&env, Arc::clone(&shared), "run-cfg");
        let user_map = json!({
            "politeness": {
                "degraded_max": 1200,
                "store": {
                    "namespace": "cfg-test",
                    "addressing": {
                        "kind": "server",
                        "url": format!("redis://localhost:{port}")
                    }
                }
            },
            "network": {
                "resolve": {
                    "localhost": ["127.0.0.2"]
                }
            }
        });
        let resolved = resolve(
            &Layers {
                user_configuration: Values::read(
                    SettingsLayer::UserConfiguration,
                    user_map.as_object().unwrap(),
                    Product::Collie,
                )
                .unwrap(),
                ..Layers::default()
            },
            Product::Collie,
            Product::Collie,
        )
        .unwrap();
        let (politeness, holder) = run.open(&resolved).await.unwrap();
        assert_eq!(
            politeness.store_degraded().borrow().degraded_max,
            Duration::from_millis(1200)
        );
        let rule = ceiling_rule(1);
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let Taken::Granted(p) = politeness
            .take("cfg.example", &holder, rule.clone(), now)
            .await
            .unwrap()
        else {
            panic!("store reached through network.resolve grants permit");
        };
        politeness
            .give_back(p, rule, Ending::Done, now)
            .await
            .unwrap();

        drop(politeness);
        drop(run);
        drop(shared);
        owner.cancel();
        owner.wait().await;
    });
}

/// Captures `goodfetch::politeness` transitions across Delegation acquire,
/// cut-off, Delegation expiry, degraded-share admission, reconnect registering
/// in-flight permits first, and stop at `politeness.degraded_max` (ADRs 0053,
/// 0060 and 0142).
async fn drive_degraded_shares_trace(env: &TestEnv, toxiproxy: &Toxiproxy, proxy: SocketAddr) {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let config = PolitenessStoreConfiguration::read(&json!({
        "namespace": "trace-ds",
        "addressing": { "kind": "server", "url": format!("redis://{proxy}") },
    }))
    .unwrap();
    let options = PolitenessOptions {
        store_delegation_ttl: Duration::from_millis(450),
        store_permit_lease: Duration::from_millis(450),
        store_heartbeat_ttl: Duration::from_millis(350),
        store_request_timeout: Duration::from_millis(150),
        store_degraded_max: Duration::from_millis(1400),
        store_reconnect_initial: Duration::from_millis(50),
        ..PolitenessOptions::default()
    };
    let store = PolitenessStore::connect(&config, None)
        .await
        .unwrap()
        .with_command_timeout(Duration::from_millis(200));
    let politeness = Politeness::open_store(
        store,
        &options,
        "trace-e1".to_owned(),
        Clock::system(),
        &owner,
    )
    .await
    .unwrap();
    let mut degraded = politeness.store_degraded();
    let holder = test_holder(env, "trace-run", true);
    let rule = ceiling_rule(2);
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);

    let Taken::Granted(p1) = politeness
        .take("trace.example", &holder, rule.clone(), now)
        .await
        .unwrap()
    else {
        panic!("delegated take");
    };
    politeness
        .give_back(p1, rule.clone(), Ending::Done, now)
        .await
        .unwrap();

    toxiproxy.cut("trace-proxy").unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !degraded.borrow().degraded {
            degraded.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(650)).await;
    let Taken::Granted(d1) = politeness
        .take("trace.example", &holder, rule.clone(), now)
        .await
        .unwrap()
    else {
        panic!("degraded take 1");
    };
    politeness
        .give_back(d1, rule.clone(), Ending::Done, now)
        .await
        .unwrap();
    let Taken::Granted(d2) = politeness
        .take("trace.example", &holder, rule.clone(), now)
        .await
        .unwrap()
    else {
        panic!("degraded take 2");
    };

    toxiproxy.heal("trace-proxy").unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while degraded.borrow().degraded {
            degraded.changed().await.unwrap();
        }
    })
    .await
    .unwrap();
    politeness
        .give_back(d2, rule.clone(), Ending::Done, now)
        .await
        .unwrap();

    toxiproxy.cut("trace-proxy").unwrap();
    tokio::time::timeout(Duration::from_secs(4), async {
        while !degraded.borrow().admission_stopped {
            degraded.changed().await.unwrap();
        }
    })
    .await
    .unwrap();

    owner.cancel();
    owner.wait().await;
}

///
/// Covers: story:3-84
#[test]
fn politeness_store_traces_degraded_shares_transitions() {
    const COMMITTED_TRACE: &str = include_str!("../trace/degraded-shares.ndjson");
    let env = TestEnv::new();
    let valkey = Valkey::start_tcp(&env);
    let toxiproxy = Toxiproxy::start(&env).unwrap();
    let proxy = toxiproxy
        .proxy(
            "trace-proxy",
            valkey.addr().unwrap(),
            IpAddr::V4(Ipv4Addr::LOCALHOST),
        )
        .unwrap();

    let captured = CapturedLog::default();
    let writer = captured.clone();
    let subscriber = tracing_subscriber::registry()
        .with(model_trace(&["goodfetch::politeness"], move || {
            writer.clone()
        }));
    let _traced = tracing::subscriber::set_default(subscriber);

    goodfetch_test_support::real_time_runtime()
        .block_on(drive_degraded_shares_trace(&env, &toxiproxy, proxy));

    let actual = String::from_utf8(captured.written()).unwrap();
    let actual_transitions: Vec<String> = actual
        .lines()
        .filter_map(|line| {
            serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v["transition"].as_str().map(str::to_owned))
        })
        .collect();
    for expected in [
        "acquire_delegation",
        "cut_off",
        "expire_delegation",
        "enter_degraded",
        "take_degraded",
        "reconnect",
        "stop_degraded",
    ] {
        assert!(
            actual_transitions.iter().any(|t| t == expected),
            "missing transition `{expected}` in captured trace: {actual_transitions:?}"
        );
    }
    assert!(
        COMMITTED_TRACE
            .lines()
            .all(|line| serde_json::from_str::<Value>(line).is_ok())
    );
}

const TEST_POLITENESS_TAKE: Operation = Operation {
    name: "test.politeness_take",
    description: "Opens a live Run's store-backed Politeness and takes + returns one permit.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<Value>,
    result: schema_of::<Value>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: PROTOCOL_VERSION,
};

async fn handle_test_politeness_take(
    params: Value,
    context: CallContext,
) -> Result<Value, CallError> {
    let run_id = RunId::parse(params["run_id"].as_str().unwrap()).unwrap();
    let key = params["key"].as_str().unwrap().to_owned();
    let run = context.runs().live(run_id).await.unwrap();
    let settings = run.identity.settings().unwrap().resolved();
    let (politeness, holder) = run.politeness.open(settings).await.unwrap();
    let rule = ceiling_rule(2);
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
    if let Taken::Granted(permit) = politeness
        .take(&key, &holder, rule.clone(), now)
        .await
        .unwrap()
    {
        politeness
            .give_back(permit, rule, Ending::Done, now)
            .await
            .unwrap();
    }
    Ok(json!({ "ok": true }))
}

#[expect(
    clippy::disallowed_methods,
    reason = "synchronous test setup writes configuration.toml and seeds Entropy outside an async worker"
)]
fn build_degraded_e2e_engine(
    env: &TestEnv,
    port: u16,
) -> (
    Engine,
    Dependencies,
    prometheus::Registry,
    opentelemetry_sdk::metrics::SdkMeterProvider,
) {
    let version = ProductVersion::parse("0.0.0").unwrap();
    let per_major = Home::new(env.home().to_owned()).per_major(&version);
    std::fs::create_dir_all(&per_major).unwrap();
    std::fs::write(
        per_major.join(goodfetch_protocol::home::USER_CONFIGURATION_FILE),
        format!(
            concat!(
                "version = \"0.1\"\n\n",
                "[settings.politeness]\n",
                "degraded_max = 1000\n\n",
                "[settings.politeness.store]\n",
                "namespace = \"e2e-degraded\"\n\n",
                "[settings.politeness.store.addressing]\n",
                "kind = \"server\"\n",
                "url = \"redis://localhost:{port}\"\n\n",
                "[settings.network.resolve]\n",
                "localhost = [\"127.0.0.2\"]\n",
            ),
            port = port
        ),
    )
    .unwrap();

    let (metrics, registry, provider) = test_metering();
    let composition = Composition::default()
        .with(goodfetch_engine::contribution())
        .with(
            Contribution::new("test-politeness").with_handler(Handler::new(
                &TEST_POLITENESS_TAKE,
                handle_test_politeness_take,
            )),
        );
    let engine = Engine::new(
        Identity {
            product: Product::Collie,
            product_version: version,
            channel: Channel::CratesIo,
        },
        &composition,
    )
    .unwrap()
    .with_metrics(metrics);

    let deps = Dependencies {
        environment: BTreeMap::new(),
        keychain: Keychain::absent(),
        home: Home::new(env.home().to_owned()),
        clock: Clock::system(),
        entropy: Entropy::seeded(env.seed()),
        blocking: BlockingPolicy::Inline,
        run_state_location: RunStateLocation::PerMajor,
        run_state: RunStateOptions::default(),
        politeness: PolitenessOptions {
            store_delegation_ttl: Duration::from_millis(450),
            store_permit_lease: Duration::from_millis(450),
            store_heartbeat_ttl: Duration::from_millis(350),
            store_request_timeout: Duration::from_millis(150),
            store_degraded_max: Duration::from_millis(1000),
            store_reconnect_initial: Duration::from_millis(50),
            ..PolitenessOptions::default()
        },
    };
    (engine, deps, registry, provider)
}

async fn e2e_send(writer: &mut goodfetch_engine::transport::ConnectionWriter, msg: Value) {
    writer
        .write_all(format!("{msg}\n").as_bytes())
        .await
        .unwrap();
}

async fn e2e_next_reply(
    lines: &mut tokio::io::Lines<BufReader<goodfetch_engine::transport::ConnectionReader>>,
    id: u64,
    events: &mut Vec<Value>,
) -> Value {
    loop {
        let line = tokio::time::timeout(Duration::from_secs(5), lines.next_line())
            .await
            .expect("engine answers")
            .unwrap()
            .unwrap();
        let msg: Value = serde_json::from_str(&line).unwrap();
        if msg.get("method").and_then(Value::as_str) == Some("$/events") {
            for env in msg["params"]["events"].as_array().into_iter().flatten() {
                if env["event"].as_str() == Some("politeness.store_degraded") {
                    events.push(env["data"].clone());
                }
            }
            continue;
        }
        if msg.get("id") == Some(&json!(id)) {
            return msg;
        }
    }
}

async fn e2e_wait_for_event(
    lines: &mut tokio::io::Lines<BufReader<goodfetch_engine::transport::ConnectionReader>>,
    events: &mut Vec<Value>,
    pred: fn(&Value) -> bool,
) {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if events.iter().any(pred) {
                return;
            }
            let line = lines.next_line().await.unwrap().unwrap();
            let msg: Value = serde_json::from_str(&line).unwrap();
            if msg.get("method").and_then(Value::as_str) == Some("$/events") {
                for env in msg["params"]["events"].as_array().into_iter().flatten() {
                    if env["event"].as_str() == Some("politeness.store_degraded") {
                        events.push(env["data"].clone());
                    }
                }
            }
        }
    })
    .await
    .expect("expected politeness.store_degraded event on Run stream");
}

/// End-to-end through `Engine` over the in-memory transport with a real
/// Valkey + Toxiproxy: a store outage emits `politeness.store_degraded` on
/// each affected Run's `events.subscribe` stream (entering degraded, stopping
/// admission at `politeness.degraded_max`, and recovering on reconnect),
/// sets `goodfetch.politeness.store_degraded` to `1` then `0`, and records
/// `politeness_store_degraded` in the Run's finished report `warnings`
/// (ADRs 0024, 0050 and 0053).
///
/// Covers: story:3-84, setting:politeness.degraded_max, event:politeness.store_degraded
#[test]
fn degraded_period_emits_run_event_metric_and_result_warning() {
    let env = TestEnv::new();
    let valkey = Valkey::start_tcp(&env);
    let toxiproxy = Toxiproxy::start(&env).unwrap();
    let port = free_loopback_port_for_proxy();
    let proxy_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2)), port);
    toxiproxy
        .proxy_on("engine-e2e", valkey.addr().unwrap(), proxy_addr)
        .unwrap();

    let (engine, deps, registry, _provider) = build_degraded_e2e_engine(&env, port);

    goodfetch_test_support::real_time_runtime().block_on(async {
        let (network, dialer) = transport::in_memory();
        let talked = async {
            let (reader, mut writer) = dialer.connect().await.unwrap().split();
            let mut lines = BufReader::new(reader).lines();
            let mut degraded_events: Vec<Value> = Vec::new();

            e2e_send(
                &mut writer,
                json!({
                    "jsonrpc": "2.0", "id": 1, "method": "hello",
                    "params": {
                        "protocol_version": PROTOCOL_VERSION.to_string(),
                        "surface": "sdk_typescript",
                        "needs": "collie",
                        "product_version": "0.0.0",
                    }
                }),
            )
            .await;
            let _ = e2e_next_reply(&mut lines, 1, &mut degraded_events).await;

            e2e_send(
                &mut writer,
                json!({ "jsonrpc": "2.0", "id": 2, "method": "run.start", "params": { "run_id": "run-e2e" } }),
            )
            .await;
            let started = e2e_next_reply(&mut lines, 2, &mut degraded_events).await;
            let run_id = started["result"]["run_id"]
                .as_str()
                .unwrap_or_else(|| panic!("run.start failed: {started}"))
                .to_owned();

            e2e_send(
                &mut writer,
                json!({ "jsonrpc": "2.0", "id": 3, "method": "events.subscribe", "params": { "run_id": run_id } }),
            )
            .await;
            e2e_send(
                &mut writer,
                json!({ "jsonrpc": "2.0", "method": "$/credit", "params": { "id": 3, "credits": 64 } }),
            )
            .await;

            e2e_send(
                &mut writer,
                json!({
                    "jsonrpc": "2.0", "id": 4, "method": "test.politeness_take",
                    "params": { "run_id": run_id, "key": "e2e.example" }
                }),
            )
            .await;
            let taken = e2e_next_reply(&mut lines, 4, &mut degraded_events).await;
            assert_eq!(taken["result"]["ok"], json!(true), "{taken}");

            toxiproxy.cut("engine-e2e").unwrap();
            e2e_wait_for_event(&mut lines, &mut degraded_events, |e| {
                e["degraded"] == json!(true) && e["admission_stopped"] == json!(false)
            })
            .await;
            assert_eq!(store_degraded_gauge(&registry), Some(1.0));

            e2e_wait_for_event(&mut lines, &mut degraded_events, |e| {
                e["degraded"] == json!(true) && e["admission_stopped"] == json!(true)
            })
            .await;

            toxiproxy.heal("engine-e2e").unwrap();
            e2e_wait_for_event(&mut lines, &mut degraded_events, |e| {
                e["degraded"] == json!(false) && e["admission_stopped"] == json!(false)
            })
            .await;
            assert_eq!(store_degraded_gauge(&registry), Some(0.0));

            e2e_send(
                &mut writer,
                json!({ "jsonrpc": "2.0", "id": 5, "method": "run.finish", "params": { "run_id": run_id } }),
            )
            .await;
            let finished = e2e_next_reply(&mut lines, 5, &mut degraded_events).await;
            let warnings = finished["result"]["warnings"].as_array().unwrap();
            assert!(
                warnings
                    .iter()
                    .any(|w| w["code"].as_str() == Some("politeness_store_degraded")),
                "Run report must record politeness_store_degraded warning, got {warnings:?}"
            );
            assert!(
                degraded_events
                    .iter()
                    .all(|e| e["run_id"].as_str() == Some(run_id.as_str())),
                "all events must name the affected run_id: {degraded_events:?}"
            );

            writer.shutdown().await.unwrap();
        };

        let (served, ()) = tokio::join!(engine.run_attached(Network::InMemory(network), deps), talked);
        served.unwrap();
    });
}
