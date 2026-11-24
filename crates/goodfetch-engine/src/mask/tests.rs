//! The run's `Mask` through its interface (spec #564's seam): every form
//! of a value replaced by its `%name%` in each scope, the short-value rule,
//! the redaction switch, a Run's end and JSON messages.
//! Covers: story:4-56

use goodfetch_test_support::{Canaries, Canary, Output, SecretKind};
use proptest::prelude::*;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;
use serde_json::value::RawValue;

use super::{Mask, Masks, Reach, Redaction, Scope};

/// A value of fourteen characters that JSON escaping, URL encoding and
/// both base64 alphabets each change.
const VALUE: &str = "pa ss\"w/rd>?9~";

const NAME: &str = "proxy.password";

const EVERY_SCOPE: [Scope; 3] = [Scope::Returned, Scope::Kept, Scope::KeptOutput];

fn joined(value: &str, reach: Reach, redaction: Redaction) -> (Masks, Mask) {
    joined_as(NAME, value, reach, redaction)
}

fn joined_as(name: &str, value: &str, reach: Reach, redaction: Redaction) -> (Masks, Mask) {
    let masks = Masks::new();
    let mask = masks.for_run(redaction);
    mask.join(name, &SecretString::from(value), reach);
    (masks, mask)
}

/// A name no value of the property tests can hide in: shorter than any.
const SHORT_NAME: &str = "n";

fn in_every_scope(masks: &Masks, text: &str) -> [String; 3] {
    EVERY_SCOPE.map(|scope| masks.apply(text, scope).into_owned())
}

#[rstest]
#[case::raw("a pa ss\"w/rd>?9~ b", "a %proxy.password% b")]
#[case::json_escaped(r#"{"p":"pa ss\"w/rd>?9~"}"#, r#"{"p":"%proxy.password%"}"#)]
#[case::url_encoded_with_percent_20("?p=pa%20ss%22w%2Frd%3E%3F9~&q=1", "?p=%proxy.password%&q=1")]
#[case::url_encoded_with_plus("p=pa+ss%22w%2Frd%3E%3F9~", "p=%proxy.password%")]
#[case::form_urlencoded("u=jb&p=pa+ss%22w%2Frd%3E%3F9%7E", "u=jb&p=%proxy.password%")]
#[case::base64("cGEgc3Midy9yZD4/OX4=", "%proxy.password%4=")]
#[case::base64url("cGEgc3Midy9yZD4_OX4=", "%proxy.password%4=")]
#[case::base64_one_byte_into_a_group("eHBhIHNzIncvcmQ+Pzl+eg==", "eH%proxy.password%eg==")]
#[case::basic_proxy_authorization(
    "Proxy-Authorization: Basic dXNlcjpwYSBzcyJ3L3JkPj85fg==",
    "Proxy-Authorization: Basic dXNlcjp%proxy.password%g=="
)]
#[case::hex_lowercase("k=706120737322772f72643e3f397e", "k=%proxy.password%")]
#[case::hex_uppercase("k=706120737322772F72643E3F397E", "k=%proxy.password%")]
fn a_resolved_value_is_masked_as_its_name_in_every_form_and_scope(
    #[case] text: &str,
    #[case] masked: &str,
) {
    let (masks, _mask) = joined(VALUE, Reach::Value, Redaction::On);

    let applied = in_every_scope(&masks, text);

    assert_eq!(applied, [masked, masked, masked].map(str::to_owned));
}

/// Covers: story:4-57
#[test]
fn a_value_shorter_than_eight_characters_is_masked_in_what_the_engine_keeps_but_not_in_a_read() {
    let (masks, _mask) = joined("4815162", Reach::Value, Redaction::On);

    let applied = in_every_scope(&masks, "price 4815162");

    assert_eq!(
        applied,
        [
            "price 4815162",
            "price %proxy.password%",
            "price %proxy.password%"
        ]
        .map(str::to_owned)
    );
}

/// Covers: story:4-57
#[test]
fn a_value_of_eight_characters_is_masked_in_a_read() {
    let (masks, _mask) = joined("48151623", Reach::Value, Redaction::On);

    let applied = masks.apply("price 48151623", Scope::Returned);

    assert_eq!(applied, "price %proxy.password%");
}

