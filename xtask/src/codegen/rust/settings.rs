//! The settings keys of Collie's Rust client crate (ADR 0083), generated
//! into `src/generated/settings.rs`, which the crate's hand-written
//! `settings` module includes: one `Setting<T>` constant a key some scope's
//! own `settings` set, in modules that mirror its dotted name
//! (`timeout::NAVIGATION` for `timeout.navigation`), with the scopes that
//! set it, which the crate checks at run time, since its one `Settings`
//! type serves every scope.
//!
//! `T` comes from the key's value schema: `Duration` for a duration, `u64`
//! for a whole number, `bool`, `String`, an enum of its own for a choice of
//! words (`content::Extractor`, an array's items `…Item`, a map's values
//! `…Entry`, and a name Rust's prelude takes `…Choice`, as
//! `run_state::batch::SyncChoice`), `Vec` and `BTreeMap` of those, `Option`
//! for a nullable value, and `serde_json::Value` for a shape these do not
//! spell, as an object of named properties or a union of other kinds. A
//! key that can be cleared takes an `Option`, whose `None` sends null.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail, ensure};
use goodfetch_protocol::catalog::ValueKind;
use serde_json::Value;

use super::super::handles::Member;
use super::super::layer::Setting;
use super::super::naming::{rust_snake, rust_type};
use super::super::settings_tree::{Node, SETTINGS, opened_scope, tree};
use super::{Setter, comment};

/// A JSON value, for a shape the module does not type.
const JSON: &str = "::serde_json::Value";

/// The names an enum of a choice does not take, which `…Choice` names
/// instead: the types and traits of Rust's prelude, which a public type of
/// the same name would make rustc spell in full in every message that
/// names them (`std::marker::Sync`), and the `settings` module's own.
const TAKEN: [&str; 30] = [
    "AsMut",
    "AsRef",
    "Box",
    "Clone",
    "Copy",
    "Default",
    "Drop",
    "Eq",
    "Extend",
    "Fn",
    "FnMut",
    "FnOnce",
    "From",
    "FromIterator",
    "Into",
    "IntoIterator",
    "Iterator",
    "Option",
    "Ord",
    "PartialEq",
    "PartialOrd",
    "Result",
    "Send",
    "Setting",
    "SettingValue",
    "Settings",
    "Sized",
    "String",
    "Sync",
    "Vec",
];

/// The setter of `member`'s parameter `name` when it is the `settings` of
/// a scope the member opens: it takes the crate's typed `Settings` and
/// fails the call, before it is sent, on a key the scope does not set (ADR
/// 0083). `None` for any other parameter.
pub(super) fn setter(
    member: &Member<'_>,
    name: &str,
    doc: &str,
    owner: &str,
) -> Result<Option<Setter>> {
    let Some(scope) = opened_scope(member).filter(|_| name == SETTINGS) else {
        return Ok(None);
    };
    // rustc checks the variant against the protocol crate's `SettingScope`.
    let variant = rust_type(scope)
        .with_context(|| format!("{owner}: it opens `{scope}`, which names no Rust variant"))?;
    Ok(Some(Setter {
        doc: comment(&format!(
            "{doc}\n\nA key of [`crate::settings`] this scope does not set fails the call before it is sent."
        )),
        declared: "crate::settings::Settings".to_owned(),
        set: SETTINGS.to_owned(),
        scope: Some(variant),
    }))
}

/// The module's constants and enums, `xtask codegen`'s header first: the
/// text the crate's `settings` module includes.
pub(super) fn module(header: &str, settings: &[Setting]) -> Result<String> {
    let root = tree(settings, None)?;
    let mut text = format!("{header}\n");
    level(&mut text, &root, "")?;
    Ok(text)
}

/// The items of one level of the tree at `path`: its settings' constants
/// and enums, then a module for each of its areas.
fn level(text: &mut String, level: &BTreeMap<String, Node<'_>>, path: &str) -> Result<()> {
    let mut enums = Enums::default();
    for (segment, node) in level {
        if let Node::Setting(setting) = node {
            constant(text, &mut enums, segment, setting)
                .with_context(|| format!("the setting `{}`", setting.name))?;
        }
    }
    text.push_str(&enums.text);
    for (segment, node) in level {
        if let Node::Area(children) = node {
            let at = if path.is_empty() {
                segment.clone()
            } else {
                format!("{path}.{segment}")
            };
            let ident = rust_snake(segment)
                .filter(|ident| ident == segment || ident.strip_prefix("r#") == Some(segment))
                .with_context(|| format!("`{at}` has a segment Rust cannot name a module"))?;
            let _ = write!(
                text,
                "\n{}pub mod {ident} {{\n",
                comment(&format!("The `{at}.*` settings."))
            );
            self::level(text, children, &at)?;
            text.push_str("}\n");
        }
    }
    Ok(())
}

