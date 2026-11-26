//! The canonical body hash as the pure function it is: a JSON body's key
//! order and a form's field order never change it, a value or a name's
//! value order does, a body with no type is read by its bytes, and a body
//! that is not what its type says is hashed as it is.

use proptest::prelude::*;
use rstest::rstest;
use sha2::{Digest as _, Sha256};

use super::body_sha256;

const JSON: Option<&str> = Some("application/json");
const FORM: Option<&str> = Some("application/x-www-form-urlencoded");

/// A JSON object of `fields`, written in their order.
fn object(fields: &[(String, i64)]) -> String {
    let members: Vec<String> = fields
        .iter()
        .map(|(key, value)| format!("{}:{value}", serde_json::Value::from(key.as_str())))
        .collect();
    format!("{{{}}}", members.join(","))
}

/// A form of `fields`, written in their order.
fn form(fields: &[(String, String)]) -> String {
    url::form_urlencoded::Serializer::new(String::new())
        .extend_pairs(fields)
        .finish()
}

/// Distinct names, each with one to three values.
fn named_values() -> impl Strategy<Value = Vec<(String, Vec<String>)>> {
    prop::collection::btree_map(
        "[a-z]{1,4}",
        prop::collection::vec("[a-z0-9 &=]{0,4}", 1..=3),
        1..=5,
    )
    .prop_map(|names| names.into_iter().collect())
}

/// `names`' fields in an order `picks` gives, which takes each name's next
/// value in turn: any interleaving of names, each name's values in order.
fn interleaved(names: &[(String, Vec<String>)], picks: &[usize]) -> Vec<(String, String)> {
    let mut next = vec![0; names.len()];
    picks
        .iter()
        .map(|&name| {
            let (key, values) = &names[name];
            next[name] += 1;
            (key.clone(), values[next[name] - 1].clone())
        })
        .collect()
}

/// Each name's index once per value it has, for `prop_shuffle`.
fn picks(names: &[(String, Vec<String>)]) -> Vec<usize> {
    names
        .iter()
        .enumerate()
        .flat_map(|(name, (_, values))| std::iter::repeat_n(name, values.len()))
        .collect()
}

proptest! {
    /// Covers: story:6-36
    #[test]
    fn a_json_objects_key_order_never_changes_its_hash(
        fields in prop::collection::btree_map("[a-zA-Z_]{1,6}", any::<i64>(), 1..8)
            .prop_map(|fields| fields.into_iter().collect::<Vec<_>>())
            .prop_flat_map(|fields| (Just(fields.clone()), Just(fields).prop_shuffle())),
    ) {
        let (sorted, shuffled) = fields;
        prop_assert_eq!(
            body_sha256(JSON, object(&sorted).as_bytes()),
            body_sha256(JSON, object(&shuffled).as_bytes())
        );
    }

    /// Covers: story:6-36
    #[test]
    fn a_forms_field_order_never_changes_its_hash_but_its_values_order_is_kept(
        (names, shuffled) in named_values()
            .prop_flat_map(|names| { let picks = picks(&names); (Just(names), Just(picks).prop_shuffle()) }),
    ) {
        let grouped = interleaved(&names, &picks(&names));
        let mixed = interleaved(&names, &shuffled);
        prop_assert_eq!(
            body_sha256(FORM, form(&grouped).as_bytes()),
            body_sha256(FORM, form(&mixed).as_bytes())
        );
    }
}

/// A body whose values differ, or whose repeated name's values come in
/// another order, is another body.
/// Covers: story:6-36
#[rstest]
#[case::json_value(JSON, r#"{"q":"shoes","page":1}"#, r#"{"page":2,"q":"shoes"}"#)]
#[case::json_array_order(JSON, r#"{"ids":[1,2]}"#, r#"{"ids":[2,1]}"#)]
#[case::form_value_order(FORM, "tag=a&q=x&tag=b", "tag=b&q=x&tag=a")]
#[case::raw_bytes(Some("text/plain"), "a=1&b=2", "b=2&a=1")]
fn another_body_hashes_otherwise(
    #[case] content_type: Option<&str>,
    #[case] one: &str,
    #[case] other: &str,
) {
    assert_ne!(
        body_sha256(content_type, one.as_bytes()),
        body_sha256(content_type, other.as_bytes())
    );
}

/// A JSON media type is read with its parameters and suffix, and nested
/// objects are sorted too.
#[rstest]
#[case::charset(Some("Application/JSON; charset=utf-8"))]
#[case::suffix(Some("application/vnd.api+json"))]
fn a_json_type_with_parameters_or_a_suffix_is_canonical(#[case] content_type: Option<&str>) {
    assert_eq!(
        body_sha256(content_type, br#"{"b":{"y":1,"x":2},"a":0}"#),
        body_sha256(JSON, br#"{"a":0,"b":{"x":2,"y":1}}"#)
    );
}

/// A body sent with no type, as a crawl request's is, is read by its bytes:
/// JSON and a form are canonical as when typed.
/// Covers: story:6-36
#[rstest]
#[case::json(JSON, r#"{"b":[1,2],"a":"x"}"#, r#" {"a":"x","b":[1,2]}"#)]
#[case::form(FORM, "q=shoes&tag=a&page=1&tag=b", "page=1&tag=a&q=shoes&tag=b")]
#[case::encoded_form(FORM, "q=red+shoes&to=%2Fa", "to=%2Fa&q=red+shoes")]
fn an_untyped_body_is_read_by_its_bytes(
    #[case] content_type: Option<&str>,
    #[case] typed: &str,
    #[case] untyped: &str,
) {
    assert_eq!(
        body_sha256(content_type, typed.as_bytes()),
        body_sha256(None, untyped.as_bytes())
    );
}

/// A body that is not what its type says, or of no type and neither JSON
/// nor a form, is hashed as it is.
#[rstest]
#[case::not_json(JSON, b"{not json")]
#[case::text(None, b"b=2 a=1")]
#[case::binary(None, &[0x80, b'=', 0x01])]
fn a_body_of_another_kind_is_hashed_as_it_is(
    #[case] content_type: Option<&str>,
    #[case] body: &[u8],
) {
    assert_eq!(
        body_sha256(content_type, body),
        body_sha256(Some("application/octet-stream"), body)
    );
}

/// A request with no body has the SHA-256 of nothing.
#[test]
fn no_body_hashes_to_nothing() {
    assert_eq!(
        body_sha256(FORM, b""),
        data_encoding::HEXLOWER.encode(&Sha256::digest(b""))
    );
}
