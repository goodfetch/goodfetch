//! The sign-in driver behind its interface (spec #592's Seam 3), on real
//! SQLite in the test's Goodfetch home and the paused `Clock`: the
//! single-user link signs the OS user in with a cookie of its own, kept in
//! the accounts store so a new driver over the same home (a restarted
//! Daemon) still knows it; a sign-in ends after its idle expiry unused,
//! when revoked or signed out, and when the Console key rotates, for every
//! major sharing the key; one address may try 10 times a minute; each
//! sign-in's `POST`s carry its CSRF token; and every change is in the
//! Audit log, by the OS user through the Console.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock (ADR 0059) and read the Audit log's file"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::net::{IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_protocol::audit::Listener;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::Compatibility;
use goodfetch_test_support::{TestEnv, runtime};
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{
    AccountsStore, ConsoleKey, OfflineKeyRefused, PeopleError, Principal, SignInDriver,
    SignInFailure, SignInPolicy, SignInRefused, SignIns, SingleUserLink, StoreError,
};
use crate::audit::AuditLog;
use crate::clock::Clock;
use crate::home::Home;
use crate::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_protocol::connection::Surface;

const LINK_TOKEN: &str = "0f1e2d3c4b5a69788796a5b4c3d2e1f00f1e2d3c4b5a69788796a5b4c3d2e1f0";

/// The address the tests sign in from.
const HERE: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// How long the tests' sign-ins last and how often an address may try: the
/// settings' defaults, 30 days and 10 attempts a minute.
const POLICY: SignInPolicy = SignInPolicy {
    idle_expiry: Duration::from_hours(30 * 24),
    rate: 10,
};

/// ADR 0059's test `Clock` start, 2026-01-05T09:00:00Z.
fn start() -> SystemTime {
    UNIX_EPOCH + Duration::from_hours(491_001)
}

fn version(text: &str) -> ProductVersion {
    ProductVersion::parse(text).unwrap()
}

/// A driver of the Daemon of `major`'s version over `env`'s home, as the
/// OS user `ada`.
fn driver_of(env: &TestEnv, major: &str, policy: SignInPolicy) -> SignInDriver {
    let home = Home::new(env.home().to_path_buf());
    SignInDriver::new(SignIns {
        link: SingleUserLink::new(LINK_TOKEN.to_owned()),
        person: "ada".to_owned(),
        store: AccountsStore::at(home.accounts_store(&version(major))),
        key: ConsoleKey::at(home.console_key()),
        audit: AuditLog::of(&home, None),
        policy,
        clock: Clock::paused(start()),
        owner: Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    })
}

fn driver(env: &TestEnv) -> SignInDriver {
    driver_of(env, "1.0.0", POLICY)
}

/// A driver over `env`'s accounts store and Console key whose Audit log
/// cannot be written: it lies under a file, not a folder.
fn driver_unrecorded(env: &TestEnv) -> SignInDriver {
    let home = Home::new(env.home().to_path_buf());
    let file = env.home().join("not-a-folder");
    std::fs::create_dir_all(env.home()).unwrap();
    std::fs::write(&file, b"").unwrap();
    SignInDriver::new(SignIns {
        link: SingleUserLink::new(LINK_TOKEN.to_owned()),
        person: "ada".to_owned(),
        store: AccountsStore::at(home.accounts_store(&version("1.0.0"))),
        key: ConsoleKey::at(home.console_key()),
        audit: AuditLog::of(&Home::new(file), None),
        policy: POLICY,
        clock: Clock::paused(start()),
        owner: Arc::new(TaskOwner::root(BlockingPolicy::Inline)),
    })
}

/// Every Audit log entry of `env`'s home, as JSON.
fn audit_entries(env: &TestEnv) -> Vec<Value> {
    let home = Home::new(env.home().to_path_buf());
    let connection = Connection::open(home.audit_log()).unwrap();
    let mut statement = connection
        .prepare("SELECT entry FROM entries ORDER BY sequence")
        .unwrap();
    statement
        .query_map([], |row| row.get::<_, String>(0))
        .unwrap()
        .map(|entry| serde_json::from_str(&entry.unwrap()).unwrap())
        .collect()
}

/// Covers: story:12-1
#[test]
fn the_single_user_link_signs_the_os_user_in_with_a_cookie() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);

        let signed_in = driver
            .with_link(LINK_TOKEN, HERE)
            .await
            .expect("the link's own token signs in");

        assert_eq!(
            driver.principal(signed_in.cookie()).await,
            Some(Principal::OsUser)
        );
        assert!(
            !signed_in.cookie().contains(LINK_TOKEN),
            "the cookie is not the link's token"
        );
    });
}

