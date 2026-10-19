//! A Proving ground task's digest (ADR 0063): the lowercase hex SHA-256 of
//! `{"entry": …, "expected": …}`, with every URL's port removed, serialized
//! in the JSON Canonicalization Scheme (RFC 8785). It is the same on every
//! port, and a changed entry or expected result changes it.

use std::fmt::Write as _;

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

/// Removes the port of every `http` or `https` URL among the strings.
fn remove_ports(value: &mut Value) {
    match value {
        Value::String(text) => {
            if let Some(url) = without_port(text) {
                *text = url;
            }
        }
        Value::Array(values) => values.iter_mut().for_each(remove_ports),
        Value::Object(fields) => fields.values_mut().for_each(remove_ports),
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// The URL without its port, or `None` when the text is no `http` or `https`
/// URL with a port.
fn without_port(text: &str) -> Option<String> {
    let (scheme, rest) = text.split_once("://")?;
    if !(scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https")) {
        return None;
    }
    let (authority, tail) = rest.split_at(rest.find(['/', '?', '#']).unwrap_or(rest.len()));
    // An IPv6 host is bracketed, and its colons are not the port's.
    let host_end = authority.rfind(']').map_or(0, |bracket| bracket + 1);
    let colon = host_end + authority[host_end..].rfind(':')?;
    let port = &authority[colon + 1..];
    port.bytes()
        .all(|byte| byte.is_ascii_digit())
        .then(|| format!("{scheme}://{}{tail}", &authority[..colon]))
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
