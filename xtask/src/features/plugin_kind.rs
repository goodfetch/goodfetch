//! The `plugin-kind:` Features: one per Plugin kind of `goodfetch-protocol`'s
//! catalog (`plugin_kinds`), named by its wire name (ADRs 0033 and 0072). A
//! kind Foxhound adds is Foxhound's alone through ADR 0087's generated table
//! of Foxhound's names. A catalog without `plugin_kinds` generates nothing.

use serde_json::Value;

use super::entry::{Carriers, Entry};
use super::files::{Kind, Status};

const SECTION: &str = "plugin_kinds";

pub(super) fn entries(catalog: &Value) -> Vec<Entry> {
    catalog[SECTION]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry["name"].as_str())
        .map(|name| Entry {
            kind: Kind::PluginKind,
            name: name.to_owned(),
            area: None,
            milestone: None,
            status: Status::Active,
            carriers: Carriers::Table(SECTION),
        })
        .collect()
}