#[test]
fn each_sign_in_has_a_cookie_of_its_own() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);

        let first = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let second = driver.with_link(LINK_TOKEN, HERE).await.unwrap();

        assert_ne!(first.cookie(), second.cookie());
        assert_eq!(
            driver.principal(first.cookie()).await,
            Some(Principal::OsUser)
        );
        assert_eq!(
            driver.principal(second.cookie()).await,
            Some(Principal::OsUser)
        );
    });
}

#[test]
fn a_token_the_link_does_not_carry_signs_nobody_in() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let wrong = LINK_TOKEN.replace('0', "1");

        assert!(matches!(
            driver.with_link(&wrong, HERE).await,
            Err(SignInFailure::Refused(SignInRefused::TokenWrong))
        ));
        assert!(matches!(
            driver.with_link("", HERE).await,
            Err(SignInFailure::Refused(SignInRefused::TokenMissing))
        ));
    });
}

#[test]
fn a_cookie_no_sign_in_was_given_is_nobodys() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let signed_in = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let (id, secret) = signed_in.cookie().split_once('.').unwrap();
        let forged = format!("{id}.{}", secret.replace(|_: char| true, "0"));

        assert_eq!(
            driver.principal(LINK_TOKEN).await,
            None,
            "the link's token is no cookie"
        );
        assert_eq!(
            driver.principal(&forged).await,
            None,
            "a sign-in's id with another secret"
        );
        assert_eq!(driver.principal("").await, None);
    });
}

/// Covers: story:12-5
#[test]
fn a_sign_in_outlives_its_daemon() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let cookie = driver(&env)
            .with_link(LINK_TOKEN, HERE)
            .await
            .unwrap()
            .cookie()
            .to_owned();

        let restarted = driver(&env);

        assert_eq!(restarted.principal(&cookie).await, Some(Principal::OsUser));
    });
}

/// Covers: story:12-5, setting:console.sign_in.idle_expiry
#[test]
fn a_sign_in_ends_after_its_idle_expiry_unused_and_each_use_starts_it_again() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let thirty_days = Duration::from_hours(30 * 24);
        let used = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let unused = driver.with_link(LINK_TOKEN, HERE).await.unwrap();

        tokio::time::advance(thirty_days).await;
        assert_eq!(
            driver.principal(used.cookie()).await,
            Some(Principal::OsUser),
            "30 days unused is still within its expiry"
        );
        tokio::time::advance(Duration::from_secs(1)).await;

        assert_eq!(
            driver.principal(unused.cookie()).await,
            None,
            "30 days and a second unused"
        );
        assert_eq!(
            driver.principal(used.cookie()).await,
            Some(Principal::OsUser),
            "its use a second ago started its expiry again"
        );
        let listed = driver.list().await.unwrap();
        assert_eq!(
            listed.len(),
            1,
            "the expired sign-in is listed no more: {listed:?}"
        );
    });
}

/// Covers: story:12-5
#[test]
fn a_shorter_idle_expiry_from_the_operator_policy_ends_sign_ins_sooner() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let policy = SignInPolicy {
            idle_expiry: Duration::from_secs(60),
            ..POLICY
        };
        let driver = driver_of(&env, "1.0.0", policy);
        let signed_in = driver.with_link(LINK_TOKEN, HERE).await.unwrap();

        tokio::time::advance(Duration::from_secs(61)).await;

        assert_eq!(driver.principal(signed_in.cookie()).await, None);
    });
}

/// The driver below `sign_in.list` and `sign_in.revoke`, which
/// `lasting_sign_ins::a_revoked_sign_in_ends_and_rotating_the_console_key_ends_them_all`
/// reaches over the protocol.
/// Covers: story:12-5, story:12-111
#[test]
fn a_revoked_sign_in_ends_and_the_others_last() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let lost = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let kept = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let (lost_id, _) = lost.cookie().split_once('.').unwrap();

        let listed = driver.list().await.unwrap();
        let revoked = driver
            .revoke(lost_id, Some(Surface::Console))
            .await
            .unwrap();

        assert_eq!(listed.len(), 2);
        assert!(listed.iter().all(|sign_in| sign_in.person == "ada"));
        assert!(listed.iter().any(|sign_in| sign_in.id == lost_id));
        assert!(revoked);
        assert_eq!(driver.principal(lost.cookie()).await, None);
        assert_eq!(
            driver.principal(kept.cookie()).await,
            Some(Principal::OsUser)
        );
        assert!(
            !driver
                .revoke(lost_id, Some(Surface::Console))
                .await
                .unwrap(),
            "an ended sign-in revokes nothing"
        );
    });
}

