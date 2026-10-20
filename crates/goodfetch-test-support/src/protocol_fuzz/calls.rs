//! The calls protocol fuzzing sends, generated from the operations' params
//! schemas: a valid call meets its operation's schema, and an invalid one
//! breaks it in one named way. Each generated call is checked against the
//! schema itself, so a keyword the generator does not know never makes a
//! valid call invalid or an invalid one valid.

use std::sync::Arc;

use proptest::prelude::*;
use proptest::strategy::{SBoxedStrategy, Union};
use proptest::string::string_regex;
use serde_json::{Map, Value, json};

use super::schema::Contract;

/// How deep the generator follows a schema's nesting before it stops adding
/// optional properties and items.
const MAX_DEPTH: u32 = 6;
/// The most items a generated array holds beyond its `minItems`.
const EXTRA_ITEMS: usize = 3;
/// The longest generated string without a `maxLength` or a `pattern`.
const STRING_LENGTH: usize = 12;

/// One call protocol fuzzing sends: an operation's method and its params,
/// and how the params break the operation's schema, if they do.
#[derive(Clone, Debug, PartialEq)]
pub struct FuzzedCall {
    method: String,
    params: Value,
    invalidity: Option<Invalidity>,
}

impl FuzzedCall {
    /// The method it calls.
    pub fn method(&self) -> &str {
        &self.method
    }

    /// Its params.
    pub fn params(&self) -> &Value {
        &self.params
    }

    /// How it breaks its operation's schema; `None` for a valid call.
    pub fn invalidity(&self) -> Option<&Invalidity> {
        self.invalidity.as_ref()
    }

    /// The request that sends it with `id`.
    pub(super) fn request(&self, id: u64) -> Value {
        json!({ "jsonrpc": "2.0", "id": id, "method": self.method, "params": self.params })
    }
}

/// How an invalid call breaks its operation's schema.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Invalidity {
    /// Its method is no operation of the catalog.
    UnknownOperation,
    /// Its params are a JSON value other than an object.
    ParamsNotAnObject,
    /// Its params lack a property the schema requires.
    MissingProperty {
        /// The property.
        property: String,
    },
    /// Its params have a property the schema does not allow.
    UnknownProperty {
        /// The property.
        property: String,
    },
    /// A property of its params holds a value the property's schema refuses.
    RefusedValue {
        /// The property.
        property: String,
    },
}

/// Every call of the sequences: valid calls of the operations named in
/// `valid`, and invalid calls of every operation and of unknown ones.
pub(super) fn calls(contract: &Arc<Contract>, valid: &[String]) -> SBoxedStrategy<FuzzedCall> {
    let mut invalid: Vec<SBoxedStrategy<FuzzedCall>> = contract
        .operations
        .iter()
        .enumerate()
        .flat_map(|(index, _)| invalid_calls(contract, index))
        .collect();
    invalid.push(unknown_operations(contract));
    let invalid = Union::new(invalid).sboxed();
    let valid: Vec<SBoxedStrategy<FuzzedCall>> = contract
        .operations
        .iter()
        .enumerate()
        .filter(|(_, operation)| valid.contains(&operation.name))
        .map(|(index, _)| valid_calls(contract, index))
        .collect();
    if valid.is_empty() {
        return invalid;
    }
    Union::new_weighted(vec![(1, Union::new(valid).sboxed()), (3, invalid)]).sboxed()
}

/// Valid calls of the operation at `index`.
fn valid_calls(contract: &Arc<Contract>, index: usize) -> SBoxedStrategy<FuzzedCall> {
    let operation = &contract.operations[index];
    let method = operation.name.clone();
    let checked = Arc::clone(contract);
    Generator::new(&contract.defs)
        .valid(&operation.params, 0)
        .prop_filter("the generated params meet the schema", move |params| {
            checked.operations[index].takes(params)
        })
        .prop_map(move |params| FuzzedCall {
            method: method.clone(),
            params,
            invalidity: None,
        })
        .sboxed()
}

