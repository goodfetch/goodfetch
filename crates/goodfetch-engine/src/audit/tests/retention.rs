//! `audit.retention`'s pruning through the Audit log's interface (spec
//! #564's seam; ADRs 0016 and 0067): the entries older than the retention
//! give way, from the log's beginning only, to an anchor that carries the
//! last pruned hash, and a pruning record chained at the tail names it; the
//! rest verifies intact from the anchor, in the store and in its export,
//! and tampering with the anchor or with an entry after it is still
//! reported, as property tests.

use std::time::Duration;

use goodfetch_protocol::audit::{AuditVerified, ExportedAuditEntry, SECRET_REFERENCE_USED};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::settings::SettingsLayer;
use goodfetch_test_support::TestEnv;
use jiff::{SignedDuration, Timestamp};
use proptest::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};

use super::export::{joined, jsonl};
use super::{at, broken, log, now, origin, tamper};
use crate::audit::{AuditLog, verify_exported};
use crate::clock::Clock;
use crate::home::Home;
use crate::settings::{Settings, SettingsFiles};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// One day, the retention the tests prune with.
const DAY: Duration = Duration::from_hours(24);

/// Ten days before [`now`], older than a day's retention.
fn long_ago() -> Timestamp {
    now() - SignedDuration::from_hours(240)
}

/// A log of `old` entries recorded ten days ago, then `new` recorded now.
async fn aged(env: &TestEnv, old: u64, new: u64) -> AuditLog {
    let log = log(env);
    for index in 1..=old + new {
        let time = if index <= old { long_ago() } else { now() };
        log.record(
            &origin(),
            &SECRET_REFERENCE_USED,
            json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
            time,
        )
        .await
        .unwrap();
    }
    log
}

fn intact(entries: u64) -> AuditVerified {
    AuditVerified {
        intact: true,
        entries,
        first_broken: None,
    }
}

/// How many entries are old and how many new, at least one of each.
fn old_and_new() -> impl Strategy<Value = (u64, u64)> {
    (1..=5_u64, 1..=4_u64)
}

/// The JSON of an exported line, without its hash.
fn without_hash(entry: &ExportedAuditEntry) -> Value {
    let mut value = serde_json::to_value(&entry.entry).unwrap();
    value.as_object_mut().unwrap().remove("hash");
    value
}

