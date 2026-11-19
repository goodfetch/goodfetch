//! Points in time in the TypeScript layer (ADRs 0092 and 0099): each
//! `date-time` is typed as a `Date`, and `times.ts` holds the table the
//! Connection reads results by, which says where each type and each
//! operation's result holds a point in time, so that the RFC 3339 text the
//! engine sends there reaches the program as a `Date`. A type is named in
//! the table only when it holds one, directly or through the types it
//! references; a type that is a point in time itself, such as
//! `PointInTime`, is written as `"time"` where it is referenced. Foxhound's
//! table adds its own types to Collie's.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use serde_json::{Map, Value};

use super::super::layer::Layer;
use super::HEADER;

/// The format a point in time takes in the catalog's schemas.
const DATE_TIME: &str = "date-time";

/// The keywords that hold subschemas the table has no way to follow; a
/// point in time under one is refused rather than left as text.
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

/// Where a value holds points in time, as the runtime's `TimePath` writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum TimePath {
    /// The value is one.
    Time,
    /// The value is of a type the table names.
    Type(String),
    /// The object's fields that hold one.
    Fields(BTreeMap<String, TimePath>),
    /// Each item of the list.
    Items(Box<TimePath>),
    /// Each value of the map.
    Values(Box<TimePath>),
}

/// A layer's table of points in time.
#[derive(Debug)]
pub(super) struct Times {
    /// The path of each of the layer's own types that holds a point in time.
    pub(super) types: BTreeMap<String, TimePath>,
    /// The path of each operation's result that holds one, by operation.
    pub(super) results: BTreeMap<String, TimePath>,
}

/// What a schema holds, before the fields without a point in time are
/// dropped: they are kept until then so that a field that is one in a
/// variant and plain text in another is refused.
#[derive(Clone, Debug)]
enum Found {
    Time,
    Type(String),
    Fields(BTreeMap<String, Option<Found>>),
    Items(Box<Found>),
    Values(Box<Found>),
}

