//! The Python layer of one SDK package (ADRs 0038 and 0095): pydantic v2
//! models from `datamodel-code-generator`, and, written here, each operation
//! as an async method and a sync one, the event registry, one exception class
//! per Error code (the seven group classes in Collie's client package, on the
//! base its hand-written `goodfetch_error` module declares), the base class
//! of the handlers of each Plugin kind, with the
//! catalog's descriptions as Google-style docstrings and its Experimental and
//! deprecated markers, and the constants a Connection opens with: the
//! protocol and Product versions `hello` names, the engine's command and
//! the variable naming its executable, Collie's handles in their ADR 0099
//! forms (`handle_classes`) and the types of its scopes' `settings`
//! (`settings`, ADR 0083), and Foxhound's those of a crawl and a Label.
//! Foxhound's layer imports what Collie's client package defines.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

use anyhow::Result;
use serde_json::Value;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::cli::ATTACHED_ENGINE;
use goodfetch_protocol::foxhound::crawl::ATTEMPT_ENDED;
use goodfetch_protocol::settings::{
    COLLIE_ENGINE_PATH_VARIABLE, DEPLOYMENT_PREFIX, DEPLOYMENT_VARIABLES,
    FOXHOUND_ENGINE_PATH_VARIABLE,
};

use super::handles;
use super::layer::{Deprecation, ErrorCode, Layer, Stability};
use super::naming::{pascal, snake};
use super::open_values::{self, OpenValues};
use super::{EXPERIMENTAL, Generated, TypesFile};

/// Collie's generated layer, as Foxhound's layer imports it.
const CLIENT_PACKAGE: &str = "goodfetch_collie_client.generated";

/// The hand-written module of Collie's client package that declares what
/// sends a call and reads its result, which every layer's methods take.
const CALLER_MODULE: &str = "goodfetch_collie_client.caller";

/// The hand-written module of Collie's client package whose functions give
/// ADR 0030's run-time warning of a deprecated or Experimental member.
const MARKERS_MODULE: &str = "goodfetch_collie_client.markers";

/// The generated module of a layer's open enumerations (ADR 0079).
const OPEN_VALUES: &str = "open_values";

/// The generated package of `product`'s layer.
fn own_package(product: Product) -> &'static str {
    match product {
        Product::Collie => CLIENT_PACKAGE,
        Product::Foxhound => "goodfetch_foxhound.generated",
    }
}

/// The module of a layer's open enumerations (ADR 0079): each its known
/// values or any other string, which its models refer to, since
/// `datamodel-code-generator` writes a closed `StrEnum` for one.
fn open_values_module(layer: &Layer, open: &[(String, OpenValues)]) -> String {
    let mut text = header(&format!(
        "{}'s open enumerations: each its known values or a value a newer engine adds (ADR 0079).",
        layer.product.name()
    ));
    text.push_str("\nfrom typing import Literal, TypeAlias\n");
    for (name, values) in open {
        let _ = writeln!(
            text,
            "\n{name}: TypeAlias = Literal[{}] | str",
            values.literals().join(", ")
        );
        let _ = writeln!(text, "\"\"\"{}\n\"\"\"", escape(&values.documentation()));
    }
    text
}

/// The hand-written module of Collie's client package that declares the
/// base of every exception class, which every layer's classes extend.
const GOODFETCH_ERROR_MODULE: &str = "goodfetch_collie_client.goodfetch_error";

/// The hand-written base of every open tagged union's `unknown` member
/// (ADR 0079), which `caller.py` builds a member it does not know as.
const UNKNOWN_MEMBER_BASE: &str = "goodfetch_collie_client.unknown_member.UnknownMember";

/// The hand-written base, outside `Exception`, of the one code's class that
/// a program's `except Exception:` must let through (ADR 0093).
const BASE_ERROR: &str = "BaseGoodfetchError";

/// What the docstring of `attempt_ended`'s class adds to the catalog's
/// description, which every language shares.
const ATTEMPT_ENDED_BASE: &str = "In Python it derives from `BaseException`, not `GoodfetchError`, as `asyncio.CancelledError` sits outside `Exception`, so a handler's `except Exception:` lets it through to what runs the attempt; it still carries its code and group (ADR 0093).";

