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
use crate::audit::{AuditError, AuditLog, verify_exported};
use crate::home::Home;
use crate::settings::SettingsFiles;

/// One day, the retention the tests prune with.
const DAY: Duration = Duration::from_hours(24);

/// Ten days before [`now`], older than a day's retention.
fn long_ago() -> Timestamp {
    now() - SignedDuration::from_hours(240)
}

/// A log of `old` entries recorded ten days ago, then `new` recorded now.
fn aged(env: &TestEnv, old: u64, new: u64) -> AuditLog {
    let log = log(env);
    for index in 1..=old + new {
        let time = if index <= old { long_ago() } else { now() };
        log.record(
            &origin(),
            &SECRET_REFERENCE_USED,
            json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
            time,
        )
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
        let log = aged(&env, old, new);
        let before = log.export().unwrap();
        let last_pruned = hash_of(&before, old);

        log.prune(DAY, Some(SettingsLayer::UserConfiguration), now()).unwrap();

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
        let log = aged(&env, old, new);

        log.prune(DAY, None, now()).unwrap();

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
        let log = aged(&env, old, new);
        log.prune(DAY, None, now()).unwrap();
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
        let log = aged(&env, old, new);
        log.prune(DAY, None, now()).unwrap();
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
        let log = aged(&env, old, new);
        log.prune(DAY, None, now()).unwrap();
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
        let log = aged(&env, old, new);
        log.prune(DAY, None, now()).unwrap();
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
#[test]
fn an_edited_anchor_in_an_export_is_reported_at_its_sequence() {
    let env = TestEnv::in_memory();
    let log = aged(&env, 2, 2);
    log.prune(DAY, None, now()).unwrap();
    let mut lines = jsonl(&log.export().unwrap());
    lines[0] = lines[0].replace("2026-01-05", "2025-01-05");

    assert_eq!(verify_exported(&joined(&lines)), broken(4, 2));
}

/// Covers: story:4-72, setting:audit.retention
#[test]
fn a_log_with_nothing_older_than_its_retention_is_not_pruned() {
    let env = TestEnv::in_memory();
    let log = aged(&env, 0, 3);
    let before = log.export().unwrap();

    let pruned = log.prune(DAY, None, now()).unwrap();

    assert_eq!((pruned, log.export().unwrap()), (None, before));
}

/// A second pruning replaces the first's anchor, and the log still
/// verifies from the new one.
/// Covers: story:4-72, setting:audit.retention
#[test]
fn a_second_pruning_moves_the_anchor_and_the_log_verifies_from_it() {
    let env = TestEnv::in_memory();
    let log = aged(&env, 2, 1);
    log.prune(DAY, None, now()).unwrap();
    let later = now() + SignedDuration::from_hours(48);
    log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), later)
        .unwrap();

    log.prune(DAY, None, later).unwrap();

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

/// An append under machine layers whose `audit.retention` cannot be read
/// is refused, appending nothing, rather than appended unpruned: the
/// retention may be one that prunes what it would keep.
/// Covers: story:4-72, setting:audit.retention
#[test]
fn an_append_whose_retention_cannot_be_read_is_refused_rather_than_never_pruned() {
    let env = TestEnv::in_memory();
    let home = Home::new(env.home().to_path_buf());
    let version = ProductVersion::parse("1.4.0").unwrap();
    let files = SettingsFiles::of(&home, &version);
    let configuration = files.user_configuration().to_owned();
    std::fs::create_dir_all(configuration.parent().unwrap()).unwrap();
    std::fs::write(&configuration, "version = \"0.1\"\n\n[settings\n").unwrap();
    let log = log(&env).retained_under(files, Product::Collie);

    let recorded = log.record(&origin(), &SECRET_REFERENCE_USED, json!({}), now());

    assert_eq!(
        (
            matches!(recorded, Err(AuditError::Retention { .. })),
            log.export().unwrap()
        ),
        (true, Vec::new())
    );
}
