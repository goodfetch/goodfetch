//! The TypeScript types of each scope's `settings` option (ADR 0083):
//! `Settings` for `connect()`, `RunSettings`, `SessionSettings` and
//! `PageSettings` in Collie's client package, `CrawlSettings` and
//! `LabelSettings` in Foxhound's (ADR 0093), each nesting the settings a
//! program's code sets there by their camelCase segments (ADR 0099), so
//! that a key outside its scope, a misspelled one or a wrong value fails to
//! compile, and the spelling the hand-written walker sends each segment
//! with.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail, ensure};
use serde_json::{Map, Value};

use goodfetch_protocol::catalog::SettingScope;

use super::super::handles::{Form, Handle, Member};
use super::super::layer::Setting;
use super::super::naming::camel;
use super::HEADER;

/// The optional parameter that holds a scope's own settings (ADR 0083).
pub(super) const SETTINGS: &str = "settings";

/// What sends a scope's settings by their wire spelling, which the
/// generated settings module declares.
pub(super) const SETTINGS_ON_WIRE: &str = "settingsOnWire";

/// Where Collie's client package's generated settings module takes the
/// hand-written walker of a scope's settings onto the wire from.
const COLLIE_WALKER: &str = "../settings.ts";

/// Where Foxhound's package's generated settings module takes it from: the
/// client package's runtime.
const FOXHOUND_WALKER: &str = "@goodfetch/collie-client/runtime";

/// A scope's settings type: the scope, its type's name and its `TSDoc`.
struct ScopeType {
    scope: SettingScope,
    name: &'static str,
    doc: &'static str,
}

/// The scopes a Collie program's code opens with its own `settings`,
/// outermost first, which Collie's client package types.
const COLLIE_SCOPES: [ScopeType; 4] = [
    ScopeType {
        scope: SettingScope::Connection,
        name: "Settings",
        doc: "The settings `connect()` takes, which every Run it starts inherits (ADR 0083).",
    },
    ScopeType {
        scope: SettingScope::Run,
        name: "RunSettings",
        doc: "The settings starting a Run takes, which its Sessions and pages inherit (ADR 0083).",
    },
    ScopeType {
        scope: SettingScope::Session,
        name: "SessionSettings",
        doc: "The settings opening a Session takes, over its Run's, which its pages inherit (ADR 0083).",
    },
    ScopeType {
        scope: SettingScope::Page,
        name: "PageSettings",
        doc: "The settings opening a page takes, over its Session's and its Run's (ADR 0083).",
    },
];

/// The scopes a crawl opens with its own `settings`, outermost first,
/// which Foxhound's package types: its Run's, Collie's and Foxhound's
/// keys, and each Label's (ADR 0093).
const FOXHOUND_SCOPES: [ScopeType; 2] = [
    ScopeType {
        scope: SettingScope::Run,
        name: "CrawlSettings",
        doc: "The settings starting a crawl takes, Collie's and Foxhound's, which its Labels and requests inherit (ADRs 0083 and 0093).",
    },
    ScopeType {
        scope: SettingScope::Label,
        name: "LabelSettings",
        doc: "The settings a crawl's Label takes, over its Run's, which its requests inherit (ADR 0093); every other key is the Run's alone.",
    },
];

/// The names of the settings types Foxhound's package generates, which its
/// generated index exports.
pub(super) fn foxhound_types() -> Vec<&'static str> {
    FOXHOUND_SCOPES.iter().map(|scope| scope.name).collect()
}

/// The JSON Schema keywords a setting's value schema may carry that type
/// nothing TypeScript checks: the engine checks them (ADR 0083).
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

