use serde_json::{Map, Value, json};

use super::{Field, Wire, WirePath, camel_fields, wire, with_date_types};

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

/// A field of a path: its wire name, its TypeScript name and its value's path.
fn field(wire: &str, name: &str, path: Option<WirePath>) -> (String, Field) {
    (
        wire.to_owned(),
        Field {
            name: name.to_owned(),
            path,
        },
    )
}

fn fields(entries: Vec<(String, Field)>) -> WirePath {
    WirePath::Fields(entries.into_iter().collect())
}

fn named(name: &str) -> WirePath {
    WirePath::Type(name.to_owned())
}

/// The table of every type of `defs`, with `results` each operation's
/// result type and no params, nor events.
fn table_of(defs: &Map<String, Value>, results: &[(&str, &str)]) -> anyhow::Result<Wire> {
    wire(
        defs,
        |_| true,
        results
            .iter()
            .map(|(operation, result)| (*operation, "", *result)),
        [],
    )
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
            fields(vec![field("started_at", "startedAt", Some(WirePath::Time))])
        )]
    );
    assert_eq!(
        table.results.into_iter().collect::<Vec<_>>(),
        [
            ("run.finish".to_owned(), named("Report")),
            ("run.name".to_owned(), WirePath::Time),
        ]
    );
}

#[test]
fn a_field_of_several_words_is_named_by_the_initialism_rule_and_a_one_word_field_is_left_out() {
    let defs = definitions(json!({
        "Navigated": { "type": "object", "properties": {
            "final_url": { "type": "string" },
            "status": { "type": "integer" },
            "page_id": { "type": "string" },
        } },
        "Plain": { "type": "object", "properties": { "name": { "type": "string" } } },
    }));

    let table = table_of(&defs, &[("page.goto", "Navigated"), ("plain.get", "Plain")]).unwrap();

    assert_eq!(
        table.types.into_iter().collect::<Vec<_>>(),
        [(
            "Navigated".to_owned(),
            fields(vec![
                field("final_url", "finalURL", None),
                field("page_id", "pageId", None),
            ])
        )]
    );
    assert_eq!(table.results.keys().collect::<Vec<_>>(), ["page.goto"]);
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
        Some(&fields(vec![
            field(
                "by_kind",
                "byKind",
                Some(WirePath::Values(Box::new(named("Usage"))))
            ),
            field(
                "rows",
                "rows",
                Some(WirePath::Items(Box::new(named("Usage"))))
            ),
            field("seen", "seen", Some(WirePath::Time)),
        ]))
    );
    assert_eq!(
        table.types.get("Usage"),
        Some(&fields(vec![field(
            "resets_at",
            "resetsAt",
            Some(WirePath::Time)
        )]))
    );
    assert!(!table.types.contains_key("Unrelated"));
    assert_eq!(table.results.keys().collect::<Vec<_>>(), ["listing.get"]);
}

#[test]
fn the_keys_of_a_map_and_of_a_free_value_are_data_and_never_named() {
    let defs = definitions(json!({
        "Extracted": { "type": "object", "properties": {
            "headers": { "type": "object", "additionalProperties": { "type": "string" } },
            "record": { "type": "object" },
            "value": {},
            "by_name": { "type": "object", "additionalProperties": true },
        } },
    }));

    let table = table_of(&defs, &[("page.extract", "Extracted")]).unwrap();

    assert_eq!(
        table.types.get("Extracted"),
        Some(&fields(vec![field("by_name", "byName", None)]))
    );
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
                "closed_by": { "type": "string" },
            } },
        ] },
    }));

    let table = table_of(&defs, &[]).unwrap();

    assert_eq!(
        table.types.get("Effect"),
        Some(&fields(vec![
            field("closed_at", "closedAt", Some(WirePath::Time)),
            field("closed_by", "closedBy", None),
            field("opened_at", "openedAt", Some(WirePath::Time)),
        ]))
    );
}

#[test]
fn an_open_union_names_its_tag_and_known_members_and_is_listed_even_when_it_holds_nothing_else() {
    let defs = definitions(json!({
        "PointInTime": point_in_time(),
        "Ending": { "open": true, "oneOf": [
            { "type": "object", "properties": {
                "kind": { "const": "failed" }, "ended_at": reference("PointInTime"),
            } },
            { "type": "object", "properties": { "kind": { "const": "skipped" } } },
        ] },
        "Stop": { "open": true, "oneOf": [
            { "type": "object", "properties": { "kind": { "const": "pause" } } },
        ] },
        "Report": { "type": "object", "properties": { "stop": reference("Stop") } },
    }));

    let table = table_of(&defs, &[("run.report", "Report")]).unwrap();

    let union = |known: &[&str], path: Option<WirePath>| WirePath::Union {
        tag: "kind".to_owned(),
        known: known.iter().map(|name| (*name).to_owned()).collect(),
        path: path.map(Box::new),
    };
    assert_eq!(
        table.types.into_iter().collect::<Vec<_>>(),
        [
            (
                "Ending".to_owned(),
                union(
                    &["failed", "skipped"],
                    Some(fields(vec![field(
                        "ended_at",
                        "endedAt",
                        Some(WirePath::Time)
                    )]))
                )
            ),
            (
                "Report".to_owned(),
                fields(vec![field("stop", "stop", Some(named("Stop")))])
            ),
            ("Stop".to_owned(), union(&["pause"], None)),
        ]
    );
    assert_eq!(table.results.get("run.report"), Some(&named("Report")));
}

