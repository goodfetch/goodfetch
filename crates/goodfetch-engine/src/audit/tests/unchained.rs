//! A run whose `security.audit_chain` is off, through the Audit log's
//! interface (spec #564's seam; ADR 0016): once its Run is unchained, the
//! entries recorded for it are written unchained, each marked
//! `chained: false` and hashed alone, and `audit verify` steps over them
//! while it verifies the chain before and after them, in the store and in
//! its export, as property tests; an entry of the chain turned unchained,
//! or an unchained one deleted, still shows; the switch itself is recorded
//! chained; and pruning through unchained entries leaves an anchor the
//! rest chains from.

use std::time::Duration;

use goodfetch_protocol::audit::{AuditVerified, SECRET_REFERENCE_USED, SECURITY_SWITCH_TURNED_OFF};
use goodfetch_protocol::run::RunId;
use goodfetch_test_support::TestEnv;
use jiff::SignedDuration;
use proptest::prelude::*;
use rusqlite::params;
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

use super::export::{joined, jsonl};
use super::{at, broken, log, now, origin, tamper};
use crate::audit::{AuditLog, Origin, verify_exported};

/// The Run the tests switch the chain off for: [`origin`]'s.
fn unchained_run() -> RunId {
    RunId::parse("audited").unwrap()
}

/// An entry's origin in another Run, whose chain stays on.
fn chained_origin() -> Origin {
    Origin {
        run_id: Some(RunId::parse("chained").unwrap()),
        ..origin()
    }
}

/// Records one secret use from `origin`, numbered `index`.
fn use_secret(log: &AuditLog, origin: &Origin, index: u64) {
    log.record(
        origin,
        &SECRET_REFERENCE_USED,
        json!({ "reference": format!("env:SECRET_{index}"), "domain": "example.com" }),
        now(),
    )
    .unwrap();
}

