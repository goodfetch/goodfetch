//! The `format:` Features: one per versioned format of the format registry
//! (ADR 0066), which the bundle carries as `formats`; a bundle without it
//! registers none.

use serde_json::Value;

use super::entry::{Carriers, Entry};
use super::files::{Kind, Status};

pub(super) fn entries(bundle: &Value) -> Vec<Entry> {
    bundle["formats"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|format| format["name"].as_str())
        .map(|name| Entry {
            kind: Kind::Format,
            name: name.to_owned(),
            area: None,
            milestone: None,
            status: Status::Active,
            carriers: Carriers::Both,
        })
        .collect()
}