/// Whether Python declares `code`'s class outside `Exception`, on
/// `BaseGoodfetchError` rather than its group's: only `attempt_ended`,
/// which ends a crawl handler's attempt (ADR 0093).
fn outside_exception(code: &ErrorCode) -> bool {
    code.name == ATTEMPT_ENDED.name
}

/// How `xtask codegen` runs `datamodel-code-generator`.
pub(super) const DATAMODEL_CODEGEN_OPTIONS: [&str; 19] = [
    "--input-file-type",
    "jsonschema",
    "--output-model-type",
    "pydantic_v2.BaseModel",
    "--target-python-version",
    "3.11",
    "--disable-timestamp",
    "--use-standard-collections",
    "--use-union-operator",
    "--use-schema-description",
    "--use-field-description",
    "--use-annotated",
    "--field-constraints",
    // An enum-typed field's default is the enum's member, which a type
    // checker accepts and which dumps as its value, never the bare string.
    "--deserialize-default-values",
    "enum",
    // Google docstrings with the summary on the first line (STYLE.md), in
    // the repository's ruff format.
    "--use-single-line-docstring",
    "--formatters",
    "ruff-format",
    "ruff-check",
];

mod handle_classes;
pub(super) mod model_defaults;
mod points_in_time;
mod settings;

/// The layer's settings module, and the plain-data fields that declare each
/// deprecated setting there, `Class.field`, by key: the release notes' list
/// names them (ADR 0099).
pub(super) fn deprecated_fields(layer: &Layer) -> (String, BTreeMap<String, Vec<String>>) {
    let fields = match layer.product {
        Product::Collie => settings::collie_deprecated_fields(&layer.settings),
        Product::Foxhound => settings::foxhound_deprecated_fields(&layer.settings),
    };
    (format!("{}.settings", own_package(layer.product)), fields)
}

pub(super) fn generate(layer: &Layer) -> Result<Generated> {
    // Collie's client package holds the handles (ADR 0117), as in
    // TypeScript; what Foxhound's operations add to them joins with its own
    // package's ticket.
    let handles = match layer.product {
        Product::Collie => handles::handles(layer)?,
        Product::Foxhound => Vec::new(),
    };
    let (mut input, _) = layer.generator_input(&format!("{}Types", layer.product.name()), |name| {
        let path = points_in_time::client_type(name).map_or_else(
            || {
                let module = if layer
                    .collie_definitions
                    .get(name)
                    .and_then(OpenValues::of)
                    .is_some()
                {
                    OPEN_VALUES
                } else {
                    "models"
                };
                format!("{CLIENT_PACKAGE}.{module}.{name}")
            },
            str::to_owned,
        );
        ("customTypePath", path)
    });
    points_in_time::use_client_type(&mut input)?;
    open_values::unknown_members(&mut input, Some(UNKNOWN_MEMBER_BASE))?;
    let open = open_values::python_input(
        &mut input,
        &format!("{}.{OPEN_VALUES}", own_package(layer.product)),
    );
    // The root holds no value: an empty model with a docstring, never
    // `RootModel[Any]`.
    input["type"] = Value::from("object");
    input["additionalProperties"] = Value::Bool(false);
    allow_unknown_fields(&mut input);
    let mut files = vec![
        ("__init__.py", init(layer)),
        ("connection.py", connection(layer)),
        ("operations.py", operations(layer)),
        ("events.py", events(layer)),
        ("errors.py", errors(layer)),
        ("plugins.py", plugins(layer)),
    ];
    if !layer.shared_types.is_empty() {
        files.push(("shared_types.py", shared_types(layer)));
    }
    if !open.is_empty() {
        files.push(("open_values.py", open_values_module(layer, &open)));
    }
    if !handles.is_empty() {
        files.push((
            "handles.py",
            handle_classes::handles_module(layer, &handles)?,
        ));
    }
    let settings_module = match layer.product {
        Product::Collie => settings::settings_module(&layer.settings)?,
        Product::Foxhound => settings::foxhound_settings_module(&layer.settings)?,
    };
    files.push(("settings.py", settings_module));
    Ok(Generated {
        files,
        types: TypesFile {
            output: "models.py",
            input,
            prefix: String::new(),
            header: docstring_header(&format!(
                "The types of {}'s entries of the catalog (ADR 0038).",
                layer.product.name()
            )),
        },
    })
}