/// The hash of an exported line.
fn hash_of(entries: &[ExportedAuditEntry], sequence: u64) -> String {
    entries
        .iter()
        .find(|entry| entry.entry.sequence == sequence)
        .unwrap()
        .hash
        .clone()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(16))]

    /// The entries after the last old one stay as they were, behind the
    /// anchor, and the pruning record follows them.
    /// Covers: story:4-72, setting:audit.retention
    #[test]
    fn pruning_leaves_an_anchor_then_the_newer_entries_as_they_were((old, new) in old_and_new()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(aged(&env, old, new));
        let before = log.export().unwrap();
        let last_pruned = hash_of(&before, old);

        runtime.block_on(log.prune(DAY, Some(SettingsLayer::UserConfiguration), now())).unwrap();

        let after = log.export().unwrap();
        let kept: Vec<&ExportedAuditEntry> = before.iter().skip(usize::try_from(old).unwrap()).collect();
        prop_assert_eq!(
            (
                without_hash(&after[0]),
                after[0].hash.clone(),
                after[1..after.len() - 1].iter().collect::<Vec<_>>(),
                without_hash(&after[after.len() - 1]),
            ),
            (
                json!({
                    "sequence": old,
                    "time": "2026-01-05T09:00:00Z",
                    "kind": "audit_log_anchor",
                    "actor": { "kind": "engine" },
                    "layer": "user_configuration",
                    "details": { "last_pruned_hash": last_pruned },
                }),
                last_pruned.clone(),
                kept,
                json!({
                    "sequence": old + new + 1,
                    "time": "2026-01-05T09:00:00Z",
                    "kind": "audit_log_pruned",
                    "actor": { "kind": "engine" },
                    "layer": "user_configuration",
                    "details": {
                        "through_sequence": old,
                        "last_pruned_hash": last_pruned,
                        "retention": 86_400_000,
                    },
                }),
            )
        );
    }

    /// The positive control: a pruned log verifies intact from its anchor,
    /// in the store and in its export.
    /// Covers: story:4-72, setting:audit.retention
    #[test]
    fn a_pruned_log_verifies_intact_from_its_anchor((old, new) in old_and_new()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(aged(&env, old, new));

        runtime.block_on(log.prune(DAY, None, now())).unwrap();

        let exported = joined(&jsonl(&log.export().unwrap()));
        prop_assert_eq!(
            (log.verify().unwrap(), verify_exported(&exported)),
            (intact(new + 2), intact(new + 2))
        );
    }

    /// Covers: story:4-72
    #[test]
    fn an_edited_entry_after_the_anchor_is_reported_at_its_sequence((old, new) in old_and_new()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(aged(&env, old, new));
        runtime.block_on(log.prune(DAY, None, now())).unwrap();
        let edited = old + new;
        tamper(&log)
            .execute(
                "UPDATE entries SET entry = replace(entry, 'example.com', 'example.org') WHERE sequence = ?1",
                [at(edited)],
            )
            .unwrap();

        prop_assert_eq!(log.verify().unwrap(), broken(new + 2, edited));
    }

    /// An anchor whose text was rewritten, as its time, no longer matches
    /// the pruning record that names it.
    /// Covers: story:4-72
    #[test]
    fn an_edited_anchor_is_reported_at_its_sequence((old, new) in old_and_new()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(aged(&env, old, new));
        runtime.block_on(log.prune(DAY, None, now())).unwrap();
        tamper(&log)
            .execute(
                "UPDATE entries SET entry = replace(entry, '2026-01-05', '2025-01-05') WHERE sequence = ?1",
                [at(old)],
            )
            .unwrap();

        prop_assert_eq!(log.verify().unwrap(), broken(new + 2, old));
    }

    /// An anchor moved past an entry, as one who deletes the oldest kept
    /// entry would leave it, is reported at its new place: the pruning
    /// record names the anchor it left.
    /// Covers: story:4-72
    #[test]
    fn an_anchor_moved_past_an_entry_is_reported_at_its_sequence((old, new) in old_and_new()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(aged(&env, old, new));
        runtime.block_on(log.prune(DAY, None, now())).unwrap();
        let exported = log.export().unwrap();
        let moved = old + 1;
        let hash = hash_of(&exported, moved);
        let mut anchor = without_hash(&exported[0]);
        anchor["sequence"] = json!(moved);
        anchor["details"]["last_pruned_hash"] = json!(hash);
        let connection = tamper(&log);
        connection
            .execute("DELETE FROM entries WHERE sequence <= ?1", [at(moved)])
            .unwrap();
        connection
            .execute(
                "INSERT INTO entries (sequence, entry, hash) VALUES (?1, ?2, ?3)",
                params![at(moved), serde_json_canonicalizer::to_string(&anchor).unwrap(), hash],
            )
            .unwrap();

        prop_assert_eq!(log.verify().unwrap(), broken(new + 1, moved));
    }

    /// An anchor whose hash was rewritten breaks the chain at the entry
    /// after it.
    /// Covers: story:4-72
    #[test]
    fn an_anchor_with_another_hash_is_reported((old, new) in old_and_new()) {
        let env = TestEnv::in_memory();
        let runtime = goodfetch_test_support::runtime();
        let log = runtime.block_on(aged(&env, old, new));
        runtime.block_on(log.prune(DAY, None, now())).unwrap();
        let other = "0".repeat(64);
        tamper(&log)
            .execute(
                "UPDATE entries SET hash = ?2, entry = json_set(entry, '$.details.last_pruned_hash', ?2) WHERE sequence = ?1",
                params![at(old), other],
            )
            .unwrap();

        prop_assert_eq!(log.verify().unwrap(), broken(new + 2, old + 1));
    }
}

/// An edited anchor shows in an export as in the store.
/// Covers: story:4-72
#[tokio::test]
async fn an_edited_anchor_in_an_export_is_reported_at_its_sequence() {
    let env = TestEnv::in_memory();
    let log = aged(&env, 2, 2).await;
    log.prune(DAY, None, now()).await.unwrap();
    let mut lines = jsonl(&log.export().unwrap());
    lines[0] = lines[0].replace("2026-01-05", "2025-01-05");

    assert_eq!(verify_exported(&joined(&lines)), broken(4, 2));
}

