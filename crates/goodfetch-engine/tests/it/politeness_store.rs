//! The Politeness store against a real Valkey (ADRs 0053 and 0059, Linux):
//! its Permits are leases that expire unless renewed, its rates are GCRA,
//! a Permit given back wakes the key's waiters through pub/sub, and every
//! time its scripts read is the server's. Each test runs its own server,
//! under a namespace of its own.

use std::collections::{BTreeMap, HashMap};
use std::ffi::OsString;
use std::net::Ipv4Addr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::home::Home;
use goodfetch_engine::politeness::store::{Lease, Pace, PolitenessStore, StoreError};
use goodfetch_engine::politeness::{
    Ending, Holder, KeyState, MemoryTables, PermitRule, Politeness, PolitenessOptions,
    RunPoliteness, RunPolitenessParts, SharedPoliteness, SharedPolitenessParts, SharedRule, Step,
    Taken, Verdict,
};
use goodfetch_engine::secrets::{Carried, Keychain, Secrets};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::politeness::store::PolitenessStoreConfiguration;
use goodfetch_protocol::settings::SettingsLayer;
#[cfg(target_os = "linux")]
use goodfetch_test_support::ValkeyCluster;
use goodfetch_test_support::{TestEnv, Valkey};
use rusqlite::Connection;
use secrecy::SecretString;
use serde_json::{Value, json};
use tokio::io::AsyncReadExt as _;

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
