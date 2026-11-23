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
//! - A read is a property, or a sync method when it is a yes or no (`is_`)
//!   or a conversion (`as_`), the same in both classes: it answers from the
//!   answer that opened the handle, which the handle is built with, and
//!   sends nothing.
//! - A result typed by the fields its call declares (ADR 0147) is the
//!   result's model: Python's type system has no mapped types.
//! - A round trip, a query or a waiter whose result has one property
//!   answers that property's value in place of the result, `None` for an
//!   optional one that is absent, as TypeScript's and Rust's do (issue
//!   2883): `goto` answers the `PageResponse` or `None`.
//!
//! - A parameter whose model wraps one wire value, a string, number or
//!   boolean of a type of its own or a choice of strings, takes that value,
//!   a choice as its `Literal` strings, and the member wraps it.
//! - A `new_<value>` member's `settings` takes the scope's settings type,
//!   which `settings_on_wire` sends, durations in whole milliseconds
//!   (`settings`, ADR 0083).
//! - A call's own `timeout` takes a `timedelta`, never a number of
//!   milliseconds, which `timeout_of` sends in whole milliseconds, rounded
//!   up, refusing one of zero or less before it is sent; the docstring
//!   gives the default of the setting it sets for the call alone, as a
//!   `timedelta` (ADR 0080).
//!
//! - A handle's listener is its `on`, overloaded once per type its
//!   listener gets so each event's listener is typed (`listeners`, ADR
//!   0081). A Session's gets the pages it opened as page handles.
//! - A member with deprecated forms, a deprecated keyword or a deprecated
//!   value of a choice, has an `@overload` free of them, then one for each,
//!   marked with PEP 702's `@deprecated`, which takes that keyword or that
//!   value alone, so pyright and mypy report a call that uses it and no
//!   other (ADR 0099), and last one that takes every form, as the
//!   implementation does, so a call whose value only its type gives still
//!   resolves.
//!
//! Each parameter's annotation is read from its schema; one the generator
//! cannot spell fails `xtask codegen` with the operation named, and the
//! repository's type checkers check each against the model's own, which
//! the handle constructs the params with.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use serde_json::Value;

use super::super::handles::{
    Builder, DEPRECATION, Form, Handle, HandleId, Member, Overload, Parameters, Read,
};
use super::super::layer::{Layer, Stability};
use super::super::naming::{pascal, snake};
use super::super::settings_tree::SETTINGS;
use super::settings::{
    DEPRECATED_SETTINGS, SETTINGS_ON_WIRE, SETTINGS_ON_WIRE_MODULE, WARN_DEPRECATED_SETTINGS,
    scope_settings,
};
use super::{CALLER_MODULE, deprecated_decorator, docstring, header, markers_import, warnings};

mod answered;
mod listeners;

use answered::{Returned, answered, returned};

/// The hand-written module of Collie's client package that declares what a
/// generated handle reaches its engine through.
const HANDLE_MODULE: &str = "goodfetch_collie_client.handle";
/// The module of the hand-written base each handle class extends: the raw
/// call on its value (ADR 0099).
const RAW_CALL_MODULE: &str = "goodfetch_collie_client.raw_call";

/// The markers module's function a listener warns of its event's markers
/// with, read from the events module's registries.
const WARN_ENTRY: &str = "warn_entry";

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
    /// The functions of the markers module its members call: a listener's
    /// `warn_entry` reads the events' markers besides.
    markers: BTreeSet<&'static str>,
    /// The names it imports from the handle module besides the hosts: the
    /// `not_given` sentinel when a member takes an optional parameter, and
    /// [`TIMEOUT_OF`] when a member takes its own `timeout`, a `timedelta`.
    handle: BTreeSet<&'static str>,
    /// The settings types its members take, from the generated settings
    /// module, with [`SETTINGS_ON_WIRE`] when there is one.
    settings: BTreeSet<&'static str>,
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
/// `null`, to a definition that wraps one wire value: of a choice, the
/// values `takes` keeps.
fn wrapped(
    schema: &Value,
    layer: &Layer,
    imports: &mut Imports,
    takes: impl Fn(&str) -> bool,
) -> Option<Wrapped> {
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
            let choices: Vec<String> = choices
                .iter()
                .filter(|choice| takes(choice))
                .map(|choice| format!("{choice:?}"))
                .collect();
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
/// added it, and `deprecation`, the deprecation it marks (ADR 0099).
const ANNOTATIONS: [&str; 8] = [
    "$comment",
    "arrived_in",
    "default",
    "deprecated",
    DEPRECATION,
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
    /// The optional ones, each set on the built model when passed.
    optional: Vec<Optional>,
}

/// An optional parameter, set on the built model when passed.
struct Optional {
    name: String,
    /// The expression that passes it.
    passed: String,
    /// The statement run first when it is passed: `settings`' warning of
    /// each deprecated setting it sets (ADR 0099).
    first: Option<String>,
}

/// The optional parameters a member types from the catalog rather than the
/// bundle: `timeout` after its timeout setting (ADR 0080), and `settings`
/// as the type of the scope it opens (ADR 0083).
#[derive(Clone, Copy, Default)]
struct CatalogTyped<'a> {
    timeout_setting: Option<&'a str>,
    settings_type: Option<&'static str>,
}

