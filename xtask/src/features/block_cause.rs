//! The `block-cause:` Features: one per Block cause, each a value of the
//! type both Products share, `block_cause`, in the catalog's
//! `shared_type_values` (ADR 0095), named by its wire value. A value Foxhound
//! adds, such as `decoy`, carries Foxhound's Product mark and is Foxhound's
//! alone. A catalog with no Block cause generates nothing.

use serde_json::Value;

use super::entry::{Carriers, Entry};
use super::files::{Kind, Status};

/// The shared type whose values are the Block causes.
const SHARED_TYPE: &str = "block_cause";

pub(super) fn entries(catalog: &Value) -> Vec<Entry> {
    catalog["shared_type_values"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|value| value["type"] == SHARED_TYPE)
        .filter_map(|value| Some((value["name"].as_str()?, &value["product"])))
        .map(|(name, product)| Entry {
            kind: Kind::BlockCause,
            name: name.to_owned(),
            area: None,
            milestone: None,
            status: Status::Active,
            carriers: if product == "foxhound" {
                Carriers::Foxhound
            } else {
                Carriers::Both
            },
        })
        .collect()
}
