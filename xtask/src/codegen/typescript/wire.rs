//! The TypeScript layer's names and points in time (ADRs 0092 and 0099):
//! each field the schemas declare is named in camelCase, by ADR 0099's
//! initialism rule, each `date-time` is typed as a `Date`, and `wire.ts`
//! holds the table the runtime converts by, which says, for each type,
//! each operation's params and result and each event's payload, which
//! fields the engine spells otherwise and where a point in time sits, so
//! that a call sends the engine's names and a result reaches the program
//! in camelCase with its `Date`s. Only declared fields are named: the keys
//! of a map or of a free value are data and keep their spelling. A type is
//! named in the table only when it holds a field to rename or a point in
//! time, directly or through the types it references; a type that is a
//! point in time itself, such as `PointInTime`, is written as `"time"`
//! where it is referenced. An open tagged union (ADR 0079) is always named,
//! with its tag and the members it knows, so that the runtime reads a
//! member a newer engine adds as its `unknown` member. Foxhound's table
//! adds its own types to Collie's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use serde_json::{Map, Value};

use super::super::layer::Layer;
use super::super::naming::camel;
use super::super::open_values::OpenUnion;
use super::HEADER;

/// The format a point in time takes in the catalog's schemas.
const DATE_TIME: &str = "date-time";

/// The keywords whose values are data, never schemas or field names.
const VALUES: [&str; 4] = ["const", "enum", "default", "examples"];

/// The keywords that hold subschemas the table has no way to follow; a
/// field to rename or a point in time under one is refused rather than
/// left as the engine spells it.
const UNFOLLOWED: [&str; 13] = [
    "prefixItems",
    "patternProperties",
    "not",
    "if",
    "then",
    "else",
    "contains",
    "propertyNames",
    "dependentSchemas",
    "unevaluatedProperties",
    "unevaluatedItems",
    "$defs",
    "additionalItems",
];

/// A field the table lists: its TypeScript name, and where its value holds
/// fields to rename or points in time.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Field {
    pub(super) name: String,
    pub(super) path: Option<WirePath>,
}

/// Where a value holds fields to rename or points in time, as the
/// runtime's `WirePath` writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum WirePath {
    /// The value is a point in time.
    Time,
    /// The value is of a type the table names.
    Type(String),
    /// The object's fields to rename or to follow, by the engine's name.
    Fields(BTreeMap<String, Field>),
    /// Each item of the list.
    Items(Box<WirePath>),
    /// Each value of the map.
    Values(Box<WirePath>),
    /// An open tagged union: the field that tags it, the members it knows,
    /// where its members hold something, and where the fields every member
    /// shares do, which a member it does not know is read by.
    Union {
        tag: String,
        known: Vec<String>,
        path: Option<Box<WirePath>>,
        common: Option<Box<WirePath>>,
    },
}

/// A layer's table.
#[derive(Debug)]
pub(super) struct Wire {
    /// The path of each of the layer's own types that holds something.
    pub(super) types: BTreeMap<String, WirePath>,
    /// The path of each operation's params that holds something, by operation.
    pub(super) params: BTreeMap<String, WirePath>,
    /// The path of each operation's result that holds something, by operation.
    pub(super) results: BTreeMap<String, WirePath>,
    /// The path of each event's payload that holds something, by event.
    pub(super) events: BTreeMap<String, WirePath>,
}

/// What a schema holds, before the fields with nothing to rename or follow
/// are dropped: they are kept until then so that a field that is a point
/// in time in a variant and plain text in another is refused.
#[derive(Clone, Debug)]
enum Found {
    Time,
    /// An object whose keys are data: a free value, or a map whose values
    /// hold nothing.
    Opaque,
    Type(String),
    Fields(BTreeMap<String, Option<Found>>),
    Items(Box<Found>),
    Values(Box<Found>),
    Union {
        tag: String,
        known: Vec<String>,
        path: Option<Box<Found>>,
        common: Option<Box<Found>>,
    },
}