/// Each way the params of the operation at `index` can break its schema,
/// one strategy per way.
fn invalid_calls(contract: &Arc<Contract>, index: usize) -> Vec<SBoxedStrategy<FuzzedCall>> {
    let operation = &contract.operations[index];
    let generator = Generator::new(&contract.defs);
    let schema = generator.resolve(&operation.params);
    let valid = generator.valid(&operation.params, 0);
    let mut ways: Vec<SBoxedStrategy<(Value, Invalidity)>> = Vec::new();
    if schema.get("type") == Some(&Value::from("object")) {
        ways.push(
            not_an_object()
                .prop_map(|params| (params, Invalidity::ParamsNotAnObject))
                .sboxed(),
        );
    }
    let properties = schema
        .get("properties")
        .and_then(Value::as_object)
        .cloned()
        .unwrap_or_default();
    for required in strings(schema.get("required")) {
        ways.push(
            valid
                .clone()
                .prop_map(move |mut params| {
                    if let Some(object) = params.as_object_mut() {
                        object.remove(&required);
                    }
                    let invalidity = Invalidity::MissingProperty {
                        property: required.clone(),
                    };
                    (params, invalidity)
                })
                .sboxed(),
        );
    }
    if schema.get("additionalProperties") == Some(&Value::Bool(false)) {
        let known: Vec<String> = properties.keys().cloned().collect();
        let name = regex("[a-z_]{1,12}")
            .prop_filter("a property the schema does not name", move |name| {
                !known.contains(name)
            });
        ways.push(
            (valid.clone(), name, any_json())
                .prop_map(|(mut params, name, value)| {
                    if let Some(object) = params.as_object_mut() {
                        object.insert(name.clone(), value);
                    }
                    (params, Invalidity::UnknownProperty { property: name })
                })
                .sboxed(),
        );
    }
    for (property, property_schema) in &properties {
        if accepts_anything(generator.resolve(property_schema)) {
            continue;
        }
        let property = property.clone();
        ways.push(
            (valid.clone(), any_json())
                .prop_map(move |(mut params, value)| {
                    if let Some(object) = params.as_object_mut() {
                        object.insert(property.clone(), value);
                    }
                    let invalidity = Invalidity::RefusedValue {
                        property: property.clone(),
                    };
                    (params, invalidity)
                })
                .sboxed(),
        );
    }
    ways.into_iter()
        .map(|way| {
            let checked = Arc::clone(contract);
            let method = operation.name.clone();
            way.prop_filter("the params break the schema", move |(params, _)| {
                !checked.operations[index].takes(params)
            })
            .prop_map(move |(params, invalidity)| FuzzedCall {
                method: method.clone(),
                params,
                invalidity: Some(invalidity),
            })
            .sboxed()
        })
        .collect()
}

/// Calls of methods no operation of the catalog has.
fn unknown_operations(contract: &Arc<Contract>) -> SBoxedStrategy<FuzzedCall> {
    let checked = Arc::clone(contract);
    regex("[a-z]{1,10}\\.[a-z_]{1,10}")
        .prop_filter("a method the catalog does not have", move |method| {
            checked.operation(method).is_none()
        })
        .prop_map(|method| FuzzedCall {
            method,
            params: Value::Object(Map::new()),
            invalidity: Some(Invalidity::UnknownOperation),
        })
        .sboxed()
}

/// Params that are JSON but no object (`null` aside, which some Clients
/// send for absent params).
fn not_an_object() -> SBoxedStrategy<Value> {
    prop_oneof![
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        regex("(?s).{0,12}").prop_map(Value::from),
        proptest::collection::vec(any_json(), 0..3).prop_map(Value::Array),
    ]
    .sboxed()
}

