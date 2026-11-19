//! The Python types of each scope's `settings` option (ADR 0083):
//! `Settings`, `RunSettings`, `SessionSettings` and `PageSettings`, nested
//! `TypedDict`s with every key `NotRequired`, by the wire's own `snake_case`
//! segments, so that a key outside its scope, a misspelled one or a wrong
//! value fails pyright and mypy. A Duration setting takes a `timedelta`,
//! and only a setting that can be cleared takes `None`; the hand-written
//! `settings_on_wire` sends what the program set, durations in whole
//! milliseconds, and checks no value the engine owns.
//!
//! A value schema's object is a `TypedDict` of its own, named after its
//! setting: a tagged alternative after its tag (`SessionSourceStateFile`),
//! a property's after the property, a list's item `Item` and a map's value
//! `Entry`. The module states no `from __future__ import annotations`, so
//! each class comes after the classes it names and `NotRequired` stays
//! readable at run time.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail, ensure};
use goodfetch_protocol::catalog::{SettingScope, ValueKind};
use serde_json::{Map, Value};

use super::super::handles::{Form, Member};
use super::super::layer::{Setting, Stability};
use super::super::naming::{pascal, snake};
use super::{docstring, docstring_header};

/// The optional parameter that takes a scope's settings.
pub(super) const SETTINGS: &str = "settings";

/// The hand-written function that sends a scope's settings as the wire
/// takes them.
pub(super) const SETTINGS_ON_WIRE: &str = "settings_on_wire";

/// The hand-written module of Collie's client package that declares
/// [`SETTINGS_ON_WIRE`].
pub(super) const SETTINGS_ON_WIRE_MODULE: &str = "goodfetch_collie_client.settings";

/// A scope's settings type: the scope, its type's name, the prefix of its
/// areas' classes and its docstring's summary.
struct ScopeType {
    scope: SettingScope,
    name: &'static str,
    areas: &'static str,
    doc: &'static str,
}

/// The scopes a program's code opens with its own `settings`, outermost
/// first.
const SCOPES: [ScopeType; 4] = [
    ScopeType {
        scope: SettingScope::Connection,
        name: "Settings",
        areas: "Connection",
        doc: "The settings of a Connection, which every Run it starts inherits (ADR 0083).",
    },
    ScopeType {
        scope: SettingScope::Run,
        name: "RunSettings",
        areas: "Run",
        doc: "The settings starting a Run takes, which its Sessions and pages inherit (ADR 0083).",
    },
    ScopeType {
        scope: SettingScope::Session,
        name: "SessionSettings",
        areas: "Session",
        doc: "The settings opening a Session takes, over its Run's, which its pages inherit (ADR 0083).",
    },
    ScopeType {
        scope: SettingScope::Page,
        name: "PageSettings",
        areas: "Page",
        doc: "The settings opening a page takes, over its Session's and its Run's (ADR 0083).",
    },
];

/// What each scope's type says of the three states.
const THREE_STATES: &str = "A key left out inherits, `None` clears a setting that can be cleared,\nand a value sets it; durations are `timedelta`s (ADR 0083).";

/// The JSON Schema keywords a setting's value schema may carry that type
/// nothing: the SDK checks no value the engine owns.
const UNTYPED_KEYWORDS: [&str; 11] = [
    "arrived_in",
    "description",
    "format",
    "pattern",
    "maxLength",
    "minLength",
    "maximum",
    "minimum",
    "minItems",
    "maxItems",
    "propertyNames",
];

/// The JSON Schema keywords that give a value schema its Python type.
const TYPED_KEYWORDS: [&str; 9] = [
    "type",
    "const",
    "enum",
    "anyOf",
    "oneOf",
    "items",
    "properties",
    "required",
    "additionalProperties",
];