/// A segment of the settings tree: a setting, or an area of more segments.
enum Node<'a> {
    Setting(&'a Setting),
    Area(BTreeMap<String, Node<'a>>),
}

/// The settings tree of those `settings` that `scope` sets, by wire segment.
fn tree(settings: &[Setting], scope: Option<SettingScope>) -> Result<BTreeMap<String, Node<'_>>> {
    let mut root = BTreeMap::new();
    for setting in settings {
        let applies = match scope {
            Some(scope) => setting.scopes.iter().any(|named| named == scope.as_str()),
            None => !setting.scopes.is_empty(),
        };
        if !applies {
            continue;
        }
        let segments: Vec<&str> = setting.name.split('.').collect();
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

/// The camelCase spelling of each of `level`'s segments, refusing two that
/// spell one.
fn spelled<'a, 'b>(
    level: &'b BTreeMap<String, Node<'a>>,
) -> Result<Vec<(String, &'b str, &'b Node<'a>)>> {
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    let mut spelled = Vec::new();
    for (segment, node) in level {
        let name = camel(segment);
        if let Some(other) = seen.insert(name.clone(), segment) {
            bail!("the setting segments `{other}` and `{segment}` both spell `{name}`");
        }
        spelled.push((name, segment.as_str(), node));
    }
    Ok(spelled)
}

/// The settings module of Collie's client package: each scope's type, and
/// `settingsOnWire`, which sends a scope's settings by their wire spelling.
pub(super) fn settings_module(settings: &[Setting]) -> Result<String> {
    module(settings, &COLLIE_SCOPES, COLLIE_WALKER)
}

/// The settings module of Foxhound's package: a crawl's and a Label's
/// types, and `settingsOnWire`, which sends either by its wire spelling.
pub(super) fn foxhound_settings_module(settings: &[Setting]) -> Result<String> {
    module(settings, &FOXHOUND_SCOPES, FOXHOUND_WALKER)
}

/// A settings module: each of `scopes`' types, and `settingsOnWire`, which
/// takes the outermost's, with the walker imported from `walker`.
fn module(settings: &[Setting], scopes: &[ScopeType], walker: &str) -> Result<String> {
    let outermost = scopes
        .first()
        .context("a settings module types one scope at least")?
        .name;
    let mut text = format!(
        "{HEADER}\nimport {{ onWire, type SettingsOnWire, type Spelling }} from \"{walker}\";\n"
    );
    for scope in scopes {
        let level = tree(settings, Some(scope.scope))?;
        let _ = write!(
            text,
            "\n/**\n * {}\n *\n * A key left out inherits, `null` clears a setting that can be cleared,\n * and a value sets it; the program needs `exactOptionalPropertyTypes`, so\n * that `undefined` is never a value.\n */\nexport interface {} ",
            scope.doc, scope.name
        );
        text.push_str(&area_type(&level, "")?);
        text.push('\n');
    }
    let every = tree(settings, None)?;
    let _ = write!(
        text,
        "\n/** Each segment's wire spelling, by its camelCase one. */\nconst SPELLING: Spelling = {};\n",
        spelling(&every, "")?
    );
    let _ = write!(
        text,
        "\n/** @internal A scope's `settings` as the wire takes them; `undefined` when left out. */\nexport function settingsOnWire(settings: {outermost} | undefined): SettingsOnWire | undefined {{\n  return onWire(settings, SPELLING);\n}}\n",
    );
    Ok(text)
}

/// The type name of the settings `opened`'s option takes.
pub(super) fn scope_type(opened: &str) -> Result<&'static str> {
    COLLIE_SCOPES
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

/// The import of the settings types `handles`' members take, and of
/// `settingsOnWire`; empty when none takes settings.
pub(super) fn handles_import(handles: &[Handle<'_>]) -> Result<String> {
    let mut scoped = BTreeSet::new();
    for member in handles.iter().flat_map(|handle| {
        handle
            .members
            .iter()
            .chain(handle.sub_objects.iter().flat_map(|held| &held.members))
    }) {
        scoped.extend(scope_settings(member)?);
    }
    if scoped.is_empty() {
        return Ok(String::new());
    }
    let imported: Vec<String> = scoped.iter().map(|name| format!("type {name}")).collect();
    Ok(format!(
        "import {{ {SETTINGS_ON_WIRE}, {} }} from \"./settings.ts\";\n",
        imported.join(", ")
    ))
}

/// An area's object type, its members indented under `indent`.
fn area_type(level: &BTreeMap<String, Node<'_>>, indent: &str) -> Result<String> {
    let inner = format!("{indent}  ");
    let mut text = "{\n".to_owned();
    for (name, _, node) in spelled(level)? {
        match node {
            Node::Setting(setting) => {
                text.push_str(&doc(&inner, setting));
                let _ = writeln!(text, "{inner}readonly {name}?: {};", value_type(setting)?);
            }
            Node::Area(children) => {
                let _ = writeln!(
                    text,
                    "{inner}readonly {name}?: {};",
                    area_type(children, &inner)?
                );
            }
        }
    }
    let _ = write!(text, "{indent}}}");
    Ok(text)
}

/// A setting's `TSDoc`: its description and its wire key.
fn doc(indent: &str, setting: &Setting) -> String {
    let mut text = format!("{indent}/**\n");
    for line in setting.description.replace("*/", "*\\/").lines() {
        let _ = writeln!(text, "{indent} * {line}");
    }
    let _ = writeln!(
        text,
        "{indent} *\n{indent} * `{}`.\n{indent} */",
        setting.name
    );
    text
}

/// The spelling tree's literal: each segment's wire name, an area's with
/// its own segments.
fn spelling(level: &BTreeMap<String, Node<'_>>, indent: &str) -> Result<String> {
    let inner = format!("{indent}  ");
    let mut text = "{\n".to_owned();
    for (name, segment, node) in spelled(level)? {
        match node {
            Node::Setting(_) => {
                let _ = writeln!(text, "{inner}{name}: {segment:?},");
            }
            Node::Area(children) => {
                let _ = writeln!(
                    text,
                    "{inner}{name}: [{segment:?}, {}],",
                    spelling(children, &inner)?
                );
            }
        }
    }
    let _ = write!(text, "{indent}}}");
    Ok(text)
}

/// A setting's value type: its schema's, with `null` when it can be cleared.
fn value_type(setting: &Setting) -> Result<String> {
    let mut members = schema_members(&setting.value)
        .with_context(|| format!("typing the setting `{}`", setting.name))?;
    if setting.clearable {
        members.push("null".to_owned());
    }
    Ok(union(members))
}

/// `members` joined as a union, each once, in their first order.
fn union(members: Vec<String>) -> String {
    let mut once: Vec<String> = Vec::new();
    for member in members {
        if !once.contains(&member) {
            once.push(member);
        }
    }
    if once.is_empty() {
        "never".to_owned()
    } else {
        once.join(" | ")
    }
}

/// The union members of the values a JSON Schema accepts.
fn schema_members(schema: &Value) -> Result<Vec<String>> {
    let Some(object) = schema.as_object() else {
        bail!("a value schema is an object, not {schema}");
    };
    for keyword in object.keys() {
        let known = UNTYPED_KEYWORDS.contains(&keyword.as_str())
            || [
                "type",
                "const",
                "enum",
                "anyOf",
                "oneOf",
                "items",
                "properties",
                "required",
                "additionalProperties",
            ]
            .contains(&keyword.as_str());
        ensure!(
            known,
            "the value schema keyword `{keyword}` has no TypeScript form"
        );
    }
    if let Some(constant) = object.get("const") {
        return Ok(vec![constant.to_string()]);
    }
    if let Some(values) = object.get("enum").and_then(Value::as_array) {
        return Ok(values.iter().map(Value::to_string).collect());
    }
    if let Some(alternatives) = object.get("anyOf").or_else(|| object.get("oneOf")) {
        let mut members = Vec::new();
        for alternative in alternatives.as_array().context("`anyOf` lists schemas")? {
            members.extend(schema_members(alternative)?);
        }
        return Ok(members);
    }
    let types: Vec<&str> = match object.get("type") {
        Some(Value::String(one)) => vec![one.as_str()],
        Some(Value::Array(several)) => several
            .iter()
            .map(|one| one.as_str().context("a `type` names a type"))
            .collect::<Result<_>>()?,
        Some(other) => bail!("a `type` of {other}"),
        None if object.contains_key("properties") => vec!["object"],
        None => return Ok(vec!["unknown".to_owned()]),
    };
    types
        .into_iter()
        .map(|one| match one {
            "string" => Ok("string".to_owned()),
            "integer" | "number" => Ok("number".to_owned()),
            "boolean" => Ok("boolean".to_owned()),
            "null" => Ok("null".to_owned()),
            "array" => {
                let item = match object.get("items") {
                    Some(items) => union(schema_members(items)?),
                    None => "unknown".to_owned(),
                };
                Ok(if item.contains(' ') {
                    format!("readonly ({item})[]")
                } else {
                    format!("readonly {item}[]")
                })
            }
            "object" => object_type(object),
            other => bail!("the JSON type `{other}` has no TypeScript form"),
        })
        .collect()
}

/// An object value's type: its properties under their wire names, a value
/// being sent as written, and its other keys' values.
fn object_type(object: &Map<String, Value>) -> Result<String> {
    let required: Vec<&str> = object
        .get("required")
        .and_then(Value::as_array)
        .map(|names| names.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let mut members = Vec::new();
    if let Some(properties) = object.get("properties").and_then(Value::as_object) {
        for (name, schema) in properties {
            let optional = if required.contains(&name.as_str()) {
                ""
            } else {
                "?"
            };
            members.push(format!(
                "readonly {}{optional}: {}",
                property_key(name),
                union(schema_members(schema)?)
            ));
        }
    }
    match object.get("additionalProperties") {
        Some(Value::Bool(true)) | None if members.is_empty() => {
            members.push("readonly [key: string]: unknown".to_owned());
        }
        Some(Value::Bool(_)) | None => {}
        Some(schema) => members.push(format!(
            "readonly [key: string]: {}",
            union(schema_members(schema)?)
        )),
    }
    Ok(format!("{{ {} }}", members.join("; ")))
}

/// A property's key, quoted unless it is an identifier.
fn property_key(name: &str) -> String {
    let identifier = name.chars().enumerate().all(|(position, character)| {
        character == '_'
            || character == '$'
            || character.is_ascii_alphabetic()
            || (position > 0 && character.is_ascii_digit())
    }) && !name.is_empty();
    if identifier {
        name.to_owned()
    } else {
        format!("{name:?}")
    }
}

#[cfg(test)]
mod tests;
