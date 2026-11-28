//! Projects Goodfetch catalog operations onto MCP tools, annotations,
//! input schemas and tool results (ADRs 0011, 0065, 0070, 0099; spec #608).
//!
//! Pure projection rules live in `goodfetch_protocol::mcp_projection` so the
//! MCP server and generated framework adapters share one canonical definition.

use std::borrow::Cow;
use std::sync::Arc;

use goodfetch_protocol::catalog::{ErrorGroup, Operation, OperationClass};
use goodfetch_protocol::cli::CommandError;
use goodfetch_protocol::jsonrpc::{ErrorObject, ProtocolFailure};
pub use goodfetch_protocol::mcp_projection::{
    MAX_DESCRIPTION_CHARS, MAX_TOOL_NAME_CHARS, ProjectionViolation, description_for,
    is_live_view_grant, is_open_world, is_valid_tool_name, tool_name, validate_operation,
};
use rmcp::model::{
    CallToolResult, ContentBlock, JsonObject, ListToolsResult, Tool, ToolAnnotations,
};
use serde_json::{Map, Value};

/// Compact JSON wire byte count of a `tools/list` response for `tools`
/// (ADR 0065, ADR 0071, issue 1360).
#[must_use]
pub fn tools_list_wire_bytes(tools: &[Tool]) -> usize {
    let payload = ListToolsResult::with_all_items(tools.to_vec());
    serde_json::to_vec(&payload)
        .expect("invariant: ListToolsResult serializes")
        .len()
}

/// Compact JSON wire byte count of a `tools/call` [`CallToolResult`]
/// (ADR 0065, ADR 0071, issue 1360).
#[must_use]
pub fn tool_result_wire_bytes(result: &CallToolResult) -> usize {
    serde_json::to_vec(result)
        .expect("invariant: CallToolResult serializes")
        .len()
}

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

/// Builds the JSON Schema object for an operation's `inputSchema`.
#[must_use]
pub fn input_schema_for(operation: &Operation) -> Arc<JsonObject> {
    Arc::new(goodfetch_protocol::mcp_projection::input_schema_for(
        operation,
    ))
}

/// Projects a single catalog [`Operation`] onto an MCP [`ProjectedTool`].
#[must_use]
pub fn project_operation(operation: &'static Operation) -> ProjectedTool {
    let spec = goodfetch_protocol::mcp_projection::project_operation(operation);
    let annotations = annotations_for(operation);
    let tool = Tool::new(
        Cow::Owned(spec.name),
        Cow::Owned(spec.description),
        Arc::new(spec.input_schema),
    )
    .with_annotations(annotations);

    ProjectedTool {
        operation: spec.operation,
        class: spec.class,
        cancellable: spec.cancellable,
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