/// A log of `before` chained entries, then `unchained` of the unchained
/// Run, then `after` chained entries.
fn around(env: &TestEnv, before: u64, unchained: u64, after: u64) -> AuditLog {
    let log = log(env);
    for index in 1..=before {
        use_secret(&log, &chained_origin(), index);
    }
    log.unchain(unchained_run());
    for index in 1..=unchained {
        use_secret(&log, &origin(), before + index);
    }
    for index in 1..=after {
        use_secret(&log, &chained_origin(), before + unchained + index);
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

/// How many chained entries come before and after the unchained ones, and
/// how many unchained ones there are, at least one.
fn before_unchained_after() -> impl Strategy<Value = (u64, u64, u64)> {
    (0..=3_u64, 1..=3_u64, 0..=3_u64)
}

/// The first unchained entry's text and hash, `SHA-256` of 32 zero bytes
/// followed by its RFC 8785 form, worked out apart with Python's
/// `hashlib`: an unchained entry is hashed alone.
/// Covers: story:4-74
#[test]
fn an_unchained_runs_entry_is_marked_unchained_and_hashed_alone() {
    let env = TestEnv::in_memory();
    let log = around(&env, 1, 1, 0);

    let stored: (String, String) = rusqlite::Connection::open(log.path())
        .unwrap()
        .query_row(
            "SELECT entry, hash FROM entries WHERE sequence = 2",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();

    assert_eq!(
        stored,
        (
            r#"{"actor":{"kind":"client_surface","surface":"cli"},"chained":false,"details":{"domain":"example.com","reference":"env:SECRET_2"},"kind":"secret_reference_used","layer":"run","run_id":"audited","sequence":2,"time":"2026-01-05T09:00:00Z"}"#.to_owned(),
            "0113dafbf863bc9c48c19d8dd042460c2a20a4b8d336a8cad06ec0bf2889ee1c".to_owned()
        )
    );
}

/// The switch itself is recorded chained, though its Run is unchained.
/// Covers: story:4-74
#[test]
fn the_switch_of_an_unchained_run_is_recorded_chained() {
    let env = TestEnv::in_memory();
    let log = around(&env, 0, 1, 0);

    log.record_chained(
        &origin(),
        &SECURITY_SWITCH_TURNED_OFF,
        json!({ "setting": "security.audit_chain", "value": false }),
        now(),
    )
    .unwrap();

    let chained: Vec<Value> = log
        .export()
        .unwrap()
        .iter()
        .map(|exported| json!([exported.entry.kind, exported.entry.chained]))
        .collect();
    assert_eq!(
        (chained, log.verify()),
        (
            vec![
                json!(["secret_reference_used", false]),
                json!(["security_switch_turned_off", true]),
            ],
            Ok(intact(2))
        )
    );
}

/// A Run chained again records chained entries.
#[test]
fn a_run_chained_again_records_chained_entries() {
    let env = TestEnv::in_memory();
    let log = around(&env, 0, 1, 0);

    let was_unchained = log.rechain(&unchained_run());
    use_secret(&log, &origin(), 2);

    let chained: Vec<bool> = log
        .export()
        .unwrap()
        .iter()
        .map(|exported| exported.entry.chained)
        .collect();
    assert_eq!((was_unchained, chained), (true, vec![false, true]));
}

/// An entry of the chain that someone marked unchained and hashed alone
/// breaks the chain at the next chained entry, which was chained to it.
#[test]
fn a_chained_entry_turned_unchained_breaks_the_chain_at_the_next() {
    let env = TestEnv::in_memory();
    let log = around(&env, 3, 0, 0);
    let entry: String = rusqlite::Connection::open(log.path())
        .unwrap()
        .query_row("SELECT entry FROM entries WHERE sequence = 2", [], |row| {
            row.get(0)
        })
        .unwrap();
    let mut forged: Value = serde_json::from_str(&entry).unwrap();
    forged["chained"] = json!(false);
    let forged = serde_json_canonicalizer::to_string(&forged).unwrap();
    let mut hasher = Sha256::new();
    hasher.update([0_u8; 32]);
    hasher.update(forged.as_bytes());
    let hash = data_encoding::HEXLOWER.encode(&hasher.finalize());
    tamper(&log)
        .execute(
            "UPDATE entries SET entry = ?1, hash = ?2 WHERE sequence = 2",
            params![forged, hash],
        )
        .unwrap();

    assert_eq!(log.verify(), Ok(broken(3, 3)));
}

/// Pruning through unchained entries leaves an anchor carrying the chain's
/// last hash, from which the rest verifies intact.
/// Covers: story:4-72
#[test]
fn pruning_through_unchained_entries_leaves_an_anchor_the_rest_chains_from() {
    let env = TestEnv::in_memory();
    let log = log(&env);
    let long_ago = now() - SignedDuration::from_hours(240);
    log.record(
        &chained_origin(),
        &SECRET_REFERENCE_USED,
        json!({ "index": 1 }),
        long_ago,
    )
    .unwrap();
    log.unchain(unchained_run());
    log.record(
        &origin(),
        &SECRET_REFERENCE_USED,
        json!({ "index": 2 }),
        long_ago,
    )
    .unwrap();
    use_secret(&log, &chained_origin(), 3);

    let pruned = log.prune(Duration::from_hours(24), None, now()).unwrap();

    assert_eq!(
        (
            pruned.map(|pruned| pruned.details["through_sequence"].clone()),
            log.verify()
        ),
        (Some(json!(2)), Ok(intact(3)))
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// Covers: story:4-70, story:4-74
    #[test]
    fn the_chain_around_an_unchained_runs_entries_verifies_intact(
        (before, unchained, after) in before_unchained_after()
    ) {
        let env = TestEnv::in_memory();
        let log = around(&env, before, unchained, after);
        let marked: Vec<bool> = log
            .export()
            .unwrap()
            .iter()
            .map(|exported| exported.entry.chained)
            .collect();
        let expected: Vec<bool> = std::iter::repeat_n(true, usize::try_from(before).unwrap())
            .chain(std::iter::repeat_n(false, usize::try_from(unchained).unwrap()))
            .chain(std::iter::repeat_n(true, usize::try_from(after).unwrap()))
            .collect();
        let total = before + unchained + after;
        prop_assert_eq!(
            (marked, log.verify(), verify_exported(&joined(&jsonl(&log.export().unwrap())))),
            (expected, Ok(intact(total)), intact(total))
        );
    }

    /// Covers: story:4-70, story:4-74
    #[test]
    fn an_edit_after_an_unchained_runs_entries_is_reported_at_its_sequence(
        (before, unchained, after) in (0..=3_u64, 1..=3_u64, 1..=3_u64)
    ) {
        let env = TestEnv::in_memory();
        let log = around(&env, before, unchained, after);
        let edited = before + unchained + 1;
        tamper(&log)
            .execute(
                "UPDATE entries SET entry = replace(entry, 'example.com', 'example.org') WHERE sequence = ?1",
                [at(edited)],
            )
            .unwrap();

        prop_assert_eq!(log.verify(), Ok(broken(before + unchained + after, edited)));
    }

    /// Covers: story:4-70
    #[test]
    fn a_deleted_unchained_entry_is_reported_at_its_sequence(
        (before, unchained, after) in before_unchained_after()
    ) {
        let env = TestEnv::in_memory();
        let log = around(&env, before, unchained, after.max(1));
        let deleted = before + 1;
        tamper(&log)
            .execute("DELETE FROM entries WHERE sequence = ?1", [at(deleted)])
            .unwrap();

        prop_assert_eq!(
            log.verify(),
            Ok(broken(before + unchained + after.max(1) - 1, deleted))
        );
    }
}