/// A segment of the settings tree: a setting, or an area of more segments.
enum Node<'a> {
    Setting(&'a Setting),
    Area(BTreeMap<String, Node<'a>>),
}

/// The settings tree of those `settings` that `scope` sets, by wire segment.
fn tree(settings: &[Setting], scope: SettingScope) -> Result<BTreeMap<String, Node<'_>>> {
    let mut root = BTreeMap::new();
    for setting in settings
        .iter()
        .filter(|setting| setting.scopes.iter().any(|named| named == scope.as_str()))
    {
        let segments: Vec<&str> = setting.name.split('.').collect();
        ensure!(
            segments.iter().all(|segment| key(segment)),
            "the setting `{}` has a segment a `TypedDict` cannot declare",
            setting.name
        );
        let (last, areas) = segments
            .split_last()
            .with_context(|| format!("the setting `{}` has no segment", setting.name))?;
        let mut level = &mut root;
        for area in areas {
            let node = level
                .entry((*area).to_owned())
                .or_insert_with(|| Node::Area(BTreeMap::new()));
            level = match node {
                Node::Area(children) => children,
                Node::Setting(other) => bail!(
                    "the setting `{}` is inside the setting `{}`",
                    setting.name,
                    other.name
                ),
            };
        }
        ensure!(
            level
                .insert((*last).to_owned(), Node::Setting(setting))
                .is_none(),
            "the setting `{}` holds other settings",
            setting.name
        );
    }
    Ok(root)
}

/// Whether a `TypedDict` class can declare `name` as a key: an identifier
/// and no keyword, as its own `snake_case` spelling.
fn key(name: &str) -> bool {
    name.chars()
        .next()
        .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && name
            .chars()
            .all(|character| character.is_ascii_alphanumeric() || character == '_')
        && snake(name) == name
}

/// The settings module of Collie's client package: each scope's type.
pub(super) fn settings_module(settings: &[Setting]) -> Result<String> {
    let mut module = Module::default();
    for scope in &SCOPES {
        let level = tree(settings, scope.scope)?;
        module.area(&level, scope, "")?;
    }
    let mut text =
        docstring_header("The settings each scope opens with (ADR 0083), typed from the catalog.");
    text.push_str("\n\n");
    text.push_str(&module.import_lines());
    for class in &module.classes {
        let _ = write!(text, "\n\n{class}");
    }
    Ok(text)
}

/// The type name of the settings `opened`'s option takes.
pub(super) fn scope_type(opened: &str) -> Result<&'static str> {
    SCOPES
        .iter()
        .find(|scope| scope.scope.as_str() == opened)
        .map(|scope| scope.name)
        .with_context(|| format!("`{opened}` opens no scope that takes settings"))
}

