//! The Audit log's export through its interface (spec #564's seam; ADRs
//! 0016 and 0067): every entry in sequence with its hash, which the JSONL
//! reader verifies off the machine, intact, or at the first line an edit,
//! a deletion or a reordering touched, as property tests.

use goodfetch_protocol::audit::{Actor, AuditVerified, CERTIFICATE_PINNED, ExportedAuditEntry};
use goodfetch_test_support::TestEnv;
use proptest::prelude::*;
use serde_json::json;

use super::{broken, log, log_and_sequence, now, written};
use crate::audit::{Origin, verify_exported};

/// The JSONL `audit export` writes of `entries`: one entry per line.
pub(super) fn jsonl(entries: &[ExportedAuditEntry]) -> Vec<String> {
    entries
        .iter()
        .map(|entry| serde_json::to_string(entry).unwrap())
        .collect()
}

/// The lines, each ended as a file holds it.
pub(super) fn joined(lines: &[String]) -> Vec<u8> {
    lines
        .iter()
        .flat_map(|line| format!("{line}\n").into_bytes())
        .collect()
}

/// The entry and hash of `the_first_entry_hashes_zero_bytes_then_its_canonical_json`,
/// worked out apart with Python's `hashlib`.
/// Covers: story:4-71
#[test]
fn an_export_holds_each_entry_with_the_hash_that_chains_it() {
    let env = TestEnv::new();
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
    .unwrap();

    let exported = log.export().unwrap();

    assert_eq!(
        jsonl(&exported),
        vec![
            r#"{"sequence":1,"time":"2026-01-05T09:00:00Z","kind":"certificate_pinned","actor":{"kind":"engine"},"details":{"host":"example.com"},"hash":"07a1ab07e3b478200081a2520a290df55a7ca813a6aa5e7b50714d205f255591"}"#
                .to_owned()
        ]
    );
}

/// Covers: story:4-71
#[test]
fn a_log_never_written_exports_nothing_and_is_not_created() {
    let env = TestEnv::new();

    let exported = log(&env).export().unwrap();

    assert_eq!(
        (exported, env.home().join("audit-log.sqlite").exists()),
        (vec![], false)
    );
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(24))]

    /// The positive control: an export as written verifies intact.
    /// Covers: story:4-71
    #[test]
    fn an_exported_log_verifies_intact((count, _sequence) in log_and_sequence()) {
        let env = TestEnv::new();
        let lines = jsonl(&written(&env, count).export().unwrap());

        prop_assert_eq!(
            verify_exported(&joined(&lines)),
            AuditVerified { intact: true, entries: count, first_broken: None }
        );
    }

    /// Covers: story:4-71
    #[test]
    fn an_edited_line_of_an_export_is_reported_at_its_sequence((count, sequence) in log_and_sequence()) {
        let env = TestEnv::new();
        let mut lines = jsonl(&written(&env, count).export().unwrap());
        let index = usize::try_from(sequence - 1).unwrap();
        lines[index] = lines[index].replace("example.com", "example.org");

        prop_assert_eq!(verify_exported(&joined(&lines)), broken(count, sequence));
    }

    /// A line deleted before the last: an export has no head file, so its
    /// tail is the last line it holds.
    /// Covers: story:4-71
    #[test]
    fn a_deleted_line_of_an_export_is_reported_at_its_sequence((count, sequence) in log_and_sequence()) {
        prop_assume!(sequence < count);
        let env = TestEnv::new();
        let mut lines = jsonl(&written(&env, count).export().unwrap());
        lines.remove(usize::try_from(sequence - 1).unwrap());

        prop_assert_eq!(verify_exported(&joined(&lines)), broken(count - 1, sequence));
    }

    /// Covers: story:4-71
    #[test]
    fn a_reordered_pair_of_lines_is_reported_at_its_first_sequence((count, sequence) in log_and_sequence()) {
        prop_assume!(sequence < count);
        let env = TestEnv::new();
        let mut lines = jsonl(&written(&env, count).export().unwrap());
        let index = usize::try_from(sequence - 1).unwrap();
        lines.swap(index, index + 1);

        prop_assert_eq!(verify_exported(&joined(&lines)), broken(count, sequence));
    }
}

/// A line that is no JSON object with a hash breaks the export there.
/// Covers: story:4-71
#[test]
fn a_line_that_is_no_entry_breaks_the_export_at_its_place() {
    let env = TestEnv::new();
    let mut lines = jsonl(&written(&env, 3).export().unwrap());
    lines[1] = "not an entry".to_owned();

    assert_eq!(verify_exported(&joined(&lines)), broken(3, 2));
}
