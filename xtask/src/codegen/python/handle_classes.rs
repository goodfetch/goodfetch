//! The handles of Collie's Python client package (ADR 0099): one async
//! class a namespace, `Async<Name>`, with its members in the form their
//! kind gives them, the values it builds without a call and its
//! sub-objects, and its sync twin `<Name>`, which repeats it under the same
//! member names without `await`.
//!
//! - A round trip, a query and a waiter are `async def` methods, plain in
//!   the twin. A required parameter is positional in the catalog's order,
//!   but a boolean, which is keyword-only like every optional one. An
//!   optional one defaults to the `not_given` sentinel and is sent only when
//!   passed, so `None` sends null where its type admits it: no timeout,
//!   where leaving it out takes the setting's default (ADR 0080).
//! - A `new_<value>` member returns the handle it opens, and a handle the
//!   program closes is an async context manager, a context manager in the
//!   twin.
//! - A value built without a call is a sync method, or a property when it
//!   takes nothing; a sub-object is a property.
//! - A result typed by the fields its call declares (ADR 0147) is the
//!   result's model: Python's type system has no mapped types.
//!
//! - A parameter whose model wraps one wire value, a string, number or
//!   boolean of a type of its own or a choice of strings, takes that value,
//!   a choice as its `Literal` strings, and the member wraps it.
//! - A call's own `timeout` takes a `timedelta`, never a number of
//!   milliseconds, which `timeout_of` sends in whole milliseconds, rounded
//!   up, refusing one of zero or less before it is sent; the docstring
//!   gives the default of the setting it sets for the call alone, as a
//!   `timedelta` (ADR 0080).
//!
//! - A handle's listener is its `on`, overloaded once a payload so each
//!   event's listener is typed (`listeners`, ADR 0081).
//!
//! Each parameter's annotation is read from its schema; one the generator
//! cannot spell fails `xtask codegen` with the operation named, and the
//! repository's type checkers check each against the model's own, which
//! the handle constructs the params with.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::super::handles::{Builder, Form, Handle, HandleId, Member, Parameters};
use super::super::layer::{Layer, Stability};
use super::super::naming::{pascal, snake};
use super::{CALLER_MODULE, deprecated_decorator, docstring, header, markers_import, warnings};

mod listeners;

/// The hand-written module of Collie's client package that declares what a
/// generated handle reaches its engine through.
const HANDLE_MODULE: &str = "goodfetch_collie_client.handle";
/// The module of the hand-written base each handle class extends: the raw
/// call on its value (ADR 0099).
const RAW_CALL_MODULE: &str = "goodfetch_collie_client.raw_call";

/// A call's own optional parameter that bounds it (ADR 0080).
const TIMEOUT: &str = "timeout";

/// The hand-written function of [`HANDLE_MODULE`] that sends a call's own
/// `timeout`, refusing one no deadline can be before it is sent (ADR 0080).
const TIMEOUT_OF: &str = "timeout_of";

/// One of the two forms of each handle: the async class or its sync twin.
#[derive(Clone, Copy)]
struct Mode {
    /// The prefix of each class name, `Async` or nothing.
    prefix: &'static str,
    /// The host type the class takes.
    host: &'static str,
    /// The hand-written class each handle extends, which gives its raw call.
    base: &'static str,
    /// `async ` or nothing, before `def`.
    asynchronous: &'static str,
    /// `await ` or nothing, before a call.
    awaits: &'static str,
    /// The methods that make a handle the program closes a context
    /// manager, and the block that holds it.
    enter: &'static str,
    exit: &'static str,
    block: &'static str,
}

const ASYNC: Mode = Mode {
    prefix: "Async",
    host: "AsyncHandleHost",
    base: "AsyncHandle",
    asynchronous: "async ",
    awaits: "await ",
    enter: "__aenter__",
    exit: "__aexit__",
    block: "an `async with` block",
};