impl Found {
    /// The runtime's path of what was found; `None` when it holds nothing
    /// to rename or follow.
    fn path(&self) -> Option<WirePath> {
        match self {
            Self::Time => Some(WirePath::Time),
            Self::Opaque => None,
            Self::Type(name) => Some(WirePath::Type(name.clone())),
            Self::Fields(fields) => {
                let fields: BTreeMap<String, Field> = fields
                    .iter()
                    .filter_map(|(key, found)| {
                        let name = camel(key);
                        let path = found.as_ref().and_then(Self::path);
                        (name != *key || path.is_some())
                            .then(|| (key.clone(), Field { name, path }))
                    })
                    .collect();
                (!fields.is_empty()).then_some(WirePath::Fields(fields))
            }
            Self::Items(found) => found.path().map(|path| WirePath::Items(Box::new(path))),
            Self::Values(found) => found.path().map(|path| WirePath::Values(Box::new(path))),
            Self::Union {
                tag,
                known,
                path,
                common,
            } => Some(WirePath::Union {
                tag: tag.clone(),
                known: known.clone(),
                path: path.as_ref().and_then(|found| found.path()).map(Box::new),
                common: common.as_ref().and_then(|found| found.path()).map(Box::new),
            }),
        }
    }
}

/// Whether `schema` is a point in time itself.
fn is_time(schema: &Value) -> bool {
    schema.get("format").and_then(Value::as_str) == Some(DATE_TIME)
}

/// The name a `$ref` of `schema` points to among the definitions.
fn referenced(schema: &Value) -> Option<&str> {
    schema
        .get("$ref")
        .and_then(Value::as_str)
        .and_then(|reference| reference.strip_prefix("#/$defs/"))
}

/// Whether `schema` may be an object whose keys no schema declares.
fn may_be_free_object(schema: &Map<String, Value>) -> bool {
    let combined = ["$ref", "anyOf", "oneOf", "allOf", "const", "enum"]
        .iter()
        .any(|keyword| schema.contains_key(*keyword));
    !combined
        && match schema.get("type") {
            None => true,
            Some(Value::String(kind)) => kind == "object",
            Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "object"),
            Some(_) => false,
        }
}

/// Whether `schema` holds a point in time or, unless `times_only`, a field
/// to rename, itself or through a type of `holding` it references.
fn holds(schema: &Value, holding: &BTreeSet<String>, times_only: bool) -> bool {
    let object = match schema {
        Value::Object(object) => object,
        Value::Array(items) => return items.iter().any(|item| holds(item, holding, times_only)),
        _ => return false,
    };
    if is_time(schema) || referenced(schema).is_some_and(|name| holding.contains(name)) {
        return true;
    }
    if !times_only && OpenUnion::of(schema).is_some() {
        return true;
    }
    object.iter().any(|(keyword, value)| {
        if VALUES.contains(&keyword.as_str()) {
            return false;
        }
        if keyword == "properties" {
            return value.as_object().is_some_and(|properties| {
                properties.iter().any(|(key, property)| {
                    (!times_only && camel(key) != *key) || holds(property, holding, times_only)
                })
            });
        }
        holds(value, holding, times_only)
    })
}

/// The definitions that hold something, by `holds`.
fn holding(definitions: &Map<String, Value>, times_only: bool) -> BTreeSet<String> {
    let mut holding = BTreeSet::new();
    loop {
        let found: Vec<String> = definitions
            .iter()
            .filter(|(name, schema)| {
                !holding.contains(*name) && holds(schema, &holding, times_only)
            })
            .map(|(name, _)| name.clone())
            .collect();
        if found.is_empty() {
            return holding;
        }
        holding.extend(found);
    }
}

/// The definitions a table is made from, which of them hold something,
/// and which hold a point in time.
struct Walk<'a> {
    definitions: &'a Map<String, Value>,
    holding: BTreeSet<String>,
    timed: BTreeSet<String>,
}

impl<'a> Walk<'a> {
    fn new(definitions: &'a Map<String, Value>) -> Self {
        Self {
            definitions,
            holding: holding(definitions, false),
            timed: holding(definitions, true),
        }
    }