/// Any JSON value, small and at most two levels deep. Proptest's
/// `prop_recursive` would box it as a strategy no other thread may hold.
fn any_json() -> SBoxedStrategy<Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        (-1.0e9..1.0e9f64).prop_map(Value::from),
        regex("(?s).{0,12}").prop_map(Value::from),
    ]
    .sboxed();
    let nest = |inner: SBoxedStrategy<Value>| {
        prop_oneof![
            3 => leaf.clone(),
            1 => proptest::collection::vec(inner.clone(), 0..3).prop_map(Value::Array),
            1 => proptest::collection::btree_map(regex("[a-z_]{1,8}"), inner, 0..3)
                .prop_map(|members| Value::Object(members.into_iter().collect())),
        ]
        .sboxed()
    };
    nest(nest(leaf.clone()))
}

/// Strings of `pattern`, which proptest's generator reads.
fn regex(pattern: &str) -> SBoxedStrategy<String> {
    string_regex(pattern)
        .unwrap_or_else(|error| {
            panic!("invariant: `{pattern}` is a pattern proptest reads: {error}")
        })
        .sboxed()
}

/// Whether `schema` accepts every value, so no value of its property is
/// refused.
fn accepts_anything(schema: &Value) -> bool {
    match schema {
        Value::Bool(accepts) => *accepts,
        Value::Object(keywords) => keywords
            .keys()
            .all(|keyword| matches!(keyword.as_str(), "description" | "arrived_in" | "title")),
        _ => false,
    }
}

/// The texts of a JSON array of strings, such as `required`.
fn strings(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Generates values that meet a schema whose references point into the
/// bundle's definitions.
struct Generator<'a> {
    defs: &'a Map<String, Value>,
}

impl<'a> Generator<'a> {
    fn new(defs: &'a Map<String, Value>) -> Self {
        Self { defs }
    }