const SYNC: Mode = Mode {
    prefix: "",
    host: "HandleHost",
    base: "Handle",
    asynchronous: "",
    awaits: "",
    enter: "__enter__",
    exit: "__exit__",
    block: "a `with` block",
};

/// What the module imports besides its fixed lines.
#[derive(Default)]
struct Imports {
    /// The names it imports from `typing`, besides `Self`.
    typing: BTreeSet<&'static str>,
    models: bool,
    deprecated: bool,
    /// Whether it names pydantic's `BaseModel`, as a listener's map of
    /// payloads does.
    base_model: bool,
    /// The functions of the markers module its members call.
    markers: BTreeSet<&'static str>,
    /// The names it imports from the handle module besides the hosts: the
    /// `not_given` sentinel when a member takes an optional parameter, and
    /// [`TIMEOUT_OF`] when a member takes its own `timeout`, a `timedelta`.
    handle: BTreeSet<&'static str>,
}

impl Imports {
    /// The model `name` as the module spells it: through its module, as
    /// TypeScript's handles reach `types`, so a model named as a handle,
    /// such as `Locator`, never shadows the handle's class.
    fn model(&mut self, name: &str) -> String {
        self.models = true;
        format!("models.{name}")
    }
}

/// A parameter whose model wraps one wire value: a string, number or
/// boolean of a type of its own, or one of a choice of strings. The member
/// takes the wire value, as TypeScript's does, a choice as its `Literal`
/// strings (ADR 0099: enum values are data), and wraps it in the model.
struct Wrapped {
    /// The annotation of the wire value.
    annotation: String,
    /// The model that wraps it.
    model: String,
    /// Whether the parameter also admits null, which is passed unwrapped.
    nullable: bool,
}

/// The Python type of a JSON Schema primitive `type`, or `None` for
/// another.
fn primitive(kind: &str) -> Option<&'static str> {
    match kind {
        "string" => Some("str"),
        "integer" => Some("int"),
        "number" => Some("float"),
        "boolean" => Some("bool"),
        _ => None,
    }
}

/// `schema`'s wrapped value, when it is a reference, alone or beside
/// `null`, to a definition that wraps one wire value.
fn wrapped(schema: &Value, layer: &Layer, imports: &mut Imports) -> Option<Wrapped> {
    let (reference, nullable) = match schema["anyOf"].as_array() {
        Some(variants) => {
            let mut values = variants.iter().filter(|variant| variant["type"] != "null");
            let only = values.next()?;
            let only = values.next().is_none().then_some(only)?;
            (only, variants.len() > 1)
        }
        None => (schema, false),
    };
    let model = reference["$ref"].as_str()?.strip_prefix("#/$defs/")?;
    let definition = layer.definitions.get(model)?;
    if definition.get("properties").is_some() {
        return None;
    }
    let choices = definition["oneOf"]
        .as_array()
        .map(|variants| {
            variants
                .iter()
                .map(|variant| variant["const"].as_str())
                .collect::<Option<Vec<_>>>()
        })
        .or_else(|| {
            definition["enum"]
                .as_array()
                .map(|values| values.iter().map(Value::as_str).collect())
        });
    let annotation = match choices {
        Some(Some(choices)) if !choices.is_empty() => {
            imports.typing.insert("Literal");
            let choices: Vec<String> = choices.iter().map(|choice| format!("{choice:?}")).collect();
            format!("Literal[{}]", choices.join(", "))
        }
        Some(_) => return None,
        None => primitive(definition["type"].as_str()?)?.to_owned(),
    };
    Some(Wrapped {
        annotation,
        model: imports.model(model),
        nullable,
    })
}

/// The keywords that describe a value without restricting it: the JSON
/// Schema annotations, and the bundle's `arrived_in`, the version that
/// added it.
const ANNOTATIONS: [&str; 7] = [
    "$comment",
    "arrived_in",
    "default",
    "deprecated",
    "description",
    "examples",
    "title",
];

