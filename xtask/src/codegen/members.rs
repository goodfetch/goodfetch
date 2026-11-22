//! The members the generators write on Collie's handles, for `xtask
//! sdk-manifests` (ADR 0099): read from the committed bundle and Collie's
//! projection by the same rules each generator applies, so a manifest line
//! is checked against the generators rather than a copy of their rules.

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use serde_json::Value;

use super::{
    PROTOCOL_GENERATED, handles,
    layer::{Layer, input_name},
    naming,
};

/// A member a generator writes on a handle.
pub(crate) struct HandleMember {
    /// The handle's type, such as `Run`.
    pub(crate) receiver: String,
    /// The catalog entry it serves, such as `run.abort`.
    pub(crate) entry: String,
    /// Its `snake_case` words, such as `wait_for_url`.
    pub(crate) words: String,
}

fn read_json(root: &Path, name: &str) -> Result<Value> {
    let path = root.join(PROTOCOL_GENERATED).join(name);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// The committed bundle and Collie's projection, which Collie's layer is
/// built from: it reads neither Foxhound's projection nor the version.
fn collie_inputs(root: &Path) -> Result<(Value, Value)> {
    Ok((
        read_json(root, "bundle.json")?,
        read_json(root, input_name(Product::Collie))?,
    ))
}

/// The entries some member every language's generator writes on Collie's
/// handles serves: each member, a sub-object's included, each read and each
/// `on` listener. Foxhound's operations add none yet: its generators write
/// no handle.
pub(crate) fn handle_entries(root: &Path) -> Result<BTreeSet<String>> {
    let (bundle, collie) = collie_inputs(root)?;
    let layer = Layer::new(&bundle, &collie, &Value::Null, Product::Collie, "")?;
    let mut entries = BTreeSet::new();
    for handle in handles::handles(&layer)? {
        let members = handle
            .members
            .iter()
            .chain(handle.sub_objects.iter().flat_map(|held| &held.members))
            .map(|member| member.operation);
        let reads = handle.reads().iter().map(|read| read.operation);
        let listeners = handle
            .listener
            .iter()
            .chain(&handle.published_listener)
            .map(|listener| listener.operation);
        entries.extend(
            members
                .chain(reads)
                .chain(listeners)
                .map(|operation| operation.name.clone()),
        );
    }
    Ok(entries)
}

/// The members every language's generator writes on Collie's handles, which
/// each Product's SDK hands out (ADR 0095). Foxhound's operations add none
/// yet: its generators write no handle.
pub(crate) fn handle_members(root: &Path) -> Result<Vec<HandleMember>> {
    let (bundle, collie) = collie_inputs(root)?;
    let layer = Layer::new(&bundle, &collie, &Value::Null, Product::Collie, "")?;
    let mut members = Vec::new();
    for handle in handles::handles(&layer)? {
        let receiver = naming::pascal(handle.namespace);
        let written = handle
            .members
            .iter()
            .map(|member| (member.operation, member.name))
            .chain(
                handle
                    .reads()
                    .iter()
                    .map(|read| (read.operation, read.name)),
            );
        for (operation, name) in written {
            members.push(HandleMember {
                receiver: receiver.clone(),
                entry: operation.name.clone(),
                words: name.to_owned(),
            });
        }
    }
    Ok(members)
}