/// Covers: story:4-57
#[test]
fn a_short_value_is_masked_in_a_read_of_the_field_it_was_typed_in() {
    let (masks, _mask) = joined("4815162", Reach::Value, Redaction::On);

    let applied = masks.apply_typed("value=4815162", &[NAME]);

    assert_eq!(applied, "value=%proxy.password%");
}

/// Covers: story:4-57
#[test]
fn a_one_time_code_counts_as_short_whatever_its_length() {
    let (masks, _mask) = joined("12345678", Reach::OneTimeCode, Redaction::On);

    let applied = in_every_scope(&masks, "id 12345678");

    assert_eq!(
        applied,
        ["id 12345678", "id %proxy.password%", "id %proxy.password%"].map(str::to_owned)
    );
}

/// Covers: story:4-76
#[test]
fn a_run_with_redaction_off_keeps_its_values_visible_in_kept_outputs_only() {
    let (masks, _mask) = joined(VALUE, Reach::Value, Redaction::Off);

    let applied = in_every_scope(&masks, "p=pa ss\"w/rd>?9~");

    assert_eq!(
        applied,
        [
            "p=%proxy.password%",
            "p=%proxy.password%",
            "p=pa ss\"w/rd>?9~"
        ]
        .map(str::to_owned)
    );
}

#[test]
fn a_runs_values_leave_the_mask_once_its_last_handle_drops() {
    let (masks, mask) = joined(VALUE, Reach::Value, Redaction::On);
    let clone = mask.clone();
    drop(mask);
    let while_a_clone_lives = masks.apply(VALUE, Scope::Kept).into_owned();

    drop(clone);

    assert_eq!(
        (
            while_a_clone_lives.as_str(),
            masks.apply(VALUE, Scope::Kept)
        ),
        ("%proxy.password%", VALUE.into())
    );
}

#[test]
fn each_runs_values_are_masked_by_their_own_names() {
    let masks = Masks::new();
    let first = masks.for_run(Redaction::On);
    let second = masks.for_run(Redaction::On);
    first.join(
        "password",
        &SecretString::from("correct horse"),
        Reach::Value,
    );
    second.join(NAME, &SecretString::from("battery staple"), Reach::Value);

    let applied = masks.apply("correct horse, battery staple", Scope::Returned);

    assert_eq!(applied, "%password%, %proxy.password%");
}

/// A value the Run's mask holds under its name already does not join it
/// again: of 1,000 joins of one value, as many Sessions' opens resolve one
/// proxy password, only the first adds it.
#[test]
fn a_value_the_run_holds_under_its_name_already_joins_its_mask_once() {
    let masks = Masks::new();
    let mask = masks.for_run(Redaction::On);
    let value = SecretString::from(VALUE);

    let joined: Vec<bool> = (0..1_000)
        .map(|_| mask.join(NAME, &value, Reach::Value))
        .collect();

    assert_eq!(
        (
            joined.iter().filter(|joined| **joined).count(),
            masks.apply(VALUE, Scope::Returned).into_owned()
        ),
        (1, "%proxy.password%".to_owned())
    );
}

/// The same value under another name, or in another Run, still joins.
#[rstest]
#[case::another_name("proxy.providers.one.password", false)]
#[case::another_run(NAME, true)]
fn a_value_joins_again_under_another_name_or_in_another_run(
    #[case] name: &str,
    #[case] other_run: bool,
) {
    let masks = Masks::new();
    let first = masks.for_run(Redaction::On);
    let second = masks.for_run(Redaction::On);
    let value = SecretString::from(VALUE);
    first.join(NAME, &value, Reach::Value);

    let joined = if other_run { &second } else { &first }.join(name, &value, Reach::Value);

    assert!(joined);
}