/// A parameter's annotation, read from its schema, or `None` for a schema
/// it cannot spell.
fn annotation(schema: &Value, imports: &mut Imports) -> Option<String> {
    let object = schema.as_object()?;
    // A schema that only describes its value, such as a JSON argument's,
    // admits any JSON.
    if object
        .keys()
        .all(|keyword| ANNOTATIONS.contains(&keyword.as_str()))
    {
        imports.typing.insert("Any");
        return Some("Any".to_owned());
    }
    if let Some(name) = schema["$ref"].as_str() {
        let name = name.strip_prefix("#/$defs/")?;
        return Some(imports.model(name));
    }
    if let Some(variants) = schema["anyOf"].as_array() {
        return union(variants.iter().map(|variant| annotation(variant, imports)));
    }
    match &schema["type"] {
        Value::Array(types) => union(types.iter().map(|kind| {
            let mut single = schema.clone();
            single["type"] = kind.clone();
            annotation(&single, imports)
        })),
        Value::String(kind) => {
            if schema.get("enum").is_some() || schema.get("const").is_some() {
                return None;
            }
            if let Some(spelled) = primitive(kind) {
                return Some(spelled.to_owned());
            }
            match kind.as_str() {
                "null" => Some("None".to_owned()),
                "array" => match schema.get("items") {
                    None => {
                        imports.typing.insert("Any");
                        Some("list[Any]".to_owned())
                    }
                    Some(items) => Some(format!("list[{}]", annotation(items, imports)?)),
                },
                "object" if schema.get("properties").is_none() => {
                    match schema.get("additionalProperties") {
                        None | Some(Value::Bool(true)) => {
                            imports.typing.insert("Any");
                            Some("dict[str, Any]".to_owned())
                        }
                        Some(values @ Value::Object(_)) => {
                            Some(format!("dict[str, {}]", annotation(values, imports)?))
                        }
                        Some(_) => None,
                    }
                }
                _ => None,
            }
        }
        _ => None,
    }
}

/// The union of `variants`, `None` last, each once.
fn union(variants: impl Iterator<Item = Option<String>>) -> Option<String> {
    let mut spelled = Vec::new();
    for variant in variants {
        let variant = variant?;
        if !spelled.contains(&variant) {
            spelled.push(variant);
        }
    }
    if let Some(null) = spelled.iter().position(|variant| variant == "None") {
        let null = spelled.remove(null);
        spelled.push(null);
    }
    (!spelled.is_empty()).then(|| spelled.join(" | "))
}

/// A method's parameter list after `self`, and the lines that build its
/// params as `model`: the handle's ID first, then the positional ones, then
/// the keyword-only ones. An optional parameter defaults to `not_given` and
/// is set on the model, which then sends it, only when passed: `None` is a
/// value, null on the wire, where its type admits it (ADR 0080).
struct Signature {
    declared: String,
    /// The constructor call, with every parameter but the optional ones.
    construct: String,
    /// The optional ones, each set on the built model when passed: its
    /// name and the expression that passes it.
    optional: Vec<(String, String)>,
}

