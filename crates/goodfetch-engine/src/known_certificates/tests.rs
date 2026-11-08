//! The known-certificates store through its interface (spec #564's `tls`
//! seam), on real SQLite in the test's folder: a pin is found for its
//! person and host only, two people's pins on one store are kept apart, a
//! pin is never replaced by a second one, a row the engine cannot read
//! reads as unreadable (which stops its host as a changed key), and the
//! format's versions open or are refused under the Usage ledger family's
//! rule, the `head/` set included.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests read the corpus files and edit the store's rows as a damaged disk would"
)]

use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::run::{PointInTime, RunId};
use goodfetch_protocol::tls::CertificatePin;
use goodfetch_test_support::TestEnv;
use jiff::Timestamp;
use rstest::rstest;
use rusqlite::Connection;

use super::{Accepted, Found, KeyHash, KnownCertificates, KnownCertificatesError, Pinned};
use crate::home::Home;

/// ADR 0059's test `Clock` start, when every pin is made.
fn now() -> Timestamp {
    "2026-01-05T09:00:00Z".parse().unwrap()
}

/// The same moment, as a pin's `pinned_at` reads.
fn at_start() -> PointInTime {
    PointInTime::from_rfc3339("2026-01-05T09:00:00Z".to_owned())
}

fn store(env: &TestEnv) -> KnownCertificates {
    KnownCertificates::of(&Home::new(env.home().to_path_buf()), None)
}

/// A key's hash, distinct for each `byte`.
fn key(byte: u8) -> KeyHash {
    KeyHash::of_public_key(&[byte; 91])
}

fn run(id: &str) -> RunId {
    RunId::parse(id).unwrap()
}

/// The hash of a key is SHA-256 over its `SubjectPublicKeyInfo`, written as
/// `sha256/` and standard base64: here the SHA-256 of no bytes, whose
/// base64 is the well-known `47DEQ…`.
#[test]
fn a_key_hash_is_written_as_sha256_and_its_base64() {
    assert_eq!(
        KeyHash::of_public_key(b"").to_string(),
        "sha256/47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="
    );
}

/// Covers: story:4-35, format:known-certificates
#[test]
fn a_pin_is_found_for_its_person_and_host_only() {
    let env = TestEnv::new();
    let store = store(&env);

    let pinned = store
        .pin(
            "os_user",
            "expired.certificates.test",
            &key(1),
            Some(&run("first")),
            now(),
        )
        .unwrap();

    assert_eq!(pinned, Pinned::First);
    assert_eq!(
        (
            store.find("os_user", "expired.certificates.test").unwrap(),
            store
                .find("os_user", "self-signed.certificates.test")
                .unwrap(),
            store
                .find("someone_else", "expired.certificates.test")
                .unwrap(),
        ),
        (Found::Pinned(key(1)), Found::Unpinned, Found::Unpinned)
    );
}

/// Story 4-43: two people's pins on one store are kept apart; each one's
/// list holds only their own, the same host pinned to another key by each.
/// Covers: story:4-43, format:known-certificates
#[test]
fn two_peoples_pins_on_one_store_are_kept_apart() {
    let env = TestEnv::new();
    let store = store(&env);
    store
        .pin(
            "ada",
            "intranet.example",
            &key(1),
            Some(&run("adas-run")),
            now(),
        )
        .unwrap();
    store
        .pin("ada", "wiki.intranet.example", &key(2), None, now())
        .unwrap();
    store
        .pin(
            "grace",
            "intranet.example",
            &key(3),
            Some(&run("graces-run")),
            now(),
        )
        .unwrap();

    assert_eq!(
        store.list("ada").unwrap(),
        [
            CertificatePin {
                host: "intranet.example".to_owned(),
                spki_sha256: Some(key(1).to_string()),
                pinned_at: Some(at_start()),
                run_id: Some(run("adas-run")),
                changed_spki_sha256: None,
            },
            CertificatePin {
                host: "wiki.intranet.example".to_owned(),
                spki_sha256: Some(key(2).to_string()),
                pinned_at: Some(at_start()),
                run_id: None,
                changed_spki_sha256: None,
            },
        ]
    );
    assert_eq!(
        store.list("grace").unwrap(),
        [CertificatePin {
            host: "intranet.example".to_owned(),
            spki_sha256: Some(key(3).to_string()),
            pinned_at: Some(at_start()),
            run_id: Some(run("graces-run")),
            changed_spki_sha256: None,
        }]
    );
    assert_eq!(
        store.find("grace", "intranet.example").unwrap(),
        Found::Pinned(key(3))
    );
}

