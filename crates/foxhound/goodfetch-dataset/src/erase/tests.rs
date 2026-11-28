use goodfetch_protocol::foxhound::erase::RunEraseParams;
use goodfetch_protocol::run::RunId;
use rusqlite::Connection;

use super::{MatchCriteria, erase_database};

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
    let erased = erase_database(&connection, &criteria).unwrap();
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
