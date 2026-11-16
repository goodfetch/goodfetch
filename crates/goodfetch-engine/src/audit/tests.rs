//! The Audit log's chain through its interface (spec #564's seam), on real
//! SQLite in the test's folder, which lies on a memory file system
//! (`TestEnv::in_memory`): each case writes its own log through a few dozen
//! synced writes and new files, which a busy disk would make last seconds
//! (fix-queue 123); the log's durability is the LazyFS test's (`tests/it/lazyfs.rs`).
//! A log written through `record` verifies
//! intact, and each way of tampering with it (an edited entry, a deleted
//! one, a reordered pair, a truncated middle, a truncated tail that the head
//! file still names) is reported at the first sequence it touched, as
//! property tests (ADR 0067). The engine's own connection cannot rewrite an
//! entry, two engines append to one chain, one writer opens the store once
//! and chains after another engine's entries, and the format's versions
//! open or are refused under the Usage ledger family's rule.
#![expect(
    clippy::disallowed_methods,
    reason = "the tests edit the Audit log's files as someone tampering with them would"
)]

use goodfetch_protocol::audit::Actor;
use goodfetch_protocol::audit::{
    AuditVerified, CERTIFICATE_PINNED, SECRET_REFERENCE_USED, SECURITY_SWITCH_TURNED_OFF,
};
use goodfetch_protocol::connection::Surface;
use goodfetch_protocol::formats::Compatibility;
use goodfetch_protocol::run::RunId;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::{TestEnv, wal_of};
use jiff::Timestamp;
use proptest::prelude::*;
use rusqlite::Connection;
use serde_json::{Value, json};

use std::sync::Arc;

use goodfetch_counted_work::{CountedWork, Row};

use super::{AuditError, AuditEvents, AuditLog, NewEntry, Origin};
use crate::home::Home;
use crate::tasks::{BlockingPolicy, TaskOwner};

mod export;
mod retention;
mod unchained;

/// ADR 0059's test `Clock` start, when every entry is recorded.
pub(super) fn now() -> Timestamp {
    "2026-01-05T09:00:00Z".parse().unwrap()
}

/// The owner of a test log's writer, which runs it as a task.
pub(super) fn owner() -> Arc<TaskOwner> {
    Arc::new(TaskOwner::root(BlockingPolicy::Inline))
}

pub(super) fn log(env: &TestEnv) -> AuditLog {
    AuditLog::of(&Home::new(env.home().to_path_buf()), None, owner())
}

pub(super) fn origin() -> Origin {
    Origin {
        actor: Actor::ClientSurface {
            surface: Surface::Cli,
        },
        surface: None,
        layer: Some(SettingsLayer::Run),
        run_id: Some(RunId::parse("audited").unwrap()),
    }
}

/// A log of `count` entries written through `record`, each with its own
/// details.
pub(super) async fn written(env: &TestEnv, count: u64) -> AuditLog {
    let log = log(env);
    for index in 1..=count {
        log.record(
            &origin(),
            &SECRET_REFERENCE_USED,
            json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
            now(),
        )
        .await
        .unwrap();
    }
    log
}

/// The store, opened as someone who edits the file would: with the
/// triggers that keep it append-only dropped.
pub(super) fn tamper(log: &AuditLog) -> Connection {
    let connection = Connection::open(log.path()).unwrap();
    connection
        .execute_batch("DROP TRIGGER entries_never_updated; DROP TRIGGER entries_never_deleted;")
        .unwrap();
    connection
}

/// A sequence as SQLite stores it.
pub(super) fn at(sequence: u64) -> i64 {
    i64::try_from(sequence).unwrap()
}

pub(super) fn broken(entries: u64, sequence: u64) -> AuditVerified {
    AuditVerified {
        intact: false,
        entries,
        first_broken: Some(sequence),
    }
}

