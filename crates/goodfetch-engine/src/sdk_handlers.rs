//! A program's SDK handlers, which its Connection's `hello` declares by
//! Plugin kind and name (ADR 0104). The only handler of a kind that one
//! setting chooses becomes that setting's value at the connection scope, in
//! the run layer with the origin `registered`: every other run-layer value
//! replaces it, configuration does not, and a lock wins over it with no
//! refusal (ADR 0104's amendment of ADR 0083). A kind with two handlers
//! gives no such value, and a kind no setting chooses gives none.

use std::collections::{BTreeMap, BTreeSet};

use goodfetch_protocol::catalog::{PluginKind, Product};
use goodfetch_protocol::entries::MODULES;
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use serde_json::{Map, Value};

use crate::call_error::CallError;

/// The Plugin kinds an engine of `engine` carries: those of the catalog
/// modules its Product contains.
pub(crate) fn carried(engine: Product) -> Vec<&'static PluginKind> {
    MODULES
        .into_iter()
        .filter(|entries| engine.contains(Product::of_module(entries.defined_in)))
        .flat_map(|entries| entries.plugin_kinds)
        .collect()
}

/// The run layer's `registered` values that `handlers`, a `hello`'s, give
/// on an engine carrying `kinds`, by dotted key: for each kind one setting
/// chooses and that has one handler, that setting set to its name.
///
/// # Errors
///
/// Invalid params for a kind not among `kinds`, which nothing would
/// otherwise answer (ADR 0083).
pub(crate) fn registered(
    handlers: &BTreeMap<String, BTreeSet<String>>,
    kinds: &[&'static PluginKind],
) -> Result<Map<String, Value>, CallError> {
    let mut values = Map::new();
    for (name, registered) in handlers {
        let Some(kind) = kinds.iter().find(|kind| kind.name == name) else {
            return Err(CallError::protocol(
                ProtocolFailure::InvalidParams,
                format!(
                    "`hello` registers handlers of the Plugin kind `{name}`, which this engine has none of"
                ),
            ));
        };
        let mut names = registered.iter();
        if let (Some(setting), Some(handler), None) = (kind.chosen_by, names.next(), names.next()) {
            values.insert(setting.name.to_owned(), Value::String(handler.clone()));
        }
    }
    Ok(values)
}

#[cfg(test)]
mod tests;