    /// `schema`, or the definition it references.
    fn resolve(&self, schema: &'a Value) -> &'a Value
    where
        Self: 'a,
    {
        let mut schema = schema;
        while let Some(name) = schema
            .get("$ref")
            .and_then(Value::as_str)
            .and_then(|reference| reference.strip_prefix("#/$defs/"))
        {
            match self.defs.get(name) {
                Some(definition) => schema = definition,
                None => break,
            }
        }
        schema
    }

    /// Values that meet `schema`, `depth` levels into a call's params.
    fn valid(&self, schema: &'a Value, depth: u32) -> SBoxedStrategy<Value> {
        let schema = self.resolve(schema);
        let Some(keywords) = schema.as_object() else {
            return any_json();
        };
        if let Some(value) = keywords.get("const") {
            return Just(value.clone()).sboxed();
        }
        if let Some(values) = keywords.get("enum").and_then(Value::as_array) {
            return proptest::sample::select(values.clone()).sboxed();
        }
        for combinator in ["oneOf", "anyOf"] {
            if let Some(branches) = keywords.get(combinator).and_then(Value::as_array) {
                let branches = branches
                    .iter()
                    .map(|branch| self.valid(branch, depth + 1))
                    .collect::<Vec<_>>();
                return Union::new(branches).sboxed();
            }
        }
        match keywords.get("type") {
            Some(Value::String(kind)) => self.of_type(kind, keywords, depth),
            Some(Value::Array(kinds)) => Union::new(
                kinds
                    .iter()
                    .filter_map(Value::as_str)
                    .map(|kind| self.of_type(kind, keywords, depth)),
            )
            .sboxed(),
            _ => any_json(),
        }
    }

    /// Values of the JSON type `kind` that meet `keywords`.
    fn of_type(
        &self,
        kind: &str,
        keywords: &'a Map<String, Value>,
        depth: u32,
    ) -> SBoxedStrategy<Value> {
        match kind {
            "object" => self.object(keywords, depth),
            "array" => self.array(keywords, depth),
            "string" => string(keywords),
            "integer" => integer(keywords),
            "number" => (-1.0e9..1.0e9f64).prop_map(Value::from).sboxed(),
            "boolean" => any::<bool>().prop_map(Value::from).sboxed(),
            _ => Just(Value::Null).sboxed(),
        }
    }

    /// Objects with every required property and some of the others.
    fn object(&self, keywords: &'a Map<String, Value>, depth: u32) -> SBoxedStrategy<Value> {
        let required = strings(keywords.get("required"));
        let members: Vec<SBoxedStrategy<Option<(String, Value)>>> = keywords
            .get("properties")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .filter(|(name, _)| depth < MAX_DEPTH || required.contains(name))
            .map(|(name, schema)| {
                let name = name.clone();
                let value = self.valid(schema, depth + 1);
                if required.contains(&name) {
                    value
                        .prop_map(move |value| Some((name.clone(), value)))
                        .sboxed()
                } else {
                    proptest::option::of(value.prop_map(move |value| (name.clone(), value)))
                        .sboxed()
                }
            })
            .collect();
        members
            .prop_map(|members| Value::Object(members.into_iter().flatten().collect()))
            .sboxed()
    }

    /// Arrays of `items` between `minItems` and `maxItems` long.
    fn array(&self, keywords: &'a Map<String, Value>, depth: u32) -> SBoxedStrategy<Value> {
        let length = |keyword: &str| {
            keywords
                .get(keyword)
                .and_then(Value::as_u64)
                .and_then(|length| usize::try_from(length).ok())
        };
        let min = length("minItems").unwrap_or(0);
        let extra = if depth < MAX_DEPTH { EXTRA_ITEMS } else { 0 };
        let max = length("maxItems").unwrap_or(usize::MAX).min(min + extra);
        let items = keywords
            .get("items")
            .map_or_else(any_json, |items| self.valid(items, depth + 1));
        proptest::collection::vec(items, min..=max)
            .prop_map(Value::Array)
            .sboxed()
    }
}

/// Strings of the `pattern`, or of any characters within the length.
fn string(keywords: &Map<String, Value>) -> SBoxedStrategy<Value> {
    let anchored = keywords
        .get("pattern")
        .and_then(Value::as_str)
        .map(|pattern| {
            let pattern = pattern.strip_prefix('^').unwrap_or(pattern);
            pattern.strip_suffix('$').unwrap_or(pattern).to_owned()
        });
    if let Some(pattern) = anchored.and_then(|pattern| string_regex(&pattern).ok()) {
        return pattern.prop_map(Value::from).sboxed();
    }
    let min = keywords
        .get("minLength")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let max = keywords
        .get("maxLength")
        .and_then(Value::as_u64)
        .unwrap_or(u64::MAX)
        .min(min + STRING_LENGTH as u64);
    regex(&format!("(?s).{{{min},{max}}}"))
        .prop_map(Value::from)
        .sboxed()
}

/// Integers within `minimum`, `maximum` and the bounds of their `format`.
fn integer(keywords: &Map<String, Value>) -> SBoxedStrategy<Value> {
    let (format_min, format_max) = match keywords.get("format").and_then(Value::as_str) {
        Some("uint8") => (0, i64::from(u8::MAX)),
        Some("uint16") => (0, i64::from(u16::MAX)),
        Some("uint32") => (0, i64::from(u32::MAX)),
        Some("uint64" | "uint") => (0, i64::MAX),
        Some("int8") => (i64::from(i8::MIN), i64::from(i8::MAX)),
        Some("int16") => (i64::from(i16::MIN), i64::from(i16::MAX)),
        Some("int32") => (i64::from(i32::MIN), i64::from(i32::MAX)),
        _ => (i64::MIN, i64::MAX),
    };
    let bound = |keyword: &str| keywords.get(keyword).and_then(Value::as_i64);
    let min = bound("minimum").map_or(format_min, |minimum| minimum.max(format_min));
    let max = bound("maximum").map_or(format_max, |maximum| maximum.min(format_max));
    (min..=max).prop_map(Value::from).sboxed()
}