/// The driver below `console.rotate_key`, which `lasting_sign_ins` reaches
/// over the protocol.
/// Covers: story:12-111
#[test]
fn rotating_the_console_key_ends_every_sign_in_of_every_major() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let first = driver_of(&env, "1.0.0", POLICY);
        let second = driver_of(&env, "2.0.0", POLICY);
        let of_first = first.with_link(LINK_TOKEN, HERE).await.unwrap();
        let also_first = first.with_link(LINK_TOKEN, HERE).await.unwrap();
        let of_second = second.with_link(LINK_TOKEN, HERE).await.unwrap();

        let signed_out = first.rotate_key(Some(Surface::Console)).await.unwrap();

        assert_eq!(signed_out, 2, "the first major's own sign-ins");
        assert_eq!(first.principal(of_first.cookie()).await, None);
        assert_eq!(first.principal(also_first.cookie()).await, None);
        assert_eq!(
            second.principal(of_second.cookie()).await,
            None,
            "the other major shares the key, so its sign-ins end too"
        );
        let after = first.with_link(LINK_TOKEN, HERE).await.unwrap();
        assert_eq!(
            first.principal(after.cookie()).await,
            Some(Principal::OsUser),
            "a sign-in under the new key"
        );
    });
}

/// Covers: story:12-110, setting:console.sign_in.rate
#[test]
fn the_eleventh_attempt_in_a_minute_from_one_address_is_refused() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let elsewhere = IpAddr::V4(Ipv4Addr::new(127, 0, 0, 2));
        let wrong = LINK_TOKEN.replace('0', "1");
        for attempt in 1..=9 {
            assert!(
                matches!(
                    driver.with_link(&wrong, HERE).await,
                    Err(SignInFailure::Refused(SignInRefused::TokenWrong))
                ),
                "attempt {attempt}, a wrong token, still counts"
            );
        }
        assert!(
            driver.with_link(LINK_TOKEN, HERE).await.is_ok(),
            "the tenth"
        );

        let eleventh = driver.with_link(LINK_TOKEN, HERE).await;

        assert!(matches!(
            eleventh,
            Err(SignInFailure::Refused(SignInRefused::RateExceeded))
        ));
        assert!(
            driver.with_link(LINK_TOKEN, elsewhere).await.is_ok(),
            "another address has its own count"
        );
        tokio::time::advance(Duration::from_secs(60)).await;
        assert!(
            driver.with_link(LINK_TOKEN, HERE).await.is_ok(),
            "a minute after the first attempt"
        );
    });
}

/// Covers: story:12-110
#[test]
fn a_sign_ins_csrf_token_is_its_own_and_changes_with_the_key() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let one = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let other = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let token = driver.csrf_token(one.cookie()).await.unwrap();

        assert!(driver.csrf_checked(one.cookie(), &token).await);
        assert!(!driver.csrf_checked(one.cookie(), "").await, "no token");
        assert!(
            !driver.csrf_checked(other.cookie(), &token).await,
            "another sign-in's token"
        );
        assert_eq!(
            driver.csrf_token(LINK_TOKEN).await,
            None,
            "no sign-in, no token"
        );
        driver.rotate_key(None).await.unwrap();
        assert!(!driver.csrf_checked(one.cookie(), &token).await);
    });
}

/// Covers: story:12-111
#[test]
fn signing_out_ends_the_sign_in() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let signed_in = driver.with_link(LINK_TOKEN, HERE).await.unwrap();

        assert!(driver.sign_out(signed_in.cookie()).await);

        assert_eq!(driver.principal(signed_in.cookie()).await, None);
        assert!(!driver.sign_out(signed_in.cookie()).await);
    });
}