/// States JSON Schema's own default, that an object without
/// `additionalProperties` allows unknown fields, on every such object, so
/// `datamodel-code-generator` writes `extra='allow'` for it: the types the
/// engine sends keep what the SDK does not know in `model_extra`, while
/// those a program sends say `false` and forbid it (ADR 0082).
fn allow_unknown_fields(schema: &mut Value) {
    match schema {
        Value::Object(object) => {
            if object.get("type").and_then(Value::as_str) == Some("object")
                && !object.contains_key("additionalProperties")
            {
                object.insert("additionalProperties".to_owned(), Value::Bool(true));
            }
            for child in object.values_mut() {
                allow_unknown_fields(child);
            }
        }
        Value::Array(items) => items.iter_mut().for_each(allow_unknown_fields),
        _ => {}
    }
}

/// A module docstring.
fn docstring_header(summary: &str) -> String {
    format!(
        "\"\"\"{summary}\n\nGenerated by `xtask codegen` from the catalog; never edit by hand.\n\"\"\""
    )
}

/// A module docstring and the imports every module opens with.
fn header(summary: &str) -> String {
    format!(
        "{}\n\nfrom __future__ import annotations\n",
        docstring_header(summary)
    )
}

/// The import lines of the models `names`: the layer's own from `.models`,
/// Collie's from the client package.
fn model_imports(layer: &Layer, names: &BTreeSet<String>) -> String {
    let (collie, own): (Vec<&String>, Vec<&String>) =
        names.iter().partition(|name| layer.is_collie_type(name));
    let mut text = String::new();
    if !collie.is_empty() {
        let _ = writeln!(
            text,
            "from {CLIENT_PACKAGE}.models import {}",
            join(&collie)
        );
    }
    if !own.is_empty() {
        let _ = writeln!(text, "from .models import {}", join(&own));
    }
    text
}