    /// What a reference to the type `name` holds.
    fn reference(&self, name: &str) -> Option<Found> {
        if !self.holding.contains(name) {
            return None;
        }
        Some(if self.definitions.get(name).is_some_and(is_time) {
            Found::Time
        } else {
            Found::Type(name.to_owned())
        })
    }

    /// Whether what was found holds a point in time.
    fn timed(&self, found: &Found) -> bool {
        match found {
            Found::Time => true,
            Found::Opaque => false,
            Found::Type(name) => self.timed.contains(name),
            Found::Fields(fields) => fields.values().flatten().any(|found| self.timed(found)),
            Found::Items(found) | Found::Values(found) => self.timed(found),
            Found::Union { path, .. } => path.as_ref().is_some_and(|found| self.timed(found)),
        }
    }

    /// What `schema` holds: an open tagged union with what its members hold
    /// and what the fields they all share hold, or as `shape` finds.
    fn found(&self, schema: &Value) -> Result<Option<Found>> {
        let Some(OpenUnion { tag, known }) = OpenUnion::of(schema) else {
            return self.shape(schema);
        };
        if camel(&tag) != tag {
            bail!("the open union's tag `{tag}` is spelled otherwise in TypeScript");
        }
        let mut shared = schema.clone();
        if let Some(object) = shared.as_object_mut() {
            object.remove("oneOf");
        }
        Ok(Some(Found::Union {
            tag,
            known,
            path: self.shape(schema)?.map(Box::new),
            common: self.shape(&shared)?.map(Box::new),
        }))
    }

    /// What `schema` holds, by its fields, items, values and variants.
    fn shape(&self, schema: &Value) -> Result<Option<Found>> {
        let Some(object) = schema.as_object() else {
            // `true` lets any value through: its keys are data.
            return Ok((schema == &Value::Bool(true)).then_some(Found::Opaque));
        };
        for keyword in UNFOLLOWED {
            if object
                .get(keyword)
                .is_some_and(|sub| holds(sub, &self.holding, false))
            {
                bail!(
                    "a field to rename or a point in time under `{keyword}`, which the wire table cannot follow"
                );
            }
        }
        let mut parts: Vec<Found> = Vec::new();
        if is_time(schema) {
            parts.push(Found::Time);
        }
        if let Some(found) = referenced(schema).and_then(|name| self.reference(name)) {
            parts.push(found);
        }
        let properties = object.get("properties").and_then(Value::as_object);
        if let Some(properties) = properties {
            let mut fields = BTreeMap::new();
            for (key, property) in properties {
                let found = self
                    .found(property)
                    .with_context(|| format!("the field `{key}`"))?;
                fields.insert(key.clone(), found);
            }
            parts.push(Found::Fields(fields));
        }
        if let Some(found) = object
            .get("items")
            .map(|items| self.found(items))
            .transpose()?
        {
            parts.extend(found.map(|found| Found::Items(Box::new(found))));
        }
        if let Some(found) = object
            .get("additionalProperties")
            .map(|values| self.found(values))
            .transpose()?
            .flatten()
        {
            // A map's keys are data; beside declared fields, other keys
            // pass as they are and only values that hold something count.
            if found.path().is_some() || properties.is_none() {
                parts.push(Found::Values(Box::new(found)));
            }
        }
        for keyword in ["anyOf", "oneOf", "allOf"] {
            let Some(variants) = object.get(keyword).and_then(Value::as_array) else {
                continue;
            };
            let found: Vec<Option<Found>> = variants
                .iter()
                .map(|variant| self.found(variant))
                .collect::<Result<_>>()?;
            if keyword != "allOf" && found.iter().any(|found| matches!(found, Some(Found::Time))) {
                let text = variants
                    .iter()
                    .zip(&found)
                    .any(|(variant, found)| found.is_none() && may_be_text(variant));
                if text {
                    bail!("a point in time beside a variant that may be other text");
                }
            }
            parts.extend(found.into_iter().flatten());
        }
        if parts.is_empty() && may_be_free_object(object) {
            return Ok(Some(Found::Opaque));
        }
        parts.into_iter().try_fold(None, |merged, part| {
            Ok(Some(match merged {
                None => part,
                Some(merged) => self.merge(merged, part)?,
            }))
        })
    }

