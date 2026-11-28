use std::collections::BTreeSet;

use goodfetch_protocol::foxhound::erase::RunEraseParams;
use goodfetch_protocol::run::RunId;
use rusqlite::Connection;
use serde_json::{Value, json};

use super::{MatchCriteria, erase_database, scrub_json_value, scrub_run_folder_files};

#[test]
fn match_criteria_requires_non_empty_value_or_key() {
    let empty = RunEraseParams {
        run_id: RunId::parse("empty-run").unwrap(),
        value: Some(String::new()),
        key: None,
    };
    assert!(MatchCriteria::from_params(&empty).is_err());

    let by_key = RunEraseParams {
        run_id: RunId::parse("key-run").unwrap(),
        value: None,
        key: Some("email".to_owned()),
    };
    assert!(MatchCriteria::from_params(&by_key).is_ok());
}

#[test]
fn erase_database_removes_matching_baseline_rows_and_key_matched_records() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE records (
                dataset TEXT NOT NULL,
                number INTEGER NOT NULL,
                url TEXT NOT NULL,
                data TEXT NOT NULL,
                provenance TEXT NOT NULL
            );
            CREATE TABLE baselines (
                dataset TEXT NOT NULL,
                \"key\" TEXT NOT NULL,
                payload TEXT NOT NULL
            );
            INSERT INTO records VALUES
                ('contacts', 1, 'https://example.test/1', '{\"id\":\"u1\",\"email\":\"ada@example.test\"}', '{}'),
                ('contacts', 2, 'https://example.test/2', '{\"id\":\"u2\",\"email\":\"bob@example.test\"}', '{}');
            INSERT INTO baselines VALUES
                ('contacts', 'u1', '{\"email\":\"ada@example.test\"}'),
                ('contacts', 'u2', '{\"email\":\"bob@example.test\"}');",
        )
        .unwrap();

    let criteria = MatchCriteria {
        value: Some("ada@example.test".to_owned()),
        key: Some("email".to_owned()),
    };
    let mut needles = criteria.initial_value_needles();
    let erased = erase_database(&connection, &criteria, &mut needles).unwrap();
    assert_eq!(erased.records, 1);
    assert_eq!(erased.baseline_rows, 1);

    let remaining_records: i64 = connection
        .prepare_cached("SELECT COUNT(*) FROM records")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    let remaining_baselines: i64 = connection
        .prepare_cached("SELECT COUNT(*) FROM baselines")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert_eq!((remaining_records, remaining_baselines), (1, 1));
}