fn signature(
    layer: &Layer,
    owner: &str,
    model: &str,
    parameters: &Parameters,
    timeout_setting: Option<&str>,
    id: &HandleId,
    imports: &mut Imports,
) -> Result<Signature> {
    let schema = layer
        .definitions
        .get(model)
        .with_context(|| format!("operation `{owner}`: the bundle has no type `{model}`"))?;
    // Each parameter's annotation, and the expression that passes it to
    // the model.
    let spell = |name: &str, imports: &mut Imports| -> Result<(String, String)> {
        if snake(name) != name {
            bail!(
                "operation `{owner}`: its parameter `{name}` is not a Python name, which a handle's member takes it by (ADR 0099)"
            );
        }
        let property = &schema["properties"][name];
        if let Some(Wrapped {
            annotation,
            model,
            nullable,
        }) = wrapped(property, layer, imports)
        {
            return Ok(if nullable {
                (
                    format!("{annotation} | None"),
                    format!("None if {name} is None else {model}({name})"),
                )
            } else {
                (annotation, format!("{model}({name})"))
            });
        }
        let Some(spelled) = annotation(property, imports) else {
            bail!(
                "operation `{owner}`: the Python handles cannot spell the type of its parameter `{name}`: give it a type of its own in the bundle (ADR 0099)"
            );
        };
        Ok((spelled, name.to_owned()))
    };
    let mut declared = Vec::new();
    let mut entries = vec![format!("{}=self._host.id", id.param)];
    for name in &parameters.positional {
        let (spelled, passed) = spell(name, imports)?;
        declared.push(format!("{name}: {spelled}"));
        entries.push(format!("{name}={passed}"));
    }
    if !(parameters.named.is_empty() && parameters.optional.is_empty()) {
        declared.push("*".to_owned());
    }
    for name in &parameters.named {
        let (spelled, passed) = spell(name, imports)?;
        declared.push(format!("{name}: {spelled}"));
        entries.push(format!("{name}={passed}"));
    }
    let mut optional = Vec::new();
    for name in &parameters.optional {
        let (spelled, passed) = match timeout_setting.filter(|_| name == TIMEOUT) {
            Some(setting) => {
                imports.handle.insert(TIMEOUT_OF);
                (
                    "timedelta | None".to_owned(),
                    format!("{TIMEOUT_OF}({name}, {setting:?})"),
                )
            }
            None => spell(name, imports)?,
        };
        imports.handle.extend(["NotGiven", "not_given"]);
        declared.push(format!("{name}: {spelled} | NotGiven = not_given"));
        optional.push((name.clone(), passed));
    }
    let model = imports.model(model);
    Ok(Signature {
        declared: declared.iter().fold(String::new(), |mut text, parameter| {
            let _ = write!(text, ", {parameter}");
            text
        }),
        construct: format!("{model}({})", entries.join(", ")),
        optional,
    })
}

/// The body lines that build `signature`'s params as `local`, and the
/// expression that names them: the constructor call itself when no
/// optional parameter follows.
fn build(signature: &Signature, local: &str, text: &mut String) -> String {
    if signature.optional.is_empty() {
        return signature.construct.clone();
    }
    let _ = writeln!(text, "        {local} = {}", signature.construct);
    for (name, passed) in &signature.optional {
        let _ = writeln!(
            text,
            "        if not isinstance({name}, NotGiven):\n            {local}.{name} = {passed}"
        );
    }
    local.to_owned()
}

/// One member, a method of the class that holds `self._host`.
fn member(
    layer: &Layer,
    member: &Member<'_>,
    id: &HandleId,
    mode: Mode,
    imports: &mut Imports,
) -> Result<String> {
    let operation = member.operation;
    let (params, result) = (&operation.params.0, &operation.result.0);
    let signature = signature(
        layer,
        &operation.name,
        params,
        &member.parameters,
        member.timeout_setting.as_deref(),
        id,
        imports,
    )?;
    let Mode {
        prefix,
        asynchronous,
        awaits,
        ..
    } = mode;
    let result = imports.model(result);
    let returns = match &member.form {
        Form::RoundTrip | Form::Query | Form::Waiter => result.clone(),
        Form::Opens(opened) => format!("{prefix}{}", pascal(opened)),
        Form::Closes => "None".to_owned(),
    };
    let mut text = String::new();
    if let Some(deprecated) = &operation.deprecated {
        imports.deprecated = true;
        text.push_str(&deprecated_decorator(deprecated));
    }
    let _ = writeln!(
        text,
        "    {asynchronous}def {}(self{}) -> {returns}:",
        member.name, signature.declared
    );
    let description = match &member.timeout_setting {
        Some(setting) => format!(
            "{}\n\n{}",
            operation.description,
            timeout_default(layer, &operation.name, setting)?
        ),
        None => operation.description.clone(),
    };
    text.push_str(&docstring(
        "        ",
        &description,
        operation.stability,
        operation.deprecated.as_ref(),
    ));
    text.push_str(&warnings(
        &operation.name,
        operation.stability,
        operation.deprecated.as_ref(),
        &mut imports.markers,
    ));
    let sent = build(&signature, "params", &mut text);
    let call = format!("{awaits}self._host.call({:?}, {sent})", operation.name);
    match &member.form {
        Form::RoundTrip | Form::Query | Form::Waiter => {
            let _ = writeln!(text, "        return result_of({result}, {call})");
        }
        Form::Opens(opened) => {
            let _ = writeln!(
                text,
                "        opened = result_of({result}, {call})\n        return {returns}(self._host.child(opened.{opened}_id))"
            );
        }
        Form::Closes => {
            let _ = writeln!(
                text,
                "        {awaits}self._host.close({:?}, {sent})",
                operation.name
            );
        }
    }
    Ok(text)
}