#[test]
fn an_open_union_is_written_as_the_runtimes_union_path() {
    let path = WirePath::Union {
        tag: "kind".to_owned(),
        known: vec!["pause".to_owned()],
        path: Some(Box::new(named("Pause"))),
    };

    assert_eq!(
        super::path_value(&path),
        r#"{ union: { tag: "kind", known: ["pause"], path: { type: "Pause" } } }"#
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
fn declared_fields_beside_a_variant_whose_keys_are_data_are_refused() {
    let defs = definitions(json!({
        "Value": { "anyOf": [
            { "type": "object", "properties": { "page_id": { "type": "string" } } },
            { "type": "object", "additionalProperties": { "type": "string" } },
        ] },
    }));

    let error = table_of(&defs, &[]).unwrap_err();

    assert!(format!("{error:#}").contains("Value"), "{error:#}");
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

    let table = wire(
        &defs,
        |name| name == "CrawlReport",
        [("crawl.serve", "", "CrawlReport")],
        [],
    )
    .unwrap();

    assert_eq!(
        table.types.into_iter().collect::<Vec<_>>(),
        [(
            "CrawlReport".to_owned(),
            fields(vec![field("usage", "usage", Some(named("Usage")))])
        )]
    );
}

#[test]
fn a_type_that_holds_a_field_to_rename_where_the_table_does_not_look_is_refused() {
    let defs = definitions(json!({
        "Odd": { "type": "object", "x-shape": { "properties": { "page_id": { "type": "string" } } } },
    }));

    let error = table_of(&defs, &[]).unwrap_err();

    assert!(format!("{error:#}").contains("Odd"), "{error:#}");
}

#[test]
fn params_results_and_event_payloads_are_listed_by_operation_and_event() {
    let defs = definitions(json!({
        "GotoParams": { "type": "object", "properties": { "page_id": { "type": "string" } } },
        "Navigated": { "type": "object", "properties": { "url": { "type": "string" } } },
        "ConsoleMessage": { "type": "object", "properties": { "page_id": { "type": "string" } } },
    }));

    let table = wire(
        &defs,
        |_| true,
        [("page.goto", "GotoParams", "Navigated")],
        [("page.console", "ConsoleMessage")],
    )
    .unwrap();

    assert_eq!(
        table.params.into_iter().collect::<Vec<_>>(),
        [("page.goto".to_owned(), named("GotoParams"))]
    );
    assert!(table.results.is_empty());
    assert_eq!(
        table.events.into_iter().collect::<Vec<_>>(),
        [("page.console".to_owned(), named("ConsoleMessage"))]
    );
}

#[test]
fn the_generator_input_names_each_declared_field_in_camel_case_and_keeps_values() {
    let mut input = json!({ "$defs": {
        "Navigated": {
            "type": "object",
            "required": ["final_url", "kind"],
            "properties": {
                "final_url": { "type": "string", "default": { "keep_me": 1 } },
                "kind": { "const": "page_loaded" },
                "properties": {
                    "type": "object",
                    "properties": { "item_type": { "enum": ["a_b"] } },
                    "additionalProperties": { "type": "string" },
                },
            },
        },
    } });

    camel_fields(&mut input).unwrap();

    assert_eq!(
        input,
        json!({ "$defs": {
            "Navigated": {
                "type": "object",
                "required": ["finalURL", "kind"],
                "properties": {
                    "finalURL": { "type": "string", "default": { "keep_me": 1 } },
                    "kind": { "const": "page_loaded" },
                    "properties": {
                        "type": "object",
                        "properties": { "itemType": { "enum": ["a_b"] } },
                        "additionalProperties": { "type": "string" },
                    },
                },
            },
        } })
    );
}

#[test]
fn two_fields_that_spell_one_name_are_refused() {
    let mut input = json!({ "$defs": {
        "Pair": { "type": "object", "properties": {
            "foo_bar": { "type": "string" },
            "fooBar": { "type": "string" },
        } },
    } });

    let error = camel_fields(&mut input).unwrap_err();

    assert!(format!("{error:#}").contains("fooBar"), "{error:#}");
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
