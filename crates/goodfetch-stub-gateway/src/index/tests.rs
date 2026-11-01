use std::sync::Arc;

use serde_json::json;

use super::{RunIndex, setting_keys};

#[test]
fn a_starts_settings_name_each_nested_key_with_its_prefixes() {
    let settings =
        json!({"crawl": {"max_requests": 10, "scope": {"include": ["a"]}}, "timeout": {"run": 5}});

    let mut keys = setting_keys(settings.as_object().unwrap());

    keys.sort_unstable();
    assert_eq!(
        keys,
        [
            "crawl",
            "crawl.max_requests",
            "crawl.scope",
            "crawl.scope.include",
            "timeout",
            "timeout.run",
        ]
    );
}

#[test]
fn a_second_start_of_a_run_id_joins_the_first_starts_reservation() {
    let mut index = RunIndex::default();
    let first = index.reserve("run-1");

    let second = index.reserve("run-1");

    assert!(Arc::ptr_eq(&first, &second));
}

#[test]
fn a_reservation_whose_start_started_no_run_is_forgotten() {
    let mut index = RunIndex::default();
    let failed = index.reserve("run-1");
    index.release("run-1", &failed);

    let again = index.reserve("run-1");

    assert!(!Arc::ptr_eq(&failed, &again));
}

#[test]
fn a_reservation_another_start_joined_stays_when_one_start_fails() {
    let mut index = RunIndex::default();
    let failed = index.reserve("run-1");
    let joined = index.reserve("run-1");
    index.release("run-1", &failed);

    let again = index.reserve("run-1");

    assert!(Arc::ptr_eq(&joined, &again));
}