/// Covers: story:12-109
#[test]
fn each_change_is_in_the_audit_log_by_the_os_user_through_the_console() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let signed_in = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let (id, _) = signed_in.cookie().split_once('.').unwrap();
        driver.revoke(id, Some(Surface::Console)).await.unwrap();
        driver.rotate_key(Some(Surface::Console)).await.unwrap();
        driver.refused(Listener::Console, "token_wrong").await;

        let entries = audit_entries(&env);

        let ada = json!({ "kind": "person", "name": "ada" });
        let shown: Vec<(&Value, &Value, &Value)> = entries
            .iter()
            .map(|entry| (&entry["kind"], &entry["actor"], &entry["surface"]))
            .collect();
        assert_eq!(
            shown,
            [
                (&json!("signed_in"), &ada, &json!("console")),
                (&json!("sign_in_revoked"), &ada, &json!("console")),
                (&json!("console_key_rotated"), &ada, &json!("console")),
                (
                    &json!("endpoint_authentication_failed"),
                    &json!({ "kind": "client_surface", "surface": "console" }),
                    &Value::Null
                ),
            ]
        );
        assert_eq!(entries[0]["details"]["sign_in"], json!(id));
        assert_eq!(
            entries[3]["details"],
            json!({ "listener": "console", "reason": "token_wrong" })
        );
        let text = serde_json::to_string(&entries).unwrap();
        assert!(
            !text.contains(signed_in.cookie()),
            "no cookie is in the Audit log"
        );
        assert!(!text.contains(LINK_TOKEN), "nor the link's token");
    });
}

/// A sign-in the Audit log cannot record is refused: it signs nobody in,
/// and the accounts store keeps none.
/// Covers: story:12-109
#[test]
fn a_sign_in_the_audit_log_cannot_record_signs_nobody_in() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let signed_in = driver_unrecorded(&env).with_link(LINK_TOKEN, HERE).await;

        assert_eq!(
            (
                matches!(
                    signed_in,
                    Err(SignInFailure::People(PeopleError::Unrecorded(_)))
                ),
                driver(&env).list().await.unwrap()
            ),
            (true, Vec::new())
        );
    });
}

/// A revocation the Audit log cannot record still ends the sign-in, since
/// refusing it would keep a sign-in its person wants ended, and answers
/// that the entry is missing.
/// Covers: story:12-109
#[test]
fn a_revocation_the_audit_log_cannot_record_ends_the_sign_in_and_says_so() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let signed_in = driver(&env).with_link(LINK_TOKEN, HERE).await.unwrap();
        let (id, _) = signed_in.cookie().split_once('.').unwrap();

        let revoked = driver_unrecorded(&env)
            .revoke(id, Some(Surface::Console))
            .await;

        assert_eq!(
            (
                matches!(revoked, Err(PeopleError::Unrecorded(_))),
                driver(&env).principal(signed_in.cookie()).await
            ),
            (true, None)
        );
    });
}

/// A key rotation the Audit log cannot record still ends every sign-in,
/// and answers that the entry is missing.
/// Covers: story:12-109
#[test]
fn a_key_rotation_the_audit_log_cannot_record_ends_every_sign_in_and_says_so() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let signed_in = driver(&env).with_link(LINK_TOKEN, HERE).await.unwrap();

        let rotated = driver_unrecorded(&env)
            .rotate_key(Some(Surface::Console))
            .await;

        assert_eq!(
            (
                matches!(rotated, Err(PeopleError::Unrecorded(_))),
                driver(&env).principal(signed_in.cookie()).await
            ),
            (true, None)
        );
    });
}

/// An engine that makes the new accounts store while another connection
/// writes the new file adds its sign-in once that write ends: its switch of
/// the file to WAL waits, as SQLite answers that switch busy at once
/// (#1773's class).
///
/// Covers: format:accounts-store
#[test]
fn an_engine_making_the_accounts_store_beside_another_write_adds_its_sign_in() {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let store = AccountsStore::at(home.accounts_store(&version("1.0.0")));

    let inserted = crate::sqlite_wal::tests::opened_beside_a_write(store.path(), || {
        store.insert(&goodfetch_protocol::people::SignInRow {
            id: "a".to_owned(),
            verifier: "b".to_owned(),
            person: "ada".to_owned(),
            created: "2026-01-05T09:00:00Z".to_owned(),
            last_used: "2026-01-05T09:00:00Z".to_owned(),
            wrapping_key: "c".to_owned(),
        })
    });

    assert_eq!(inserted, Ok(()));
}

/// Covers: format:accounts-store
#[test]
fn the_accounts_store_keeps_no_cookie() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let signed_in = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let (_, secret) = signed_in.cookie().split_once('.').unwrap();
        let home = Home::new(env.home().to_path_buf());

        let path = home.accounts_store(&version("1.0.0"));
        // Its WAL stays beside it as its connections close (#2110).
        let mut store = std::fs::read(&path).unwrap();
        let mut wal = path.into_os_string();
        wal.push("-wal");
        store.extend(std::fs::read(wal).unwrap_or_default());

        let text = String::from_utf8_lossy(&store);
        assert!(
            !text.contains(secret),
            "the store holds the cookie's secret"
        );
    });
}

