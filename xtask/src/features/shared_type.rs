//! The Features of the types both Products share, read from the catalog's
//! `shared_type_values` (ADR 0095): one `block-cause:` per Block cause and one
//! `mcp-profile:` per MCP profile, each named by its wire value. A value
//! Foxhound adds, such as the Block cause `decoy` or the profile `crawl`,
//! carries Foxhound's Product mark and is Foxhound's alone. A type with no
//! value in the catalog generates nothing.

use goodfetch_protocol::catalog::SharedType;
use serde_json::Value;

use super::entry::{Carriers, Entry};
use super::files::{Kind, Status};

/// Each shared type whose values are Features, with their kind.
const KINDS: [(SharedType, Kind); 2] = [
    (SharedType::BlockCause, Kind::BlockCause),
    (SharedType::McpProfile, Kind::McpProfile),
];

pub(super) fn entries(catalog: &Value) -> Vec<Entry> {
    catalog["shared_type_values"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|value| {
            let (_, kind) = KINDS
                .iter()
                .find(|(shared_type, _)| value["type"] == shared_type.as_str())?;
            Some(Entry {
                kind: *kind,
                name: value["name"].as_str()?.to_owned(),
                area: None,
                milestone: None,
                status: Status::Active,
                carriers: if value["product"] == "foxhound" {
                    Carriers::Foxhound
                } else {
                    Carriers::Both
                },
            })
        })
        .collect()
}
