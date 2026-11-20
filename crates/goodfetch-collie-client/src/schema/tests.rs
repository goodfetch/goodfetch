//! The Schema adapter (ADR 0082): a `schemars` type and a JSON Schema
//! written by hand each give the engine their wire schema, encode a value as
//! it parses back and decode an answer to their type; only JSON crosses, and
//! what is not is named by its JSON Pointer, while a program's own keys,
//! `__proto__` included, cross untouched.
//! Covers: story:11-66, story:11-68, story:11-69

use std::collections::BTreeMap;

use proptest::prelude::*;
use rstest::rstest;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::{Schema, SchemaError};

#[derive(Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
struct Book {
    #[serde(rename = "bookTitle")]
    title: String,
    #[serde(default)]
    pages: u32,
}

#[derive(Debug, PartialEq, JsonSchema, Serialize, Deserialize)]
enum Shape {
    Circle { radius: f64 },
    Segment(f64, f64),
}

#[derive(Debug, JsonSchema, Serialize, Deserialize)]
struct Noted {
    title: String,
    #[serde(skip_deserializing)]
    note: String,
}

#[test]
fn a_derived_types_wire_schema_is_its_deserialize_side_by_its_wire_names() {
    let wire = Schema::<Book>::derived().wire_schema().clone();

    assert_eq!(
        (&wire["properties"]["bookTitle"]["type"], &wire["required"]),
        (&json!("string"), &json!(["bookTitle"]))
    );
}

#[test]
fn a_hand_written_json_schema_is_the_wire_schema_as_written() {
    let document = json!({"type": "object", "required": ["title"]});

    let schema = Schema::json(document.clone()).unwrap();

    assert_eq!(schema.wire_schema(), &document);
}

#[rstest]
#[case::a_number(json!(3), "a number")]
#[case::an_array(json!([]), "an array")]
#[case::null(Value::Null, "null")]
fn a_hand_written_schema_that_is_not_one_is_refused(#[case] document: Value, #[case] found: &str) {
    assert_eq!(
        Schema::json(document).unwrap_err(),
        SchemaError::NotASchema {
            found: found.to_owned()
        }
    );
}

#[test]
fn a_derived_schema_encodes_a_value_as_the_json_it_parses_back() {
    let book = Book {
        title: "Dune".to_owned(),
        pages: 412,
    };

    let encoded = Schema::<Book>::derived().encode(&book).unwrap();

    assert_eq!(encoded, json!({"bookTitle": "Dune", "pages": 412}));
}

#[test]
fn a_derived_schema_decodes_an_answer_to_its_type() {
    let read = Schema::<Book>::derived()
        .decode(json!({"bookTitle": "Dune"}))
        .unwrap();

    assert_eq!(
        read,
        Book {
            title: "Dune".to_owned(),
            pages: 0
        }
    );
}

#[test]
fn a_hand_written_schema_decodes_an_answer_to_its_json() {
    let answer = json!({"title": "Dune", "tags": ["sf"]});

    let read = Schema::json(json!(true)).unwrap().decode(answer.clone());

    assert_eq!(read, Ok(answer));
}

#[test]
fn an_answer_the_type_does_not_read_is_rejected() {
    let read = Schema::<Book>::derived().decode(json!({"bookTitle": 3}));

    assert!(
        matches!(&read, Err(SchemaError::Rejected { .. })),
        "{read:?}"
    );
}

#[test]
fn a_field_that_does_not_parse_back_is_refused_before_sending() {
    let noted = Noted {
        title: "Dune".to_owned(),
        note: "read twice".to_owned(),
    };

    let refused = Schema::<Noted>::derived().encode(&noted).unwrap_err();

    assert_eq!(
        refused,
        SchemaError::ChangesOnReadBack {
            path: "/note".to_owned(),
            read_as: json!("")
        }
    );
}

fn not_json<T: JsonSchema + Serialize + for<'de> Deserialize<'de>>(value: &T) -> (String, String) {
    match Schema::<T>::derived().encode(value) {
        Err(SchemaError::NotJson { path, found }) => (path, found),
        other => panic!("expected a value that is not JSON, got {other:?}"),
    }
}

#[test]
fn a_nan_in_a_list_is_named_by_its_index() {
    assert_eq!(
        not_json(&vec![1.5, f64::NAN]),
        ("/1".to_owned(), "NaN".to_owned())
    );
}