#[test]
fn every_string_of_a_json_value_is_masked_its_members_names_included() {
    let (masks, _mask) = joined(VALUE, Reach::Value, Redaction::On);
    let mut value = json!({ VALUE: ["x pa ss\"w/rd>?9~", 1, { "k": VALUE }] });

    masks.apply_value(&mut value, Scope::Returned);

    assert_eq!(
        value,
        json!({ "%proxy.password%": ["x %proxy.password%", 1, { "k": "%proxy.password%" }] })
    );
}

/// `Masks::json` writes what `apply_value` leaves, from the value as it
/// is: a member renamed takes the place its new name has among the others,
/// and of two members left with one name the later stays.
#[rstest]
#[case::a_string(json!("x pa ss\"w/rd>?9~"), r#""x %proxy.password%""#)]
#[case::a_members_name_and_what_it_holds(
    json!({ VALUE: ["x pa ss\"w/rd>?9~", 1, { "k": VALUE }] }),
    r#"{"%proxy.password%":["x %proxy.password%",1,{"k":"%proxy.password%"}]}"#
)]
#[case::no_value(
    json!({ "b": [null, true, 1.5, "plain"], "a": {} }),
    r#"{"a":{},"b":[null,true,1.5,"plain"]}"#
)]
#[case::a_renamed_member_moves_to_its_names_place(
    json!({ "a": 1, VALUE: 2 }),
    r#"{"%proxy.password%":2,"a":1}"#
)]
#[case::of_two_members_left_with_one_name_the_later_stays(
    json!({ "%proxy.password%": "first", VALUE: "second", "z": 0 }),
    r#"{"%proxy.password%":"second","z":0}"#
)]
fn a_json_values_masked_text_is_written_as_the_masked_value_encodes(
    #[case] value: serde_json::Value,
    #[case] text: &str,
) {
    let (masks, _mask) = joined(VALUE, Reach::Value, Redaction::On);
    let mut masked = value.clone();
    masks.apply_value(&mut masked, Scope::Kept);

    assert_eq!(
        (masks.json(&value, Scope::Kept), masked.to_string()),
        (text.to_owned(), text.to_owned())
    );
}

#[test]
fn a_json_values_text_is_its_own_while_no_value_is_masked() {
    let value = json!({ "b": ["pa ss\"w/rd>?9~", 1], "a": null });

    assert_eq!(
        Masks::new().json(&value, Scope::Kept),
        r#"{"a":null,"b":["pa ss\"w/rd>?9~",1]}"#
    );
}

/// `Masks::json_text` writes a JSON text as `Masks::json` writes its
/// value, but each object's members in the order written: a renamed member
/// keeps its place, and of two members of one name, written so or left so
/// by the mask, the first's place keeps the later's value.
#[rstest]
#[case::members_in_the_order_written(
    r#"{ "title": "Café", "price": 3, "author": { "name": "Zoé", "born": 1802 } }"#,
    r#"{"title":"Café","price":3,"author":{"name":"Zoé","born":1802}}"#
)]
#[case::a_string_and_a_members_name(
    r#"{"pa ss\"w/rd>?9~": ["x pa ss\"w/rd>?9~", 1.50, {"k": "pa ss\"w/rd>?9~"}]}"#,
    r#"{"%proxy.password%":["x %proxy.password%",1.5,{"k":"%proxy.password%"}]}"#
)]
#[case::a_renamed_member_keeps_its_place(
    r#"{"z": 1, "pa ss\"w/rd>?9~": 2, "a": 3}"#,
    r#"{"z":1,"%proxy.password%":2,"a":3}"#
)]
#[case::of_two_members_written_with_one_name_the_first_place_keeps_the_later_value(
    r#"{"b": "first", "a": 0, "b": "second"}"#,
    r#"{"b":"second","a":0}"#
)]
#[case::of_two_members_left_with_one_name_the_first_place_keeps_the_later_value(
    r#"{"%proxy.password%": "first", "z": 0, "pa ss\"w/rd>?9~": "second"}"#,
    r#"{"%proxy.password%":"second","z":0}"#
)]
fn a_json_texts_members_keep_the_order_written_once_masked(#[case] json: &str, #[case] text: &str) {
    let (masks, _mask) = joined(VALUE, Reach::Value, Redaction::On);
    let raw = RawValue::from_string(json.to_owned()).unwrap();

    assert_eq!(masks.json_text(&raw, Scope::Kept), text);
}

