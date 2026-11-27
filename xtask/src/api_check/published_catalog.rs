//! API breakage comparison of the published Operation catalog file
//! (`crates/goodfetch-protocol/generated/goodfetch-catalog.json`, ADRs 0030,
//! 0032 and 0095; issue 1662).
//!
//! Compares the current commit's `goodfetch-catalog.json` against the previous
//! release's `goodfetch-catalog.json` when the baseline commit has one:
//! - `format_version` and `protocol_version` (major change is a break, minor
//!   bump is an addition);
//! - `$schema` draft (a change is a break);
//! - both Products' catalog entries derived from `goodfetch-catalog.json`'s
//!   `product` marks (catching removals, schema narrowing/widening, and a
//!   `collie` entry moving to `foxhound`);
//! - `connection` framing types;
//! - `$defs` completeness (every `#/$defs/...` reference resolves inside the
//!   file).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::{Product, ProtocolVersion};
use goodfetch_protocol::published_catalog::PublishedCatalog;
use serde::Deserialize as _;
use serde_json::{Map, Value};

use super::{compare_projection, connection};
use crate::api_baseline::Changes;
use crate::compat;

/// Validates `current`'s `goodfetch-catalog.json` on its own: valid
/// [`PublishedCatalog`], every `#/$defs/...` reference resolves in `$defs`,
/// and every entry in every array section of `catalog` carries a valid
/// `product` mark.
pub(super) fn validate_current(
    bytes: &[u8],
    current: &Value,
    entries: &mut BTreeMap<String, Changes>,
) -> Result<()> {
    let parsed: PublishedCatalog = serde_json::from_slice(bytes)
        .context("validating current crates/goodfetch-protocol/generated/goodfetch-catalog.json")?;
    let defs: Map<String, Value> = parsed.defs.into_iter().collect();
    if let Err(error) = compat::reached(&current["catalog"], &defs) {
        entries
            .entry("published-catalog $defs".to_owned())
            .or_default()
            .breaks("catalog", error.to_string());
    }
    if let Err(error) = compat::reached(&current["connection"], &defs) {
        entries
            .entry("published-catalog $defs".to_owned())
            .or_default()
            .breaks("connection", error.to_string());
    }
    for (section, items) in &parsed.catalog {
        let Some(items) = items.as_array() else {
            continue;
        };
        for item in items {
            let valid_mark = item
                .get("product")
                .is_some_and(|mark| Product::deserialize(mark).is_ok());
            if !valid_mark {
                let name = item
                    .get("name")
                    .and_then(Value::as_str)
                    .unwrap_or("<unnamed>");
                entries
                    .entry(format!("published-catalog {section} {name}"))
                    .or_default()
                    .breaks("product", "missing or invalid Product mark");
            }
        }
    }
    Ok(())
}

/// Compares `previous` and `current` `goodfetch-catalog.json` documents.
pub(super) fn compare(
    previous: &Value,
    current: &Value,
    entries: &mut BTreeMap<String, Changes>,
    duplicates: &mut BTreeSet<String>,
) -> Result<()> {
    compare_version(
        "published-catalog format_version",
        previous["format_version"].as_str(),
        current["format_version"].as_str(),
        entries,
    )?;
    compare_version(
        "published-catalog protocol_version",
        previous["protocol_version"].as_str(),
        current["protocol_version"].as_str(),
        entries,
    )?;
    if previous["$schema"] != current["$schema"] {
        entries
            .entry("published-catalog $schema".to_owned())
            .or_default()
            .breaks(
                "$schema",
                format!(
                    "changed from {} to {}",
                    previous["$schema"], current["$schema"]
                ),
            );
    }

    let previous_collie = project_from_published(previous, Product::Collie)?;
    let current_collie = project_from_published(current, Product::Collie)?;
    compare_projection(
        "collie",
        &previous_collie,
        &current_collie,
        entries,
        duplicates,
    )?;

    let previous_foxhound = project_from_published(previous, Product::Foxhound)?;
    let current_foxhound = project_from_published(current, Product::Foxhound)?;
    compare_projection(
        "foxhound",
        &previous_foxhound,
        &current_foxhound,
        entries,
        duplicates,
    )?;

    connection::compare(previous, current, entries);
    Ok(())
}

fn compare_version(
    subject: &str,
    previous: Option<&str>,
    current: Option<&str>,
    entries: &mut BTreeMap<String, Changes>,
) -> Result<()> {
    let previous: ProtocolVersion = previous
        .with_context(|| format!("{subject}: previous published catalog has no version string"))?
        .parse()
        .with_context(|| format!("{subject}: invalid previous version"))?;
    let current: ProtocolVersion = current
        .with_context(|| format!("{subject}: current published catalog has no version string"))?
        .parse()
        .with_context(|| format!("{subject}: invalid current version"))?;
    if current == previous {
        return Ok(());
    }
    let changes = entries.entry(subject.to_owned()).or_default();
    // ADR 0030: during 0.y previews each 0.y counts as its own major, so any
    // change involving major == 0 breaks compatibility.
    if previous.major == 0
        || current.major == 0
        || current.major != previous.major
        || current.minor < previous.minor
    {
        changes.breaks("version", format!("changed from {previous} to {current}"));
    } else {
        changes.adds("version", format!("bumped from {previous} to {current}"));
    }
    Ok(())
}

/// Builds a Product's projection view from a `goodfetch-catalog.json` document
/// so [`compare_projection`] can check every entry by its Product mark.
fn project_from_published(document: &Value, product: Product) -> Result<Value> {
    let Some(catalog) = document.get("catalog").and_then(Value::as_object) else {
        bail!("published catalog has no `catalog` object");
    };
    let mut view = Map::new();
    for (section, value) in catalog {
        let projected = match value {
            Value::Array(items) => Value::Array(
                items
                    .iter()
                    .filter(|item| match product {
                        Product::Collie => {
                            item.get("product")
                                .and_then(|mark| Product::deserialize(mark).ok())
                                == Some(Product::Collie)
                        }
                        Product::Foxhound => true,
                    })
                    .cloned()
                    .collect(),
            ),
            other => other.clone(),
        };
        view.insert(section.clone(), projected);
    }
    if let Some(defs) = document.get("$defs") {
        view.insert("$defs".to_owned(), defs.clone());
    }
    Ok(Value::Object(view))
}