    /// What two parts of one value hold, together.
    fn merge(&self, left: Found, right: Found) -> Result<Found> {
        Ok(match (left, right) {
            (Found::Time, Found::Time) => Found::Time,
            (Found::Opaque, Found::Opaque) => Found::Opaque,
            (Found::Opaque, other) | (other, Found::Opaque) => {
                if other.path().is_some() {
                    bail!("declared fields beside a value whose keys are data: {other:?}");
                }
                Found::Opaque
            }
            (Found::Type(left), Found::Type(right)) if left == right => Found::Type(left),
            (Found::Items(left), Found::Items(right)) => {
                Found::Items(Box::new(self.merge(*left, *right)?))
            }
            (Found::Values(left), Found::Values(right)) => {
                Found::Values(Box::new(self.merge(*left, *right)?))
            }
            (Found::Fields(mut left), Found::Fields(right)) => {
                for (key, right) in right {
                    // A field one part lacks takes the other's.
                    let Some(found) = left.remove(&key) else {
                        left.insert(key, right);
                        continue;
                    };
                    let merged = match (found, right) {
                        (None, None) => None,
                        (Some(found), None) | (None, Some(found)) => {
                            if self.timed(&found) {
                                bail!(
                                    "the field `{key}` holds a point in time in one variant and not in another"
                                );
                            }
                            Some(found)
                        }
                        (Some(left), Some(right)) => Some(
                            self.merge(left, right)
                                .with_context(|| format!("the field `{key}`"))?,
                        ),
                    };
                    left.insert(key, merged);
                }
                Found::Fields(left)
            }
            (Found::Type(name), other) | (other, Found::Type(name)) => {
                let expanded = self
                    .definitions
                    .get(&name)
                    .map(|schema| self.found(schema))
                    .transpose()?
                    .flatten();
                match expanded {
                    Some(expanded) if !matches!(expanded, Found::Type(_)) => {
                        self.merge(expanded, other)?
                    }
                    _ => bail!("the type `{name}` and another shape in one value"),
                }
            }
            (left, right) => bail!("two shapes in one value: {left:?} and {right:?}"),
        })
    }
}

/// Whether a variant that holds no point in time may still be text, which
/// the runtime could not tell from one.
fn may_be_text(variant: &Value) -> bool {
    match variant.get("type") {
        Some(Value::String(kind)) => kind == "string",
        Some(Value::Array(kinds)) => kinds.iter().any(|kind| kind == "string"),
        _ => variant.get("const").is_none_or(Value::is_string),
    }
}