/// Covers: story:4-69, story:4-70
#[tokio::test]
async fn a_log_written_through_record_verifies_intact() {
    let env = TestEnv::in_memory();
    let log = written(&env, 3).await;

    let verified = log.verify();

    assert_eq!(
        verified,
        Ok(AuditVerified {
            intact: true,
            entries: 3,
            first_broken: None,
        })
    );
}

#[test]
fn a_log_never_written_verifies_intact_and_empty() {
    let env = TestEnv::in_memory();

    let verified = log(&env).verify();

    assert_eq!(
        (verified, env.home().join("audit-log.sqlite").exists()),
        (
            Ok(AuditVerified {
                intact: true,
                entries: 0,
                first_broken: None,
            }),
            false
        )
    );
}

/// Covers: story:4-69
#[tokio::test]
async fn an_entry_holds_its_sequence_time_kind_actor_layer_run_and_details() {
    let env = TestEnv::in_memory();
    let log = written(&env, 1).await;

    let second = log
        .record(
            &Origin {
                actor: Actor::Engine,
                surface: None,
                layer: Some(SettingsLayer::OperatorPolicy),
                run_id: None,
            },
            &SECURITY_SWITCH_TURNED_OFF,
            json!({ "setting": "security.browser_sandbox" }),
            now(),
        )
        .await
        .unwrap();

    assert_eq!(
        serde_json::to_value(second).unwrap(),
        json!({
            "sequence": 2,
            "time": "2026-01-05T09:00:00Z",
            "kind": "security_switch_turned_off",
            "actor": { "kind": "engine" },
            "layer": "operator_policy",
            "details": { "setting": "security.browser_sandbox" },
        })
    );
}

/// The first entry's hash, `SHA-256` of 32 zero bytes followed by the
/// entry's RFC 8785 form, worked out apart with Python's `hashlib`: the stored
/// hash is that one, so another implementation can verify the chain.
#[tokio::test]
async fn the_first_entry_hashes_zero_bytes_then_its_canonical_json() {
    let env = TestEnv::in_memory();
    let log = log(&env);
    log.record(
        &Origin {
            actor: Actor::Engine,
            surface: None,
            layer: None,
            run_id: None,
        },
        &CERTIFICATE_PINNED,
        json!({ "host": "example.com" }),
        now(),
    )
    .await
    .unwrap();

    let stored: (String, String) = Connection::open(log.path())
        .unwrap()
        .query_row("SELECT entry, hash FROM entries", [], |row| {
            Ok((row.get(0)?, row.get(1)?))
        })
        .unwrap();

    assert_eq!(
        stored,
        (
            r#"{"actor":{"kind":"engine"},"details":{"host":"example.com"},"kind":"certificate_pinned","sequence":1,"time":"2026-01-05T09:00:00Z"}"#.to_owned(),
            "07a1ab07e3b478200081a2520a290df55a7ca813a6aa5e7b50714d205f255591".to_owned()
        )
    );
}

/// Covers: story:4-69
#[rstest::rstest]
#[case::an_update("UPDATE entries SET entry = '{}' WHERE sequence = 1")]
#[case::a_delete("DELETE FROM entries WHERE sequence = 1")]
#[tokio::test]
async fn the_engines_own_connection_cannot_rewrite_an_entry(#[case] statement: &str) {
    let env = TestEnv::in_memory();
    let log = written(&env, 2).await;
    let connection = log.store.open(false).unwrap();

    let tried = connection.execute(statement, []).is_err();

    assert_eq!(
        (tried, log.verify()),
        (
            true,
            Ok(AuditVerified {
                intact: true,
                entries: 2,
                first_broken: None,
            })
        )
    );
}

/// Covers: story:4-69
#[tokio::test]
async fn two_engines_append_to_one_chain() {
    let env = TestEnv::in_memory();
    let (first, second) = (log(&env), log(&env));

    for _ in 0..2 {
        first
            .record(
                &origin(),
                &SECRET_REFERENCE_USED,
                json!({ "by": "first" }),
                now(),
            )
            .await
            .unwrap();
        second
            .record(
                &origin(),
                &SECRET_REFERENCE_USED,
                json!({ "by": "second" }),
                now(),
            )
            .await
            .unwrap();
    }

    assert_eq!(
        first.verify(),
        Ok(AuditVerified {
            intact: true,
            entries: 4,
            first_broken: None,
        })
    );
}

