//! What the generators write, for `xtask sdk-manifests` (ADR 0099): the
//! members of Collie's handles, read from the committed bundle and Collie's
//! projection by the same rules each generator applies, and each language's
//! spelling of a name's words, so a manifest line is checked against the
//! generators rather than a copy of their rules.

use std::path::Path;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use serde_json::Value;

use super::{PROTOCOL_GENERATED, handles, layer::Layer, naming};

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

/// The members every language's generator writes on Collie's handles, which
/// each Product's SDK hands out (ADR 0095). Foxhound's operations add none
/// yet: its generators write no handle.
pub(crate) fn handle_members(root: &Path) -> Result<Vec<HandleMember>> {
    let bundle = read_json(root, "bundle.json")?;
    let collie = read_json(root, "collie.json")?;
    // Collie's layer reads neither Foxhound's projection nor the version.
    let layer = Layer::new(&bundle, &collie, &Value::Null, Product::Collie, "")?;
    let mut members = Vec::new();
    for handle in handles::handles(&layer)? {
        let receiver = naming::pascal(handle.namespace);
        for member in &handle.members {
            members.push(HandleMember {
                receiver: receiver.clone(),
                entry: member.operation.name.clone(),
                words: member.name.to_owned(),
            });
        }
    }
    Ok(members)
}

/// A TypeScript member's spelling of `words`, joined by `_`.
pub(crate) fn typescript_member(words: &str) -> String {
    naming::camel(words)
}

/// A TypeScript or Python type's spelling of `words`.
pub(crate) fn class_name(words: &str) -> String {
    naming::pascal(words)
}

/// A Rust member's spelling of `words`; `None` for one Rust cannot spell.
pub(crate) fn rust_member(words: &str) -> Option<String> {
    naming::rust_snake(words)
}

/// A Rust type's spelling of `words`; `None` for one Rust cannot spell.
pub(crate) fn rust_type(words: &str) -> Option<String> {
    naming::rust_type(words)
}