/// The constant of `setting`, the last segment of its key `segment`, and
/// the enums its type names.
fn constant(text: &mut String, enums: &mut Enums, segment: &str, setting: &Setting) -> Result<()> {
    ensure!(
        segment
            .chars()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase())
            && segment
                .chars()
                .all(|character| character.is_ascii_lowercase()
                    || character.is_ascii_digit()
                    || character == '_'),
        "`{segment}` is no segment Rust can name a constant"
    );
    let hint = rust_type(segment).with_context(|| format!("`{segment}` names no Rust type"))?;
    let mut ty = enums.value_type(&setting.value, &hint)?;
    if setting.clearable && !ty.starts_with("Option<") {
        ty = format!("Option<{ty}>");
    }
    let scopes: Vec<String> = setting
        .scopes
        .iter()
        .map(|scope| {
            rust_type(scope)
                .map(|variant| format!("::goodfetch_protocol::catalog::SettingScope::{variant}"))
                .with_context(|| format!("`{scope}` is no scope"))
        })
        .collect::<Result<_>>()?;
    let cleared = if setting.clearable {
        " `None` clears it."
    } else {
        ""
    };
    let described = if setting.description.is_empty() {
        format!("`{}`.{cleared}", setting.name)
    } else {
        format!(
            "{}\n\n`{}`.{cleared}",
            setting.description.trim(),
            setting.name
        )
    };
    let _ = write!(
        text,
        "\n{}pub const {}: crate::settings::Setting<{ty}> = crate::settings::Setting::new({:?}, &[{}]);\n",
        comment(&described),
        segment.to_uppercase(),
        setting.name,
        scopes.join(", ")
    );
    Ok(())
}

/// The enums of one module's choices, by name, and their text.
#[derive(Default)]
struct Enums {
    names: BTreeSet<String>,
    text: String,
}

impl Enums {
    /// The Rust type of the values `schema` accepts; a choice of words is
    /// the enum `hint`, which this module then defines.
    fn value_type(&mut self, schema: &Value, hint: &str) -> Result<String> {
        if *schema == ValueKind::Duration.schema() {
            return Ok("::std::time::Duration".to_owned());
        }
        if let Some(alternatives) = schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Value::as_array)
        {
            return Ok(match nullable(alternatives) {
                Some(value) => format!("Option<{}>", self.value_type(value, hint)?),
                None => JSON.to_owned(),
            });
        }
        if let Some(words) = schema.get("enum").and_then(Value::as_array) {
            return self.choice(words, hint);
        }
        let ty = match schema.get("type") {
            Some(Value::String(one)) => one.as_str(),
            Some(Value::Array(several)) => {
                let named: Vec<&str> = several.iter().filter_map(Value::as_str).collect();
                return Ok(match named.as_slice() {
                    [one, "null"] | ["null", one] => {
                        let mut alone = schema.clone();
                        alone["type"] = Value::from(*one);
                        format!("Option<{}>", self.value_type(&alone, hint)?)
                    }
                    _ => JSON.to_owned(),
                });
            }
            _ => return Ok(JSON.to_owned()),
        };
        Ok(match ty {
            "boolean" => "bool".to_owned(),
            "integer" => "u64".to_owned(),
            "string" => "String".to_owned(),
            "array" => match schema.get("items") {
                Some(items) => format!("Vec<{}>", self.value_type(items, &format!("{hint}Item"))?),
                None => format!("Vec<{JSON}>"),
            },
            "object" => match schema.get("additionalProperties") {
                Some(entry @ Value::Object(_)) if schema.get("properties").is_none() => format!(
                    "::std::collections::BTreeMap<String, {}>",
                    self.value_type(entry, &format!("{hint}Entry"))?
                ),
                _ => JSON.to_owned(),
            },
            _ => JSON.to_owned(),
        })
    }

    /// The enum `name` of a choice of `words`, each a variant named after
    /// it, which turns into its word on the wire.
    fn choice(&mut self, words: &[Value], name: &str) -> Result<String> {
        let Some(words) = words
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<&str>>>()
        else {
            return Ok(JSON.to_owned());
        };
        let name = if TAKEN.contains(&name) {
            format!("{name}Choice")
        } else {
            name.to_owned()
        };
        let name = name.as_str();
        ensure!(
            self.names.insert(name.to_owned()),
            "two choices of one module are both named `{name}`"
        );
        let mut variants = BTreeSet::new();
        let mut body = String::new();
        let mut arms = String::new();
        for word in words {
            let variant = rust_type(&word.replace('-', "_"))
                .filter(|variant| {
                    variant
                        .chars()
                        .next()
                        .is_some_and(|first| first.is_ascii_uppercase())
                        && variant
                            .chars()
                            .all(|character| character.is_ascii_alphanumeric())
                })
                .with_context(|| format!("the word `{word}` names no Rust variant"))?;
            if !variants.insert(variant.clone()) {
                bail!("two words of `{name}` are both the variant `{variant}`");
            }
            let _ = writeln!(body, "{}{variant},", comment(&format!("`{word}`.")));
            let _ = writeln!(arms, "Self::{variant} => {word:?},");
        }
        let _ = write!(
            self.text,
            "\n{}#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]\npub enum {name} {{\n{body}}}\n\nimpl crate::settings::sealed::Wire for {name} {{\n    fn into_wire(self) -> ::serde_json::Value {{\n        ::serde_json::Value::from(match self {{\n{arms}        }})\n    }}\n}}\n\nimpl crate::settings::SettingValue for {name} {{}}\n",
            comment("A word a setting of this module takes, as the catalog spells it.")
        );
        Ok(name.to_owned())
    }
}

/// The one alternative of `alternatives` besides null, when there are
/// exactly these two.
fn nullable(alternatives: &[Value]) -> Option<&Value> {
    let null = |schema: &Value| schema.get("type").and_then(Value::as_str) == Some("null");
    match alternatives {
        [value, other] | [other, value] if null(other) && !null(value) => Some(value),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