/// A log through `vfs`, in `env`'s home, its writer of `owner`.
fn log_through(env: &TestEnv, vfs: &str, owner: Arc<TaskOwner>) -> AuditLog {
    AuditLog::of(
        &Home::new(env.home().to_path_buf()),
        Some(vfs.to_owned()),
        owner,
    )
}

/// Three entries of `origin()`'s Run, each with its own details.
fn three_new() -> Vec<NewEntry> {
    (1..=3)
        .map(|index| NewEntry {
            origin: origin(),
            kind: &SECRET_REFERENCE_USED,
            details: json!({ "reference": format!("env:SECRET_{index}") }),
            always_chained: false,
        })
        .collect()
}

/// Entries recorded together, as a Run's switched-off controls are as it
/// starts, append in one synced commit, chained in their order after the
/// entries before them; an entry recorded alone in a log that exists syncs
/// once, on the connection its writer keeps open (#2110, #1886).
#[tokio::test]
async fn entries_recorded_together_commit_with_one_sync_chained_in_their_order() {
    let env = TestEnv::new();
    let vfs = goodfetch_test_support::CountingVfs::register();
    let log = log_through(&env, vfs.name(), owner());
    log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
        .await
        .unwrap();

    let before = vfs.syncs();
    log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
        .await
        .unwrap();
    let alone = vfs.syncs() - before;
    let before = vfs.syncs();
    let recorded = log.record_together(three_new(), now()).await.unwrap();
    let together = vfs.syncs() - before;

    let sequences: Vec<u64> = recorded.iter().map(|entry| entry.sequence).collect();
    let exported: Vec<Value> = log
        .export()
        .unwrap()
        .into_iter()
        .map(|exported| exported.entry.details)
        .collect();
    assert_eq!(
        (
            alone,
            together,
            sequences,
            log.verify(),
            exported[2..].to_vec()
        ),
        (
            1,
            1,
            vec![3, 4, 5],
            Ok(AuditVerified {
                intact: true,
                entries: 5,
                first_broken: None,
            }),
            vec![
                json!({ "reference": "env:SECRET_1" }),
                json!({ "reference": "env:SECRET_2" }),
                json!({ "reference": "env:SECRET_3" }),
            ]
        )
    );
}

/// A new log's first entries sync its file's switch to WAL, its rollback
/// journal once and the file once, and their own commit, which covers the
/// tables its opening committed unsynced and the WAL's header (#2117).
#[tokio::test]
async fn a_new_logs_first_entries_sync_its_switch_to_wal_and_their_commit() {
    let env = TestEnv::new();
    let vfs = goodfetch_test_support::CountingVfs::register();
    let log = log_through(&env, vfs.name(), owner());

    log.record_together(three_new(), now()).await.unwrap();

    assert_eq!(vfs.syncs(), 3);
}

/// Entries recorded, alone or together, are in the log after a power cut
/// once the call answered, while the writer still holds the store open and
/// has copied nothing of the write-ahead log into the file: each commit
/// synced it (ADR 0016, #2110, #1886).
#[tokio::test]
async fn recorded_entries_survive_a_power_cut_while_their_writer_holds_the_store() {
    let env = TestEnv::new();
    let vfs =
        goodfetch_test_support::CrashVfs::register(&env.unique_name("audit-power-cut"), env.seed())
            .unwrap();
    let log = log_through(&env, vfs.name(), owner());
    log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
        .await
        .unwrap();
    log.record_together(three_new(), now()).await.unwrap();

    vfs.crash(goodfetch_test_support::CrashKind::PowerCut);
    let connection =
        goodfetch_test_support::after_crash(&vfs, &log.path().to_string_lossy()).unwrap();

    let sequences: Vec<i64> = connection
        .prepare("SELECT sequence FROM entries ORDER BY sequence")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(sequences, [1, 2, 3, 4]);
}

