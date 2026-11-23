//! The error classes of a TypeScript layer (ADR 0079): in Collie's client
//! package, the seven group classes on the base its hand-written
//! `goodfetch-error.ts` declares, with the wire's group tables and the CLI's
//! exit codes; in each package, one class per Error code of its Product, the
//! Product's code union, error union and guard.

use std::collections::BTreeSet;
use std::fmt::Write as _;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use serde_json::Value;

use super::{CLIENT_PACKAGE, HEADER, doc, type_imports, wire};
use crate::codegen::layer::{ErrorCode, Layer, Stability};
use crate::codegen::naming::{camel, pascal};

/// The hand-written module of Collie's client package that declares the
/// base of every error class, which its layer's classes extend and
/// re-export.
const GOODFETCH_ERROR_MODULE: &str = "../goodfetch-error.ts";

/// The doc of `retryAfter` on the class of a code whose details name
/// `retry_after` as a point in time (ADRs 0092 and 0118).
const RETRY_AFTER_DOC: &str = "  /**\n   * When the site's `Retry-After` asks to be tried again: a point in time in\n   * UTC, not a duration; `null` when it asks for none. The details keep it\n   * as sent, as `retry_after`.\n   */\n";

fn group_class(group: &str) -> String {
    format!("{}Error", pascal(group))
}

/// The doc of `typedDetails` on the class of a code whose details the SDK
/// reads typed (ADR 0079).
const TYPED_DETAILS_DOC: &str = "  /**\n   * The code's details, typed; `undefined` when they lack a field the type\n   * requires, as those of an error a program builds without details do.\n   */\n";

/// How the class of a code whose details the SDK reads typed reads them:
/// the type, the fields it requires, in their TypeScript names, and the
/// Product's wire table it reads them through when it holds fields to
/// rename or points in time, as a response is read.
struct DetailsReading<'a> {
    name: &'a str,
    required: Vec<String>,
    through: Option<String>,
}

/// The typed details of `code`'s class, when the catalog names its details
/// type and the code has a class of its own.
fn typed_details<'a>(layer: &'a Layer, code: &'a ErrorCode) -> Result<Option<DetailsReading<'a>>> {
    let Some(name) = code.class_details().map(|name| name.0.as_str()) else {
        return Ok(None);
    };
    let through = if wire::converts(layer, name)? {
        // The layer's table holds its own types only.
        if layer.is_collie_type(name) && layer.product != Product::Collie {
            bail!(
                "`{}`'s details type `{name}` is Collie's, whose wire table {}'s errors do not import",
                code.name,
                layer.product.name()
            );
        }
        Some(wire::table_name(layer.product))
    } else {
        None
    };
    let schema = layer
        .definitions
        .get(name)
        .or_else(|| layer.collie_definitions.get(name))
        .with_context(|| {
            format!(
                "`{}`'s details type `{name}` is no type of the layer",
                code.name
            )
        })?;
    let required = schema
        .get("required")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .map(camel)
        .collect();
    Ok(Some(DetailsReading {
        name,
        required,
        through,
    }))
}

pub(super) fn errors(layer: &Layer) -> Result<String> {
    let product = layer.product.name();
    let lower = product.to_lowercase();
    let mut text = format!("{HEADER}\n");
    let mut typed = Vec::new();
    for code in &layer.error_codes {
        typed.push(typed_details(layer, code)?);
    }
    let detail_types: BTreeSet<String> = typed
        .iter()
        .flatten()
        .map(|details| details.name.to_owned())
        .collect();
    let wire_import = typed
        .iter()
        .flatten()
        .find_map(|details| details.through.as_ref())
        .map(|table| format!("import {{ {table} }} from \"./wire.ts\";\n"))
        .unwrap_or_default();
    match layer.product {
        Product::Collie => {
            text.push_str(&collie_header(layer, &detail_types, &wire_import));
        }
        Product::Foxhound => {
            let mut imported: BTreeSet<String> = layer
                .error_codes
                .iter()
                .map(|code| group_class(&code.group))
                .collect();
            imported.extend(["isCollieError".to_owned(), "isGoodfetchError".to_owned()]);
            if layer.reads_retry_after() {
                imported.insert("retryAfterOf".to_owned());
            }
            if !detail_types.is_empty() {
                imported.insert("detailsOf".to_owned());
            }
            let names: Vec<String> = imported.into_iter().collect();
            let _ = write!(
                text,
                "import {{ {} }} from \"{CLIENT_PACKAGE}\";\nimport type {{ CollieError, ErrorClass, ErrorFields }} from \"{CLIENT_PACKAGE}\";\n{}{wire_import}",
                names.join(", "),
                type_imports(layer, &detail_types)
            );
        }
    }
    let mut registry = Vec::new();
    // Each code's class with its code as a literal, which `code` narrows
    // without `instanceof` (ADR 0079).
    let mut members = Vec::new();
    for (code, typed) in layer.error_codes.iter().zip(&typed) {
        let group = group_class(&code.group);
        // A code named like its group is raised as its group's class,
        // which every code the registry lacks falls back to.
        if code.name == code.group {
            members.push(format!("{group}<{:?}>", code.name));
            continue;
        }
        let class = format!("{}Error", pascal(&code.name));
        text.push_str(&error_class(code, typed.as_ref(), &class, &group));
        members.push(class.clone());
        registry.push((code.name.clone(), class));
    }
    let _ = write!(
        text,
        "\n/** The error class of each of {product}'s Error codes. */\nexport const {lower}ErrorClasses: Readonly<Record<string, ErrorClass>> = {{\n"
    );
    for (code, class) in registry {
        let _ = writeln!(text, "  {code}: {class},");
    }
    text.push_str("};\n");
    text.push_str(&product_errors(layer, &members));
    Ok(text)
}