#[expect(
    clippy::too_many_arguments,
    reason = "each is one input of the one signature, which a struct would only rename"
)]
fn signature(
    layer: &Layer,
    owner: &str,
    model: &str,
    parameters: &Parameters,
    typed: CatalogTyped<'_>,
    id: &HandleId,
    overload: Overload<'_>,
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
        }) = wrapped(property, layer, imports, |value| {
            overload.takes(name, value)
        }) {
            // A deprecated value's overload takes that value alone.
            let nullable = nullable && !overload.chosen_value_of(name);
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
        if overload.leaves_out(name) {
            continue;
        }
        let mut first = None;
        let (spelled, passed) = match (
            typed.timeout_setting.filter(|_| name == TIMEOUT),
            typed.settings_type.filter(|_| name == SETTINGS),
        ) {
            (Some(setting), _) => {
                imports.handle.insert(TIMEOUT_OF);
                (
                    "timedelta | None".to_owned(),
                    format!("{TIMEOUT_OF}({name}, {setting:?})"),
                )
            }
            (None, Some(settings)) => {
                imports.settings.insert(settings);
                imports.markers.insert(WARN_DEPRECATED_SETTINGS);
                first = Some(format!(
                    "{WARN_DEPRECATED_SETTINGS}({name}, {DEPRECATED_SETTINGS})"
                ));
                (settings.to_owned(), format!("{SETTINGS_ON_WIRE}({name})"))
            }
            (None, None) => spell(name, imports)?,
        };
        // A deprecated form's overload is chosen by passing it.
        if overload.chosen_by(name) {
            declared.push(format!("{name}: {spelled}"));
        } else {
            imports.handle.extend(["NotGiven", "not_given"]);
            declared.push(format!("{name}: {spelled} | NotGiven = not_given"));
        }
        optional.push(Optional {
            name: name.clone(),
            passed,
            first,
        });
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
    for Optional {
        name,
        passed,
        first,
    } in &signature.optional
    {
        let _ = writeln!(text, "        if not isinstance({name}, NotGiven):");
        if let Some(first) = first {
            let _ = writeln!(text, "            {first}");
        }
        let _ = writeln!(text, "            {local}.{name} = {passed}");
    }
    local.to_owned()
}