/// An engine that makes a home's Audit log while another engine writes the
/// new file records its entry once the other's write ends (#1773): its
/// switch of the file to WAL waits, as SQLite answers that switch busy at
/// once.
///
/// Covers: story:4-69
#[test]
fn an_engine_making_the_log_beside_another_engines_write_records_its_entry() {
    let env = TestEnv::new();
    let log = log(&env);
    let runtime = goodfetch_test_support::runtime();

    let recorded = crate::sqlite_wal::tests::opened_beside_a_write(log.path(), || {
        runtime.block_on(log.record(
            &origin(),
            &SECRET_REFERENCE_USED,
            json!({ "by": "second" }),
            now(),
        ))
    });

    assert_eq!(recorded.map(drop), Ok(()));
}

/// Covers: story:4-70
#[tokio::test]
async fn a_removed_head_file_breaks_the_log_past_its_last_entry() {
    let env = TestEnv::in_memory();
    let log = written(&env, 2).await;
    std::fs::remove_file(env.home().join("audit-log-head.json")).unwrap();

    assert_eq!(log.verify(), Ok(broken(2, 3)));
}

/// Covers: story:4-70
#[tokio::test]
async fn a_head_file_naming_another_hash_breaks_the_log_at_its_sequence() {
    let env = TestEnv::in_memory();
    let log = written(&env, 3).await;
    std::fs::write(
        env.home().join("audit-log-head.json"),
        r#"{"sequence":2,"hash":"0000000000000000000000000000000000000000000000000000000000000000"}"#,
    )
    .unwrap();

    assert_eq!(log.verify(), Ok(broken(3, 2)));
}

/// Writes a log whose format row says `version`, and answers the log of an
/// engine that opens it afterwards.
async fn log_at_version(env: &TestEnv, version: &str) -> AuditLog {
    let written = written(env, 1).await;
    Connection::open(written.path())
        .unwrap()
        .execute("UPDATE format SET version = ?1", [version])
        .unwrap();
    log(env)
}

/// Covers: format:audit-log
#[tokio::test]
async fn a_log_of_a_newer_minor_opens_and_verifies() {
    let env = TestEnv::in_memory();
    let log = log_at_version(&env, "0.9").await;

    assert_eq!(
        log.verify(),
        Ok(AuditVerified {
            intact: true,
            entries: 1,
            first_broken: None,
        })
    );
}

/// Covers: format:audit-log
#[tokio::test]
async fn a_log_of_another_major_is_refused() {
    let env = TestEnv::in_memory();
    let log = log_at_version(&env, "1.0").await;

    assert_eq!(
        log.record(&origin(), &SECRET_REFERENCE_USED, Value::Null, now())
            .await,
        Err(AuditError::Format {
            found: "1.0".to_owned(),
            compatibility: Compatibility::NewerMajor,
        })
    );
}

/// The `head/` set the corpus writer wrote, read by today's engine (ADR
/// 0066): it verifies intact, and an entry appended to it chains on, past
/// the unchained entry it ends with.
/// Covers: format:audit-log
#[tokio::test]
async fn the_head_sets_log_verifies_and_takes_another_entry() {
    let env = TestEnv::in_memory();
    let head = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../compat/audit-log/head");
    let log = log(&env);
    Connection::open(log.path())
        .unwrap()
        .execute_batch(&std::fs::read_to_string(head.join("audit-log.sql")).unwrap())
        .unwrap();
    std::fs::copy(head.join("audit-log-head.json"), log.head_path()).unwrap();
    log.record(
        &origin(),
        &SECRET_REFERENCE_USED,
        json!({ "after": "head" }),
        now(),
    )
    .await
    .unwrap();

    assert_eq!(
        log.verify(),
        Ok(AuditVerified {
            intact: true,
            entries: 6,
            first_broken: None,
        })
    );
}