/// Covers: format:accounts-store
#[rstest::rstest]
#[case::next_preview_minor_which_counts_as_a_major("0.2", Compatibility::NewerMajor)]
#[case::newer_major("1.0", Compatibility::NewerMajor)]
fn an_accounts_store_of_a_version_this_engine_cannot_read_is_refused(
    #[case] found: &str,
    #[case] compatibility: Compatibility,
) {
    let env = TestEnv::new();
    let home = Home::new(env.home().to_path_buf());
    let path = home.accounts_store(&version("1.0.0"));
    let store = AccountsStore::at(path.clone());
    assert_eq!(store.all().unwrap(), Vec::new());
    store
        .insert(&goodfetch_protocol::people::SignInRow {
            id: "a".to_owned(),
            verifier: "b".to_owned(),
            person: "ada".to_owned(),
            created: "2026-01-05T09:00:00Z".to_owned(),
            last_used: "2026-01-05T09:00:00Z".to_owned(),
            wrapping_key: "c".to_owned(),
        })
        .unwrap();
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE format SET version = ?1", [found])
        .unwrap();

    let refused = store.find("a");

    assert_eq!(
        refused,
        Err(StoreError::Format {
            found: found.to_owned(),
            compatibility
        })
    );
}

#[rstest::rstest]
#[case::unix(&[("USER", "ada"), ("LOGNAME", "lovelace")], "ada")]
#[case::login_name(&[("LOGNAME", "lovelace")], "lovelace")]
#[case::windows(&[("USERNAME", "ada")], "ada")]
#[case::none(&[], "os_user")]
fn the_os_users_name_comes_from_the_environment(
    #[case] variables: &[(&str, &str)],
    #[case] expected: &str,
) {
    let environment = variables
        .iter()
        .map(|(name, value)| {
            (
                std::ffi::OsString::from(name),
                std::ffi::OsString::from(value),
            )
        })
        .collect();

    assert_eq!(super::os_user(&environment), expected);
}

/// Covers: story:12-126
#[test]
fn a_sign_ins_offline_key_unwraps_only_for_its_cookie_and_outlives_its_daemon() {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let own = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let other = driver.with_link(LINK_TOKEN, HERE).await.unwrap();

        let created = driver
            .offline_key(own.cookie(), None)
            .await
            .expect("a sign-in gets an offline key");
        let restarted = driver_of(&env, "1.0.0", POLICY);
        let unwrapped = restarted
            .offline_key(own.cookie(), Some(created.wrapped()))
            .await
            .expect("its own cookie unwraps it, after a restart too");

        assert_eq!(created.key().len(), 32);
        assert_eq!(unwrapped.key(), created.key());
        assert_eq!(unwrapped.wrapped(), created.wrapped());
        assert!(
            !created
                .wrapped()
                .windows(created.key().len())
                .any(|window| window == created.key()),
            "the wrapped form holds no clear key"
        );
        assert!(matches!(
            driver
                .offline_key(other.cookie(), Some(created.wrapped()))
                .await,
            Err(OfflineKeyRefused::DoesNotOpen)
        ));
        assert!(matches!(
            driver.offline_key("nobody.nothing", None).await,
            Err(OfflineKeyRefused::NotSignedIn)
        ));
        let again = driver.offline_key(own.cookie(), None).await.unwrap();
        assert_ne!(again.key(), created.key(), "each new key is drawn afresh");
    });
}

/// Covers: story:12-126
#[rstest::rstest]
#[case::signed_out("signed_out")]
#[case::revoked("revoked")]
#[case::key_rotated("key_rotated")]
fn an_ended_sign_ins_offline_key_unwraps_no_more(#[case] ending: &str) {
    let env = TestEnv::new();
    runtime().block_on(async {
        let driver = driver(&env);
        let signed_in = driver.with_link(LINK_TOKEN, HERE).await.unwrap();
        let created = driver.offline_key(signed_in.cookie(), None).await.unwrap();
        let (id, _) = signed_in.cookie().split_once('.').unwrap();

        match ending {
            "signed_out" => assert!(driver.sign_out(signed_in.cookie()).await),
            "revoked" => assert!(driver.revoke(id, Some(Surface::Console)).await.unwrap()),
            _ => assert_eq!(driver.rotate_key(Some(Surface::Console)).await.unwrap(), 1),
        }

        assert!(matches!(
            driver
                .offline_key(signed_in.cookie(), Some(created.wrapped()))
                .await,
            Err(OfflineKeyRefused::NotSignedIn)
        ));
    });
}