/// The sentence that gives the default of `setting`, the Duration setting
/// a call's own `timeout` sets for the call alone, as a `timedelta`, in
/// the largest unit that keeps it whole, or as none.
fn timeout_default(layer: &Layer, owner: &str, setting: &str) -> Result<String> {
    let default = match layer
        .setting_defaults
        .get(setting)
        .map(|default| (default, default.as_u64()))
    {
        Some((Value::Null, _)) => "none".to_owned(),
        Some((_, Some(millis))) => {
            let (unit, size) = [
                ("days", 86_400_000),
                ("hours", 3_600_000),
                ("minutes", 60_000),
                ("seconds", 1_000),
            ]
            .into_iter()
            .find(|(_, size)| millis > 0 && millis % size == 0)
            .unwrap_or(("milliseconds", 1));
            format!("`timedelta({unit}={})`", millis / size)
        }
        _ => bail!(
            "operation `{owner}`: its `timeout` sets `{setting}`, which the catalog gives no default of whole milliseconds or null"
        ),
    };
    Ok(format!(
        "Left out, `timeout` takes the Run's `{setting}`, by default {default}."
    ))
}

/// A value `handle` builds without a call: a sync method, or a property
/// when it takes nothing.
fn builder(
    layer: &Layer,
    builder: &Builder,
    id: &HandleId,
    mode: Mode,
    imports: &mut Imports,
) -> Result<String> {
    let built = format!("{}{}", mode.prefix, pascal(&builder.builds));
    let signature = signature(
        layer,
        &builder.builds,
        &builder.type_name,
        &builder.parameters,
        None,
        id,
        imports,
    )?;
    let mut text = String::new();
    if signature.declared.is_empty() {
        text.push_str("    @property\n");
    }
    let _ = writeln!(
        text,
        "    def {}(self{}) -> {built}:",
        snake(&builder.builds),
        signature.declared
    );
    text.push_str(&docstring(
        "        ",
        &format!(
            "{}\n\nBuilt without a call (ADR 0099).",
            builder.description
        ),
        Stability::Stable,
        None,
    ));
    let value = build(&signature, "value", &mut text);
    let _ = writeln!(text, "        return {built}(self._host.child({value}))");
    Ok(text)
}