/// A log's length and a sequence in it, from 1.
pub(super) fn log_and_sequence() -> impl Strategy<Value = (u64, u64)> {
    (2..=8_u64).prop_flat_map(|count| (Just(count), 1..=count))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Covers: story:4-70
    #[test]
    fn an_edited_entry_is_reported_at_its_sequence((count, sequence) in log_and_sequence()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(written(&env, count));
        tamper(&log)
            .execute(
                "UPDATE entries SET entry = replace(entry, 'example.com', 'example.org') WHERE sequence = ?1",
                [at(sequence)],
            )
            .unwrap();

        prop_assert_eq!(log.verify(), Ok(broken(count, sequence)));
    }

    /// Covers: story:4-70
    #[test]
    fn a_deleted_entry_is_reported_at_its_sequence((count, sequence) in log_and_sequence()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(written(&env, count));
        tamper(&log)
            .execute("DELETE FROM entries WHERE sequence = ?1", [at(sequence)])
            .unwrap();

        prop_assert_eq!(log.verify(), Ok(broken(count - 1, sequence)));
    }

    /// Covers: story:4-70
    #[test]
    fn a_reordered_pair_is_reported_at_its_first_sequence((count, sequence) in log_and_sequence()) {
        prop_assume!(sequence < count);
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(written(&env, count));
        tamper(&log)
            .execute_batch(&format!(
                "UPDATE entries SET sequence = -1 WHERE sequence = {sequence};
                 UPDATE entries SET sequence = {sequence} WHERE sequence = {sequence} + 1;
                 UPDATE entries SET sequence = {sequence} + 1 WHERE sequence = -1;"
            ))
            .unwrap();

        prop_assert_eq!(log.verify(), Ok(broken(count, sequence)));
    }

    /// Covers: story:4-70
    #[test]
    fn a_truncated_middle_is_reported_at_its_first_sequence((count, sequence) in log_and_sequence()) {
        prop_assume!(sequence + 1 < count);
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(written(&env, count));
        tamper(&log)
            .execute(
                "DELETE FROM entries WHERE sequence BETWEEN ?1 AND ?1 + 1",
                [at(sequence)],
            )
            .unwrap();

        prop_assert_eq!(log.verify(), Ok(broken(count - 2, sequence)));
    }

    /// A tail cut off the store, which the head file still names.
    /// Covers: story:4-70
    #[test]
    fn a_truncated_tail_is_reported_at_its_first_sequence((count, sequence) in log_and_sequence()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(written(&env, count));
        tamper(&log)
            .execute("DELETE FROM entries WHERE sequence >= ?1", [at(sequence)])
            .unwrap();

        prop_assert_eq!(log.verify(), Ok(broken(sequence - 1, sequence)));
    }

    /// The positive control: the same logs, untouched.
    /// Covers: story:4-70
    #[test]
    fn an_untouched_log_verifies_intact((count, _sequence) in log_and_sequence()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(written(&env, count));

        prop_assert_eq!(
            log.verify(),
            Ok(AuditVerified {
                intact: true,
                entries: count,
                first_broken: None,
            })
        );
    }
}

/// The Audit log holds to the synced per-user store's durability: its WAL
/// stays beside it once its writer ends and closes its connection (#2110).
#[tokio::test]
async fn the_wal_stays_beside_the_log_once_its_writer_closed_its_connection() {
    let env = TestEnv::in_memory();
    let owner = owner();
    let log = AuditLog::of(
        &Home::new(env.home().to_path_buf()),
        None,
        Arc::clone(&owner),
    );
    log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
        .await
        .unwrap();
    let path = log.path().to_owned();

    drop(log);
    owner.wait().await;

    assert!(wal_of(&path).exists());
}