/// Only a person replaces a pin (`certs accept`, #629): a second
/// connection presenting another key never does, and learns the first.
/// Covers: story:4-35
#[test]
fn a_second_pin_of_a_host_keeps_the_first() {
    let env = TestEnv::new();
    let store = store(&env);
    store
        .pin("os_user", "intranet.example", &key(1), None, now())
        .unwrap();

    let second = store
        .pin("os_user", "intranet.example", &key(2), None, now())
        .unwrap();

    assert_eq!(second, Pinned::Already(Found::Pinned(key(1))));
    assert_eq!(
        store.find("os_user", "intranet.example").unwrap(),
        Found::Pinned(key(1))
    );
}

/// A lookup only reads: it answers while another engine's connection holds
/// the store's write lock, as a pin or a backup tool would, and never waits
/// for that writer.
/// Covers: story:4-35
#[rstest]
#[case::find("find")]
#[case::list("list")]
#[case::waiting("waiting")]
fn a_lookup_answers_while_another_connection_holds_the_stores_write_lock(#[case] lookup: &str) {
    let env = TestEnv::new();
    let store = store(&env);
    store
        .pin("os_user", "intranet.example", &key(1), None, now())
        .unwrap();
    store
        .record_change("os_user", "intranet.example", &key(2), None, later())
        .unwrap();
    let writer = Connection::open(store.path()).unwrap();
    writer.execute_batch("BEGIN IMMEDIATE").unwrap();

    let answered = match lookup {
        "find" => store.find("os_user", "intranet.example").map(|_| ()),
        "list" => store.list("os_user").map(|_| ()),
        _ => store.waiting("os_user").map(|_| ()),
    };

    assert_eq!(answered, Ok(()));
}

/// An engine that makes the new store while another engine writes the new
/// file pins its key once that write ends: its switch of the file to WAL
/// waits, as SQLite answers that switch busy at once (#1773's class).
///
/// Covers: story:4-35, format:known-certificates
#[test]
fn an_engine_making_the_store_beside_another_engines_write_pins_its_key() {
    let env = TestEnv::new();
    let store = store(&env);

    let pinned = crate::sqlite_wal::tests::opened_beside_a_write(store.path(), || {
        store.pin(
            "os_user",
            "expired.certificates.test",
            &key(1),
            Some(&run("first")),
            now(),
        )
    });

    assert_eq!(pinned, Ok(Pinned::First));
}

/// A moment an hour after [`now`], when a change is seen or accepted.
fn later() -> Timestamp {
    "2026-01-05T10:00:00Z".parse().unwrap()
}

/// Story 4-36: the key that stopped a pinned host waits beside its pin, as
/// `certs list` shows it, until the person accepts it: the pin then holds
/// that key, dated when it was accepted and by the Run that met it, and no
/// change waits any more; the answer names the key it replaced.
/// Covers: story:4-36, format:known-certificates
#[test]
fn an_accepted_change_replaces_the_pin_with_the_key_that_stopped_the_host() {
    let env = TestEnv::new();
    let store = store(&env);
    store
        .pin(
            "os_user",
            "intranet.example",
            &key(1),
            Some(&run("first")),
            now(),
        )
        .unwrap();
    store
        .record_change(
            "os_user",
            "intranet.example",
            &key(2),
            Some(&run("stopped")),
            now(),
        )
        .unwrap();
    let waiting = store.list("os_user").unwrap();

    let accepted = store
        .accept("os_user", "intranet.example", &key(2), later())
        .unwrap();

    assert_eq!(waiting[0].changed_spki_sha256, Some(key(2).to_string()));
    let pin = CertificatePin {
        host: "intranet.example".to_owned(),
        spki_sha256: Some(key(2).to_string()),
        pinned_at: Some(PointInTime::from_rfc3339("2026-01-05T10:00:00Z".to_owned())),
        run_id: Some(run("stopped")),
        changed_spki_sha256: None,
    };
    assert_eq!(
        accepted,
        Accepted {
            previous: Found::Pinned(key(1)),
            pin: pin.clone(),
        }
    );
    assert_eq!(
        (
            store.find("os_user", "intranet.example").unwrap(),
            store.list("os_user").unwrap(),
        ),
        (Found::Pinned(key(2)), vec![pin])
    );
}

