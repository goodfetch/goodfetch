//! What one SDK package generates (ADR 0095): the typed entries of its
//! Product's own projection, the types they define and the types they take
//! from Collie's client package, read once from the projections' JSON so
//! each language's generator works on parsed values.

use std::collections::BTreeSet;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use serde::Deserialize;
use serde_json::{Map, Value};

use super::projection;

/// The generators' input file name for `product`'s layer.
pub(super) fn input_name(product: Product) -> &'static str {
    match product {
        Product::Collie => "collie.json",
        Product::Foxhound => "foxhound.json",
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Stability {
    Stable,
    Experimental,
}

#[derive(Debug, Deserialize)]
pub(super) struct Deprecation {
    pub(super) since: String,
    pub(super) note: String,
}

/// A reference to a type of the bundle, `{"$ref": "#/$defs/<name>"}`.
#[derive(Debug, Deserialize)]
#[serde(try_from = "Reference")]
pub(super) struct TypeName(pub(super) String);

#[derive(Deserialize)]
struct Reference {
    #[serde(rename = "$ref")]
    reference: String,
}

impl TryFrom<Reference> for TypeName {
    type Error = String;

    fn try_from(value: Reference) -> Result<Self, Self::Error> {
        value
            .reference
            .strip_prefix("#/$defs/")
            .map(|name| Self(name.to_owned()))
            .ok_or_else(|| format!("`{}` is no type of the bundle", value.reference))
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Operation {
    pub(super) name: String,
    pub(super) product: Product,
    pub(super) description: String,
    pub(super) params: TypeName,
    pub(super) result: TypeName,
    pub(super) cancellable: bool,
    pub(super) stability: Stability,
    pub(super) deprecated: Option<Deprecation>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Event {
    pub(super) name: String,
    pub(super) product: Product,
    pub(super) description: String,
    pub(super) payload: TypeName,
    pub(super) stability: Stability,
    pub(super) deprecated: Option<Deprecation>,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorGroup {
    pub(super) name: String,
    pub(super) description: String,
    /// The JSON-RPC error code the wire carries for the group's codes.
    pub(super) json_rpc_class: i64,
}

#[derive(Debug, Deserialize)]
pub(super) struct ErrorCode {
    pub(super) name: String,
    pub(super) product: Product,
    pub(super) description: String,
    pub(super) group: String,
    pub(super) retryable: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct PluginMethod {
    pub(super) name: String,
    pub(super) description: String,
    pub(super) params: TypeName,
    pub(super) result: TypeName,
}

#[derive(Debug, Deserialize)]
pub(super) struct PluginKind {
    pub(super) name: String,
    pub(super) product: Product,
    pub(super) description: String,
    pub(super) methods: Vec<PluginMethod>,
    pub(super) stability: Stability,
}

/// The sections of a projection that the SDK layers read; any other section
/// is no SDK's (metrics, for instance).
#[derive(Deserialize)]
struct Projection {
    protocol_version: String,
    operations: Vec<Operation>,
    events: Vec<Event>,
    error_groups: Vec<ErrorGroup>,
    error_codes: Vec<ErrorCode>,
    plugin_kinds: Vec<PluginKind>,
    #[serde(rename = "$defs")]
    definitions: Map<String, Value>,
}

/// One SDK package's layer.
pub(super) struct Layer {
    pub(super) product: Product,
    /// The protocol version `major.minor` the catalog speaks.
    pub(super) protocol_version: String,
    pub(super) operations: Vec<Operation>,
    pub(super) events: Vec<Event>,
    /// The seven groups, generated once, in Collie's client package.
    pub(super) error_groups: Vec<ErrorGroup>,
    pub(super) error_codes: Vec<ErrorCode>,
    pub(super) plugin_kinds: Vec<PluginKind>,
    /// The types this layer defines.
    pub(super) definitions: Map<String, Value>,
    /// The types this layer's entries take from Collie's client package.
    pub(super) collie_types: BTreeSet<String>,
}

impl Layer {
    /// The layer of `product`'s package: Collie's entries for Collie's client
    /// package, with the types of the bundle's `connection` section, which
    /// every engine speaks and the client package's runtime sends; Foxhound's
    /// own for Foxhound's, which builds on it.
    pub(super) fn new(
        bundle: &Value,
        collie: &Value,
        foxhound: &Value,
        product: Product,
    ) -> Result<Self> {
        let collie: Projection =
            Projection::deserialize(collie).context("reading Collie's projection")?;
        match product {
            Product::Collie => {
                let mut definitions = collie.definitions;
                let bundle_definitions = bundle
                    .get("$defs")
                    .and_then(Value::as_object)
                    .context("the bundle has no `$defs`")?;
                let connection = bundle.get("connection").cloned().unwrap_or(Value::Null);
                for name in projection::reached(&connection, bundle_definitions)? {
                    if let Some(schema) = bundle_definitions.get(&name) {
                        definitions.insert(name, schema.clone());
                    }
                }
                definitions.sort_keys();
                Ok(Self {
                    product,
                    protocol_version: collie.protocol_version,
                    operations: collie.operations,
                    events: collie.events,
                    error_groups: collie.error_groups,
                    error_codes: collie.error_codes,
                    plugin_kinds: collie.plugin_kinds,
                    definitions,
                    collie_types: BTreeSet::new(),
                })
            }
            Product::Foxhound => {
                let all: Projection =
                    Projection::deserialize(foxhound).context("reading Foxhound's projection")?;
                let own = |entry_product: Product| entry_product == Product::Foxhound;
                let operations: Vec<Operation> = all
                    .operations
                    .into_iter()
                    .filter(|entry| own(entry.product))
                    .collect();
                let events: Vec<Event> = all
                    .events
                    .into_iter()
                    .filter(|entry| own(entry.product))
                    .collect();
                let error_codes: Vec<ErrorCode> = all
                    .error_codes
                    .into_iter()
                    .filter(|entry| own(entry.product))
                    .collect();
                let plugin_kinds: Vec<PluginKind> = all
                    .plugin_kinds
                    .into_iter()
                    .filter(|entry| own(entry.product))
                    .collect();
                let roots = roots(&operations, &events, &plugin_kinds);
                let reached = projection::reached(&roots, &all.definitions)?;
                let (collie_types, own_types): (BTreeSet<String>, BTreeSet<String>) = reached
                    .into_iter()
                    .partition(|name| collie.definitions.contains_key(name));
                Ok(Self {
                    product,
                    protocol_version: all.protocol_version,
                    operations,
                    events,
                    error_groups: Vec::new(),
                    error_codes,
                    plugin_kinds,
                    definitions: all
                        .definitions
                        .into_iter()
                        .filter(|(name, _)| own_types.contains(name))
                        .collect(),
                    collie_types,
                })
            }
        }
    }

    /// Whether `name` is a type of Collie's client package rather than this
    /// layer's.
    pub(super) fn is_collie_type(&self, name: &str) -> bool {
        self.collie_types.contains(name)
    }

    /// The `$defs` a generator turns into types: this layer's own, with each
    /// reference to one of Collie's types replaced by `replacement(name)`,
    /// the generator's way of naming a type it imports. Also returns the
    /// names it replaced.
    pub(super) fn generator_input(
        &self,
        title: &str,
        replacement: impl Fn(&str) -> (&'static str, String),
    ) -> (Value, BTreeSet<String>) {
        let mut definitions = Value::Object(self.definitions.clone());
        let mut replaced = BTreeSet::new();
        replace_collie_references(
            &mut definitions,
            &self.collie_types,
            &replacement,
            &mut replaced,
        );
        let input = serde_json::json!({
            "$schema": "https://json-schema.org/draft/2020-12/schema",
            "title": title,
            "description": "The root of the layer's types, which holds no value of its own.",
            "$defs": definitions,
        });
        (input, replaced)
    }
}

/// A value whose `$ref`s are every type the entries name.
fn roots(operations: &[Operation], events: &[Event], plugin_kinds: &[PluginKind]) -> Value {
    let reference = |name: &TypeName| serde_json::json!({ "$ref": format!("#/$defs/{}", name.0) });
    let operations = operations
        .iter()
        .flat_map(|operation| [reference(&operation.params), reference(&operation.result)]);
    let events = events.iter().map(|event| reference(&event.payload));
    let methods = plugin_kinds
        .iter()
        .flat_map(|kind| &kind.methods)
        .flat_map(|method| [reference(&method.params), reference(&method.result)]);
    Value::Array(operations.chain(events).chain(methods).collect())
}

fn replace_collie_references(
    value: &mut Value,
    collie_types: &BTreeSet<String>,
    replacement: &impl Fn(&str) -> (&'static str, String),
    replaced: &mut BTreeSet<String>,
) {
    match value {
        Value::Object(object) => {
            let collie_type = object
                .get("$ref")
                .and_then(Value::as_str)
                .and_then(|reference| reference.strip_prefix("#/$defs/"))
                .filter(|name| collie_types.contains(*name))
                .map(str::to_owned);
            if let Some(name) = collie_type {
                object.remove("$ref");
                let (key, value) = replacement(&name);
                object.insert(key.to_owned(), Value::String(value));
                replaced.insert(name);
            }
            for child in object.values_mut() {
                replace_collie_references(child, collie_types, replacement, replaced);
            }
        }
        Value::Array(items) => {
            for item in items {
                replace_collie_references(item, collie_types, replacement, replaced);
            }
        }
        _ => {}
    }
}
