//! The shared Unicode record (ADR 0073): hard text committed once as
//! `test-support/fixtures/unicode-record.json`, which the TypeScript and
//! Python packages read too, so every Sink conformance case and Export
//! format checks the same text round-trips byte for byte.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The Unicode record: one Record's fields, each a kind of hard text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnicodeRecord {
    /// The fields, in the committed file's order: `bidi` (right-to-left text
    /// with marks and an isolate), `nfc` and `nfd` (one word in each
    /// normalization form), `zwj_emoji` (a ZWJ emoji sequence), `astral` (a
    /// character beyond the Basic Multilingual Plane), `zero_width_joiner`,
    /// `bom` (text that starts with a byte order mark) and `tab_and_newline`.
    pub fields: Vec<UnicodeField>,
}

/// One field of the Unicode record.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct UnicodeField {
    /// The field's name, in snake case.
    pub name: String,
    /// Its text.
    pub text: String,
}

/// The committed record's text.
const COMMITTED: &str = include_str!("../../../test-support/fixtures/unicode-record.json");

/// The committed Unicode record.
pub fn unicode_record() -> UnicodeRecord {
    serde_json::from_str(COMMITTED)
        .expect("invariant: the committed Unicode record parses, which the Quick suite checks")
}

#[cfg(test)]
mod tests;
