//! Projects Goodfetch catalog operations onto MCP tools, annotations,
//! input schemas and tool results (ADRs 0011, 0065, 0070, 0099; spec #608).
//!
//! Tool names are the canonical catalog name with `_` for `.`, in `[a-z0-9_]`
//! and at most [`MAX_TOOL_NAME_CHARS`] characters. Descriptions come verbatim
//! from the catalog (at most [`MAX_DESCRIPTION_CHARS`] characters), with a
//! deprecation prefix when deprecated. Engine Error codes become tool results
//! with `isError: true`, `structuredContent` holding the CLI's
//! [`CommandError`], and `fix` first in the text block.

use std::borrow::Cow;
use std::sync::Arc;

use goodfetch_protocol::catalog::{ErrorGroup, Operation, OperationClass};
use goodfetch_protocol::cli::CommandError;
use goodfetch_protocol::jsonrpc::{ErrorObject, ProtocolFailure};
use rmcp::model::{CallToolResult, ContentBlock, JsonObject, Tool, ToolAnnotations};
use schemars::generate::SchemaSettings;
use serde_json::{Map, Value};

/// Maximum length of an MCP tool name (ADR 0099, spec #608).
pub const MAX_TOOL_NAME_CHARS: usize = 39;

/// Maximum length of an MCP tool description (ADR 0065, spec #608).
pub const MAX_DESCRIPTION_CHARS: usize = 2048;

/// Parameter omitted from `run_start` and `recipe_start` input schemas on MCP
/// surfaces because accepting a plugin change is a granting operation
/// (ADR 0011, ADR 0033, spec #608).
const OMITTED_GRANTING_PARAMS: &[(&str, &str)] = &[
    ("run.start", "accept_plugin_change"),
    ("recipe.start", "accept_plugin_change"),
];

/// An operation projected onto an MCP [`Tool`].
#[derive(Clone, Debug)]
pub struct ProjectedTool {
    /// Canonical catalog operation name, e.g. `run.start`.
    pub operation: &'static str,
    /// The operation's class (read, idempotent write or Effect).
    pub class: OperationClass,
    /// Whether the engine cancels an in-flight call on `$/cancel`.
    pub cancellable: bool,
    /// The MCP tool definition exposed by `tools/list`.
    pub tool: Tool,
}

/// Converts a canonical catalog operation name (`namespace.member`) to its MCP
/// tool name (`namespace_member`).
#[must_use]
pub fn tool_name(canonical: &str) -> String {
    canonical.replace('.', "_")
}

/// Whether `name` satisfies the MCP tool-name rule: non-empty, at most
/// [`MAX_TOOL_NAME_CHARS`] characters, and every character in `[a-z0-9_]`.
#[must_use]
pub fn is_valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.chars().count() <= MAX_TOOL_NAME_CHARS
        && name
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// Whether `operation` reaches a site, a search provider or a model, so its
/// MCP `openWorldHint` annotation is `true` (spec #608).
#[must_use]
pub fn is_open_world(operation: &Operation) -> bool {
    let name = operation.name;
    let namespace = name.split('.').next().unwrap_or_default();
    if matches!(namespace, "search" | "fetch" | "crawl" | "model") {
        return true;
    }
    if name == "run.research" || name == "catalog_run.start" || name == "recipe.start" {
        return true;
    }
    if namespace == "page" {
        return matches!(
            name,
            "page.goto"
                | "page.reload"
                | "page.go_back"
                | "page.go_forward"
                | "page.click"
                | "page.fill"
                | "page.press"
                | "page.type"
                | "page.check"
                | "page.uncheck"
                | "page.select_option"
                | "page.hover"
                | "page.focus"
                | "page.evaluate"
                | "page.act"
                | "page.observe"
                | "page.extract"
                | "page.agent"
                | "page.invoke_site_tool"
                | "page.site_tools"
                | "page.cancel_site_tool"
                | "page.wait_for_navigation"
                | "page.wait_for_response"
                | "page.wait_for_request"
        );
    }
    if namespace == "locator" {
        return operation.class == OperationClass::Effect;
    }
    false
}

