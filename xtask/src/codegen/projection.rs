//! Each Product's projection of the catalog (ADR 0095): the entries it
//! carries, Collie's own for Collie and every entry for Foxhound, which
//! contains Collie, with the bundle's types they reach. Every section of the
//! catalog is projected the same way, so a contract that joins the catalog
//! joins each projection and the containment check with no change here.

use anyhow::{Context as _, Result};
use serde::Deserialize as _;
use serde_json::{Map, Value};

use crate::compat;
use goodfetch_protocol::catalog::Product;

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
    let reached = compat::reached(&Value::Object(projection.clone()), definitions)?;
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