/// One handle's classes in `mode`: its sub-objects', then its own.
fn classes(
    layer: &Layer,
    handle: &Handle<'_>,
    mode: Mode,
    imports: &mut Imports,
) -> Result<String> {
    let class = format!("{}{}", mode.prefix, pascal(handle.namespace));
    let host = format!("{}[{}]", mode.host, imports.model(&handle.id.type_name));
    let constructor = format!(
        "\n    def __init__(self, host: {host}) -> None:\n        \"\"\"Takes what it reaches its engine through; a program never constructs one.\"\"\"\n        self._host = host\n"
    );
    let mut text = String::new();
    // The async classes come first, so the listener's map precedes both.
    if mode.prefix == ASYNC.prefix {
        text.push_str(&listeners::declarations(handle, imports));
    }
    for sub_object in &handle.sub_objects {
        let _ = write!(
            text,
            "\n\nclass {class}{}:\n    \"\"\"The `{}` of a `{}` (ADR 0099).\"\"\"\n{constructor}",
            pascal(sub_object.name),
            sub_object.name,
            handle.namespace
        );
        for held in &sub_object.members {
            text.push('\n');
            text.push_str(&member(layer, held, &handle.id, mode, imports)?);
        }
    }
    let _ = write!(
        text,
        "\n\nclass {class}({}):\n    \"\"\"A `{}` a program holds: the members the catalog gives it (ADR 0099).\"\"\"\n{}\n        super().__init__(host, {:?}, {:?})\n",
        mode.base,
        handle.namespace,
        constructor.trim_end(),
        handle.namespace,
        handle.id.param
    );
    for sub_object in &handle.sub_objects {
        let _ = writeln!(
            text,
            "        self._{} = {class}{}(host)",
            sub_object.name,
            pascal(sub_object.name)
        );
    }
    for sub_object in &handle.sub_objects {
        let _ = write!(
            text,
            "\n    @property\n    def {name}(self) -> {class}{}:\n        \"\"\"Its `{name}`.\"\"\"\n        return self._{name}\n",
            pascal(sub_object.name),
            name = sub_object.name
        );
    }
    for held in &handle.members {
        text.push('\n');
        text.push_str(&member(layer, held, &handle.id, mode, imports)?);
    }
    if handle.listener.is_some() {
        text.push('\n');
        text.push_str(&listeners::member(layer, handle, mode, imports)?);
    }
    for built in &handle.builders {
        text.push('\n');
        text.push_str(&builder(layer, built, &handle.id, mode, imports)?);
    }
    if handle.closes() {
        let Mode {
            asynchronous,
            awaits,
            enter,
            exit,
            block,
            ..
        } = mode;
        let _ = write!(
            text,
            "\n    {asynchronous}def {enter}(self) -> Self:\n        \"\"\"Holds it for {block}, which closes it.\"\"\"\n        return self\n\n    {asynchronous}def {exit}(self, *_: object) -> None:\n        \"\"\"Closes it.\"\"\"\n        {awaits}self.close()\n"
        );
    }
    Ok(text)
}

/// The handles module: each handle's async classes, then their sync twins.
pub(super) fn handles_module(layer: &Layer, handles: &[Handle<'_>]) -> Result<String> {
    let mut imports = Imports::default();
    let mut body = String::new();
    for mode in [ASYNC, SYNC] {
        for handle in handles {
            body.push_str(&classes(layer, handle, mode, &mut imports)?);
        }
    }
    let mut text = header(&format!(
        "The handles a program holds of {}'s values (ADR 0099), each an async class and its sync twin.",
        layer.product.name()
    ));
    text.push('\n');
    if imports.handle.contains(TIMEOUT_OF) {
        text.push_str("from datetime import timedelta\n");
    }
    imports.typing.insert("Self");
    let typing: Vec<&str> = imports.typing.into_iter().collect();
    let _ = writeln!(text, "from typing import {}", typing.join(", "));
    if imports.deprecated || imports.base_model {
        text.push('\n');
    }
    if imports.base_model {
        text.push_str("from pydantic import BaseModel\n");
    }
    if imports.deprecated {
        text.push_str("from typing_extensions import deprecated\n");
    }
    let handle = imports
        .handle
        .iter()
        .fold(String::new(), |names, name| names + ", " + name);
    let _ = writeln!(
        text,
        "\nfrom {CALLER_MODULE} import result_of\nfrom {HANDLE_MODULE} import AsyncHandleHost, HandleHost{handle}"
    );
    text.push_str(&markers_import(&imports.markers));
    let _ = writeln!(text, "from {RAW_CALL_MODULE} import AsyncHandle, Handle");
    if imports.models {
        text.push_str("\nfrom . import models\n");
    }
    text.push_str(&body);
    Ok(text)
}