/// A person accepts only the key that stopped the host: another key, or a
/// host with no change waiting, is refused and the pin stays.
/// Covers: story:4-36
#[rstest]
#[case::another_key(Some(3))]
#[case::no_change_waiting(None)]
fn a_key_that_did_not_stop_the_host_is_not_accepted(#[case] changed: Option<u8>) {
    let env = TestEnv::new();
    let store = store(&env);
    store
        .pin("os_user", "intranet.example", &key(1), None, now())
        .unwrap();
    if let Some(changed) = changed {
        store
            .record_change("os_user", "intranet.example", &key(changed), None, now())
            .unwrap();
    }

    let refused = store.accept("os_user", "intranet.example", &key(2), later());

    assert_eq!(
        refused,
        Err(KnownCertificatesError::NoSuchChange {
            host: "intranet.example".to_owned()
        })
    );
    assert_eq!(
        store.find("os_user", "intranet.example").unwrap(),
        Found::Pinned(key(1))
    );
}

/// Story 4-43: a pin row the engine cannot read stops its host, and the key
/// that host presented then is what the person accepts in its place.
/// Covers: story:4-36, story:4-43
#[test]
fn an_unreadable_pin_row_is_replaced_by_the_accepted_key() {
    let env = TestEnv::new();
    let store = store(&env);
    store
        .pin("os_user", "intranet.example", &key(1), None, now())
        .unwrap();
    Connection::open(store.path())
        .unwrap()
        .execute("UPDATE pins SET spki_sha256 = x'0102'", [])
        .unwrap();
    store
        .record_change("os_user", "intranet.example", &key(2), None, now())
        .unwrap();

    let accepted = store
        .accept("os_user", "intranet.example", &key(2), later())
        .unwrap();

    assert_eq!(
        (
            accepted.previous,
            store.find("os_user", "intranet.example").unwrap()
        ),
        (Found::Unreadable, Found::Pinned(key(2)))
    );
}

/// Story 4-37: `certs forget` removes the person's pin and the change
/// waiting for it, and nothing of another person's; a host with no pin is
/// forgotten as nothing.
/// Covers: story:4-37, format:known-certificates
#[test]
fn forgetting_a_host_removes_its_pin_and_its_waiting_change_only() {
    let env = TestEnv::new();
    let store = store(&env);
    for person in ["os_user", "grace"] {
        store
            .pin(person, "intranet.example", &key(1), None, now())
            .unwrap();
        store
            .record_change(person, "intranet.example", &key(2), None, now())
            .unwrap();
    }

    let forgotten = store.forget("os_user", "intranet.example").unwrap();
    let again = store.forget("os_user", "intranet.example").unwrap();

    assert_eq!((forgotten, again), (true, false));
    assert_eq!(
        (
            store.find("os_user", "intranet.example").unwrap(),
            store.list("os_user").unwrap(),
            store.find("grace", "intranet.example").unwrap(),
            store.list("grace").unwrap()[0].changed_spki_sha256.clone(),
        ),
        (
            Found::Unpinned,
            Vec::new(),
            Found::Pinned(key(1)),
            Some(key(2).to_string())
        )
    );
    assert_eq!(
        store
            .pin("os_user", "intranet.example", &key(3), None, later())
            .unwrap(),
        Pinned::First
    );
}

/// Story 4-43 (ADR 0042): a pin row the engine cannot read, its key's
/// bytes or its date damaged on disk, reads as unreadable, never as no
/// pin, so its host stops as a changed key; `certs list` still shows its
/// host, without a hash.
/// Covers: story:4-43, format:known-certificates
#[rstest]
#[case::key_bytes_cut_short("UPDATE pins SET spki_sha256 = x'0102'")]
#[case::key_bytes_too_long("UPDATE pins SET spki_sha256 = zeroblob(33)")]
#[case::date_no_time("UPDATE pins SET pinned_at = 'yesterday'")]
#[case::run_id_no_run_id("UPDATE pins SET run_id = 'NOT A RUN'")]
fn a_pin_row_the_engine_cannot_read_reads_as_unreadable(#[case] damage: &str) {
    let env = TestEnv::new();
    let store = store(&env);
    store
        .pin(
            "os_user",
            "intranet.example",
            &key(1),
            Some(&run("pinned")),
            now(),
        )
        .unwrap();
    Connection::open(store.path())
        .unwrap()
        .execute(damage, [])
        .unwrap();

    assert_eq!(
        (
            store.find("os_user", "intranet.example").unwrap(),
            store.list("os_user").unwrap(),
        ),
        (
            Found::Unreadable,
            vec![CertificatePin {
                host: "intranet.example".to_owned(),
                spki_sha256: None,
                pinned_at: None,
                run_id: None,
                changed_spki_sha256: None,
            }]
        )
    );
}