/// Builds MCP [`ToolAnnotations`] from an operation's class and open-world rule.
#[must_use]
pub fn annotations_for(operation: &Operation) -> ToolAnnotations {
    let open_world = is_open_world(operation);
    match operation.class {
        OperationClass::Read => ToolAnnotations::new()
            .read_only(true)
            .open_world(open_world),
        OperationClass::IdempotentWrite => ToolAnnotations::new()
            .read_only(false)
            .destructive(false)
            .idempotent(true)
            .open_world(open_world),
        OperationClass::Effect => ToolAnnotations::new()
            .read_only(false)
            .destructive(true)
            .idempotent(false)
            .open_world(open_world),
    }
}

/// Builds the MCP tool description from the catalog operation's one text,
/// prepending the standard deprecation line when deprecated (ADR 0099, #1360).
#[must_use]
pub fn description_for(operation: &Operation) -> String {
    match &operation.deprecated {
        Some(deprecation) => {
            let major = deprecation.earliest_removal.major;
            let prefix = match deprecation.replacement {
                Some(replacement) => {
                    format!(
                        "Deprecated: use `{}`, removed in {major}",
                        tool_name(replacement)
                    )
                }
                None => format!("Deprecated: removed in {major}"),
            };
            format!("{prefix}. {}", operation.description)
        }
        None => operation.description.to_owned(),
    }
}

/// Builds the JSON Schema object for an operation's `inputSchema`.
///
/// The root is always an object schema (`"type": "object"`) with any referenced
/// definitions carried in `"$defs"`. Granting parameters excluded from agent
/// surfaces are stripped, and deprecated operations carry `"deprecated": true`.
#[must_use]
pub fn input_schema_for(operation: &Operation) -> Arc<JsonObject> {
    let mut generator = SchemaSettings::draft2020_12().into_generator();
    let root_schema = (operation.params)(&mut generator);
    let defs = generator.definitions().clone();
    let mut object = resolve_root_object(root_schema.to_value(), &defs);

    if !defs.is_empty() && !object.contains_key("$defs") {
        object.insert("$defs".to_owned(), Value::Object(defs));
    }

    for &(op_name, param) in OMITTED_GRANTING_PARAMS {
        if operation.name == op_name {
            strip_property(&mut object, param);
        }
    }

    if operation.deprecated.is_some() {
        object.insert("deprecated".to_owned(), Value::Bool(true));
    }

    if !object.contains_key("type") {
        object.insert("type".to_owned(), Value::String("object".to_owned()));
    }
    if !object.contains_key("properties") {
        object.insert("properties".to_owned(), Value::Object(Map::new()));
    }

    Arc::new(object)
}

/// Resolves a `$ref` root schema against `$defs` into an inline object schema
/// so every MCP client receives a top-level `"type": "object"` schema.
fn resolve_root_object(root: Value, defs: &Map<String, Value>) -> JsonObject {
    let Value::Object(mut map) = root else {
        return Map::new();
    };
    if let Some(reference) = map.get("$ref").and_then(Value::as_str)
        && let Some(def_name) = reference.strip_prefix("#/$defs/")
        && let Some(Value::Object(resolved)) = defs.get(def_name)
    {
        let mut merged = resolved.clone();
        for (key, value) in map {
            if key != "$ref" {
                merged.insert(key, value);
            }
        }
        map = merged;
    }
    map
}

/// Removes `param` from `properties` and `required` of an object schema.
fn strip_property(schema: &mut JsonObject, param: &str) {
    if let Some(Value::Object(properties)) = schema.get_mut("properties") {
        properties.remove(param);
    }
    if let Some(Value::Array(required)) = schema.get_mut("required") {
        required.retain(|item| item.as_str() != Some(param));
        if required.is_empty() {
            schema.remove("required");
        }
    }
}

