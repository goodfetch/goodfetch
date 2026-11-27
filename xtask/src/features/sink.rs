//! The `sink:` Features: one per built-in Sink listed in
//! `crates/foxhound/goodfetch-sink/sinks.toml` (ADRs 0026 and 0072), each
//! named by its `SinkType` wire value and carried by Foxhound alone
//! (ADR 0087). A workspace without that file generates nothing; an unknown
//! `SinkType`, a duplicate name or an unknown field fails generation.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Result, bail};
use goodfetch_protocol::foxhound::sink::SinkType;
use serde::Deserialize;

use super::entry::{Carriers, Entry, unreadable};
use super::files::{self, Kind, Status};

/// The list of built-in Sinks carried in `goodfetch-sink`.
const SINKS_FILE: &str = "crates/foxhound/goodfetch-sink/sinks.toml";

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct SinksFile {
    #[serde(default)]
    sinks: Vec<SinkType>,
}

pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let file: SinksFile = files::read_optional_toml(root, SINKS_FILE)
        .map_err(|error| unreadable(error, SINKS_FILE))?;
    let mut seen = BTreeSet::new();
    let mut entries = Vec::with_capacity(file.sinks.len());
    for sink in file.sinks {
        let name = serde_json::to_value(sink)
            .expect("invariant: SinkType serializes as a JSON string")
            .as_str()
            .expect("invariant: SinkType serializes as a string")
            .to_owned();
        if !seen.insert(name.clone()) {
            bail!("{SINKS_FILE} lists built-in Sink `{name}` more than once");
        }
        entries.push(Entry {
            kind: Kind::Sink,
            name,
            area: None,
            milestone: None,
            status: Status::Active,
            carriers: Carriers::Foxhound,
        });
    }
    Ok(entries)
}
