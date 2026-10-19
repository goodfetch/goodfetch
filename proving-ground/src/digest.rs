//! A Proving ground task's digest (ADR 0063): the lowercase hex SHA-256 of
//! `{"entry": …, "expected": …}`, with every URL's port removed wherever it
//! appears in a string, serialized in the JSON Canonicalization Scheme
//! (RFC 8785). It is the same on every port, and a changed entry or expected
//! result changes it.

use std::fmt::Write as _;
use std::ops::Range;

use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

/// The digest of a task's entry (without its digest) and expected result.
pub(crate) fn of(entry: &impl Serialize, expected: &impl Serialize) -> String {
    let mut pinned = serde_json::json!({ "entry": entry, "expected": expected });
    remove_ports(&mut pinned);
    let canonical = serde_json_canonicalizer::to_vec(&pinned)
        .expect("invariant: a JSON value serializes canonically");
    Sha256::digest(canonical)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            write!(hex, "{byte:02x}").expect("invariant: writing to a String succeeds");
            hex
        })
}

/// Removes the port of every `http` or `https` URL in the strings, wherever
/// it appears in one, such as an absolute link inside a Markdown body.
fn remove_ports(value: &mut Value) {
    match value {
        Value::String(text) => *text = without_ports(text),
        Value::Array(values) => values.iter_mut().for_each(remove_ports),
        Value::Object(fields) => fields.values_mut().for_each(remove_ports),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The text with the port of each `http` or `https` URL in it removed.
fn without_ports(text: &str) -> String {
    let mut portless = String::with_capacity(text.len());
    let mut copied = 0;
    let mut searched = 0;
    while let Some(found) = text[searched..].find("://") {
        let authority = searched + found + "://".len();
        searched = authority;
        if !ends_with_http_scheme(&text[..authority - "://".len()]) {
            continue;
        }
        if let Some(port) = port_span(&text[authority..]) {
            portless.push_str(&text[copied..authority + port.start]);
            copied = authority + port.end;
            searched = copied;
        }
    }
    portless.push_str(&text[copied..]);
    portless
}

/// Whether the scheme just before a `://` is `http` or `https`.
fn ends_with_http_scheme(before: &str) -> bool {
    let start = before
        .char_indices()
        .rev()
        .take_while(|&(_, c)| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
        .last()
        .map_or(before.len(), |(index, _)| index);
    let scheme = &before[start..];
    scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")
}

/// Where the port (its colon included) sits in the text after a URL's `://`,
/// or `None` when the URL has none.
fn port_span(after: &str) -> Option<Range<usize>> {
    let authority_end = after
        .find(|c: char| c.is_whitespace() || matches!(c, '/' | '?' | '#'))
        .unwrap_or(after.len());
    let host_start = after[..authority_end].rfind('@').map_or(0, |at| at + 1);
    let host = &after[host_start..];
    // An IPv6 host is bracketed, and its colons are not the port's.
    let host_length = if host.starts_with('[') {
        host.find(']')? + 1
    } else {
        host.find(|c: char| {
            !(c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '~' | '%'))
        })
        .unwrap_or(host.len())
    };
    let colon = host_start + host_length;
    let digits = after[colon..].strip_prefix(':')?;
    let digit_count = digits.bytes().take_while(u8::is_ascii_digit).count();
    let ends_the_authority = digits[digit_count..]
        .chars()
        .next()
        .is_none_or(|c| !c.is_ascii_alphanumeric());
    (digit_count > 0 && ends_the_authority).then_some(colon..colon + 1 + digit_count)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use serde_json::json;

    use super::of;

    fn entry(start_url: &str) -> serde_json::Value {
        json!({
            "id": "catalog/1",
            "family": "catalog",
            "kind": "crawl",
            "start_url": start_url,
            "description": "Crawl.",
        })
    }

    fn expected(price: &str) -> serde_json::Value {
        json!({ "records": [{ "id": "P-0001", "name": "Amber Lamp", "price": price }] })
    }

    #[test]
    fn a_task_has_the_digest_of_its_canonical_portless_json() {
        // Python's `json.dumps(…, sort_keys=True, separators=(",", ":"))` of
        // the portless object, which is RFC 8785 for this ASCII data, hashed
        // with `hashlib.sha256`.
        let digest = of(
            &entry("http://catalog.test:51234/catalog/1"),
            &expected("12.34"),
        );

        assert_eq!(
            digest,
            "117faf69c6fbf57c2b64d98172c19d3a6bbc435548f613728e118c3799107891"
        );
    }

    #[rstest]
    #[case::name_and_port("http://catalog.test:8080/catalog/1", "http://catalog.test/catalog/1")]
    #[case::https("https://catalog.test:8443/", "https://catalog.test/")]
    #[case::ipv6("http://[::1]:51234/tasks?x=1", "http://[::1]/tasks?x=1")]
    #[case::credentials(
        "http://user:secret@catalog.test:80/",
        "http://user:secret@catalog.test/"
    )]
    #[case::inside_a_markdown_link(
        "[Next](http://catalog.test:8080/catalog/2)",
        "[Next](http://catalog.test/catalog/2)"
    )]
    #[case::two_in_one_string(
        "From https://catalog.test:8443/ to http://[::1]:51234/tasks.",
        "From https://catalog.test/ to http://[::1]/tasks."
    )]
    #[case::at_a_sentences_end(
        "Start at http://catalog.test:8080.",
        "Start at http://catalog.test."
    )]
    fn the_digest_ignores_a_urls_port(#[case] with_port: &str, #[case] without: &str) {
        let digests = (
            of(&entry(with_port), &expected("12.34")),
            of(&entry(without), &expected("12.34")),
        );

        assert_eq!(digests.0, digests.1);
    }

    #[rstest]
    #[case::not_a_url("catalog.test:8080", "catalog.test")]
    #[case::another_scheme("ftp://catalog.test:21/", "ftp://catalog.test/")]
    #[case::another_path("http://catalog.test/catalog/1", "http://catalog.test/catalog/2")]
    fn the_digest_keeps_everything_but_a_urls_port(#[case] one: &str, #[case] other: &str) {
        let digests = (
            of(&entry(one), &expected("12.34")),
            of(&entry(other), &expected("12.34")),
        );

        assert_ne!(digests.0, digests.1);
    }

    #[test]
    fn another_expected_result_has_another_digest() {
        let url = "http://catalog.test:8080/catalog/1";

        let digests = (
            of(&entry(url), &expected("12.34")),
            of(&entry(url), &expected("12.35")),
        );

        assert_ne!(digests.0, digests.1);
    }
}