#[test]
fn key_only_erasure_collects_actual_values_and_never_rewrites_json_keys() {
    let connection = Connection::open_in_memory().unwrap();
    connection
        .execute_batch(
            "CREATE TABLE records (
                dataset TEXT NOT NULL,
                number INTEGER NOT NULL,
                url TEXT NOT NULL,
                data TEXT NOT NULL,
                provenance TEXT NOT NULL
            );
            CREATE TABLE rejected_records (
                dataset TEXT NOT NULL,
                number INTEGER NOT NULL,
                data TEXT NOT NULL,
                rejection TEXT NOT NULL
            );
            CREATE TABLE trace_notes (
                id TEXT PRIMARY KEY,
                payload TEXT NOT NULL
            );
            INSERT INTO records VALUES
                ('contacts', 1, 'https://example.test/1', '{\"id\":\"u1\",\"email\":\"ada@example.test\"}', '{}'),
                ('contacts', 2, 'https://example.test/2', '{\"id\":\"u2\",\"note\":\"send email later\"}', '{}');
            INSERT INTO rejected_records VALUES
                ('contacts', 1, '{\"email\":\"bad@example.test\"}', '{\"reason\":\"missing id\"}');
            INSERT INTO trace_notes VALUES
                ('n1', '{\"email\":\"ada@example.test\",\"note\":\"keep email label\"}'),
                ('n2', 'plain text mentioning email and ada@example.test');",
        )
        .unwrap();

    let criteria = MatchCriteria {
        value: None,
        key: Some("email".to_owned()),
    };
    let mut needles = criteria.initial_value_needles();
    assert!(needles.is_empty(), "key name must not become a text needle");

    let erased = erase_database(&connection, &criteria, &mut needles).unwrap();
    assert_eq!(erased.records, 1);
    assert_eq!(erased.rejected_records, 1);
    assert!(needles.contains("ada@example.test"));
    assert!(needles.contains("bad@example.test"));
    assert!(!needles.contains("email"));

    // Record 2 without an `email` field survives untouched.
    let remaining: String = connection
        .prepare_cached("SELECT data FROM records")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert!(remaining.contains("send email later"), "{remaining}");

    // JSON trace row removes the `email` key structurally and keeps the word "email".
    let n1: String = connection
        .prepare_cached("SELECT payload FROM trace_notes WHERE id = 'n1'")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    let parsed_n1: Value = serde_json::from_str(&n1).unwrap();
    assert_eq!(parsed_n1, json!({ "note": "keep email label" }));

    // Plain text replaces the collected personal value, not the word "email".
    let n2: String = connection
        .prepare_cached("SELECT payload FROM trace_notes WHERE id = 'n2'")
        .unwrap()
        .query_row([], |row| row.get(0))
        .unwrap();
    assert_eq!(n2, "plain text mentioning email and [erased]");
}

#[test]
#[expect(
    clippy::disallowed_methods,
    reason = "unit test creates and reads fixture files in its test folder"
)]
fn scrub_json_value_traverses_dotted_key_paths_and_scrubs_bodies_dir() {
    let mut doc = json!({
        "id": "1",
        "profile": { "phone": "+33102030405", "city": "Paris" },
        "nested": [{ "profile": { "phone": "+33999999999" } }]
    });
    let criteria = MatchCriteria {
        value: Some("+33102030405".to_owned()),
        key: Some("profile.phone".to_owned()),
    };
    let mut needles = criteria.initial_value_needles();
    assert!(scrub_json_value(&mut doc, &criteria, &mut needles));
    assert_eq!(
        doc,
        json!({
            "id": "1",
            "profile": { "city": "Paris" },
            "nested": [{ "profile": { "phone": "+33999999999" } }]
        })
    );

    // Key-only dotted path removes both nested `profile.phone` entries and scrubs `<run>/bodies/`.
    let env = goodfetch_test_support::TestEnv::in_memory();
    let run_dir = env.folder().join("run-bodies-test");
    let bodies_dir = run_dir.join("bodies");
    let trace_dir = run_dir.join("trace");
    std::fs::create_dir_all(&bodies_dir).unwrap();
    std::fs::create_dir_all(&trace_dir).unwrap();
    std::fs::write(
        bodies_dir.join("abcd"),
        "<html>contact +33102030405 for phone</html>",
    )
    .unwrap();
    std::fs::write(
        trace_dir.join("events.jsonl"),
        "{\"profile\":{\"phone\":\"+33102030405\",\"city\":\"Paris\"}}\n",
    )
    .unwrap();

    let key_only = MatchCriteria {
        value: None,
        key: Some("profile.phone".to_owned()),
    };
    let mut collected = BTreeSet::from(["+33102030405".to_owned()]);
    scrub_run_folder_files(&run_dir, &key_only, &mut collected).unwrap();

    let body_after = std::fs::read_to_string(bodies_dir.join("abcd")).unwrap();
    assert_eq!(body_after, "<html>contact [erased] for phone</html>");
    let trace_after = std::fs::read_to_string(trace_dir.join("events.jsonl")).unwrap();
    assert_eq!(
        serde_json::from_str::<Value>(trace_after.trim()).unwrap(),
        json!({ "profile": { "city": "Paris" } })
    );
}