fn join(names: &[&String]) -> String {
    names
        .iter()
        .map(|name| name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

fn escape(text: &str) -> String {
    text.replace('\\', "\\\\").replace("\"\"\"", "\\\"\\\"\\\"")
}

/// A docstring, indented by `indent`.
fn docstring(
    indent: &str,
    description: &str,
    stability: Stability,
    deprecated: Option<&Deprecation>,
) -> String {
    let mut paragraphs = vec![escape(description)];
    if stability == Stability::Experimental {
        paragraphs.push(EXPERIMENTAL.to_owned());
    }
    if let Some(deprecated) = deprecated {
        paragraphs.push(format!("Deprecated. {}", escape(&deprecated.message())));
    }
    if let [one] = paragraphs.as_slice()
        && !one.contains('\n')
    {
        return format!("{indent}\"\"\"{one}\"\"\"\n");
    }
    // The summary opens the docstring; every other line of the
    // description follows, indented, then each marker's paragraph.
    let mut description = paragraphs[0].lines();
    let mut text = format!("{indent}\"\"\"{}\n", description.next().unwrap_or_default());
    // A blank line takes no indent, which would be trailing whitespace.
    let line = |text: &mut String, line: &str| {
        let indent = if line.is_empty() { "" } else { indent };
        let _ = writeln!(text, "{indent}{line}");
    };
    for each in description {
        line(&mut text, each);
    }
    for paragraph in &paragraphs[1..] {
        let _ = writeln!(text);
        for each in paragraph.lines() {
            line(&mut text, each);
        }
    }
    let _ = writeln!(text, "{indent}\"\"\"");
    text
}

/// PEP 702's marker of a deprecated member: with no category, so it warns
/// nothing at run time and the member's [`warnings`] warn once per process.
fn deprecated_decorator(deprecated: &Deprecation) -> String {
    format!(
        "    @deprecated({:?}, category=None)\n",
        deprecated.message()
    )
}

/// The lines a member of the entry `name` opens its body with: ADR 0030's
/// run-time warning of each marker it carries, once per process. Each name
/// it calls joins `imported`, which [`markers_import`] imports.
fn warnings(
    name: &str,
    stability: Stability,
    deprecated: Option<&Deprecation>,
    imported: &mut BTreeSet<&'static str>,
) -> String {
    let mut text = String::new();
    if stability == Stability::Experimental {
        imported.insert("warn_experimental");
        let _ = writeln!(text, "        warn_experimental({name:?})");
    }
    if let Some(deprecated) = deprecated {
        imported.insert("warn_deprecated");
        let _ = writeln!(
            text,
            "        warn_deprecated({name:?}, {:?})",
            deprecated.message()
        );
    }
    text
}

/// The import of the [`warnings`] a module calls, or nothing.
fn markers_import(imported: &BTreeSet<&'static str>) -> String {
    if imported.is_empty() {
        return String::new();
    }
    let names: Vec<&str> = imported.iter().copied().collect();
    format!("from {MARKERS_MODULE} import {}\n", names.join(", "))
}

/// Each type both Products share that the layer adds values to (ADR 0095),
/// as its values' `Literal` open to any other string, which the package
/// keeps as received (ADR 0079); Foxhound's adds its values to Collie's.
fn shared_types(layer: &Layer) -> String {
    let mut text = docstring_header(&format!(
        "The values {} adds to the types both Products share (ADR 0095).",
        layer.product.name()
    ));
    text.push_str(
        "\n\nfrom __future__ import annotations\n\nfrom typing import Literal, TypeAlias\n",
    );
    let extended: Vec<String> = layer
        .shared_types
        .iter()
        .filter(|shared| shared.extends_collie)
        .map(|shared| {
            let name = pascal(&shared.name);
            format!("{name} as Collie{name}")
        })
        .collect();
    if !extended.is_empty() {
        let _ = writeln!(
            text,
            "\nfrom {CLIENT_PACKAGE}.shared_types import {}",
            extended.join(", ")
        );
    }
    for shared in &layer.shared_types {
        let name = pascal(&shared.name);
        let literals: Vec<String> = shared
            .values
            .iter()
            .map(|(value, _)| format!("{value:?}"))
            .collect();
        let literal = format!("Literal[{}]", literals.join(", "));
        if shared.extends_collie {
            let _ = writeln!(text, "\n{name}: TypeAlias = Collie{name} | {literal}");
        } else {
            let _ = writeln!(text, "\n{name}: TypeAlias = {literal} | str");
        }
        let mut description = if shared.extends_collie {
            format!(
                "A `{}`, a type both Products share (ADR 0095).\n\nCollie's values and {}'s own below; a value this package does not know is\nkept as received (ADR 0079).\n",
                shared.name,
                layer.product.name()
            )
        } else {
            format!(
                "A `{}`, a type both Products share (ADR 0095).\n\n{}'s values below; a value this package does not know is kept as\nreceived (ADR 0079).\n",
                shared.name,
                layer.product.name()
            )
        };
        for (value, value_description) in &shared.values {
            let _ = write!(description, "\n- `{value}`: {value_description}");
        }
        let _ = writeln!(text, "\"\"\"{}\n\"\"\"", escape(&description));
    }
    text
}

fn init(layer: &Layer) -> String {
    header(&format!(
        "The layer generated from {}'s entries of the catalog (ADR 0095).",
        layer.product.name()
    ))
}

/// What a package's runtime opens a Connection with. Collie's client
/// package holds what both Products' Connections share: the protocol version
/// its `hello` names (ADRs 0024 and 0066), the Product version the package
/// carries (ADRs 0090 and 0095) and the hidden command that serves one
/// Connection. Each Product's layer holds how its own engine is found: its
/// executable and its engine-path variable (ADR 0090).
fn connection(layer: &Layer) -> String {
    let (shared, name, constant, variable) = match layer.product {
        Product::Collie => (
            format!(
                "\nPROTOCOL_VERSION: Final = {:?}\n\"\"\"The protocol version `major.minor` this package's `hello` names (ADRs 0024 and 0066).\"\"\"\n\nWEBSOCKET_SUBPROTOCOL: Final = {:?}\n\"\"\"The WebSocket subprotocol a Client offers an Exposed Daemon, `goodfetch.v<major>` of the protocol version (ADR 0049).\"\"\"\n\nPRODUCT_VERSION: Final = {:?}\n\"\"\"The Product version this package carries, `hello`'s `product_version` (ADRs 0090 and 0095).\"\"\"\n\nATTACHED_ENGINE_COMMAND: Final = {:?}\n\"\"\"The executable's hidden command that serves one Connection over stdio.\"\"\"\n\nDEPLOYMENT_PREFIX: Final = {DEPLOYMENT_PREFIX:?}\n\"\"\"The prefix every deployment variable has, which ADR 0034 reserves.\"\"\"\n\nDEPLOYMENT_VARIABLES: Final = (\n{}\n)\n\"\"\"The deployment variables, the closed list of `GOODFETCH_` variables `connect()` takes from its environment map (ADR 0083).\"\"\"\n",
                layer.protocol_version.to_string(),
                layer.protocol_version.websocket_subprotocol(),
                layer.product_version,
                ATTACHED_ENGINE,
                DEPLOYMENT_VARIABLES
                    .iter()
                    .map(|variable| format!("    {variable:?},"))
                    .collect::<Vec<_>>()
                    .join("\n"),
            ),
            "Collie",
            "COLLIE",
            COLLIE_ENGINE_PATH_VARIABLE,
        ),
        Product::Foxhound => (
            String::new(),
            "Foxhound",
            "FOXHOUND",
            FOXHOUND_ENGINE_PATH_VARIABLE,
        ),
    };
    let mut text = header(&format!(
        "What {name}'s runtime opens a Connection with (ADR 0090)."
    ));
    text.push_str("\nfrom typing import Final\n");
    text.push_str(&shared);
    let _ = write!(
        text,
        "\n{constant}_ENGINE_PATH_VARIABLE: Final = {variable:?}\n\"\"\"The variable naming {name}'s executable (ADR 0090).\"\"\"\n\n{constant}_EXECUTABLE: Final = {:?}\n\"\"\"The name of {name}'s executable (ADR 0090).\"\"\"\n",
        layer.product.executable_name(),
    );
    text
}

fn operations(layer: &Layer) -> String {
    let product = layer.product.name();
    // A read sends nothing, so no client calls it by name: its handle
    // answers it (issue 2798).
    let sent = || {
        layer
            .operations
            .iter()
            .filter(|operation| operation.sends())
    };
    let types: BTreeSet<String> = sent()
        .flat_map(|operation| [operation.params.0.clone(), operation.result.0.clone()])
        .collect();
    let deprecates = sent().any(|operation| operation.deprecated.is_some());
    let mut text = header(&format!(
        "{product}'s operations, each an async method and a sync one (ADR 0038)."
    ));
    text.push_str("\nfrom collections.abc import Mapping\nfrom typing import Final, Literal\n\n");
    let returns = sent().next().is_some();
    if deprecates {
        text.push_str("from typing_extensions import deprecated\n\n");
    }
    let _ = writeln!(
        text,
        "from {CALLER_MODULE} import AsyncCaller, Caller{}",
        if returns { ", result_of" } else { "" }
    );
    let mut warned = BTreeSet::new();
    let mut body = String::new();
    for (prefix, caller, asynchronous) in [("Async", "AsyncCaller", true), ("", "Caller", false)] {
        let mode = if asynchronous { "async" } else { "sync" };
        let _ = write!(
            body,
            "\n\nclass {prefix}{product}Operations:\n    \"\"\"{product}'s operations, each a{} {mode} method.\"\"\"\n\n    def __init__(self, caller: {caller}) -> None:\n        \"\"\"Takes the runtime that sends each call.\"\"\"\n        self._caller = caller\n",
            if asynchronous { "n" } else { "" }
        );
        for operation in sent() {
            body.push('\n');
            if let Some(deprecated) = &operation.deprecated {
                body.push_str(&deprecated_decorator(deprecated));
            }
            let (params, result) = (&operation.params.0, &operation.result.0);
            let _ = writeln!(
                body,
                "    {}def {}(self, params: {params}) -> {result}:",
                if asynchronous { "async " } else { "" },
                snake(&operation.name)
            );
            body.push_str(&docstring(
                "        ",
                &operation.description,
                operation.stability,
                operation.deprecated.as_ref(),
            ));
            body.push_str(&warnings(
                &operation.name,
                operation.stability,
                operation.deprecated.as_ref(),
                &mut warned,
            ));
            // A run layer taken as raw JSON is checked for deprecated keys
            // as a typed member's settings are (ADR 0099).
            if layer.takes_raw_settings(operation) {
                warned.insert(settings::WARN_DEPRECATED_SETTINGS);
                body.push_str(&settings::raw_settings_check());
            }
            let _ = writeln!(
                body,
                "        return result_of({result}, {}self._caller.call({:?}, params))",
                if asynchronous { "await " } else { "" },
                operation.name
            );
        }
    }
    text.push_str(&markers_import(&warned));
    let imports = model_imports(layer, &types);
    if !imports.is_empty() {
        text.push('\n');
        text.push_str(&imports);
    }
    if warned.contains(settings::WARN_DEPRECATED_SETTINGS) {
        let _ = writeln!(
            text,
            "from .settings import {}",
            settings::DEPRECATED_SETTINGS
        );
    }
    text.push_str(&body);
    text.push_str(&registries(
        "operations",
        layer.operations.iter().map(|operation| {
            (
                operation.name.as_str(),
                operation.stability,
                operation.deprecated.as_ref(),
            )
        }),
    ));
    operation_classes(&mut text, layer);
    text
}

/// The map from each operation's catalog name to its class (ADR 0011),
/// which says whether a call in flight is sent again after a reconnect
/// (ADR 0079): TypeScript's `<product>OperationClasses`.
fn operation_classes(text: &mut String, layer: &Layer) {
    text.push_str(
        "\nOPERATION_CLASSES: Final[Mapping[str, Literal[\"read\", \"idempotent_write\", \"effect\"]]] = {\n",
    );
    for operation in &layer.operations {
        let _ = writeln!(
            text,
            "    {:?}: {:?},",
            operation.name,
            operation.class.as_str()
        );
    }
    text.push_str(
        "}\n\"\"\"The class of each operation (ADR 0011): after a reconnect, a call in flight is sent again only when it is a read or an idempotent write (ADR 0079).\"\"\"\n",
    );
}

/// The registries of a section's markers, `EXPERIMENTAL_<SECTION>` and
/// `DEPRECATED_<SECTION>`, which a hand-written member serving an entry, or
/// a listener of an event, warns of through `warn_entry` (ADR 0099).
fn registries<'a>(
    section: &str,
    entries: impl Iterator<Item = (&'a str, Stability, Option<&'a Deprecation>)> + Clone,
) -> String {
    let upper = section.to_uppercase();
    let mut text = format!("\nEXPERIMENTAL_{upper}: Final[frozenset[str]] = frozenset({{\n");
    for (name, stability, _) in entries.clone() {
        if stability == Stability::Experimental {
            let _ = writeln!(text, "    {name:?},");
        }
    }
    let _ = write!(
        text,
        "}})\n\"\"\"The {section} outside semver until they are promoted (ADR 0030).\"\"\"\n\nDEPRECATED_{upper}: Final[Mapping[str, str]] = {{\n"
    );
    for (name, _, deprecated) in entries {
        if let Some(deprecated) = deprecated {
            let _ = writeln!(text, "    {name:?}: {:?},", deprecated.message());
        }
    }
    let singular = section.trim_end_matches('s');
    let _ = writeln!(
        text,
        "}}\n\"\"\"Each deprecated {singular}, with its deprecation's one text.\"\"\""
    );
    text
}

fn events(layer: &Layer) -> String {
    let product = layer.product.name();
    let types: BTreeSet<String> = layer
        .events
        .iter()
        .map(|event| event.payload.0.clone())
        .collect();
    let mut text = header(&format!("The registry of {product}'s events (ADR 0038)."));
    text.push_str("\nfrom collections.abc import Mapping\nfrom typing import Final\n\nfrom pydantic import BaseModel\n");
    let imports = model_imports(layer, &types);
    if !imports.is_empty() {
        text.push('\n');
        text.push_str(&imports);
    }
    text.push_str("\nEVENTS: Final[Mapping[str, type[BaseModel]]] = {\n");
    for event in &layer.events {
        let _ = writeln!(text, "    {:?}: {},", event.name, event.payload.0);
    }
    text.push_str("}\n\"\"\"The payload type of each event");
    if layer.events.is_empty() {
        text.push_str(".\"\"\"\n");
    } else {
        text.push_str(":\n\n");
        for event in &layer.events {
            let _ = writeln!(
                text,
                "- `{}`: {}",
                event.name,
                escape(&event.description).replace('\n', " ")
            );
        }
        text.push_str("\"\"\"\n");
    }
    text.push_str(&registries(
        "events",
        layer.events.iter().map(|event| {
            (
                event.name.as_str(),
                event.stability,
                event.deprecated.as_ref(),
            )
        }),
    ));
    text
}

fn group_class(group: &str) -> String {
    format!("{}Error", pascal(group))
}

/// The rest of the import lines of Foxhound's `errors` module: the group
/// classes from Collie's, and the base outside `Exception` when a code's
/// class derives from it.
fn foxhound_error_imports(layer: &Layer) -> String {
    let imported: BTreeSet<String> = layer
        .error_codes
        .iter()
        .filter(|code| !outside_exception(code))
        .map(|code| group_class(&code.group))
        .collect();
    let names: Vec<&String> = imported.iter().collect();
    let base = if layer.error_codes.iter().any(outside_exception) {
        format!("{BASE_ERROR}, ")
    } else {
        String::new()
    };
    let helper = if layer.reads_retry_after() {
        ", retry_after_of"
    } else {
        ""
    };
    format!(
        ", Unpack\n\nfrom {CLIENT_PACKAGE}.errors import {}\nfrom {GOODFETCH_ERROR_MODULE} import {base}ErrorClass, ErrorFields{helper}\n",
        join(&names)
    )
}

/// The docstring of `retry_after` on the class of a code whose details
/// name it as a point in time (ADRs 0092 and 0118).
const RETRY_AFTER_DOC: &str = "When the site's `Retry-After` asks to be tried again: a point in time in UTC, not a duration; `None` when it asks for none. The details keep it as sent.";

/// The docstring of `typed_details` on the class of a code whose details
/// the SDK reads typed (ADR 0079).
const TYPED_DETAILS_DOC: &str = "The code's details, typed; `None` when they lack a field the model requires, as those of an error a program builds without details do.";

/// The class `class` of one Error code, which reads its `retry_after`
/// when its details name one as a point in time, and its `typed_details`
/// when the catalog names their type.
fn error_class(code: &ErrorCode, class: &str) -> String {
    let (base, description) = if outside_exception(code) {
        (
            BASE_ERROR.to_owned(),
            format!("{}\n\n{ATTEMPT_ENDED_BASE}", code.description),
        )
    } else {
        (group_class(&code.group), code.description.clone())
    };
    let mut text = format!("\n\nclass {class}({base}):\n");
    text.push_str(&docstring("    ", &description, Stability::Stable, None));
    let mut field = String::new();
    let mut set = String::new();
    if code.retry_after_is_point_in_time() {
        let _ = write!(
            field,
            "\n    retry_after: datetime | None\n    \"\"\"{RETRY_AFTER_DOC}\"\"\"\n"
        );
        set.push_str("        self.retry_after = retry_after_of(self.details)\n");
    }
    if let Some(details) = &code.details {
        let name = &details.0;
        let _ = write!(
            field,
            "\n    typed_details: {name} | None\n    \"\"\"{TYPED_DETAILS_DOC}\"\"\"\n"
        );
        let _ = writeln!(
            set,
            "        self.typed_details = details_of({name}, self.details)"
        );
    }
    let _ = write!(
        text,
        "{field}\n    def __init__(self, message: str, **fields: Unpack[ErrorFields]) -> None:\n        \"\"\"Takes the message the engine rendered, and what it carries.\"\"\"\n        super().__init__(message, code={:?}, group={:?}, retryable={}, **fields)\n{set}",
        code.name,
        code.group,
        if code.retryable { "True" } else { "False" }
    );
    text
}

fn errors(layer: &Layer) -> String {
    let product = layer.product.name();
    let mut text = header(&format!(
        "The exception class of each of {product}'s Error codes (ADR 0079)."
    ));
    let retry_after = layer.reads_retry_after();
    text.push_str("\nfrom collections.abc import Mapping\n");
    if retry_after {
        text.push_str("from datetime import datetime\n");
    }
    text.push_str("from typing import Final");
    let detail_types: BTreeSet<String> = layer
        .error_codes
        .iter()
        .filter_map(|code| code.details.as_ref().map(|name| name.0.clone()))
        .collect();
    let (details_helper, details_models) = if detail_types.is_empty() {
        (String::new(), String::new())
    } else {
        (
            format!("from {CALLER_MODULE} import details_of\n"),
            model_imports(layer, &detail_types),
        )
    };
    match layer.product {
        Product::Collie => {
            let helper = if retry_after { ", retry_after_of" } else { "" };
            let _ = write!(
                text,
                ", Literal, Unpack\n\n{details_helper}from {GOODFETCH_ERROR_MODULE} import ErrorClass, ErrorFields, GoodfetchError{helper}\n{details_models}\nErrorGroup = Literal["
            );
            text.push_str(
                &layer
                    .error_groups
                    .iter()
                    .map(|group| format!("{:?}", group.name))
                    .collect::<Vec<_>>()
                    .join(", "),
            );
            text.push_str(
                "]\n\"\"\"The seven groups of Error codes, named for their remedy.\"\"\"\n",
            );
            for group in &layer.error_groups {
                let _ = write!(
                    text,
                    "\n\nclass {}(GoodfetchError):\n",
                    group_class(&group.name)
                );
                text.push_str(&docstring(
                    "    ",
                    &group.description,
                    Stability::Stable,
                    None,
                ));
            }
        }
        Product::Foxhound => {
            text.push_str(&foxhound_error_imports(layer));
            text.push_str(&details_helper);
            text.push_str(&details_models);
        }
    }
    let mut registry = Vec::new();
    for code in &layer.error_codes {
        // A code named like its group is raised as its group's class,
        // which every code the registry lacks falls back to, as in
        // TypeScript: the group's class takes the code's fields.
        if code.name == code.group {
            continue;
        }
        let class = format!("{}Error", pascal(&code.name));
        text.push_str(&error_class(code, &class));
        registry.push((code.name.clone(), class));
    }
    if layer.product == Product::Collie {
        text.push_str("\n\nERROR_GROUP_OF_WIRE_CLASS: Final[Mapping[int, ErrorGroup]] = {\n");
        for group in &layer.error_groups {
            let _ = writeln!(text, "    {}: {:?},", group.json_rpc_class, group.name);
        }
        text.push_str("}\n\"\"\"The group of each JSON-RPC error code the engine answers with, its wire class (ADR 0038).\"\"\"\n");
        text.push_str("\nERROR_GROUP_OF_PROTOCOL_FAILURE: Final[Mapping[int, ErrorGroup]] = {\n");
        for (code, group) in &layer.protocol_failure_groups {
            let _ = writeln!(text, "    {code}: {group:?},");
        }
        text.push_str("}\n\"\"\"The group of each JSON-RPC code a failure of the protocol itself takes, which JSON-RPC 2.0 predefines (ADRs 0038 and 0079).\"\"\"\n");
        text.push_str(
            "\nERROR_GROUP_CLASSES: Final[Mapping[ErrorGroup, type[GoodfetchError]]] = {\n",
        );
        for group in &layer.error_groups {
            let _ = writeln!(text, "    {:?}: {},", group.name, group_class(&group.name));
        }
        text.push_str("}\n\"\"\"The class of each group, which raises a code the SDK does not know, keeping the code (ADR 0079).\"\"\"\n");
    }
    text.push_str("\n\nERROR_CLASSES: Final[Mapping[str, ErrorClass]] = {\n");
    for (code, class) in registry {
        let _ = writeln!(text, "    {code:?}: {class},");
    }
    text.push_str("}\n\"\"\"The exception class of each Error code.\"\"\"\n");
    text
}

fn plugins(layer: &Layer) -> String {
    let product = layer.product.name();
    let types: BTreeSet<String> = layer
        .plugin_kinds
        .iter()
        .flat_map(|kind| &kind.methods)
        .flat_map(|method| [method.params.0.clone(), method.result.0.clone()])
        .collect();
    let mut text = header(&format!(
        "The base classes of SDK handlers for each of {product}'s Plugin kinds (ADR 0033)."
    ));
    if layer.plugin_kinds.is_empty() {
        return text;
    }
    text.push_str("\nfrom abc import ABC, abstractmethod\n");
    let imports = model_imports(layer, &types);
    if !imports.is_empty() {
        text.push('\n');
        text.push_str(&imports);
    }
    for kind in &layer.plugin_kinds {
        let _ = write!(text, "\n\nclass {}Handler(ABC):\n", pascal(&kind.name));
        text.push_str(&docstring("    ", &kind.description, kind.stability, None));
        for method in &kind.methods {
            let _ = write!(
                text,
                "\n    @abstractmethod\n    async def {}(self, params: {}) -> {}:\n",
                snake(&method.name),
                method.params.0,
                method.result.0
            );
            text.push_str(&docstring(
                "        ",
                &method.description,
                Stability::Stable,
                None,
            ));
        }
    }
    text
}
