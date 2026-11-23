//! The Politeness store against a real Valkey (ADRs 0053 and 0059, Linux):
//! its Permits are leases that expire unless renewed, its rates are GCRA,
//! a Permit given back wakes the key's waiters through pub/sub, and every
//! time its scripts read is the server's. Each test runs its own server,
//! under a namespace of its own.

use std::net::Ipv4Addr;
use std::time::Duration;

use goodfetch_engine::politeness::store::{Lease, Pace, PolitenessStore, StoreError};
use goodfetch_protocol::politeness::store::PolitenessStoreConfiguration;
use goodfetch_test_support::{TestEnv, Valkey};
use secrecy::SecretString;
use serde_json::json;
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
