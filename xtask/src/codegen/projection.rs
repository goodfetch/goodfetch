//! Each Product's projection of the catalog (ADR 0095): the entries it
//! carries, Collie's own for Collie and every entry for Foxhound, which
//! contains Collie, with the bundle's types they reach and those of the
//! formats Collie's client package reads itself, which every Product's SDK
//! hands out. Every section of the catalog is projected the same way, so a
//! contract that joins the catalog joins each projection and the containment
//! check with no change here.

use anyhow::{Context as _, Result};
use serde::Deserialize as _;
use serde_json::{Map, Value, json};

use crate::compat;
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::published_catalog::{PUBLISHED_CATALOG_VERSION, SCHEMA_DRAFT};

pub(super) fn project(catalog: &Value, bundle: &Value, product: Product) -> Result<Value> {
    let catalog = catalog
        .as_object()
        .context("the catalog is no JSON object")?;
    let mut projection = Map::new();
    for (key, value) in catalog {
        let projected = match value {
            Value::Array(entries) => Value::Array(
                entries
                    .iter()
                    .filter(|entry| carries(product, entry))
                    .cloned()
                    .collect(),
            ),
            other => other.clone(),
        };
        projection.insert(key.clone(), projected);
    }
    let definitions = bundle
        .get("$defs")
        .and_then(Value::as_object)
        .context("the bundle has no `$defs`")?;
    let mut reached = compat::reached(&Value::Object(projection.clone()), definitions)?;
    for schema in client_read_formats(bundle) {
        reached.extend(compat::reached(schema, definitions)?);
    }
    projection.insert(
        "$defs".to_owned(),
        Value::Object(
            definitions
                .iter()
                .filter(|(name, _)| reached.contains_key(name.as_str()))
                .map(|(name, schema)| (name.clone(), schema.clone()))
                .collect(),
        ),
    );
    Ok(Value::Object(projection))
}

/// The self-contained published Operation catalog (`goodfetch-catalog.json`)
/// holding both Products' entries with their Product marks, the protocol
/// version, the Connection framing types, and every `$defs` entry they refer
/// to (ADRs 0030, 0032 and 0095; issue 1662).
pub(super) fn published_catalog(catalog: &Value, bundle: &Value) -> Result<Value> {
    let catalog_object = catalog
        .as_object()
        .context("the catalog is no JSON object")?;
    let connection = bundle
        .get("connection")
        .and_then(Value::as_object)
        .context("the bundle has no `connection` section")?;
    let definitions = bundle
        .get("$defs")
        .and_then(Value::as_object)
        .context("the bundle has no `$defs`")?;
    let mut reached = compat::reached(catalog, definitions)?;
    reached.extend(compat::reached(
        &Value::Object(connection.clone()),
        definitions,
    )?);
    let defs: Map<String, Value> = definitions
        .iter()
        .filter(|(name, _)| reached.contains_key(name.as_str()))
        .map(|(name, schema)| (name.clone(), schema.clone()))
        .collect();
    Ok(json!({
        "format_version": PUBLISHED_CATALOG_VERSION.to_string(),
        "protocol_version": PROTOCOL_VERSION.to_string(),
        "$schema": SCHEMA_DRAFT,
        "catalog": catalog_object,
        "connection": connection,
        "$defs": defs,
    }))
}

/// The formats a Collie client package reads itself, such as the delivery
/// its webhook verification helper parses (issue 1391), whose types each
/// projection holds though no entry names them, so that every Product's SDK
/// may hand them out.
const CLIENT_READ_FORMATS: &[&str] = &["notification-payload"];

/// The schemas of the bundle's [`CLIENT_READ_FORMATS`]; a bundle without
/// one, such as a test fixture's, gives none.
fn client_read_formats(bundle: &Value) -> impl Iterator<Item = &Value> {
    bundle
        .get("formats")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|format| {
            format
                .get("name")
                .and_then(Value::as_str)
                .is_some_and(|name| CLIENT_READ_FORMATS.contains(&name))
        })
        .filter_map(|format| format.get("schema"))
}

/// Whether `product`'s catalog holds `entry`: Collie's holds Collie's entries,
/// Foxhound's every entry, since Foxhound contains Collie.
fn carries(product: Product, entry: &Value) -> bool {
    match product {
        Product::Collie => {
            entry
                .get("product")
                .and_then(|mark| Product::deserialize(mark).ok())
                == Some(Product::Collie)
        }
        Product::Foxhound => true,
    }
}