/// The description of `member`'s docstring: its operation's, then the
/// default its own `timeout` takes and each of its deprecated forms.
fn description(layer: &Layer, member: &Member<'_>) -> Result<String> {
    let operation = member.operation;
    let description = match &member.timeout_setting {
        Some(setting) => format!(
            "{}\n\n{}",
            operation.description,
            timeout_default(layer, &operation.name, setting)?
        ),
        None => operation.description.clone(),
    };
    Ok(member
        .deprecated_forms
        .iter()
        .fold(description, |mut text, form| {
            let used = match &form.value {
                Some(value) => format!("`{}={value:?}`", form.param),
                None => format!("`{}`", form.param),
            };
            let _ = write!(
                text,
                "\n\nDeprecated: {used}. {}",
                form.deprecation.message()
            );
            text
        }))
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
    let typed = CatalogTyped {
        timeout_setting: member.timeout_setting.as_deref(),
        settings_type: scope_settings(member)?,
    };
    let signature_of = |overload, imports: &mut Imports| {
        signature(
            layer,
            &operation.name,
            params,
            &member.parameters,
            typed,
            id,
            overload,
            imports,
        )
    };
    let signature = signature_of(Overload::Every, imports)?;
    let Mode {
        prefix,
        asynchronous,
        awaits,
        ..
    } = mode;
    let answered = answered(layer, member, imports)?;
    let result = imports.model(result);
    let returns = match &member.form {
        Form::RoundTrip | Form::Query | Form::Waiter => answered
            .as_ref()
            .map_or_else(|| result.clone(), |(returns, _)| returns.clone()),
        Form::Opens(opened) => format!("{prefix}{}", pascal(opened)),
        Form::Closes => "None".to_owned(),
    };
    let mut text = String::new();
    if let Some(deprecated) = &operation.deprecated {
        imports.deprecated = true;
        text.push_str(&deprecated_decorator(deprecated));
    }
    if !member.deprecated_forms.is_empty() {
        imports.typing.insert("overload");
        imports.deprecated = true;
        let current = signature_of(Overload::Current(&member.deprecated_forms), imports)?;
        let _ = write!(
            text,
            "    @overload\n    {asynchronous}def {}(self{}) -> {returns}: ...\n\n",
            member.name, current.declared
        );
        for form in &member.deprecated_forms {
            let deprecated = signature_of(Overload::Deprecated(form), imports)?;
            let _ = write!(
                text,
                "    @overload\n{}    {asynchronous}def {}(self{}) -> {returns}: ...\n\n",
                deprecated_decorator(&form.deprecation),
                member.name,
                deprecated.declared
            );
        }
        let _ = write!(
            text,
            "    @overload\n    {asynchronous}def {}(self{}) -> {returns}: ...\n\n",
            member.name, signature.declared
        );
    }
    let _ = writeln!(
        text,
        "    {asynchronous}def {}(self{}) -> {returns}:",
        member.name, signature.declared
    );
    text.push_str(&docstring(
        "        ",
        &description(layer, member)?,
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
    let property = answered.map(|(_, property)| property);
    text.push_str(&returned(
        member,
        Returned {
            result: &result,
            returns: &returns,
            property: property.as_deref(),
        },
        awaits,
        &sent,
    ));
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
        CatalogTyped::default(),
        id,
        Overload::Every,
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

/// One read, the same in both modes since it sends nothing: a property, or
/// a method for a yes or no (`is_`) or a conversion (`as_`), which
/// answers the value of the answer that opened its handle (issue 2798).
fn read(layer: &Layer, read: &Read<'_>, imports: &mut Imports) -> Result<String> {
    let operation = read.operation;
    let property = layer
        .definitions
        .get(&operation.result.0)
        .map(|result| &result["properties"][&read.property])
        .with_context(|| format!("operation `{}` has no result type", operation.name))?;
    let Some(returns) = annotation(property, imports) else {
        bail!(
            "operation `{}`: no Python annotation for its result's `{}`",
            operation.name,
            read.property
        );
    };
    let decorator = if read.name.starts_with("is_") || read.name.starts_with("as_") {
        ""
    } else {
        "    @property\n"
    };
    let mut text = decorator.to_owned();
    if let Some(deprecated) = &operation.deprecated {
        imports.deprecated = true;
        text.push_str(&deprecated_decorator(deprecated));
    }
    let _ = writeln!(text, "    def {}(self) -> {returns}:", read.name);
    text.push_str(&docstring(
        "        ",
        &operation.description,
        operation.stability,
        operation.deprecated.as_ref(),
    ));
    text.push_str(&warnings(
        &operation.name,
        operation.stability,
        operation.deprecated.as_ref(),
        &mut imports.markers,
    ));
    let _ = writeln!(text, "        return self._opened.{}", read.property);
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
    // A handle with reads is built with the answer that opened it, which
    // they answer from.
    let own_constructor = match &handle.reads {
        None => constructor.trim_end().to_owned(),
        Some(reads) => format!(
            "\n    def __init__(self, host: {host}, opened: {}) -> None:\n        \"\"\"Takes what it reaches its engine through and the answer that opened it; a program never constructs one.\"\"\"\n        self._host = host\n        self._opened = opened",
            imports.model(&reads.opened)
        ),
    };
    let _ = write!(
        text,
        "\n\nclass {class}({}):\n    \"\"\"A `{}` a program holds: the members the catalog gives it (ADR 0099).\"\"\"\n{own_constructor}\n        super().__init__(host, {:?}, {:?})\n",
        mode.base, handle.namespace, handle.namespace, handle.id.param
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
    for held in handle.reads() {
        text.push('\n');
        text.push_str(&read(layer, held, imports)?);
    }
    if handle.listener.is_some() || handle.published_listener.is_some() {
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
    if !imports.settings.is_empty() {
        let _ = writeln!(
            text,
            "from {SETTINGS_ON_WIRE_MODULE} import {SETTINGS_ON_WIRE}"
        );
    }
    let events = imports.markers.contains(WARN_ENTRY);
    if imports.models || events || !imports.settings.is_empty() {
        text.push('\n');
    }
    if imports.models {
        text.push_str("from . import models\n");
    }
    if events {
        text.push_str("from .events import DEPRECATED_EVENTS, EXPERIMENTAL_EVENTS\n");
    }
    if !imports.settings.is_empty() {
        let types: Vec<&str> = imports.settings.iter().copied().collect();
        let _ = writeln!(
            text,
            "from .settings import {DEPRECATED_SETTINGS}, {}",
            types.join(", ")
        );
    }
    text.push_str(&body);
    Ok(text)
}
