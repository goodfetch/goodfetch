use serde_json::json;

use super::{SINK_SHAPE, SinkSettings, SinkType, WriteMode};

/// Covers: setting:sinks
#[test]
fn a_sink_entry_reads_its_type_datasets_destination_and_target_and_appends_by_default() {
    let entry = json!({
        "type": "sqlite",
        "datasets": ["products", "reviews"],
        "destination": "env:SINK_SQLITE_PATH",
        "target": "records",
    });

    (SINK_SHAPE.check)(&entry).unwrap();
    let read: SinkSettings = serde_json::from_value(entry).unwrap();

    assert_eq!(
        (
            read.sink_type,
            read.datasets.as_slice(),
            read.write_mode(),
            read.destination.as_deref(),
            read.target.as_deref(),
        ),
        (
            Some(SinkType::Sqlite),
            ["products".to_owned(), "reviews".to_owned()].as_slice(),
            WriteMode::Append,
            Some("env:SINK_SQLITE_PATH"),
            Some("records"),
        )
    );
}

/// A destination is a Secret reference, never the DSN itself, and the
/// refusal never repeats what it was given (ADR 0015).
///
/// Covers: setting:sinks
#[test]
fn a_literal_destination_is_refused_without_repeating_it() {
    let entry = json!({
        "type": "postgres",
        "datasets": ["products"],
        "destination": "postgres://crawler:hunter2@db.example/warehouse",
    });

    let refused = (SINK_SHAPE.check)(&entry).unwrap_err();

    assert!(refused.contains("destination"), "{refused}");
    assert!(!refused.contains("hunter2"), "{refused}");
}

/// A webhook's signing secret is a Secret reference too.
///
/// Covers: setting:sinks
#[test]
fn a_literal_signing_secret_is_refused_without_repeating_it() {
    let entry = json!({
        "type": "webhook",
        "datasets": ["products"],
        "destination": "env:HOOK_URL",
        "webhook": { "signing_secret": "whsec_hunter2" },
    });

    let refused = (SINK_SHAPE.check)(&entry).unwrap_err();

    assert!(refused.contains("signing_secret"), "{refused}");
    assert!(!refused.contains("hunter2"), "{refused}");
}

/// Covers: setting:sinks
#[rstest::rstest]
#[case::no_datasets(json!({ "type": "sqlite", "destination": "env:PATH_TO_DB" }))]
#[case::empty_datasets(json!({ "type": "sqlite", "datasets": [], "destination": "env:PATH_TO_DB" }))]
#[case::unknown_key(json!({ "type": "sqlite", "datasets": ["a"], "destination": "env:PATH_TO_DB", "table": "a" }))]
#[case::unknown_type(json!({ "type": "oracle", "datasets": ["a"], "destination": "env:PATH_TO_DB" }))]
#[case::unknown_mode(json!({ "type": "sqlite", "datasets": ["a"], "destination": "env:PATH_TO_DB", "mode": "merge" }))]
#[case::no_destination(json!({ "type": "sqlite", "datasets": ["a"] }))]
fn an_entry_outside_the_shape_is_refused(#[case] entry: serde_json::Value) {
    assert!((SINK_SHAPE.check)(&entry).is_err(), "{entry}");
}

/// A program's own Sink handler serves the entry of its name, with no
/// `type` and nothing to reach (ADR 0104).
///
/// Covers: setting:sinks
#[test]
fn a_programs_own_sink_takes_neither_type_nor_destination() {
    let entry = json!({ "datasets": ["products"], "mode": "upsert", "key": ["sku"] });

    (SINK_SHAPE.check)(&entry).unwrap();
}

/// Covers: setting:sinks
#[test]
fn every_example_is_of_the_shape() {
    let refused: Vec<String> = (SINK_SHAPE.examples)()
        .iter()
        .filter_map(|example| (SINK_SHAPE.check)(example).err())
        .collect();

    assert_eq!(refused, Vec::<String>::new());
}