#[test]
fn a_json_texts_members_keep_the_order_written_while_no_value_is_masked() {
    let raw = RawValue::from_string(r#"{ "b": ["x", 1], "a": null }"#.to_owned()).unwrap();

    assert_eq!(
        Masks::new().json_text(&raw, Scope::Kept),
        r#"{"b":["x",1],"a":null}"#
    );
}

/// A Record nested past `serde_json`'s own limit of 128, as a code-free
/// page's microdata may be, is written whole (issue 2394).
#[test]
fn a_json_text_nested_past_serde_jsons_limit_is_written_whole() {
    let deep = format!("{}1{}", r#"{"a":["#.repeat(400), "]}".repeat(400));
    let raw = RawValue::from_string(deep.clone()).unwrap();

    assert_eq!(Masks::new().json_text(&raw, Scope::Kept), deep);
}

#[test]
fn a_json_message_holding_a_value_in_a_string_is_masked_as_text() {
    let (masks, _mask) = joined(VALUE, Reach::Value, Redaction::On);
    let line = br#"{"jsonrpc":"2.0","result":{"p":"pa ss\"w/rd>?9~"}}
"#
    .to_vec();

    let masked = masks.apply_json_line(line, Scope::Returned);

    assert_eq!(
        String::from_utf8(masked).expect("the line stays UTF-8"),
        "{\"jsonrpc\":\"2.0\",\"result\":{\"p\":\"%proxy.password%\"}}\n"
    );
}

#[test]
fn a_value_whose_raw_form_spans_a_json_messages_syntax_leaves_the_message_whole() {
    let (masks, _mask) = joined("5678,\"b\"", Reach::Value, Redaction::On);
    let line = b"{\"a\":12345678,\"b\":\"x\"}\n".to_vec();

    let masked = masks.apply_json_line(line, Scope::Returned);

    assert_eq!(
        String::from_utf8(masked).expect("the line stays UTF-8"),
        "{\"a\":12345678,\"b\":\"x\"}\n"
    );
}

/// One of Goodfetch's encodings of `value` inside a longer text, built by
/// the libraries the formatters use, never by the `Mask`.
#[derive(Clone, Copy, Debug)]
enum Form {
    Raw,
    Json,
    Percent,
    Plus,
    /// A form's body or a GET form's query, as the WHATWG URL Standard's
    /// `application/x-www-form-urlencoded` serializer writes it.
    Urlencoded,
    /// As JavaScript's `encodeURIComponent` writes it.
    UriComponent,
    Base64 {
        url: bool,
        before: usize,
    },
    Basic,
    HexLower,
    HexUpper,
}

fn form() -> impl Strategy<Value = Form> {
    prop_oneof![
        Just(Form::Raw),
        Just(Form::Json),
        Just(Form::Percent),
        Just(Form::Plus),
        Just(Form::Urlencoded),
        Just(Form::UriComponent),
        (any::<bool>(), 0..6_usize).prop_map(|(url, before)| Form::Base64 { url, before }),
        Just(Form::Basic),
        Just(Form::HexLower),
        Just(Form::HexUpper),
    ]
}

fn encoded(value: &str, form: Form) -> String {
    let percent = || {
        value
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-._~".contains(&byte) {
                    char::from(byte).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect::<String>()
    };
    match form {
        Form::Raw => value.to_owned(),
        Form::Json => serde_json::to_string(value).expect("a string encodes"),
        Form::Percent => percent(),
        Form::Plus => percent().replace("%20", "+"),
        Form::Urlencoded => url::form_urlencoded::byte_serialize(value.as_bytes()).collect(),
        Form::UriComponent => value
            .bytes()
            .map(|byte| {
                if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
                    char::from(byte).to_string()
                } else {
                    format!("%{byte:02X}")
                }
            })
            .collect(),
        Form::Base64 { url, before } => {
            let mut bytes = vec![b'x'; before];
            bytes.extend_from_slice(value.as_bytes());
            bytes.push(b'z');
            let alphabet = if url {
                &data_encoding::BASE64URL
            } else {
                &data_encoding::BASE64
            };
            alphabet.encode(&bytes)
        }
        Form::Basic => format!(
            "Basic {}",
            data_encoding::BASE64.encode(format!("user:{value}").as_bytes())
        ),
        Form::HexLower => data_encoding::HEXLOWER.encode(value.as_bytes()),
        Form::HexUpper => data_encoding::HEXUPPER.encode(value.as_bytes()),
    }
}

/// The canary search, an oracle of its own: no form of `value` it knows
/// (raw, JSON-escaped, URL-encoded, base64 at each alignment, hex) is in
/// `output`, and neither are the forms it does not search: the `+` form, a
/// form's `application/x-www-form-urlencoded` and `encodeURIComponent`'s.
fn leaks(value: &str, output: &[u8]) -> Vec<String> {
    let mut canaries = Canaries::new(0);
    canaries.add(Canary::known("value", SecretKind::ProxyPassword, value));
    let mut leaks: Vec<String> = canaries
        .leaks(&[Output::Bytes {
            name: "output",
            bytes: output,
        }])
        .expect("bytes always read")
        .into_iter()
        .map(|hit| format!("{hit:?}"))
        .collect();
    for form in [Form::Plus, Form::Urlencoded, Form::UriComponent] {
        let spelt = encoded(value, form);
        if output
            .windows(spelt.len())
            .any(|window| window == spelt.as_bytes())
        {
            leaks.push(format!("the {form:?} form"));
        }
    }
    leaks
}

/// A value of 8 to 24 characters: printable ASCII, with the characters the
/// encodings change, or any Unicode.
fn value() -> impl Strategy<Value = String> {
    prop_oneof!["[ -~]{8,24}", "\\PC{8,16}"]
}

/// Text around a value that holds none of it: no letter or digit.
fn around() -> impl Strategy<Value = String> {
    "[ :=,;\\[\\]{}()<>!|-]{0,12}"
}

proptest! {
    #[test]
    fn no_form_of_a_value_passes_any_scope(
        value in value(),
        form in form(),
        before in around(),
        after in around(),
    ) {
        let (masks, _mask) = joined_as(SHORT_NAME, &value, Reach::Value, Redaction::On);
        let text = format!("{before}{}{after}", encoded(&value, form));

        let applied = in_every_scope(&masks, &text);

        for masked in applied {
            prop_assert_eq!(leaks(&value, masked.as_bytes()), Vec::<String>::new());
        }
    }

    #[test]
    fn a_json_message_never_holds_a_value_in_any_string_once_masked(
        value in value(),
        before in around(),
    ) {
        let (masks, _mask) = joined_as(SHORT_NAME, &value, Reach::Value, Redaction::On);
        let message = json!({ "result": { "text": format!("{before}{value}"), "list": [value] } });
        let mut line = serde_json::to_vec(&message).expect("a message encodes");
        line.push(b'\n');

        let masked = masks.apply_json_line(line, Scope::Returned);

        let read: serde_json::Value = serde_json::from_slice(&masked).expect("the line stays one JSON message");
        prop_assert_eq!(leaks(&value, read.to_string().as_bytes()), Vec::<String>::new());
    }

    #[test]
    fn a_line_masked_into_a_buffer_follows_what_it_held_masked_as_on_its_own(
        value in value(),
        joins in any::<bool>(),
        held in "[ -~]{0,8}",
        holds in any::<bool>(),
        form in form(),
        around in around(),
    ) {
        let masks = Masks::new();
        let mask = masks.for_run(Redaction::On);
        if joins {
            mask.join(SHORT_NAME, &SecretString::from(value.as_str()), Reach::Value);
        }
        let line = if holds {
            format!("{around}{}{around}\n", encoded(&value, form))
        } else {
            format!("{around}\n")
        };
        let mut buffer = held.clone().into_bytes();

        masks.apply_bytes_into(line.as_bytes(), Scope::KeptOutput, &mut buffer);

        let alone = masks.apply_bytes(line.as_bytes(), Scope::KeptOutput);
        prop_assert_eq!(buffer, [held.as_bytes(), &alone].concat());
    }
}

/// A value hard to redact (TESTING.md, Secrets): quotes, backslashes,
/// newlines and `%`, any Unicode, a common word, or very long.
fn hard_value() -> impl Strategy<Value = String> {
    prop_oneof![
        "[\"\\\\%\n a-z0-9]{8,24}",
        "\\PC{8,16}",
        prop::sample::select(vec![
            "password", "settings", "document", "function", "username", "customer",
        ])
        .prop_map(str::to_owned),
        "[ -~]{100,400}",
    ]
}

/// Every formatter and serializer the Mask writes through, each with the
/// output it makes of `text`: `apply` in each scope, `apply_bytes`,
/// `apply_bytes_into`, `apply_value` and `json` over a JSON value holding
/// the text, `apply_json_line` over a message holding it, and `apply_typed` for a read of a field the
/// values were typed in.
fn through_every_formatter(masks: &Masks, text: &str, typed: &[&str]) -> Vec<(String, Vec<u8>)> {
    let mut outputs: Vec<(String, Vec<u8>)> = EVERY_SCOPE
        .iter()
        .map(|scope| {
            (
                format!("apply {scope:?}"),
                masks.apply(text, *scope).into_owned().into_bytes(),
            )
        })
        .collect();
    outputs.push((
        "apply_bytes".to_owned(),
        masks.apply_bytes(text.as_bytes(), Scope::Kept).into_owned(),
    ));
    let mut into = Vec::new();
    masks.apply_bytes_into(text.as_bytes(), Scope::Kept, &mut into);
    outputs.push(("apply_bytes_into".to_owned(), into));
    let mut value = json!({ text: [text, { "text": text }] });
    masks.apply_value(&mut value, Scope::Returned);
    outputs.push(("apply_value".to_owned(), value.to_string().into_bytes()));
    outputs.push((
        "json".to_owned(),
        masks
            .json(&json!({ text: [text, { "text": text }] }), Scope::Kept)
            .into_bytes(),
    ));
    let mut line =
        serde_json::to_vec(&json!({ "result": { "text": text } })).expect("a message encodes");
    line.push(b'\n');
    outputs.push((
        "apply_json_line".to_owned(),
        masks.apply_json_line(line, Scope::Returned),
    ));
    outputs.push((
        "apply_typed".to_owned(),
        masks.apply_typed(text, typed).into_owned().into_bytes(),
    ));
    outputs
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:4-91
    #[test]
    fn no_form_of_two_values_one_inside_the_other_passes_any_formatter(
        inner in hard_value(),
        before in "[ -~]{0,3}",
        after in "[ -~]{0,3}",
        forms in prop::collection::vec(form(), 1..4),
        around in around(),
    ) {
        let outer = format!("{before}{inner}{after}");
        let masks = Masks::new();
        let mask = masks.for_run(Redaction::On);
        mask.join("i", &SecretString::from(inner.as_str()), Reach::Value);
        mask.join("o", &SecretString::from(outer.as_str()), Reach::Value);
        let text = forms
            .iter()
            .flat_map(|form| [encoded(&inner, *form), encoded(&outer, *form)])
            .collect::<Vec<_>>()
            .join(&around);

        for (formatter, output) in through_every_formatter(&masks, &text, &["i", "o"]) {
            for value in [&inner, &outer] {
                prop_assert_eq!(leaks(value, &output), Vec::<String>::new(), "{}", formatter);
            }
        }
    }

    /// An empty value masks nothing, through every formatter: no text is
    /// rewritten around it.
    /// Covers: story:4-91
    #[test]
    fn an_empty_value_leaves_every_formatters_output_as_it_was(text in "[ -~]{0,40}") {
        let masks = Masks::new();
        let mask = masks.for_run(Redaction::On);
        mask.join("e", &SecretString::from(""), Reach::Value);
        let unmasked = Masks::new();

        prop_assert_eq!(
            through_every_formatter(&masks, &text, &["e"]),
            through_every_formatter(&unmasked, &text, &[])
        );
    }
}