/// A log's writer opens its store once for all its entries, and its clones
/// share it (#1886): that open and its schema set-up's commits count once a
/// log, each entry its own commit (#1887).
#[tokio::test]
async fn a_logs_writer_opens_its_store_once_for_every_entry_of_its_clones() {
    let env = TestEnv::in_memory();
    let work = CountedWork::default();
    let log = log(&env).counting_on(work.clone());

    for clone in [log.clone(), log.clone(), log] {
        clone
            .record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
            .await
            .unwrap();
    }

    assert_eq!(
        [
            Row::AuditLogOpens,
            Row::AuditLogTransactions,
            Row::AuditOpens,
            Row::AuditTransactions,
        ]
        .map(|row| work.read(row)),
        [1, 2, 0, 3]
    );
}

/// A write that fails drops the writer's connection, so the next entry opens
/// the store again: that open and its set-up's commit are the entry's work,
/// not the log's (#1887).
#[tokio::test]
async fn an_entry_after_a_failed_write_counts_the_stores_open_again() {
    let env = TestEnv::in_memory();
    let work = CountedWork::default();
    let log = log(&env).counting_on(work.clone());
    let record = || async {
        log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
            .await
    };
    record().await.unwrap();
    // A folder where the head file goes: the next entry commits, then
    // fails to write its head file.
    std::fs::remove_file(log.head_path()).unwrap();
    std::fs::create_dir(log.head_path()).unwrap();
    assert!(matches!(record().await, Err(AuditError::Head { .. })));
    std::fs::remove_dir(log.head_path()).unwrap();

    record().await.unwrap();

    assert_eq!(
        [
            Row::AuditLogOpens,
            Row::AuditLogTransactions,
            Row::AuditOpens,
            Row::AuditTransactions,
        ]
        .map(|row| work.read(row)),
        [1, 2, 1, 4]
    );
}

/// Each entry's event carries the line of an export that holds it: the
/// same fields, its hash among them (#1886's E2-4).
#[tokio::test]
async fn an_entry_sends_its_event_with_its_line_of_an_export() {
    let env = TestEnv::in_memory();
    let lines = Arc::new(std::sync::Mutex::new(Vec::new()));
    let sent = Arc::clone(&lines);
    let log = log(&env).sending_to(Some(AuditEvents::new(move |_, line: &str| {
        sent.lock().unwrap().push(line.to_owned());
    })));
    log.record(
        &origin(),
        &SECRET_REFERENCE_USED,
        json!({ "reference": "env:SECRET", "domain": "example.com", "ratio": 1.5 }),
        now(),
    )
    .await
    .unwrap();
    log.record_together(three_new(), now()).await.unwrap();

    let sent: Vec<Value> = lines
        .lock()
        .unwrap()
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    let exported: Vec<Value> = export::jsonl(&log.export().unwrap())
        .iter()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!((sent.len(), sent), (4, exported));
}

/// A log whose writer's owner stopped, as an engine's stop ends its
/// writer, answers that its writer is not running, and writes nothing more.
#[tokio::test]
async fn a_log_whose_owner_stopped_answers_its_writer_is_not_running() {
    let env = TestEnv::in_memory();
    let owner = owner();
    let log = AuditLog::of(
        &Home::new(env.home().to_path_buf()),
        None,
        Arc::clone(&owner),
    );
    log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
        .await
        .unwrap();
    owner.stop().await;

    let recorded = log
        .record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
        .await;

    assert!(
        matches!(recorded, Err(AuditError::Writer { .. })),
        "{recorded:?}"
    );
    assert_eq!(log.export().unwrap().len(), 1);
}

/// A log written from a runtime of its own after the one its writer ran on
/// ended, as a harness that builds a runtime per step writes it, starts its
/// writer again and records, its owner still running.
#[test]
fn a_log_whose_writers_runtime_ended_records_from_the_next() {
    let env = TestEnv::in_memory();
    let log = log(&env);
    let record = || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .build()
            .unwrap();
        runtime
            .block_on(log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now()))
            .map(|entry| entry.sequence)
    };

    let recorded = (record(), record());

    assert_eq!(recorded, (Ok(1), Ok(2)));
    assert_eq!(log.export().unwrap().len(), 2);
}