/// The head of Collie's errors file: what it imports and re-exports, its
/// groups' classes, the tables that map a wire code to its group, and the
/// exit codes; `wire_import` imports the wire table its typed details read
/// through, when one does.
fn collie_header(layer: &Layer, detail_types: &BTreeSet<String>, wire_import: &str) -> String {
    let mut text = String::new();
    let _ = write!(
        text,
        "import {{\n  detailsOf,\n  GoodfetchError,\n  isGoodfetchError,\n  retryAfterOf,\n  type ErrorClass,\n  type ErrorFields,\n  type ErrorIdentity,\n  type ErrorLimit,\n}} from \"{GOODFETCH_ERROR_MODULE}\";\n{}{wire_import}\nexport {{\n  detailsOf,\n  GoodfetchError,\n  isGoodfetchError,\n  retryAfterOf,\n  type ErrorClass,\n  type ErrorFields,\n  type ErrorIdentity,\n  type ErrorLimit,\n}};\n\n/** The seven groups of Error codes, named for their remedy (ADR 0079). */\nexport type ErrorGroup = {};\n",
        type_imports(layer, detail_types),
        layer
            .error_groups
            .iter()
            .map(|group| format!("{:?}", group.name))
            .collect::<Vec<_>>()
            .join(" | ")
    );
    for group in &layer.error_groups {
        let class = group_class(&group.name);
        text.push('\n');
        text.push_str(&doc("", &group.description, Stability::Stable, None));
        let _ = write!(
            text,
            "export class {class}<Code extends string = string> extends GoodfetchError<Code> {{\n  declare readonly group: {:?};\n\n  constructor(message: string, init: ErrorFields & ErrorIdentity<Code>) {{\n    super(message, {{ name: {class:?}, ...init }});\n  }}\n}}\n",
            group.name
        );
    }
    text.push_str(
        "\n/** The group of each JSON-RPC error code the engine answers with, its wire class (ADR 0038). */\nexport const errorGroupOfWireClass: Readonly<Record<number, ErrorGroup>> = {\n",
    );
    for group in &layer.error_groups {
        let _ = writeln!(text, "  [{}]: {:?},", group.json_rpc_class, group.name);
    }
    text.push_str(
        "};\n\n/** The group of each JSON-RPC code a failure of the protocol itself takes, which JSON-RPC 2.0 predefines (ADRs 0038 and 0079). */\nexport const errorGroupOfProtocolFailure: Readonly<Record<number, ErrorGroup>> = {\n",
    );
    for (code, group) in &layer.protocol_failure_groups {
        let _ = writeln!(text, "  [{code}]: {group:?},");
    }
    text.push_str(
        "};\n\n/** The class of each group, which raises a code the SDK does not know, keeping the code (ADR 0079). */\nexport const errorGroupClasses: Readonly<Record<ErrorGroup, typeof GoodfetchError>> = {\n",
    );
    for group in &layer.error_groups {
        let _ = writeln!(text, "  {}: {},", group.name, group_class(&group.name));
    }
    text.push_str("};\n");
    text.push_str(&exit_codes(layer));
    text
}

