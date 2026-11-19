use serde_json::{Map, Value, json};

use super::{TimePath, Times, times, with_date_types};

/// A definitions map from a JSON object.
fn definitions(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        _ => unreachable!("the test gives an object"),
    }
}

fn reference(name: &str) -> Value {
    json!({ "$ref": format!("#/$defs/{name}") })
}

fn point_in_time() -> Value {
    json!({ "type": "string", "format": "date-time" })
}

fn fields(entries: &[(&str, TimePath)]) -> TimePath {
    TimePath::Fields(
        entries
            .iter()
            .map(|(key, path)| ((*key).to_owned(), path.clone()))
            .collect(),
    )
}

fn named(name: &str) -> TimePath {
    TimePath::Type(name.to_owned())
}

/// The table of every type of `defs`, with `results` the operations' result types.
fn table_of(defs: &Map<String, Value>, results: &[(&str, &str)]) -> anyhow::Result<Times> {
    times(defs, |_| true, results.iter().copied())
}

#[test]
fn a_reference_to_a_point_in_time_type_is_a_time_and_the_type_itself_is_left_out() {
    let defs = definitions(json!({
        "PointInTime": point_in_time(),
        "Report": { "type": "object", "properties": {
            "started_at": reference("PointInTime"),
            "name": { "type": "string" },
        } },
    }));

    let table = table_of(
        &defs,
        &[("run.finish", "Report"), ("run.name", "PointInTime")],
    )
    .unwrap();

    assert_eq!(
        table.types.into_iter().collect::<Vec<_>>(),
        [(
            "Report".to_owned(),
            fields(&[("started_at", TimePath::Time)])
        )]
    );
    assert_eq!(
        table.results.into_iter().collect::<Vec<_>>(),
        [
            ("run.finish".to_owned(), named("Report")),
            ("run.name".to_owned(), TimePath::Time),
        ]
    );
}

#[test]
fn nested_types_lists_maps_and_nullable_times_keep_their_place() {
    let defs = definitions(json!({
        "PointInTime": point_in_time(),
        "Usage": { "type": "object", "properties": {
            "resets_at": { "anyOf": [reference("PointInTime"), { "type": "null" }] },
        } },
        "Listing": { "type": "object", "properties": {
            "rows": { "type": "array", "items": reference("Usage") },
            "by_kind": { "type": "object", "additionalProperties": reference("Usage") },
            "seen": { "type": ["string", "null"], "format": "date-time" },
            "count": { "type": "integer" },
        } },
        "Unrelated": { "type": "object", "properties": { "name": { "type": "string" } } },
    }));

    let table = table_of(
        &defs,
        &[("listing.get", "Listing"), ("other.get", "Unrelated")],
    )
    .unwrap();

    assert_eq!(
        table.types.get("Listing"),
        Some(&fields(&[
            ("by_kind", TimePath::Values(Box::new(named("Usage")))),
            ("rows", TimePath::Items(Box::new(named("Usage")))),
            ("seen", TimePath::Time),
        ]))
    );
    assert_eq!(
        table.types.get("Usage"),
        Some(&fields(&[("resets_at", TimePath::Time)]))
    );
    assert!(!table.types.contains_key("Unrelated"));
    assert_eq!(table.results.keys().collect::<Vec<_>>(), ["listing.get"]);
}

#[test]
fn the_variants_of_a_union_merge_their_fields() {
    let defs = definitions(json!({
        "PointInTime": point_in_time(),
        "Effect": { "oneOf": [
            { "type": "object", "properties": {
                "kind": { "const": "opened" }, "opened_at": reference("PointInTime"),
            } },
            { "type": "object", "properties": {
                "kind": { "const": "closed" }, "closed_at": reference("PointInTime"),
            } },
        ] },
    }));

    let table = table_of(&defs, &[]).unwrap();

    assert_eq!(
        table.types.get("Effect"),
        Some(&fields(&[
            ("closed_at", TimePath::Time),
            ("opened_at", TimePath::Time)
        ]))
    );
}

#[test]
fn a_field_that_is_a_time_in_one_variant_and_not_in_another_is_refused() {
    let defs = definitions(json!({
        "PointInTime": point_in_time(),
        "Effect": { "oneOf": [
            { "type": "object", "properties": { "at": reference("PointInTime") } },
            { "type": "object", "properties": { "at": { "type": "string" } } },
        ] },
    }));

    let error = table_of(&defs, &[]).unwrap_err();

    assert!(format!("{error:#}").contains("Effect"), "{error:#}");
}

#[test]
fn a_time_under_a_keyword_the_table_cannot_follow_is_refused() {
    let defs = definitions(json!({
        "PointInTime": point_in_time(),
        "Pair": { "type": "array", "prefixItems": [reference("PointInTime")] },
    }));

    let error = table_of(&defs, &[]).unwrap_err();

    assert!(format!("{error:#}").contains("Pair"), "{error:#}");
}

#[test]
fn only_the_types_the_layer_owns_are_listed_and_references_to_others_stay_named() {
    let defs = definitions(json!({
        "PointInTime": point_in_time(),
        "Usage": { "type": "object", "properties": { "resets_at": reference("PointInTime") } },
        "CrawlReport": { "type": "object", "properties": { "usage": reference("Usage") } },
    }));

    let table = times(
        &defs,
        |name| name == "CrawlReport",
        [("crawl.serve", "CrawlReport")],
    )
    .unwrap();

    assert_eq!(
        table.types.into_iter().collect::<Vec<_>>(),
        [(
            "CrawlReport".to_owned(),
            fields(&[("usage", named("Usage"))])
        )]
    );
}

#[test]
fn every_point_in_time_is_typed_as_a_date() {
    let mut input = json!({ "$defs": {
        "PointInTime": { "type": "string", "format": "date-time", "description": "A point in time." },
        "Seen": { "type": "object", "properties": {
            "at": { "type": ["string", "null"], "format": "date-time" },
            "name": { "type": "string" },
        } },
    } });

    with_date_types(&mut input);

    assert_eq!(input["$defs"]["PointInTime"]["tsType"], "Date");
    assert_eq!(
        input["$defs"]["Seen"]["properties"]["at"]["tsType"],
        "Date | null"
    );
    assert_eq!(
        input["$defs"]["Seen"]["properties"]["name"].get("tsType"),
        None
    );
}