/// Covers: story:4-72, setting:audit.retention
#[tokio::test]
async fn a_log_with_nothing_older_than_its_retention_is_not_pruned() {
    let env = TestEnv::in_memory();
    let log = aged(&env, 0, 3).await;
    let before = log.export().unwrap();

    let pruned = log.prune(DAY, None, now()).await.unwrap();

    assert_eq!((pruned, log.export().unwrap()), (None, before));
}

/// A second pruning replaces the first's anchor, and the log still
/// verifies from the new one.
/// Covers: story:4-72, setting:audit.retention
#[tokio::test]
async fn a_second_pruning_moves_the_anchor_and_the_log_verifies_from_it() {
    let env = TestEnv::in_memory();
    let log = aged(&env, 2, 1).await;
    log.prune(DAY, None, now()).await.unwrap();
    let later = now() + SignedDuration::from_hours(48);
    log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), later)
        .await
        .unwrap();

    log.prune(DAY, None, later).await.unwrap();

    let exported = log.export().unwrap();
    assert_eq!(
        (
            exported
                .iter()
                .map(|entry| (entry.entry.sequence, entry.entry.kind.clone()))
                .collect::<Vec<_>>(),
            log.verify().unwrap()
        ),
        (
            vec![
                (4, "audit_log_anchor".to_owned()),
                (5, "secret_reference_used".to_owned()),
                (6, "audit_log_pruned".to_owned()),
            ],
            intact(3)
        )
    );
}

/// The machine's settings files of `env`, its user configuration setting
/// `audit.retention` to `retention` when there is one.
fn retaining(env: &TestEnv, retention: Option<Duration>) -> SettingsFiles {
    let files = SettingsFiles::of(
        &Home::new(env.home().to_path_buf()),
        &ProductVersion::parse("1.4.0").unwrap(),
    );
    if let Some(retention) = retention {
        configure(
            &files,
            &format!(
                "version = \"0.1\"\n\n[settings]\naudit.retention = {}\n",
                retention.as_millis()
            ),
        );
    }
    files
}

/// Writes `configuration` as the user configuration of `files`.
fn configure(files: &SettingsFiles, configuration: &str) {
    std::fs::create_dir_all(files.user_configuration().parent().unwrap()).unwrap();
    std::fs::write(files.user_configuration(), configuration).unwrap();
}

/// The sequence and kind of each of `log`'s entries, in order.
fn entries(log: &AuditLog) -> Vec<(u64, String)> {
    log.export()
        .unwrap()
        .into_iter()
        .map(|entry| (entry.entry.sequence, entry.entry.kind))
        .collect()
}

/// `(sequence, kind)` pairs, the kinds written short.
fn listed(entries: &[(u64, &str)]) -> Vec<(u64, String)> {
    entries
        .iter()
        .map(|(sequence, kind)| (*sequence, (*kind).to_owned()))
        .collect()
}

/// The log prunes as the engine starts and then once a day on its Clock,
/// never at an append (ADR 0140): an old entry goes at start, one that grew
/// old goes a day later, each time behind an anchor and with a pruning
/// record, the log verifying intact from its anchor; a tail cut off after
/// the day's pass is still reported, at the anchor only the cut-off
/// pruning record named.
/// Covers: story:4-72, setting:audit.retention
#[test]
fn the_log_prunes_as_the_engine_starts_then_once_a_day_never_at_an_append() {
    let env = TestEnv::in_memory();
    let files = retaining(&env, Some(DAY));

    let (log, [at_start, appended, a_day_later]) =
        goodfetch_test_support::runtime().block_on(async {
            let owner = TaskOwner::root(BlockingPolicy::Inline);
            let clock = Clock::paused(now().into());
            let log = aged(&env, 2, 0).await;
            let half_a_day_ago = now() - SignedDuration::from_hours(12);
            log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), half_a_day_ago)
                .await
                .unwrap();
            let log = log.retained_under(Settings::new(files, Product::Collie, owner.child()));

            log.start_pruning(clock, &owner);
            tokio::time::sleep(Duration::from_secs(1)).await;
            let at_start = entries(&log);
            log.record(
                &origin(),
                &SECRET_REFERENCE_USED,
                json!({}),
                clock.timestamp(),
            )
            .await
            .unwrap();
            let appended = entries(&log);
            tokio::time::sleep(DAY).await;
            let a_day_later = entries(&log);
            owner.stop().await;
            (log, [at_start, appended, a_day_later])
        });
    let verified = log.verify().unwrap();
    tamper(&log)
        .execute("DELETE FROM entries WHERE sequence >= ?1", [at(5)])
        .unwrap();

    assert_eq!(
        (
            at_start,
            appended,
            a_day_later,
            verified,
            log.verify().unwrap()
        ),
        (
            listed(&[
                (2, "audit_log_anchor"),
                (3, "secret_reference_used"),
                (4, "audit_log_pruned"),
            ]),
            listed(&[
                (2, "audit_log_anchor"),
                (3, "secret_reference_used"),
                (4, "audit_log_pruned"),
                (5, "secret_reference_used"),
            ]),
            listed(&[
                (3, "audit_log_anchor"),
                (4, "audit_log_pruned"),
                (5, "secret_reference_used"),
                (6, "audit_log_pruned"),
            ]),
            intact(4),
            broken(2, 3),
        )
    );
}