#[rstest]
#[case::infinity(f64::INFINITY, "infinity")]
#[case::negative_infinity(f64::NEG_INFINITY, "-infinity")]
fn an_infinity_in_a_variants_field_is_named_by_its_path(#[case] radius: f64, #[case] found: &str) {
    assert_eq!(
        not_json(&Shape::Circle { radius }),
        ("/Circle/radius".to_owned(), found.to_owned())
    );
}

#[test]
fn a_nan_in_a_tuple_variant_is_named_by_its_index() {
    assert_eq!(
        not_json(&Shape::Segment(0.0, f64::NAN)),
        ("/Segment/1".to_owned(), "NaN".to_owned())
    );
}

#[test]
fn a_key_that_is_not_a_string_is_named_by_its_maps_path() {
    let counts = BTreeMap::from([("pages".to_owned(), BTreeMap::from([(1_u32, true)]))]);

    assert_eq!(
        not_json(&counts),
        ("/pages".to_owned(), "the key 1, not a string".to_owned())
    );
}

#[test]
fn an_integer_wider_than_json_carries_is_refused() {
    assert_eq!(
        not_json(&vec![u128::MAX]),
        (
            "/0".to_owned(),
            format!(
                "the integer {}, wider than JSON's 64-bit numbers",
                u128::MAX
            )
        )
    );
}

#[test]
fn a_value_not_json_at_the_root_is_named_by_the_empty_pointer() {
    assert_eq!(not_json(&f64::NAN), (String::new(), "NaN".to_owned()));
}

#[test]
fn a_key_a_json_pointer_escapes_is_named_escaped() {
    let prices = BTreeMap::from([("a~1b/c".to_owned(), vec![f64::NAN])]);

    assert_eq!(
        not_json(&prices),
        ("/a~01b~1c/0".to_owned(), "NaN".to_owned())
    );
}

/// A program's keys, the ones a JavaScript object treats apart and the
/// ones a JSON Pointer escapes among them.
fn key() -> impl Strategy<Value = String> {
    prop_oneof![
        Just("__proto__".to_owned()),
        Just("constructor".to_owned()),
        Just("prototype".to_owned()),
        Just("a~1b/c".to_owned()),
        Just(String::new()),
        ".{0,8}",
    ]
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<f64>()
            .prop_filter("JSON carries finite numbers", |number| number.is_finite())
            .prop_map(Value::from),
        ".{0,8}".prop_map(Value::from),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::from),
            prop::collection::btree_map(key(), inner, 0..6)
                .prop_map(|members| Value::Object(members.into_iter().collect())),
        ]
    })
}

/// A JSON Pointer's step for `key` (RFC 6901), written apart from the
/// encoder's.
fn escaped(key: &str) -> String {
    key.replace('~', "~0").replace('/', "~1")
}

proptest! {
    #[test]
    fn any_json_value_crosses_unchanged_under_a_hand_written_schema(value in json_value()) {
        let schema = Schema::json(json!({})).unwrap();

        prop_assert_eq!(schema.encode(&value), Ok(value.clone()));
        prop_assert_eq!(schema.decode(value.clone()), Ok(value));
    }

    #[test]
    fn a_programs_keys_cross_untouched_in_its_own_type(
        members in prop::collection::btree_map(key(), json_value(), 0..6),
    ) {
        let schema = Schema::<BTreeMap<String, Value>>::derived();

        let encoded = schema.encode(&members).unwrap();

        prop_assert_eq!(schema.decode(encoded), Ok(members));
    }

    #[test]
    fn a_nan_planted_under_any_key_is_named_by_it(
        mut members in prop::collection::btree_map(key(), prop::collection::vec(-1e9..1e9_f64, 1..4), 1..4),
        pick in any::<prop::sample::Index>(),
        at in any::<prop::sample::Index>(),
    ) {
        let key = pick.get(&members.keys().cloned().collect::<Vec<_>>()).clone();
        let numbers = members.get_mut(&key).expect("invariant: the key was picked from the map");
        let index = at.index(numbers.len());
        numbers[index] = f64::NAN;

        prop_assert_eq!(
            not_json(&members),
            (format!("/{}/{index}", escaped(&key)), "NaN".to_owned())
        );
    }
}
