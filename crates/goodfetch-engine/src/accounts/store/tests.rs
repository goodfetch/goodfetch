//! The Accounts store through its interface, on real SQLite in the test's
//! folder: what it holds survives the store being opened again, a
//! recovery code is used once, the format's versions open or are refused by
//! ADR 0030's default rule, the corpus's `head/` set opens, and no recovery
//! code or Named token value reaches the disk, only its hash.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the store's files and the corpus as bytes and relabel the store's version as an older or newer engine would; on no Shared path (ADR 0140)"
)]

use std::path::{Path, PathBuf};

use goodfetch_protocol::accounts::StoredAccount;
use goodfetch_protocol::formats::Compatibility;
use goodfetch_test_support::TestEnv;
use jiff::{SignedDuration, Timestamp};
use rstest::rstest;
use rusqlite::Connection;
use secrecy::SecretString;

use super::{AccountsStore, AccountsStoreError, hash};

/// ADR 0059's test `Clock` start.
fn now() -> Timestamp {
    "2026-01-05T09:00:00Z".parse().unwrap()
}

fn store_file(env: &TestEnv) -> PathBuf {
    env.home().join("0.0").join("accounts-store.sqlite")
}

fn open(path: &Path) -> AccountsStore {
    AccountsStore::open(path, None).unwrap()
}

fn secret(value: &str) -> SecretString {
    SecretString::from(value)
}

/// Covers: format:accounts-store
#[test]
fn what_the_store_holds_survives_it_being_opened_again() {
    let env = TestEnv::new();
    let store = open(&store_file(&env));
    assert!(store.make_account("Grace", now()).unwrap());
    store
        .add_passkey(b"credential", b"public key", now())
        .unwrap();
    store
        .add_named_token("ci", &secret("token value"), now())
        .unwrap();
    drop(store);

    let store = open(&store_file(&env));

    assert_eq!(
        store.account().unwrap(),
        Some(StoredAccount {
            name: "Grace".to_owned(),
            created: "2026-01-05T09:00:00Z".to_owned(),
        })
    );
    assert!(!store.make_account("Ada", now()).unwrap());
    assert_eq!(
        store.passkey(b"credential").unwrap(),
        Some(b"public key".to_vec())
    );
    assert_eq!(store.passkey(b"another").unwrap(), None);
    assert_eq!(
        store.named_token(&secret("token value")).unwrap(),
        Some("ci".to_owned())
    );
    assert_eq!(store.named_token(&secret("another value")).unwrap(), None);
}

/// Covers: format:accounts-store
#[test]
fn a_recovery_code_is_used_once_and_new_codes_replace_the_old() {
    let env = TestEnv::new();
    let store = open(&store_file(&env));
    store
        .replace_recovery_codes(&[secret("first"), secret("second")])
        .unwrap();

    assert!(store.use_recovery_code(&secret("first"), now()).unwrap());
    assert!(!store.use_recovery_code(&secret("first"), now()).unwrap());
    assert!(!store.use_recovery_code(&secret("unknown"), now()).unwrap());

    store.replace_recovery_codes(&[secret("third")]).unwrap();
    assert!(!store.use_recovery_code(&secret("second"), now()).unwrap());
    assert!(store.use_recovery_code(&secret("third"), now()).unwrap());
}

/// A store of this engine relabelled `version`, as another engine would
/// have left it.
fn store_at_version(env: &TestEnv, version: &str) -> PathBuf {
    let path = store_file(env);
    open(&path).make_account("Grace", now()).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute("UPDATE format SET version = ?1", [version])
        .unwrap();
    path
}

/// A later preview's store is another major, since a `0.y` minor counts as
/// a major (ADR 0030), as a release's is.
/// Covers: format:accounts-store
#[rstest]
#[case::a_later_preview("0.9")]
#[case::a_release("1.0")]
fn a_store_of_another_major_is_refused(#[case] version: &str) {
    let env = TestEnv::new();
    let path = store_at_version(&env, version);

    assert_eq!(
        AccountsStore::open(&path, None).unwrap_err(),
        AccountsStoreError::Format {
            found: version.to_owned(),
            compatibility: Compatibility::NewerMajor,
        }
    );
}

/// The `head/` set the corpus writer wrote, read by today's engine (ADR
/// 0066).
/// Covers: format:accounts-store
#[test]
fn the_head_sets_store_opens_and_checks_its_credentials() {
    let env = TestEnv::new();
    let head = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compat/accounts-store/head");
    let path = store_file(&env);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    Connection::open(&path)
        .unwrap()
        .execute_batch(&std::fs::read_to_string(head.join("accounts-store.sql")).unwrap())
        .unwrap();

    let store = open(&path);

    assert_eq!(store.account().unwrap().unwrap().name, "Grace");
    assert_eq!(
        store.named_token(&secret("the ci token's value")).unwrap(),
        Some("ci".to_owned())
    );
    assert!(
        !store
            .use_recovery_code(&secret("used-recovery-code"), now())
            .unwrap()
    );
    assert!(
        store
            .use_recovery_code(&secret("unused-recovery-code"), now())
            .unwrap()
    );
}

/// A canary: every file of the store, its WAL included, holds the hash of
/// each recovery code and Named token, and never its value.
/// Covers: format:accounts-store
#[rstest]
#[case::kept_open(false)]
#[case::closed(true)]
fn no_recovery_code_or_token_value_reaches_the_disk(#[case] closed: bool) {
    let env = TestEnv::new();
    let path = store_file(&env);
    let store = open(&path);
    let code = secret("canary-recovery-code-5f1e");
    let token = secret("canary-named-token-9a2c");
    store
        .replace_recovery_codes(std::slice::from_ref(&code))
        .unwrap();
    store
        .use_recovery_code(&code, now() + SignedDuration::from_hours(1))
        .unwrap();
    store.add_named_token("ci", &token, now()).unwrap();
    if closed {
        drop(store);
    }

    let mut bytes = Vec::new();
    for entry in std::fs::read_dir(path.parent().unwrap()).unwrap() {
        bytes.extend(std::fs::read(entry.unwrap().path()).unwrap());
    }

    let holds = |needle: &str| {
        bytes
            .windows(needle.len())
            .any(|window| window == needle.as_bytes())
    };
    for value in [
        "canary-recovery-code-5f1e",
        "canary-named-token-9a2c",
        "canary",
    ] {
        assert!(!holds(value), "`{value}` is on disk");
    }
    assert!(holds(&hash(&code)));
    assert!(holds(&hash(&token)));
}
