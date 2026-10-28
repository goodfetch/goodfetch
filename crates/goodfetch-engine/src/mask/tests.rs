//! The run's `Mask` through its interface (spec #564's seam): every form
//! of a value replaced by its `%name%` in each scope, the short-value rule,
//! the redaction switch, a Run's end, JSON messages and streams.
//! Covers: story:4-56

use std::io::Write as _;

use goodfetch_test_support::{Canaries, Canary, Output, SecretKind};
use proptest::prelude::*;
use rstest::rstest;
use secrecy::SecretString;
use serde_json::json;

use super::{Mask, Masks, Reach, Redaction, Scope};

mod writers;

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

#[test]
fn a_value_split_across_writes_is_masked_in_what_the_writer_passes_on() {
    let (masks, _mask) = joined(VALUE, Reach::Value, Redaction::On);
    let mut writer = masks.writer(Vec::new(), Scope::KeptOutput);

    for chunk in ["line pa s", "s\"w/r", "d>?9~ end\n"] {
        writer
            .write_all(chunk.as_bytes())
            .expect("a Vec takes every write");
    }
    let written = writer.finish().expect("a Vec takes every write");

    assert_eq!(
        String::from_utf8(written).expect("the text stays UTF-8"),
        "line %proxy.password% end\n"
    );
}

#[test]
fn a_writer_dropped_unfinished_still_writes_what_it_held_back_masked() {
    let (masks, _mask) = joined(VALUE, Reach::Value, Redaction::On);
    let mut written = Vec::new();

    {
        let mut writer = masks.writer(&mut written, Scope::KeptOutput);
        writer
            .write_all(b"tail pa ss\"w/rd>?9~")
            .expect("a Vec takes every write");
    }

    assert_eq!(written, b"tail %proxy.password%");
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
    FormUrlencoded,
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
        Just(Form::FormUrlencoded),
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
        Form::FormUrlencoded => url::form_urlencoded::byte_serialize(value.as_bytes()).collect(),
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
    for form in [Form::Plus, Form::FormUrlencoded, Form::UriComponent] {
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
    fn a_stream_cut_anywhere_is_masked_as_its_whole_text_is(
        value in value(),
        forms in prop::collection::vec(form(), 1..4),
        cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..8),
    ) {
        let (masks, _mask) = joined_as(SHORT_NAME, &value, Reach::Value, Redaction::On);
        let text = forms
            .iter()
            .map(|form| encoded(&value, *form))
            .collect::<Vec<_>>()
            .join(" | ")
            .into_bytes();
        let mut cuts: Vec<usize> = cuts.iter().map(|cut| cut.index(text.len() + 1)).collect();
        cuts.push(0);
        cuts.push(text.len());
        cuts.sort_unstable();
        let mut writer = masks.writer(Vec::new(), Scope::KeptOutput);

        for pair in cuts.windows(2) {
            writer.write_all(&text[pair[0]..pair[1]]).expect("a Vec takes every write");
        }
        let written = writer.finish().expect("a Vec takes every write");

        prop_assert_eq!(&written, &masks.apply_bytes(&text, Scope::KeptOutput).into_owned());
        prop_assert_eq!(leaks(&value, &written), Vec::<String>::new());
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
/// `apply_value` over a JSON value holding the text, `apply_json_line` over
/// a message holding it, a writer given it in two writes, and
/// `apply_typed` for a read of a field the values were typed in.
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
    let mut value = json!({ text: [text, { "text": text }] });
    masks.apply_value(&mut value, Scope::Returned);
    outputs.push(("apply_value".to_owned(), value.to_string().into_bytes()));
    let mut line =
        serde_json::to_vec(&json!({ "result": { "text": text } })).expect("a message encodes");
    line.push(b'\n');
    outputs.push((
        "apply_json_line".to_owned(),
        masks.apply_json_line(line, Scope::Returned),
    ));
    let mut writer = masks.writer(Vec::new(), Scope::KeptOutput);
    let half = text.len() / 2;
    writer
        .write_all(&text.as_bytes()[..half])
        .expect("a Vec takes every write");
    writer
        .write_all(&text.as_bytes()[half..])
        .expect("a Vec takes every write");
    outputs.push((
        "writer".to_owned(),
        writer.finish().expect("a Vec takes every write"),
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
