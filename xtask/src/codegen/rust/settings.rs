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
//! `run_state::batch::SyncChoice`), the protocol's own type for a shape it
//! defines once (`session.page_check`'s `PageCheck`), `Vec` and `BTreeMap`
//! of those, and `Option` for a nullable value. A key that can be cleared
//! takes an `Option`, whose `None` sends null. A schema none of these spell
//! fails the generation, naming its key, so no key takes untyped JSON.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail, ensure};
use goodfetch_protocol::catalog::{ValueKind, ValueShape};
use goodfetch_protocol::egress::LOCAL_ADDRESSES_SHAPE;
use goodfetch_protocol::run::durability::BATCH_THRESHOLD_SHAPE;
use goodfetch_protocol::settings::page_check::PAGE_CHECK_SHAPE;
use goodfetch_protocol::settings::session_source::SESSION_SOURCE_SHAPE;
use goodfetch_protocol::tls::CLIENT_CERTIFICATE_SHAPE;
use serde_json::Value;

use super::super::handles::Member;
use super::super::layer::Setting;
use super::super::naming::{rust_snake, rust_type};
use super::super::settings_tree::{Node, SETTINGS, opened_scope, tree};
use super::{Setter, comment};

/// The protocol's type of each shape a key of the crate takes (ADR 0028's
/// `ValueKind::Shape`), by the shape: rustc checks the path when the crate
/// builds, and the shape's own `check` reads that type. A new shape a key
/// takes fails the generation until it is listed here.
const SHAPES: [(&ValueShape, &str); 5] = [
    (
        &LOCAL_ADDRESSES_SHAPE,
        "::goodfetch_protocol::egress::LocalAddresses",
    ),
    (
        &BATCH_THRESHOLD_SHAPE,
        "::goodfetch_protocol::run::durability::BatchThreshold",
    ),
    (
        &PAGE_CHECK_SHAPE,
        "::goodfetch_protocol::settings::page_check::PageCheck",
    ),
    (
        &SESSION_SOURCE_SHAPE,
        "::goodfetch_protocol::settings::session_source::SessionSource",
    ),
    (
        &CLIENT_CERTIFICATE_SHAPE,
        "::goodfetch_protocol::tls::ClientCertificateConfiguration",
    ),
];

/// The names an enum of a choice does not take, which `…Choice` names
/// instead: the types and traits of Rust's prelude, which a public type of
/// the same name would make rustc spell in full in every message that
/// names them (`std::marker::Sync`), and the `settings` module's own.
const TAKEN: [&str; 31] = [
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
    "IntoSettingValue",
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

/// The module's constants and enums, `xtask codegen`'s header first, then
/// how each protocol type the keys take goes on the wire: the text the
/// crate's `settings` module includes.
pub(super) fn module(header: &str, settings: &[Setting]) -> Result<String> {
    let root = tree(settings, None)?;
    let mut text = format!("{header}\n");
    let mut shapes = BTreeSet::new();
    level(&mut text, &mut shapes, &root, "")?;
    for shape in shapes {
        let _ = write!(
            text,
            "\n{}impl crate::settings::sealed::Wire for {shape} {{\n    fn into_wire(self) -> ::serde_json::Value {{\n        ::serde_json::to_value(self).expect(\"invariant: a value of the protocol's shapes is JSON\")\n    }}\n}}\n\nimpl crate::settings::SettingValue for {shape} {{}}\n",
            comment("The protocol's own JSON of the value.")
        );
    }
    Ok(text)
}

/// The items of one level of the tree at `path`: its settings' constants
/// and enums, then a module for each of its areas. `shapes` gathers the
/// protocol's types their values take.
fn level(
    text: &mut String,
    shapes: &mut BTreeSet<&'static str>,
    level: &BTreeMap<String, Node<'_>>,
    path: &str,
) -> Result<()> {
    let mut enums = Enums {
        names: BTreeSet::new(),
        text: String::new(),
        shapes,
    };
    for (segment, node) in level {
        if let Node::Setting(setting) = node {
            constant(text, &mut enums, segment, setting)
                .with_context(|| format!("the setting `{}`", setting.name))?;
        }
    }
    text.push_str(&enums.text);
    let shapes = enums.shapes;
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
            self::level(text, shapes, children, &at)?;
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
    let mut ty = enums
        .value_type(&setting.value, &hint)?
        .with_context(|| format!("its values have no Rust type: {}", setting.value))?;
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

/// The enums of one module's choices, by name, and their text, and the
/// protocol's types the whole tree's values take.
struct Enums<'shapes> {
    names: BTreeSet<String>,
    text: String,
    shapes: &'shapes mut BTreeSet<&'static str>,
}

impl Enums<'_> {
    /// The Rust type of the values `schema` accepts, `None` for a schema
    /// it has none for; a choice of words is the enum `hint`, which this
    /// module then defines.
    fn value_type(&mut self, schema: &Value, hint: &str) -> Result<Option<String>> {
        if *schema == ValueKind::Duration.schema() {
            return Ok(Some("::std::time::Duration".to_owned()));
        }
        if let Some((_, ty)) = SHAPES.iter().find(|(shape, _)| (shape.schema)() == *schema) {
            self.shapes.insert(ty);
            return Ok(Some((*ty).to_owned()));
        }
        if let Some(alternatives) = schema
            .get("anyOf")
            .or_else(|| schema.get("oneOf"))
            .and_then(Value::as_array)
        {
            return Ok(match nullable(alternatives) {
                Some(value) => self
                    .value_type(value, hint)?
                    .map(|ty| format!("Option<{ty}>")),
                None => None,
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
                        self.value_type(&alone, hint)?
                            .map(|ty| format!("Option<{ty}>"))
                    }
                    _ => None,
                });
            }
            _ => return Ok(None),
        };
        Ok(match ty {
            "boolean" => Some("bool".to_owned()),
            "integer" => Some("u64".to_owned()),
            "string" => Some("String".to_owned()),
            "array" => match schema.get("items") {
                Some(items) => self
                    .value_type(items, &format!("{hint}Item"))?
                    .map(|ty| format!("Vec<{ty}>")),
                None => None,
            },
            "object" => match schema.get("additionalProperties") {
                Some(entry @ Value::Object(_)) if schema.get("properties").is_none() => self
                    .value_type(entry, &format!("{hint}Entry"))?
                    .map(|ty| format!("::std::collections::BTreeMap<String, {ty}>")),
                _ => None,
            },
            _ => None,
        })
    }

    /// The enum `name` of a choice of `words`, each a variant named after
    /// it, which turns into its word on the wire; `None` when a word is no
    /// text.
    fn choice(&mut self, words: &[Value], name: &str) -> Result<Option<String>> {
        let Some(words) = words
            .iter()
            .map(Value::as_str)
            .collect::<Option<Vec<&str>>>()
        else {
            return Ok(None);
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
        Ok(Some(name.to_owned()))
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
