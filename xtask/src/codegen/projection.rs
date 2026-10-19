//! Each Product's projection of the catalog (ADR 0095): the entries it
//! carries, Collie's own for Collie and every entry for Foxhound, which
//! contains Collie, with the bundle's types they reach. Every section of the
//! catalog is projected the same way, so a contract that joins the catalog
//! joins each projection and the containment check with no change here.

use std::collections::BTreeSet;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize as _;
use serde_json::{Map, Value};

use goodfetch_protocol::catalog::Product;

const DEFINITIONS: &str = "#/$defs/";

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
    let reached = reached(&Value::Object(projection.clone()), definitions)?;
    projection.insert(
        "$defs".to_owned(),
        Value::Object(
            definitions
                .iter()
                .filter(|(name, _)| reached.contains(name.as_str()))
                .map(|(name, schema)| (name.clone(), schema.clone()))
                .collect(),
        ),
    );
    Ok(Value::Object(projection))
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

/// The names of every type `value` refers to, directly or through other
/// types.
pub(super) fn reached(value: &Value, definitions: &Map<String, Value>) -> Result<BTreeSet<String>> {
    let mut reached = BTreeSet::new();
    let mut pending = references(value);
    while let Some(name) = pending.pop() {
        if !reached.insert(name.clone()) {
            continue;
        }
        let Some(schema) = definitions.get(&name) else {
            bail!("`#/$defs/{name}` is no type of the bundle");
        };
        pending.extend(references(schema));
    }
    Ok(reached)
}

/// The type names of every `$ref` in `value`.
fn references(value: &Value) -> Vec<String> {
    let mut names = Vec::new();
    let mut pending = vec![value];
    while let Some(current) = pending.pop() {
        match current {
            Value::Object(object) => {
                if let Some(name) = object
                    .get("$ref")
                    .and_then(Value::as_str)
                    .and_then(|reference| reference.strip_prefix(DEFINITIONS))
                {
                    names.push(name.to_owned());
                }
                pending.extend(object.values());
            }
            Value::Array(items) => pending.extend(items),
            _ => {}
        }
    }
    names
}