impl Found {
    /// The runtime's path of what was found; `None` when it holds no point in time.
    fn path(&self) -> Option<TimePath> {
        match self {
            Self::Time => Some(TimePath::Time),
            Self::Type(name) => Some(TimePath::Type(name.clone())),
            Self::Fields(fields) => {
                let fields: BTreeMap<String, TimePath> = fields
                    .iter()
                    .filter_map(|(key, found)| Some((key.clone(), found.as_ref()?.path()?)))
                    .collect();
                (!fields.is_empty()).then_some(TimePath::Fields(fields))
            }
            Self::Items(found) => found.path().map(|path| TimePath::Items(Box::new(path))),
            Self::Values(found) => found.path().map(|path| TimePath::Values(Box::new(path))),
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

/// The definitions a table is made from, and which of them hold a point in time.
struct Walk<'a> {
    definitions: &'a Map<String, Value>,
    holding: BTreeSet<String>,
}

impl<'a> Walk<'a> {
    fn new(definitions: &'a Map<String, Value>) -> Self {
        let mut walk = Self {
            definitions,
            holding: BTreeSet::new(),
        };
        loop {
            let found: Vec<String> = definitions
                .iter()
                .filter(|(name, schema)| !walk.holding.contains(*name) && walk.holds(schema))
                .map(|(name, _)| name.clone())
                .collect();
            if found.is_empty() {
                return walk;
            }
            walk.holding.extend(found);
        }
    }

    /// Whether `schema` holds a point in time, itself or through a type it references.
    fn holds(&self, schema: &Value) -> bool {
        match schema {
            Value::Object(object) => {
                is_time(schema)
                    || referenced(schema).is_some_and(|name| self.holding.contains(name))
                    || object.values().any(|child| self.holds(child))
            }
            Value::Array(items) => items.iter().any(|item| self.holds(item)),
            _ => false,
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

    /// What `schema` holds.
    fn found(&self, schema: &Value) -> Result<Option<Found>> {
        let Some(object) = schema.as_object() else {
            return Ok(None);
        };
        for keyword in UNFOLLOWED {
            if object.get(keyword).is_some_and(|sub| self.holds(sub)) {
                bail!("a point in time under `{keyword}`, which the table of times cannot follow");
            }
        }
        let mut parts: Vec<Found> = Vec::new();
        if is_time(schema) {
            parts.push(Found::Time);
        }
        if let Some(found) = referenced(schema).and_then(|name| self.reference(name)) {
            parts.push(found);
        }
        if let Some(properties) = object.get("properties").and_then(Value::as_object) {
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
        {
            parts.extend(found.map(|found| Found::Values(Box::new(found))));
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
                            if found.path().is_some() {
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
/// holds a point in time, and of each of `results`, an operation and its
/// result's type, that does.
pub(super) fn times<'a>(
    definitions: &Map<String, Value>,
    own: impl Fn(&str) -> bool,
    results: impl IntoIterator<Item = (&'a str, &'a str)>,
) -> Result<Times> {
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
        if let Some(path) = path {
            types.insert(name.clone(), path);
        }
    }
    let results = results
        .into_iter()
        .filter_map(|(operation, result)| {
            let path = walk.reference(result)?.path()?;
            Some((operation.to_owned(), path))
        })
        .collect();
    Ok(Times { types, results })
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

/// `path` as the runtime's `TimePath` value.
fn path_value(path: &TimePath) -> String {
    match path {
        TimePath::Time => literal("time"),
        TimePath::Type(name) => format!("{{ type: {} }}", literal(name)),
        TimePath::Fields(fields) => {
            let fields: Vec<String> = fields
                .iter()
                .map(|(key, path)| format!("{}: {}", literal(key), path_value(path)))
                .collect();
            format!("{{ fields: {{ {} }} }}", fields.join(", "))
        }
        TimePath::Items(path) => format!("{{ items: {} }}", path_value(path)),
        TimePath::Values(path) => format!("{{ values: {} }}", path_value(path)),
    }
}

/// The name of `product`'s table.
fn table_name(product: Product) -> String {
    format!("{}Times", product.name().to_lowercase())
}

/// The layer's `times.ts`: Collie's table, or Foxhound's, which adds its
/// own types to Collie's.
pub(super) fn file(layer: &Layer) -> Result<String> {
    let mut definitions = layer.collie_definitions.clone();
    definitions.extend(layer.definitions.clone());
    let table = times(
        &definitions,
        |name| layer.definitions.contains_key(name),
        layer
            .operations
            .iter()
            .map(|operation| (operation.name.as_str(), operation.result.0.as_str())),
    )?;
    let product = layer.product.name();
    let mut text = format!("{HEADER}\n");
    match layer.product {
        Product::Collie => text.push_str("import type { Times } from \"../times.ts\";\n"),
        Product::Foxhound => text.push_str(
            "import { collieTimes, type Times } from \"@goodfetch/collie-client/runtime\";\n",
        ),
    }
    let _ = writeln!(
        text,
        "import type {{ {product}OperationTypes }} from \"./operations.ts\";\n"
    );
    let _ = writeln!(
        text,
        "/** Where {product}'s types and operation results hold points in time, which a result reads as `Date`s (ADRs 0092 and 0099). */"
    );
    let _ = writeln!(
        text,
        "export const {}: Times<keyof {product}OperationTypes & string> = {{",
        table_name(layer.product)
    );
    text.push_str("  types: {\n");
    if layer.product == Product::Foxhound {
        text.push_str("    ...collieTimes.types,\n");
    }
    for (name, path) in &table.types {
        let _ = writeln!(text, "    {}: {},", literal(name), path_value(path));
    }
    text.push_str("  },\n  results: {\n");
    for (operation, path) in &table.results {
        let _ = writeln!(text, "    {}: {},", literal(operation), path_value(path));
    }
    text.push_str("  },\n};\n");
    Ok(text)
}

#[cfg(test)]
mod tests;