/// The table of `definitions`: the path of each type `own` names that
/// holds something, of each of `operations`' params and result that does,
/// an operation given with its params' and its result's types, and of each
/// of `events`' payload that does, an event given with its payload's type.
pub(super) fn wire<'a>(
    definitions: &Map<String, Value>,
    own: impl Fn(&str) -> bool,
    operations: impl IntoIterator<Item = (&'a str, &'a str, &'a str)>,
    events: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<Wire> {
    let walk = Walk::new(definitions);
    let mut types = BTreeMap::new();
    for (name, schema) in definitions {
        if !own(name) || !walk.holding.contains(name) || is_time(schema) {
            continue;
        }
        let path = walk
            .found(schema)
            .with_context(|| format!("the type `{name}`"))?
            .and_then(|found| found.path());
        let Some(path) = path else {
            bail!(
                "the type `{name}` holds a field to rename or a point in time where the wire table does not look"
            );
        };
        types.insert(name.clone(), path);
    }
    let path_of = |name: &str| walk.reference(name).and_then(|found| found.path());
    let mut params = BTreeMap::new();
    let mut results = BTreeMap::new();
    for (operation, params_type, result_type) in operations {
        if let Some(path) = path_of(params_type) {
            params.insert(operation.to_owned(), path);
        }
        if let Some(path) = path_of(result_type) {
            results.insert(operation.to_owned(), path);
        }
    }
    let events = events
        .into_iter()
        .filter_map(|(event, payload)| Some((event.to_owned(), path_of(payload)?)))
        .collect();
    Ok(Wire {
        types,
        params,
        results,
        events,
    })
}

/// Where a value schema that references no type, such as a setting's,
/// holds fields to rename or points in time; `None` when it holds neither.
pub(super) fn inline_path(schema: &Value) -> Result<Option<WirePath>> {
    let definitions = Map::new();
    Ok(Walk::new(&definitions)
        .found(schema)?
        .and_then(|found| found.path()))
}

/// Names each field a generator's input declares in camelCase, in each
/// object's `properties` and `required`, and leaves values (`const`,
/// `enum`, `default`, `examples`) as they are. Two fields of one object
/// that spell one name are refused.
pub(super) fn camel_fields(input: &mut Value) -> Result<()> {
    match input {
        Value::Object(object) => {
            for (keyword, value) in object.iter_mut() {
                match (keyword.as_str(), value) {
                    (keyword, _) if VALUES.contains(&keyword) => {}
                    ("properties", Value::Object(properties)) => {
                        let mut named = Map::new();
                        for (key, mut property) in std::mem::take(properties) {
                            camel_fields(&mut property)
                                .with_context(|| format!("the field `{key}`"))?;
                            let name = camel(&key);
                            if named.contains_key(&name) {
                                bail!(
                                    "two fields are named `{name}` in TypeScript, one of them `{key}`"
                                );
                            }
                            named.insert(name, property);
                        }
                        *properties = named;
                    }
                    ("required", Value::Array(names)) => {
                        for name in names.iter_mut() {
                            if let Value::String(text) = name {
                                *text = camel(text);
                            }
                        }
                    }
                    (keyword, value) => {
                        camel_fields(value).with_context(|| format!("under `{keyword}`"))?;
                    }
                }
            }
            Ok(())
        }
        Value::Array(items) => items.iter_mut().try_for_each(camel_fields),
        _ => Ok(()),
    }
}

/// Types each point in time of a generator's input as a `Date`, or
/// `Date | null` where the schema also allows null.
pub(super) fn with_date_types(input: &mut Value) {
    match input {
        Value::Object(object) => {
            if object.get("format").and_then(Value::as_str) == Some(DATE_TIME) {
                let nullable = object
                    .get("type")
                    .and_then(Value::as_array)
                    .is_some_and(|kinds| kinds.iter().any(|kind| kind == "null"));
                let date = if nullable { "Date | null" } else { "Date" };
                object.insert("tsType".to_owned(), Value::String(date.to_owned()));
            }
            object.values_mut().for_each(with_date_types);
        }
        Value::Array(items) => items.iter_mut().for_each(with_date_types),
        _ => {}
    }
}

/// `text` as a TypeScript string literal.
fn literal(text: &str) -> String {
    Value::String(text.to_owned()).to_string()
}

/// `path` as the runtime's `WirePath` value.
pub(super) fn path_value(path: &WirePath) -> String {
    match path {
        WirePath::Time => literal("time"),
        WirePath::Type(name) => format!("{{ type: {} }}", literal(name)),
        WirePath::Fields(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(key, field)| {
                    let value = match &field.path {
                        None => literal(&field.name),
                        Some(path) => format!("[{}, {}]", literal(&field.name), path_value(path)),
                    };
                    format!("{}: {value}", literal(key))
                })
                .collect();
            format!("{{ fields: {{ {} }} }}", fields.join(", "))
        }
        WirePath::Items(path) => format!("{{ items: {} }}", path_value(path)),
        WirePath::Values(path) => format!("{{ values: {} }}", path_value(path)),
        WirePath::Union {
            tag,
            known,
            path,
            common,
        } => {
            let known: Vec<String> = known.iter().map(|name| literal(name)).collect();
            let entry = |name: &str, path: &Option<Box<WirePath>>| {
                path.as_ref()
                    .map(|path| format!(", {name}: {}", path_value(path)))
                    .unwrap_or_default()
            };
            format!(
                "{{ union: {{ tag: {}, known: [{}]{}{} }} }}",
                literal(tag),
                known.join(", "),
                entry("path", path),
                entry("common", common)
            )
        }
    }
}