/// A store never written holds no pin, and reading it creates nothing.
/// Covers: format:known-certificates
#[test]
fn a_store_never_written_holds_no_pin_and_is_not_created() {
    let env = TestEnv::new();
    let store = store(&env);

    assert_eq!(
        (
            store.find("os_user", "intranet.example").unwrap(),
            store.list("os_user").unwrap(),
        ),
        (Found::Unpinned, Vec::new())
    );
    assert!(!store.path().exists());
}

/// A store of `version`, written by this engine then relabelled.
fn store_at_version(env: &TestEnv, version: &str) -> KnownCertificates {
    let store = store(env);
    store
        .pin("os_user", "intranet.example", &key(1), None, now())
        .unwrap();
    Connection::open(store.path())
        .unwrap()
        .execute("UPDATE format SET version = ?1", [version])
        .unwrap();
    store
}

/// Every minor of the format's one major opens: a newer minor's pins are
/// read and kept.
/// Covers: story:4-43, format:known-certificates
#[test]
fn a_store_of_a_newer_minor_opens_and_keeps_its_pins() {
    let env = TestEnv::new();
    let store = store_at_version(&env, "0.9");

    assert_eq!(
        store.find("os_user", "intranet.example").unwrap(),
        Found::Pinned(key(1))
    );
}

/// Covers: format:known-certificates
#[test]
fn a_store_of_another_major_is_refused() {
    let env = TestEnv::new();
    let store = store_at_version(&env, "1.0");

    assert_eq!(
        store.find("os_user", "intranet.example"),
        Err(KnownCertificatesError::Format {
            found: "1.0".to_owned(),
            compatibility: Compatibility::NewerMajor,
        })
    );
}

/// The `head/` set the corpus writer wrote, read by today's engine (ADR
/// 0066): its pins are found for their people, and it takes another.
/// Covers: story:4-43, format:known-certificates
#[test]
fn the_head_sets_store_opens_and_takes_another_pin() {
    let env = TestEnv::new();
    let head = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../compat/known-certificates/head");
    let store = store(&env);
    std::fs::create_dir_all(store.path().parent().unwrap()).unwrap();
    Connection::open(store.path())
        .unwrap()
        .execute_batch(&std::fs::read_to_string(head.join("known-certificates.sql")).unwrap())
        .unwrap();

    let added = store
        .pin("os_user", "added.example", &key(9), None, now())
        .unwrap();

    assert_eq!(added, Pinned::First);
    assert_eq!(
        store
            .list("os_user")
            .unwrap()
            .into_iter()
            .map(|pin| pin.host)
            .collect::<Vec<_>>(),
        [
            "added.example",
            "expired.certificates.test",
            "intranet.example"
        ]
    );
    assert_eq!(
        store
            .list("grace")
            .unwrap()
            .into_iter()
            .map(|pin| pin.host)
            .collect::<Vec<_>>(),
        ["intranet.example"]
    );
    let changed = KeyHash::of_public_key(b"the expired leaf's changed key");
    let accepted = store
        .accept("os_user", "expired.certificates.test", &changed, now())
        .unwrap();
    assert_eq!(accepted.pin.spki_sha256, Some(changed.to_string()));
}

/// The store's commits sync as every synced per-user store's do: each through
/// `F_FULLFSYNC` on macOS, and its WAL stays beside it as its last
/// connection closes (#2110).
#[test]
fn each_commit_syncs_through_fullfsync_on_macos_and_the_wal_stays_as_it_closes() {
    let env = TestEnv::in_memory();
    let store = store(&env);

    let connection = store.open().unwrap();
    let fullfsync: i64 = connection
        .pragma_query_value(None, "fullfsync", |row| row.get(0))
        .unwrap();
    drop(connection);

    assert_eq!(fullfsync, i64::from(cfg!(target_os = "macos")));
    let mut wal = store.path().as_os_str().to_owned();
    wal.push("-wal");
    assert!(std::path::Path::new(&wal).exists());
}
