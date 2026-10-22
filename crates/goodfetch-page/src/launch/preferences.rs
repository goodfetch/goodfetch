//! The preferences a Run's browser starts on (ADR 0119): a PDF is
//! downloaded, never shown in the browser's viewer, so a navigation to one
//! is a download. They are set in the profile's `Preferences` file, keeping
//! every other preference it already holds; a file that is no JSON object,
//! which Chromium itself would discard, starts again.

use serde_json::{Map, Value};

/// The `Preferences` file a launch writes, from the one the profile holds,
/// if any.
pub(super) fn starting_preferences(existing: Option<&str>) -> String {
    let mut preferences = existing
        .and_then(|text| serde_json::from_str::<Map<String, Value>>(text).ok())
        .unwrap_or_default();
    let plugins = preferences.entry("plugins").or_insert(Value::Null);
    if !plugins.is_object() {
        *plugins = Value::Object(Map::new());
    }
    plugins["always_open_pdf_externally"] = Value::Bool(true);
    Value::Object(preferences).to_string()
}

#[cfg(test)]
mod tests;