/// The name of `product`'s table.
fn table_name(product: Product) -> String {
    format!("{}Wire", product.name().to_lowercase())
}

/// One section of the table, each entry by its literal key.
fn section(
    text: &mut String,
    name: &str,
    spread: Option<&str>,
    paths: &BTreeMap<String, WirePath>,
) {
    let _ = writeln!(text, "  {name}: {{");
    if let Some(spread) = spread {
        let _ = writeln!(text, "    ...{spread},");
    }
    for (key, path) in paths {
        let _ = writeln!(text, "    {}: {},", literal(key), path_value(path));
    }
    text.push_str("  },\n");
}

/// Whether the type `name`, the layer's own or Collie's, holds fields to
/// rename or points in time, which reading it from the wire converts.
pub(super) fn converts(layer: &Layer, name: &str) -> Result<bool> {
    let table = wire(&reachable(layer), |own| own == name, [], [])?;
    Ok(table.types.contains_key(name))
}

/// The layer's table: where its own types, operations and events hold
/// fields to rename or points in time.
fn table(layer: &Layer) -> Result<Wire> {
    wire(
        &reachable(layer),
        |name| layer.definitions.contains_key(name),
        layer.operations.iter().map(|operation| {
            (
                operation.name.as_str(),
                operation.params.0.as_str(),
                operation.result.0.as_str(),
            )
        }),
        layer
            .events
            .iter()
            .map(|event| (event.name.as_str(), event.payload.0.as_str())),
    )
}

/// Every type the layer's own reach: Collie's, then the layer's, which
/// win over Collie's of the same name.
fn reachable(layer: &Layer) -> Map<String, Value> {
    let mut definitions = layer.collie_definitions.clone();
    definitions.extend(layer.definitions.clone());
    definitions
}

/// The layer's `wire.ts`: Collie's table, or Foxhound's, which adds its
/// own types to Collie's.
pub(super) fn file(layer: &Layer) -> Result<String> {
    let table = table(layer)?;
    let product = layer.product.name();
    let mut text = format!("{HEADER}\n");
    match layer.product {
        Product::Collie => text.push_str("import type { Wire } from \"../wire.ts\";\n"),
        Product::Foxhound => text.push_str(
            "import { collieWire, type CollieWireType, type Wire } from \"@goodfetch/collie-client/runtime\";\n",
        ),
    }
    let _ = writeln!(
        text,
        "import type {{ {product}EventPayloads }} from \"./events.ts\";\nimport type {{ {product}OperationTypes }} from \"./operations.ts\";\n"
    );
    let mut names: Vec<String> = table.types.keys().map(|name| literal(name)).collect();
    if layer.product == Product::Foxhound {
        names.insert(0, "CollieWireType".to_owned());
    }
    if names.is_empty() {
        names.push("never".to_owned());
    }
    let _ = writeln!(
        text,
        "/** The names of the types {product}'s table holds. */\nexport type {product}WireType =\n  | {};\n",
        names.join("\n  | ")
    );
    let _ = writeln!(
        text,
        "/** Where {product}'s types, operation params and results and event payloads hold fields the engine names otherwise, and points in time, which a program reads as `Date`s (ADRs 0092 and 0099). */"
    );
    let _ = writeln!(
        text,
        "export const {}: Wire<keyof {product}OperationTypes & string, keyof {product}EventPayloads & string, {product}WireType> = {{",
        table_name(layer.product)
    );
    let spread = (layer.product == Product::Foxhound).then_some("collieWire.types");
    section(&mut text, "types", spread, &table.types);
    section(&mut text, "params", None, &table.params);
    section(&mut text, "results", None, &table.results);
    section(&mut text, "events", None, &table.events);
    text.push_str("};\n");
    Ok(text)
}

#[cfg(test)]
mod tests;
