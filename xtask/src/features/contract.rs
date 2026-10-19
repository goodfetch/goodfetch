//! The Features of `goodfetch-protocol`'s catalog: one per operation
//! (`operation:`), Error code (`error:`), event (`event:`), setting
//! (`setting:`) and exit code (`exit:`), named by the entry's own name. A
//! section the catalog does not have yet generates nothing.

use serde_json::Value;

use super::entry::Entry;
use super::files::{Kind, Status};

/// Each kind, with the catalog section that defines its Features.
const SECTIONS: [(Kind, &str); 5] = [
    (Kind::Operation, "operations"),
    (Kind::Error, "error_codes"),
    (Kind::Event, "events"),
    (Kind::Setting, "settings"),
    (Kind::Exit, "exit_codes"),
];

pub(super) fn entries(catalog: &Value) -> Vec<Entry> {
    SECTIONS
        .iter()
        .flat_map(|(kind, section)| {
            catalog[*section]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|entry| entry["name"].as_str())
                .map(|name| Entry {
                    kind: *kind,
                    name: name.to_owned(),
                    area: None,
                    milestone: None,
                    status: Status::Active,
                    section: Some(section),
                })
        })
        .collect()
}