/// Projects a single catalog [`Operation`] onto an MCP [`ProjectedTool`].
#[must_use]
pub fn project_operation(operation: &'static Operation) -> ProjectedTool {
    let name = tool_name(operation.name);
    let description = description_for(operation);
    let input_schema = input_schema_for(operation);
    let annotations = annotations_for(operation);
    let tool = Tool::new(Cow::Owned(name), Cow::Owned(description), input_schema)
        .with_annotations(annotations);

    ProjectedTool {
        operation: operation.name,
        class: operation.class,
        cancellable: operation.cancellable,
        tool,
    }
}

/// Determines the [`ErrorGroup`] of a JSON-RPC [`ErrorObject`] using the same
/// rule as the CLI (ADR 0069, ADR 0079).
#[must_use]
pub fn error_group(error: &ErrorObject) -> ErrorGroup {
    ErrorGroup::ALL
        .into_iter()
        .find(|group| group.json_rpc_class() == error.code)
        .or_else(|| {
            ProtocolFailure::ALL
                .into_iter()
                .find(|failure| failure.name() == error.data.code)
                .map(ProtocolFailure::group)
        })
        .unwrap_or(ErrorGroup::Internal)
}

/// Converts a JSON-RPC [`ErrorObject`] into the CLI's [`CommandError`] object.
#[must_use]
pub fn command_error(error: &ErrorObject, operation: Option<&str>) -> CommandError {
    CommandError {
        code: error.data.code.clone(),
        group: error_group(error),
        retryable: error.data.retryable,
        message: error.message.clone(),
        operation: operation.map(str::to_owned),
        details: error.data.details.clone(),
    }
}

/// Formats an engine [`ErrorObject`] as an MCP [`CallToolResult`] with
/// `isError: true`, `structuredContent` carrying the CLI's [`CommandError`],
/// and `fix` first in the text block when present (ADR 0070, #1359).
#[must_use]
pub fn error_tool_result(error: &ErrorObject, operation: Option<&str>) -> CallToolResult {
    let command_err = command_error(error, operation);
    let structured =
        serde_json::to_value(&command_err).expect("invariant: CommandError serializes");
    let json_text = serde_json::to_string(&structured).expect("invariant: JSON value serializes");
    let text = match command_err.details.get("fix").and_then(Value::as_str) {
        Some(fix) if !fix.is_empty() => format!("{fix}\n{json_text}"),
        _ => json_text,
    };
    let mut result = CallToolResult::error(vec![ContentBlock::text(text)]);
    result.structured_content = Some(structured);
    result
}

/// Formats a successful engine result as an MCP [`CallToolResult`] carrying
/// `structuredContent` (`snake_case`), its JSON serialized as text for hosts
/// without structure, and an image content block when the result carries a
/// screenshot (ADR 0070, #1359).
#[must_use]
pub fn success_tool_result(value: Value) -> CallToolResult {
    let json_text = serde_json::to_string(&value).expect("invariant: JSON value serializes");
    let mut content = vec![ContentBlock::text(json_text)];
    if let Some((data, mime_type)) = extract_screenshot(&value) {
        content.push(ContentBlock::image(data, mime_type));
    }
    let mut result = CallToolResult::success(content);
    result.structured_content = Some(value);
    result
}

/// Extracts base64 screenshot `(data, mime_type)` if the result carries one.
fn extract_screenshot(value: &Value) -> Option<(String, String)> {
    let object = value.as_object()?;
    if let Some((data, mime)) = image_fields(object) {
        return Some((data, mime));
    }
    if let Some(nested) = object.get("screenshot").and_then(Value::as_object) {
        return image_fields(nested);
    }
    None
}

fn image_fields(object: &Map<String, Value>) -> Option<(String, String)> {
    let mime = object
        .get("mime_type")
        .or_else(|| object.get("content_type"))
        .and_then(Value::as_str)?;
    if !mime.starts_with("image/") {
        return None;
    }
    let data = object
        .get("data")
        .or_else(|| object.get("base64"))
        .and_then(Value::as_str)?;
    if data.is_empty() {
        return None;
    }
    Some((data.to_owned(), mime.to_owned()))
}