/// The settings type of the scope `member` opens, when it takes `settings`;
/// an error when the scope it opens takes none.
pub(super) fn scope_settings(member: &Member<'_>) -> Result<Option<&'static str>> {
    match &member.form {
        Form::Opens(opened)
            if member
                .parameters
                .optional
                .iter()
                .any(|name| name == SETTINGS) =>
        {
            scope_type(opened).map(Some)
        }
        _ => Ok(None),
    }
}

/// One member of a union: a literal value, which joins the union's one
/// `Literal`, or a type.
enum Arm {
    Literal(String),
    Type(String),
}

/// The module's classes, in the order each comes after those it names, and
/// the names its annotations take from outside it, each as its module and
/// name.
#[derive(Default)]
struct Module {
    classes: Vec<String>,
    defined: BTreeMap<String, usize>,
    imports: BTreeSet<(&'static str, &'static str)>,
}

impl Module {
    /// The import lines of the names the classes take, `TypedDict` always.
    fn import_lines(&self) -> String {
        let mut by_module = BTreeMap::<&str, Vec<&str>>::new();
        // `TypedDict` sorts after the other names `typing` gives.
        for (module, name) in self.imports.iter().chain([&("typing", "TypedDict")]) {
            by_module.entry(module).or_default().push(name);
        }
        let mut lines = String::new();
        for (module, names) in by_module {
            let _ = writeln!(lines, "from {module} import {}", names.join(", "));
        }
        lines
    }

    /// Defines the class `name` once: a second definition must be the same.
    fn define(&mut self, name: &str, class: String) -> Result<()> {
        if let Some(&index) = self.defined.get(name) {
            ensure!(
                self.classes[index] == class,
                "two different settings classes are both named `{name}`"
            );
            return Ok(());
        }
        self.defined.insert(name.to_owned(), self.classes.len());
        self.classes.push(class);
        Ok(())
    }

    /// The class of an area of `scope`'s settings at `path`, or of the
    /// scope's own type when `path` is empty, after the classes it names.
    fn area(
        &mut self,
        level: &BTreeMap<String, Node<'_>>,
        scope: &ScopeType,
        path: &str,
    ) -> Result<String> {
        let (name, summary) = if path.is_empty() {
            (
                scope.name.to_owned(),
                format!("{}\n\n{THREE_STATES}", scope.doc),
            )
        } else {
            (
                format!("{}{}Settings", scope.areas, pascal(path)),
                format!("The `{path}` settings of `{}`.", scope.name),
            )
        };
        let mut class = format!(
            "class {name}(TypedDict):\n{}",
            docstring("    ", &summary, Stability::Stable, None)
        );
        for (segment, node) in level {
            self.imports.insert(("typing", "NotRequired"));
            match node {
                Node::Setting(setting) => {
                    let annotation = self
                        .value_type(setting)
                        .with_context(|| format!("typing the setting `{}`", setting.name))?;
                    let described = if setting.description.is_empty() {
                        format!("`{}`.", setting.name)
                    } else {
                        format!("{}\n\n`{}`.", setting.description, setting.name)
                    };
                    let _ = write!(
                        class,
                        "\n    {segment}: NotRequired[{annotation}]\n{}",
                        docstring("    ", &described, Stability::Stable, None)
                    );
                }
                Node::Area(children) => {
                    let inner = if path.is_empty() {
                        segment.clone()
                    } else {
                        format!("{path}.{segment}")
                    };
                    let child = self.area(children, scope, &inner)?;
                    let _ = write!(class, "\n    {segment}: NotRequired[{child}]\n");
                }
            }
        }
        self.define(&name, class)?;
        Ok(name)
    }

    /// A setting's value type: its schema's, with `None` when it can be
    /// cleared.
    fn value_type(&mut self, setting: &Setting) -> Result<String> {
        let owner = pascal(&setting.name);
        let mut arms = self.arms(&setting.value, &owner, &owner)?;
        if setting.clearable {
            arms.push(Arm::Type("None".to_owned()));
        }
        Ok(self.union(arms))
    }

    /// `arms` as one annotation: each type once, the literals in one
    /// `Literal` where the first stood, `None` last.
    fn union(&mut self, arms: Vec<Arm>) -> String {
        let mut members: Vec<String> = Vec::new();
        let mut literals: Vec<String> = Vec::new();
        let mut literal_at = None;
        let mut none = false;
        for arm in arms {
            match arm {
                Arm::Literal(value) => {
                    literal_at.get_or_insert(members.len());
                    if !literals.contains(&value) {
                        literals.push(value);
                    }
                }
                Arm::Type(name) if name == "None" => none = true,
                Arm::Type(name) => {
                    if !members.contains(&name) {
                        members.push(name);
                    }
                }
            }
        }
        if let Some(at) = literal_at {
            self.imports.insert(("typing", "Literal"));
            members.insert(at, format!("Literal[{}]", literals.join(", ")));
        }
        if none {
            members.push("None".to_owned());
        }
        if members.is_empty() {
            "object".to_owned()
        } else {
            members.join(" | ")
        }
    }

    /// The union arms of the values a JSON Schema accepts; an object with
    /// properties is the class `owner`, a tagged alternative's named after
    /// its tag under `setting`.
    fn arms(&mut self, schema: &Value, owner: &str, setting: &str) -> Result<Vec<Arm>> {
        let Some(object) = schema.as_object() else {
            bail!("a value schema is an object, not {schema}");
        };
        for keyword in object.keys() {
            ensure!(
                UNTYPED_KEYWORDS.contains(&keyword.as_str())
                    || TYPED_KEYWORDS.contains(&keyword.as_str()),
                "the value schema keyword `{keyword}` has no Python form"
            );
        }
        if *schema == ValueKind::Duration.schema() {
            self.imports.insert(("datetime", "timedelta"));
            return Ok(vec![Arm::Type("timedelta".to_owned())]);
        }
        if let Some(constant) = object.get("const") {
            return Ok(vec![literal(constant)]);
        }
        if let Some(values) = object.get("enum").and_then(Value::as_array) {
            return Ok(values.iter().map(literal).collect());
        }
        if let Some(alternatives) = object.get("anyOf").or_else(|| object.get("oneOf")) {
            let mut arms = Vec::new();
            for (index, alternative) in alternatives
                .as_array()
                .context("`anyOf` and `oneOf` list schemas")?
                .iter()
                .enumerate()
            {
                let named = match tag(alternative) {
                    Some(tag) => format!("{setting}{}", pascal(tag)),
                    None if alternative.get("properties").is_some() => {
                        format!("{owner}{}", index + 1)
                    }
                    None => owner.to_owned(),
                };
                arms.extend(self.arms(alternative, &named, setting)?);
            }
            return Ok(arms);
        }
        let types: Vec<&str> = match object.get("type") {
            Some(Value::String(one)) => vec![one.as_str()],
            Some(Value::Array(several)) => several
                .iter()
                .map(|one| one.as_str().context("a `type` names a type"))
                .collect::<Result<_>>()?,
            Some(other) => bail!("a `type` of {other}"),
            None if object.contains_key("properties") => vec!["object"],
            None => return Ok(vec![Arm::Type("object".to_owned())]),
        };
        types
            .into_iter()
            .map(|one| {
                Ok(Arm::Type(match one {
                    "string" => "str".to_owned(),
                    "integer" => "int".to_owned(),
                    "number" => "float".to_owned(),
                    "boolean" => "bool".to_owned(),
                    "null" => "None".to_owned(),
                    "array" => {
                        let item = match object.get("items") {
                            Some(items) => {
                                let arms = self.arms(items, &format!("{owner}Item"), setting)?;
                                self.union(arms)
                            }
                            None => "object".to_owned(),
                        };
                        format!("list[{item}]")
                    }
                    "object" => self.object_type(object, owner, setting)?,
                    other => bail!("the JSON type `{other}` has no Python form"),
                }))
            })
            .collect()
    }

    /// An object value's type: the class `owner` of its properties, or a
    /// `Mapping` of its other keys' values.
    fn object_type(
        &mut self,
        object: &Map<String, Value>,
        owner: &str,
        setting: &str,
    ) -> Result<String> {
        let properties = object.get("properties").and_then(Value::as_object);
        let others = object.get("additionalProperties");
        match (properties, others) {
            (Some(properties), None | Some(Value::Bool(_))) if !properties.is_empty() => {
                self.object_class(object, properties, owner, setting)
            }
            (Some(properties), Some(_)) if !properties.is_empty() => {
                bail!(
                    "an object with properties and a schema for its other keys has no Python form"
                )
            }
            (_, Some(schema)) if !schema.is_boolean() => {
                let arms = self.arms(schema, &format!("{owner}Entry"), setting)?;
                let entry = self.union(arms);
                self.imports.insert(("collections.abc", "Mapping"));
                Ok(format!("Mapping[str, {entry}]"))
            }
            _ => {
                self.imports.insert(("collections.abc", "Mapping"));
                Ok("Mapping[str, object]".to_owned())
            }
        }
    }

    /// The class `owner` of an object's `properties`, a required one plain
    /// and every other `NotRequired`, after the classes it names.
    fn object_class(
        &mut self,
        object: &Map<String, Value>,
        properties: &Map<String, Value>,
        owner: &str,
        setting: &str,
    ) -> Result<String> {
        let required: Vec<&str> = object
            .get("required")
            .and_then(Value::as_array)
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        let summary = object
            .get("description")
            .and_then(Value::as_str)
            .map_or_else(|| format!("A value of `{owner}`."), str::to_owned);
        let mut class = format!(
            "class {owner}(TypedDict):\n{}",
            docstring("    ", &summary, Stability::Stable, None)
        );
        for (name, schema) in properties {
            ensure!(
                key(name),
                "the property `{name}` is a key a `TypedDict` cannot declare"
            );
            let arms = self.arms(schema, &format!("{owner}{}", pascal(name)), setting)?;
            let mut annotation = self.union(arms);
            if !required.contains(&name.as_str()) {
                self.imports.insert(("typing", "NotRequired"));
                annotation = format!("NotRequired[{annotation}]");
            }
            let _ = write!(class, "\n    {name}: {annotation}\n");
            if let Some(description) = schema.get("description").and_then(Value::as_str) {
                class.push_str(&docstring("    ", description, Stability::Stable, None));
            }
        }
        self.define(owner, class)?;
        Ok(owner.to_owned())
    }
}

/// The tag that names an object alternative: its first property whose value
/// is one constant string, such as `kind`'s.
fn tag(alternative: &Value) -> Option<&str> {
    alternative
        .get("properties")?
        .as_object()?
        .values()
        .find_map(|property| property.get("const")?.as_str())
}

/// A constant as a `Literal`'s member, or `None`.
fn literal(value: &Value) -> Arm {
    match value {
        Value::Null => Arm::Type("None".to_owned()),
        Value::Bool(true) => Arm::Literal("True".to_owned()),
        Value::Bool(false) => Arm::Literal("False".to_owned()),
        other => Arm::Literal(other.to_string()),
    }
}

#[cfg(test)]
mod tests;