/// The class `class` of one Error code, on its group's class `group`, which
/// reads its `retryAfter` when its details name one as a point in time, and
/// its `typedDetails` when the catalog names their type.
fn error_class(
    code: &ErrorCode,
    typed: Option<&DetailsReading<'_>>,
    class: &str,
    group: &str,
) -> String {
    let mut text = format!("\n{}", doc("", &code.description, Stability::Stable, None));
    let mut field = String::new();
    let mut set = String::new();
    if code.retry_after_is_point_in_time() {
        let _ = write!(
            field,
            "{RETRY_AFTER_DOC}  readonly retryAfter: Date | null;\n\n"
        );
        set.push_str("\n    this.retryAfter = retryAfterOf(fields.details);");
    }
    if let Some(typed) = typed {
        let name = typed.name;
        let required: Vec<String> = typed
            .required
            .iter()
            .map(|field| format!("{field:?}"))
            .collect();
        let _ = write!(
            field,
            "{TYPED_DETAILS_DOC}  readonly typedDetails: Readonly<{name}> | undefined;\n\n"
        );
        let through = typed
            .through
            .as_ref()
            .map(|table| format!(", {{ path: {{ type: {name:?} }}, types: {table}.types }}"))
            .unwrap_or_default();
        let _ = write!(
            set,
            "\n    this.typedDetails = detailsOf<{name}>(fields.details, [{}]{through});",
            required.join(", ")
        );
    }
    let _ = write!(
        text,
        "export class {class} extends {group}<{:?}> {{\n{field}  constructor(message: string, fields: ErrorFields = {{}}) {{\n    super(message, {{ ...fields, name: {class:?}, code: {:?}, group: {:?}, retryable: {} }});{set}\n  }}\n}}\n",
        code.name, code.name, code.group, code.retryable
    );
    text
}

/// A Product's Error codes as a literal union, the union of their errors,
/// and the guard that tells one of them by its code, which holds across two
/// copies of the package in one bundle where `instanceof` fails (ADR 0079).
/// Foxhound's union and guard take Collie's in, since it raises them too.
fn product_errors(layer: &Layer, members: &[String]) -> String {
    let product = layer.product.name();
    let lower = product.to_lowercase();
    let codes: Vec<String> = layer
        .error_codes
        .iter()
        .map(|code| format!("{:?}", code.name))
        .collect();
    let mut text =
        format!("\n/** {product}'s own Error codes. */\nexport type {product}ErrorCode =");
    if codes.is_empty() {
        text.push_str(" never;\n");
    } else {
        for code in &codes {
            let _ = write!(text, "\n  | {code}");
        }
        text.push_str(";\n");
    }
    let _ = write!(
        text,
        "\nconst {lower}ErrorCodes: ReadonlySet<string> = new Set<{product}ErrorCode>([{}]);\n",
        codes.join(", ")
    );
    let mut union: Vec<String> = members.to_vec();
    let mut guard = format!("isGoodfetchError(value) && {lower}ErrorCodes.has(value.code)");
    if layer.product == Product::Foxhound {
        union.insert(0, "CollieError".to_owned());
        guard = format!("isCollieError(value) || ({guard})");
    }
    let _ = write!(
        text,
        "\n/** An error with one of the Error codes {product} raises, which its `code` narrows to the code's class. */\nexport type {product}Error ="
    );
    if union.is_empty() {
        text.push_str(" never;\n");
    } else {
        for member in &union {
            let _ = write!(text, "\n  | {member}");
        }
        text.push_str(";\n");
    }
    let _ = write!(
        text,
        "\n/** Whether `value` is an error with one of the Error codes {product} raises, from any copy of the package. */\nexport function is{product}Error(value: unknown): value is {product}Error {{\n  return {guard};\n}}\n"
    );
    text
}

/// ADR 0069's exit codes of a failed command, which the `goodfetch-collie`
/// bin exits with when it cannot run the executable: each group's row, and
/// each Error code's own row, which wins. Nothing when the projection lists
/// no exit-code table (the codegen fixture's).
fn exit_codes(layer: &Layer) -> String {
    let groups: Option<Vec<(&str, u8)>> = layer
        .error_groups
        .iter()
        .map(|group| group.exit_code.map(|code| (group.name.as_str(), code)))
        .collect();
    let Some(groups) = groups else {
        return String::new();
    };
    let mut text = String::from(
        "\n/** The CLI's exit code of a failure with each group's Error codes, unless the code has a row of its own (ADR 0069). */\nexport const errorGroupExitCodes: Readonly<Record<ErrorGroup, number>> = {\n",
    );
    for (group, code) in groups {
        let _ = writeln!(text, "  {group}: {code},");
    }
    text.push_str(
        "};\n\n/** The Error codes with an exit code of their own, which wins over their group's (ADR 0069). */\nexport const errorCodeExitCodes: Readonly<Record<string, number>> = {\n",
    );
    for code in &layer.error_codes {
        if let Some(exit) = code.exit_code {
            let _ = writeln!(text, "  {}: {exit},", code.name);
        }
    }
    text.push_str("};\n");
    text
}
