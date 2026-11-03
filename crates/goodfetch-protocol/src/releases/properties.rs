//! Each validated string type of `compat/releases.toml` states its rule twice:
//! in its parser and as its JSON Schema's `pattern`, which the SDKs and
//! editors read. These properties hold the two to one rule.

use proptest::prelude::*;
use proptest::string::string_regex;
use regex::Regex;
use schemars::{JsonSchema, SchemaGenerator};

use super::{FormatName, FormatVersion, HttpsUrl, ProductVersion, Sha256};

/// `T`'s JSON Schema `pattern`.
fn pattern_of<T: JsonSchema>() -> String {
    let schema = T::json_schema(&mut SchemaGenerator::default());
    schema
        .get("pattern")
        .and_then(serde_json::Value::as_str)
        .expect("the type's schema has a pattern")
        .to_owned()
}

/// Strings the pattern matches, near misses built from `near`, and any text.
fn candidates(pattern: &str, near: &str) -> impl Strategy<Value = String> {
    let unanchored = pattern.trim_start_matches('^').trim_end_matches('$');
    prop_oneof![
        string_regex(unanchored).expect("the pattern is a regex proptest reads"),
        string_regex(near).expect("the near misses are a regex"),
        any::<String>(),
    ]
}

/// Whether `pattern` matches all of `text`, as a JSON Schema validator reads it.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
fn matches(pattern: &str, text: &str) -> bool {
    Regex::new(pattern)
        .expect("the pattern is a regex")
        .is_match(text)
}

proptest! {
    #[test]
    fn product_version_parser_and_pattern_agree(
        text in candidates(&pattern_of::<ProductVersion>(), "[0-9.]{0,14}")
    ) {
        prop_assert_eq!(
            ProductVersion::try_from(text.clone()).is_ok(),
            matches(&pattern_of::<ProductVersion>(), &text)
        );
    }

    #[test]
    fn format_version_parser_and_pattern_agree(
        text in candidates(&pattern_of::<FormatVersion>(), "[0-9.]{0,14}")
    ) {
        prop_assert_eq!(
            FormatVersion::try_from(text.as_str()).is_ok(),
            matches(&pattern_of::<FormatVersion>(), &text)
        );
    }

    #[test]
    fn format_name_parser_and_pattern_agree(
        text in candidates(&pattern_of::<FormatName>(), "[a-zA-Z0-9_-]{0,12}")
    ) {
        prop_assert_eq!(
            FormatName::try_from(text.clone()).is_ok(),
            matches(&pattern_of::<FormatName>(), &text)
        );
    }

    #[test]
    fn sha256_parser_and_pattern_agree(
        text in candidates(&pattern_of::<Sha256>(), "[0-9a-fA-G]{62,66}")
    ) {
        prop_assert_eq!(
            Sha256::try_from(text.clone()).is_ok(),
            matches(&pattern_of::<Sha256>(), &text)
        );
    }

    #[test]
    fn https_url_parser_and_pattern_agree(
        text in candidates(&pattern_of::<HttpsUrl>(), "(https?://)?[a-z/?#. \t\u{a0}\u{feff}]{0,10}")
    ) {
        prop_assert_eq!(
            HttpsUrl::try_from(text.clone()).is_ok(),
            matches(&pattern_of::<HttpsUrl>(), &text)
        );
    }
}