/// Settings files the machine's layers cannot be read from say nothing of
/// the log: a pass under them prunes nothing, entries older than any
/// retention included, and the next append is recorded.
/// Covers: story:4-72, setting:audit.retention
#[rstest::rstest]
#[case::a_typo_in_user_configuration(None, "version = \"0.1\"\n\n[settings\n")]
#[case::a_lock_on_a_key_user_configuration_sets(
    Some("version = \"0.1\"\n\n[lock.timeout]\nnavigation = 10000\n"),
    "version = \"0.1\"\n\n[settings.timeout]\nnavigation = 2000\n"
)]
fn a_pass_under_settings_that_cannot_be_read_prunes_nothing(
    #[case] policy: Option<&str>,
    #[case] configuration: &str,
) {
    let env = TestEnv::in_memory();
    let files = retaining(&env, None);
    configure(&files, configuration);
    if let Some(policy) = policy {
        std::fs::write(files.operator_policy(), policy).unwrap();
    }

    let kept = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let clock = Clock::paused(now().into());
        let log = aged(&env, 2, 0).await.retained_under(Settings::new(
            files,
            Product::Collie,
            owner.child(),
        ));
        log.start_pruning(clock, &owner);
        tokio::time::sleep(Duration::from_secs(1)).await;
        log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now())
            .await
            .unwrap();
        owner.stop().await;
        entries(&log)
    });

    assert_eq!(
        kept,
        listed(&[
            (1, "secret_reference_used"),
            (2, "secret_reference_used"),
            (3, "secret_reference_used"),
        ])
    );
}

/// Each pass prunes by the `audit.retention` of the engine's snapshot of
/// the machine's settings, which reads no file per pass: a hand edit
/// setting it is not seen by the next day's pass, and the engine's own
/// write of it, which refreshes the snapshot, is by the pass after
/// (ADR 0140).
/// Covers: story:4-72, setting:audit.retention
#[test]
fn a_pass_prunes_by_the_engines_own_write_of_the_retention_not_by_a_hand_edit() {
    let env = TestEnv::in_memory();
    let files = retaining(&env, None);

    let [at_start, edited, written] = goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let clock = Clock::paused(now().into());
        let settings = Settings::new(files.clone(), Product::Collie, owner.child());
        let log = aged(&env, 2, 0).await.retained_under(settings.clone());
        log.start_pruning(clock, &owner);
        tokio::time::sleep(Duration::from_secs(1)).await;
        let at_start = kinds(&log);
        configure(
            &files,
            &format!(
                "version = \"0.1\"\n\n[settings]\naudit.retention = {}\n",
                DAY.as_millis()
            ),
        );
        tokio::time::sleep(DAY).await;
        let edited = kinds(&log);
        settings.refresh().await;
        tokio::time::sleep(DAY).await;
        let written = kinds(&log);
        owner.stop().await;
        [at_start, edited, written]
    });

    let pruned = |kinds: &[String]| {
        kinds
            .iter()
            .filter(|kind| *kind == "audit_log_pruned")
            .count()
    };
    assert_eq!(
        (pruned(&at_start), pruned(&edited), pruned(&written)),
        (0, 0, 1),
        "{written:?}"
    );
}

/// The kinds of `log`'s entries, in order.
fn kinds(log: &AuditLog) -> Vec<String> {
    log.export()
        .unwrap()
        .into_iter()
        .map(|entry| entry.entry.kind)
        .collect()
}
