//! Conformance fixture [`ServerHandler`] implementing the MCP conformance
//! suite's fixture tools, resources, resource templates, prompts, completions,
//! logging, subscriptions, skills, and `2026-07-28` multi-round-trip
//! `InputRequiredResult` handlers (ADR 0070, ticket #414).
//!
//! Size: one cohesive conformance fixture catalog and `ServerHandler` dispatch
//! table covering every scenario of `@modelcontextprotocol/conformance`.
#![expect(
    clippy::too_many_lines,
    reason = "fixture catalog dispatch table covers all conformance suite scenarios in one cohesive module"
)]
#![expect(
    deprecated,
    reason = "fixture server implements legacy sampling, roots, and logging scenarios required by 2025-06-18 and 2025-11-25 conformance suites (ADR 0070)"
)]

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_mcp::revisions;
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    AudioContent, CallToolRequestParams, CallToolResponse, CallToolResult, CompleteRequestParams,
    CompleteResult, CompletionInfo, ContentBlock, CreateMessageRequest, CreateMessageRequestParams,
    ElicitRequest, ElicitRequestParams, EmbeddedResource, ErrorCode, GetPromptRequestParams,
    GetPromptResponse, GetPromptResult, GetSkillRequestParams, GetSkillResult, ImageContent,
    Implementation, InputRequest, InputRequests, InputRequiredResult, ListPromptsResult,
    ListResourceTemplatesResult, ListResourcesResult, ListRootsRequest, ListSkillsResult,
    ListToolsResult, LoggingLevel, LoggingMessageNotificationParam, PaginatedRequestParams,
    ProgressNotificationParam, Prompt, PromptArgument, PromptMessage, ProtocolVersion,
    ReadResourceRequestParams, ReadResourceResponse, ReadResourceResult, Resource,
    ResourceContents, ResourceTemplate, Role, SamplingContent, SamplingMessage, ServerCapabilities,
    ServerConfig, ServerRequest, SetLevelRequestParams, Skill, SkillFrontmatter, SkillResource,
    SkillResources, SubscribeRequestParams, SubscriptionFilter, TextContent, Tool,
    UnsubscribeRequestParams,
};
use rmcp::service::{RequestContext, RoleServer};
use serde_json::{Map, Value, json};

/// Minimal 1x1 red PNG, base64-encoded.
const TINY_PNG_B64: &str = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8DwHwAFBQIAX8jx0gAAAABJRU5ErkJggg==";

/// Minimal valid 44-byte silent WAV header, base64-encoded.
const TINY_WAV_B64: &str = "UklGRiQAAABXQVZFZm10IBAAAAABAAEARKwAAIhYAQACABAAZGF0YQAAAAA=";

/// Integrity-checked state token used by `test_input_required_result_tampered_state`.
const SIGNED_REQUEST_STATE: &str = "gf-fixture-state-v1.sig=9a4e2c1b8d7f6053";

/// Fixture SKILL.md content using the `skill://` URI scheme (SEP-2640).
const FIXTURE_SKILL_URI: &str = "skill://goodfetch-fixture/SKILL.md";
const FIXTURE_SKILL_NAME: &str = "goodfetch-fixture";
const FIXTURE_SKILL_DESC: &str =
    "Fixture skill served by Goodfetch's MCP conformance fixture server.";
const FIXTURE_SKILL_MD: &str = "---\nname: goodfetch-fixture\ndescription: Fixture skill served by Goodfetch's MCP conformance fixture server.\n---\n\n# Goodfetch Fixture Skill\n\nUsed by the MCP conformance suite.\n";

/// Test-only conformance fixture server handler.
#[derive(Clone, Default)]
pub(crate) struct FixtureServer;

impl FixtureServer {
    pub(crate) fn new() -> Self {
        Self
    }
}

fn empty_schema() -> Arc<Map<String, Value>> {
    let mut map = Map::new();
    map.insert("type".to_owned(), Value::String("object".to_owned()));
    map.insert("properties".to_owned(), Value::Object(Map::new()));
    Arc::new(map)
}

fn string_arg_schema(arg_name: &str) -> Arc<Map<String, Value>> {
    let mut prop = Map::new();
    prop.insert("type".to_owned(), Value::String("string".to_owned()));
    let mut props = Map::new();
    props.insert(arg_name.to_owned(), Value::Object(prop));
    let mut map = Map::new();
    map.insert("type".to_owned(), Value::String("object".to_owned()));
    map.insert("properties".to_owned(), Value::Object(props));
    map.insert(
        "required".to_owned(),
        Value::Array(vec![Value::String(arg_name.to_owned())]),
    );
    Arc::new(map)
}

fn text_block(text: impl Into<String>) -> ContentBlock {
    ContentBlock::Text(TextContent::new(text))
}

fn image_block(data: impl Into<String>, mime: impl Into<String>) -> ContentBlock {
    ContentBlock::Image(ImageContent::new(data, mime))
}

fn audio_block(data: impl Into<String>, mime: impl Into<String>) -> ContentBlock {
    ContentBlock::Audio(AudioContent::new(data, mime))
}

fn resource_block(contents: ResourceContents) -> ContentBlock {
    ContentBlock::Resource(EmbeddedResource::new(contents))
}

fn fixture_tools() -> Vec<Tool> {
    let simple = |name: &'static str, desc: &'static str| Tool::new(name, desc, empty_schema());
    vec![
        simple("test_simple_text", "Returns simple text content."),
        simple("test_image_content", "Returns image content."),
        simple("test_audio_content", "Returns audio content."),
        simple("test_embedded_resource", "Returns an embedded resource."),
        simple(
            "test_multiple_content_types",
            "Returns text, image, and embedded resource.",
        ),
        simple(
            "test_tool_with_logging",
            "Emits three info log notifications.",
        ),
        simple("test_error_handling", "Returns an error tool result."),
        simple("test_tool_with_progress", "Emits progress notifications."),
        Tool::new(
            "test_sampling",
            "Requests LLM sampling from the client.",
            string_arg_schema("prompt"),
        ),
        Tool::new(
            "test_elicitation",
            "Requests user elicitation from the client.",
            string_arg_schema("message"),
        ),
        simple(
            "test_elicitation_sep1034_defaults",
            "Elicitation with SEP-1034 primitive defaults.",
        ),
        simple(
            "test_elicitation_sep1330_enums",
            "Elicitation with SEP-1330 enum variants.",
        ),
        simple(
            "test_missing_capability",
            "Requires sampling capability in _meta.",
        ),
        simple(
            "test_streaming_elicitation",
            "Returns InputRequiredResult for streaming elicitation.",
        ),
        simple(
            "test_logging_tool",
            "Only logs when _meta logLevel is declared.",
        ),
        simple(
            "test_input_required_result_elicitation",
            "MRTR elicitation fixture.",
        ),
        simple(
            "test_input_required_result_sampling",
            "MRTR sampling fixture.",
        ),
        simple(
            "test_input_required_result_list_roots",
            "MRTR roots/list fixture.",
        ),
        simple(
            "test_input_required_result_request_state",
            "MRTR requestState fixture.",
        ),
        simple(
            "test_input_required_result_multiple_inputs",
            "MRTR multiple inputRequests fixture.",
        ),
        simple(
            "test_input_required_result_multi_round",
            "MRTR multi-round fixture.",
        ),
        simple(
            "test_input_required_result_tampered_state",
            "MRTR tampered requestState fixture.",
        ),
        simple(
            "test_input_required_result_capabilities",
            "MRTR client capability check fixture.",
        ),
    ]
}

fn fixture_skill() -> Skill {
    let frontmatter = SkillFrontmatter::new(FIXTURE_SKILL_NAME, FIXTURE_SKILL_DESC);
    let manifest_file = SkillResource::from_bytes(FIXTURE_SKILL_URI, FIXTURE_SKILL_MD.as_bytes());
    Skill::new(
        FIXTURE_SKILL_URI,
        frontmatter,
        SkillResources::Files(vec![manifest_file]),
    )
}

fn fixture_resources() -> Vec<Resource> {
    vec![
        Resource::new("test://static-text", "Static Text Resource")
            .with_title("Static Text Resource")
            .with_description("A static plain text resource for conformance testing.")
            .with_mime_type("text/plain")
            .with_size(48),
        Resource::new("test://static-binary", "Static Binary Resource")
            .with_title("Static Binary Resource")
            .with_description("A static binary PNG resource for conformance testing.")
            .with_mime_type("image/png")
            .with_size(67),
        Resource::new("test://watched-resource", "Watched Resource")
            .with_title("Watched Resource")
            .with_description("A subscribable resource for conformance testing.")
            .with_mime_type("text/plain")
            .with_size(25),
        Resource::new(FIXTURE_SKILL_URI, FIXTURE_SKILL_NAME)
            .with_title("Goodfetch Fixture Skill")
            .with_description(FIXTURE_SKILL_DESC)
            .with_mime_type("text/markdown")
            .with_size(FIXTURE_SKILL_MD.len() as u64),
    ]
}

fn fixture_resource_templates() -> Vec<ResourceTemplate> {
    vec![
        ResourceTemplate::new("test://template/{id}/data", "Template Resource")
            .with_title("Template Resource")
            .with_description("A parameterized resource template.")
            .with_mime_type("application/json"),
    ]
}

fn fixture_prompts() -> Vec<Prompt> {
    vec![
        Prompt::new("test_simple_prompt", Some("A simple test prompt."), None),
        Prompt::new(
            "test_prompt_with_arguments",
            Some("A test prompt with arguments."),
            Some(vec![
                PromptArgument::new("arg1")
                    .with_description("First argument")
                    .with_required(true),
                PromptArgument::new("arg2")
                    .with_description("Second argument")
                    .with_required(false),
            ]),
        ),
        Prompt::new(
            "test_prompt_with_embedded_resource",
            Some("A test prompt with an embedded resource."),
            Some(vec![
                PromptArgument::new("resourceUri")
                    .with_description("Resource URI to embed")
                    .with_required(true),
            ]),
        ),
        Prompt::new(
            "test_prompt_with_image",
            Some("A test prompt with an image."),
            None,
        ),
        Prompt::new(
            "test_input_required_result_prompt",
            Some("A prompt that requests elicitation input."),
            None,
        ),
    ]
}

fn make_elicitation_params(message: &str, schema: &Value) -> Result<ElicitRequestParams, McpError> {
    serde_json::from_value(json!({
        "message": message,
        "requestedSchema": schema,
    }))
    .map_err(|err| McpError::internal_error(err.to_string(), None))
}

fn make_elicitation_input_request(message: &str, schema: &Value) -> InputRequest {
    let params = make_elicitation_params(message, schema)
        .expect("invariant: static valid elicitation params");
    InputRequest::Elicitation(ElicitRequest::new(params))
}

fn make_sampling_input_request(prompt_text: &str) -> InputRequest {
    let params =
        CreateMessageRequestParams::new(vec![SamplingMessage::user_text(prompt_text)], 100);
    InputRequest::CreateMessage(CreateMessageRequest::new(params))
}

fn client_capabilities_meta(context: &RequestContext<RoleServer>) -> Option<&Map<String, Value>> {
    context
        .meta
        .get("io.modelcontextprotocol/clientCapabilities")
        .and_then(Value::as_object)
}

fn client_has_capability(context: &RequestContext<RoleServer>, cap: &str) -> bool {
    client_capabilities_meta(context).is_some_and(|caps| caps.contains_key(cap))
}

fn client_log_level(context: &RequestContext<RoleServer>) -> Option<&str> {
    context
        .meta
        .get("io.modelcontextprotocol/logLevel")
        .and_then(Value::as_str)
}

async fn send_elicitation(
    context: &RequestContext<RoleServer>,
    params: ElicitRequestParams,
) -> Result<Value, McpError> {
    let reply = context
        .peer
        .send_request(ServerRequest::ElicitRequest(ElicitRequest::new(params)))
        .await
        .map_err(|err| McpError::internal_error(err.to_string(), None))?;
    match reply {
        rmcp::model::ClientResult::ElicitResult(res) => {
            serde_json::to_value(res).map_err(|err| McpError::internal_error(err.to_string(), None))
        }
        other => Ok(json!({ "unexpected": format!("{other:?}") })),
    }
}

impl ServerHandler for FixtureServer {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_resources_subscribe()
            .enable_prompts()
            .enable_completions()
            .enable_logging()
            .enable_skills()
            .build();
        ServerConfig::new(capabilities)
            .with_protocol_version(revisions::DEFAULT_HANDSHAKE_VERSION)
            .with_server_info(Implementation::new(
                "goodfetch-mcp-fixture-server",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions("Goodfetch test-only MCP conformance fixture server.")
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(revisions::SUPPORTED)
    }

    fn set_level(
        &self,
        _request: SetLevelRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Ok(()))
    }

    fn accepted_subscription_filter(
        &self,
        requested: &SubscriptionFilter,
    ) -> Option<SubscriptionFilter> {
        Some(requested.clone())
    }

    fn subscribe(
        &self,
        _request: SubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Ok(()))
    }

    fn unsubscribe(
        &self,
        _request: UnsubscribeRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<(), McpError>> + Send + '_ {
        std::future::ready(Ok(()))
    }

    fn complete(
        &self,
        request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<CompleteResult, McpError>> + Send + '_ {
        let prefix = request.argument.value;
        let candidates = ["paris", "park", "party", "test1", "test2"];
        let values: Vec<String> = candidates
            .iter()
            .filter(|c| c.starts_with(&prefix))
            .map(|c| (*c).to_owned())
            .collect();
        let info = CompletionInfo::with_all_values(values).unwrap_or_default();
        std::future::ready(Ok(CompleteResult::new(info)))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult::with_all_items(fixture_resources())))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourceTemplatesResult, McpError>> + Send + '_
    {
        std::future::ready(Ok(ListResourceTemplatesResult::with_all_items(
            fixture_resource_templates(),
        )))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResponse, McpError>> + Send + '_ {
        let uri = request.uri;
        let outcome = match uri.as_str() {
            "test://static-text" => Ok(ReadResourceResult::new(vec![
                ResourceContents::TextResourceContents {
                    uri,
                    mime_type: Some("text/plain".to_owned()),
                    text: "This is the content of the static text resource.".to_owned(),
                    meta: None,
                },
            ])),
            "test://static-binary" => Ok(ReadResourceResult::new(vec![
                ResourceContents::BlobResourceContents {
                    uri,
                    mime_type: Some("image/png".to_owned()),
                    blob: TINY_PNG_B64.to_owned(),
                    meta: None,
                },
            ])),
            "test://watched-resource" => Ok(ReadResourceResult::new(vec![
                ResourceContents::TextResourceContents {
                    uri,
                    mime_type: Some("text/plain".to_owned()),
                    text: "Watched resource content.".to_owned(),
                    meta: None,
                },
            ])),
            FIXTURE_SKILL_URI => Ok(ReadResourceResult::new(vec![
                ResourceContents::TextResourceContents {
                    uri,
                    mime_type: Some("text/markdown".to_owned()),
                    text: FIXTURE_SKILL_MD.to_owned(),
                    meta: None,
                },
            ])),
            other => {
                if let Some(rest) = other.strip_prefix("test://template/")
                    && let Some(id) = rest.strip_suffix("/data")
                    && !id.is_empty()
                {
                    Ok(ReadResourceResult::new(vec![
                        ResourceContents::TextResourceContents {
                            uri: other.to_owned(),
                            mime_type: Some("application/json".to_owned()),
                            text: json!({ "id": id, "templateTest": true }).to_string(),
                            meta: None,
                        },
                    ]))
                } else {
                    Err(McpError::resource_not_found(
                        format!("unknown resource `{other}`"),
                        Some(json!({ "uri": other })),
                    ))
                }
            }
        };
        std::future::ready(outcome.map(ReadResourceResponse::from))
    }

    fn list_skills(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListSkillsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListSkillsResult::with_all_items(vec![fixture_skill()])))
    }

    fn get_skill(
        &self,
        request: GetSkillRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<GetSkillResult, McpError>> + Send + '_ {
        let outcome = if request.uri == FIXTURE_SKILL_URI {
            Ok(GetSkillResult::new(fixture_skill()))
        } else {
            Err(McpError::invalid_params(
                format!("unknown skill `{}`", request.uri),
                None,
            ))
        };
        std::future::ready(outcome)
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListPromptsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListPromptsResult::with_all_items(fixture_prompts())))
    }

    fn get_prompt(
        &self,
        request: GetPromptRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<GetPromptResponse, McpError>> + Send + '_ {
        let outcome: Result<GetPromptResponse, McpError> = match request.name.as_str() {
            "test_simple_prompt" => Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                Role::User,
                "This is a simple prompt for testing.",
            )])
            .into()),
            "test_prompt_with_arguments" => {
                let args = request.arguments.unwrap_or_default();
                let first = args
                    .get("arg1")
                    .and_then(Value::as_str)
                    .unwrap_or("default1");
                let second = args
                    .get("arg2")
                    .and_then(Value::as_str)
                    .unwrap_or("default2");
                Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                    Role::User,
                    format!("Prompt with arguments: arg1={first}, arg2={second}"),
                )])
                .into())
            }
            "test_prompt_with_embedded_resource" => {
                let uri = request
                    .arguments
                    .as_ref()
                    .and_then(|a| a.get("resourceUri"))
                    .and_then(Value::as_str)
                    .unwrap_or("test://static-text");
                Ok(GetPromptResult::new(vec![PromptMessage::new(
                    Role::User,
                    resource_block(ResourceContents::TextResourceContents {
                        uri: uri.to_owned(),
                        mime_type: Some("text/plain".to_owned()),
                        text: "Embedded resource content for prompt testing.".to_owned(),
                        meta: None,
                    }),
                )])
                .into())
            }
            "test_prompt_with_image" => Ok(GetPromptResult::new(vec![
                PromptMessage::new(Role::User, image_block(TINY_PNG_B64, "image/png")),
                PromptMessage::new_text(Role::User, "Please analyze the image above."),
            ])
            .into()),
            "test_input_required_result_prompt" => {
                if let Some(responses) = request.input_responses.as_ref()
                    && let Some(ctx_val) = responses.get("user_context")
                {
                    Ok(GetPromptResult::new(vec![PromptMessage::new_text(
                        Role::User,
                        format!("Prompt completed with context: {ctx_val}"),
                    )])
                    .into())
                } else {
                    let mut reqs: InputRequests = BTreeMap::new();
                    reqs.insert(
                        "user_context".to_owned(),
                        make_elicitation_input_request(
                            "What context should the prompt use?",
                            &json!({
                                "type": "object",
                                "properties": { "context": { "type": "string" } },
                                "required": ["context"]
                            }),
                        ),
                    );
                    Ok(InputRequiredResult::from_input_requests(reqs).into())
                }
            }
            other => Err(McpError::invalid_params(
                format!("unknown prompt `{other}`"),
                None,
            )),
        };
        std::future::ready(outcome)
    }

    fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListToolsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListToolsResult::with_all_items(fixture_tools())))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let args = request.arguments.unwrap_or_default();
        let responses = request.input_responses;
        let request_state = request.request_state;

        match request.name.as_ref() {
            "test_simple_text" => Ok(CallToolResult::success(vec![text_block(
                "This is a simple text response for testing.",
            )])
            .into()),
            "test_image_content" => {
                Ok(CallToolResult::success(vec![image_block(TINY_PNG_B64, "image/png")]).into())
            }
            "test_audio_content" => {
                Ok(CallToolResult::success(vec![audio_block(TINY_WAV_B64, "audio/wav")]).into())
            }
            "test_embedded_resource" => Ok(CallToolResult::success(vec![resource_block(
                ResourceContents::TextResourceContents {
                    uri: "test://embedded-resource".to_owned(),
                    mime_type: Some("text/plain".to_owned()),
                    text: "This is an embedded resource content.".to_owned(),
                    meta: None,
                },
            )])
            .into()),
            "test_multiple_content_types" => Ok(CallToolResult::success(vec![
                text_block("Multiple content types test:"),
                image_block(TINY_PNG_B64, "image/png"),
                resource_block(ResourceContents::TextResourceContents {
                    uri: "test://mixed-content-resource".to_owned(),
                    mime_type: Some("application/json".to_owned()),
                    text: json!({ "test": "data", "value": 123 }).to_string(),
                    meta: None,
                }),
            ])
            .into()),
            "test_tool_with_logging" => {
                for msg in [
                    "Tool execution started",
                    "Tool processing data",
                    "Tool execution completed",
                ] {
                    let mut note = LoggingMessageNotificationParam::new(
                        LoggingLevel::Info,
                        Value::String(msg.to_owned()),
                    );
                    note.logger = Some("goodfetch-fixture".to_owned());
                    let _ = context.peer.notify_logging_message(note).await;
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                Ok(
                    CallToolResult::success(vec![text_block("Tool with logging completed.")])
                        .into(),
                )
            }
            "test_error_handling" => Ok(CallToolResult::error(vec![text_block(
                "This tool intentionally returns an error for testing",
            )])
            .into()),
            "test_tool_with_progress" => {
                let token = context.meta.get_progress_token();
                for progress in [0.0_f64, 50.0, 100.0] {
                    if let Some(ref tok) = token {
                        let _ = context
                            .peer
                            .notify_progress(
                                ProgressNotificationParam::new(tok.clone(), progress)
                                    .with_total(100.0),
                            )
                            .await;
                    }
                    if progress < 100.0 {
                        tokio::time::sleep(Duration::from_millis(50)).await;
                    }
                }
                Ok(
                    CallToolResult::success(vec![text_block("Tool with progress completed.")])
                        .into(),
                )
            }
            "test_sampling" => {
                let prompt = args
                    .get("prompt")
                    .and_then(Value::as_str)
                    .unwrap_or("Hello from fixture");
                let params =
                    CreateMessageRequestParams::new(vec![SamplingMessage::user_text(prompt)], 100);
                let sampled = context
                    .peer
                    .create_message(params)
                    .await
                    .map_err(|err| McpError::internal_error(err.to_string(), None))?;
                let text = match sampled.message.content {
                    SamplingContent::Single(c) => format!("{c:?}"),
                    SamplingContent::Multiple(cs) => format!("{cs:?}"),
                };
                Ok(
                    CallToolResult::success(vec![text_block(format!("LLM response: {text}"))])
                        .into(),
                )
            }
            "test_elicitation" => {
                let message = args
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("Please provide input");
                let params = make_elicitation_params(
                    message,
                    &json!({
                        "type": "object",
                        "properties": {
                            "username": { "type": "string", "description": "User's response" },
                            "email": { "type": "string", "description": "User's email address" }
                        },
                        "required": ["username", "email"]
                    }),
                )?;
                let result = send_elicitation(&context, params).await?;
                Ok(
                    CallToolResult::success(vec![text_block(format!("User response: {result}"))])
                        .into(),
                )
            }
            "test_elicitation_sep1034_defaults" => {
                let params = make_elicitation_params(
                    "Provide defaults test values",
                    &json!({
                        "type": "object",
                        "properties": {
                            "name": { "type": "string", "default": "John Doe" },
                            "age": { "type": "integer", "default": 30 },
                            "score": { "type": "number", "default": 95.5 },
                            "status": {
                                "type": "string",
                                "enum": ["active", "inactive", "pending"],
                                "default": "active"
                            },
                            "verified": { "type": "boolean", "default": true }
                        }
                    }),
                )?;
                let result = send_elicitation(&context, params).await?;
                Ok(CallToolResult::success(vec![text_block(result.to_string())]).into())
            }
            "test_elicitation_sep1330_enums" => {
                let params = make_elicitation_params(
                    "Provide enum test values",
                    &json!({
                        "type": "object",
                        "properties": {
                            "untitledSingle": {
                                "type": "string",
                                "enum": ["option1", "option2", "option3"]
                            },
                            "titledSingle": {
                                "type": "string",
                                "oneOf": [
                                    { "const": "value1", "title": "First Option" },
                                    { "const": "value2", "title": "Second Option" },
                                    { "const": "value3", "title": "Third Option" }
                                ]
                            },
                            "legacyEnum": {
                                "type": "string",
                                "enum": ["opt1", "opt2", "opt3"],
                                "enumNames": ["Option One", "Option Two", "Option Three"]
                            },
                            "untitledMulti": {
                                "type": "array",
                                "items": {
                                    "type": "string",
                                    "enum": ["option1", "option2", "option3"]
                                }
                            },
                            "titledMulti": {
                                "type": "array",
                                "items": {
                                    "anyOf": [
                                        { "const": "value1", "title": "First Choice" },
                                        { "const": "value2", "title": "Second Choice" },
                                        { "const": "value3", "title": "Third Choice" }
                                    ]
                                }
                            }
                        }
                    }),
                )?;
                let result = send_elicitation(&context, params).await?;
                Ok(CallToolResult::success(vec![text_block(result.to_string())]).into())
            }
            "test_missing_capability" => {
                if !client_has_capability(&context, "sampling") {
                    return Err(McpError::new(
                        ErrorCode(-32021),
                        "Missing required client capability: sampling",
                        Some(json!({ "requiredCapabilities": { "sampling": {} } })),
                    ));
                }
                Ok(CallToolResult::success(vec![text_block("Capability present.")]).into())
            }
            "test_streaming_elicitation" => {
                let mut reqs: InputRequests = BTreeMap::new();
                reqs.insert(
                    "confirm".to_owned(),
                    make_elicitation_input_request(
                        "Confirm streaming elicitation",
                        &json!({
                            "type": "object",
                            "properties": { "ok": { "type": "boolean" } },
                            "required": ["ok"]
                        }),
                    ),
                );
                Ok(InputRequiredResult::from_input_requests(reqs).into())
            }
            "test_logging_tool" => {
                if client_log_level(&context).is_some() {
                    let mut note = LoggingMessageNotificationParam::new(
                        LoggingLevel::Info,
                        Value::String("Conditional log message".to_owned()),
                    );
                    note.logger = Some("goodfetch-fixture".to_owned());
                    let _ = context.peer.notify_logging_message(note).await;
                }
                Ok(CallToolResult::success(vec![text_block("Logging tool done.")]).into())
            }
            "test_input_required_result_elicitation" => {
                if let Some(resps) = responses.as_ref()
                    && let Some(user_name) = resps.get("user_name")
                {
                    let Some(obj) = user_name.as_object() else {
                        return Err(McpError::invalid_params(
                            "invalid inputResponses: `user_name` must be an object",
                            None,
                        ));
                    };
                    if !obj.get("action").is_some_and(Value::is_string) {
                        return Err(McpError::invalid_params(
                            "invalid inputResponses: `user_name.action` must be a string",
                            None,
                        ));
                    }
                    Ok(
                        CallToolResult::success(vec![text_block(format!("Hello, {user_name}"))])
                            .into(),
                    )
                } else {
                    let mut reqs: InputRequests = BTreeMap::new();
                    reqs.insert(
                        "user_name".to_owned(),
                        make_elicitation_input_request(
                            "What is your name?",
                            &json!({
                                "type": "object",
                                "properties": { "name": { "type": "string" } },
                                "required": ["name"]
                            }),
                        ),
                    );
                    Ok(InputRequiredResult::from_input_requests(reqs).into())
                }
            }
            "test_input_required_result_sampling" => {
                if let Some(resps) = responses.as_ref()
                    && let Some(answer) = resps.get("capital_question")
                {
                    Ok(CallToolResult::success(vec![text_block(format!(
                        "Sampling answer: {answer}"
                    ))])
                    .into())
                } else {
                    let mut reqs: InputRequests = BTreeMap::new();
                    reqs.insert(
                        "capital_question".to_owned(),
                        make_sampling_input_request("What is the capital of France?"),
                    );
                    Ok(InputRequiredResult::from_input_requests(reqs).into())
                }
            }
            "test_input_required_result_list_roots" => {
                if let Some(resps) = responses.as_ref()
                    && let Some(roots) = resps.get("client_roots")
                {
                    Ok(
                        CallToolResult::success(vec![text_block(format!("Client roots: {roots}"))])
                            .into(),
                    )
                } else {
                    let mut reqs: InputRequests = BTreeMap::new();
                    reqs.insert(
                        "client_roots".to_owned(),
                        InputRequest::ListRoots(ListRootsRequest::default()),
                    );
                    Ok(InputRequiredResult::from_input_requests(reqs).into())
                }
            }
            "test_input_required_result_request_state" => {
                if let Some(resps) = responses.as_ref()
                    && let Some(confirm) = resps.get("confirm")
                {
                    let echoed = request_state.as_deref().unwrap_or("");
                    if echoed != "state-token-round-1" {
                        return Err(McpError::invalid_params(
                            "missing or wrong requestState",
                            None,
                        ));
                    }
                    Ok(CallToolResult::success(vec![text_block(format!(
                        "Confirmed with state {echoed}: {confirm}"
                    ))])
                    .into())
                } else {
                    let mut reqs: InputRequests = BTreeMap::new();
                    reqs.insert(
                        "confirm".to_owned(),
                        make_elicitation_input_request(
                            "Please confirm",
                            &json!({
                                "type": "object",
                                "properties": { "ok": { "type": "boolean" } },
                                "required": ["ok"]
                            }),
                        ),
                    );
                    Ok(
                        InputRequiredResult::new(
                            Some(reqs),
                            Some("state-token-round-1".to_owned()),
                        )
                        .into(),
                    )
                }
            }
            "test_input_required_result_multiple_inputs" => {
                if let Some(resps) = responses.as_ref()
                    && resps.contains_key("user_name")
                    && resps.contains_key("capital_question")
                    && resps.contains_key("client_roots")
                {
                    Ok(CallToolResult::success(vec![text_block(
                        serde_json::to_string(resps).unwrap_or_default(),
                    )])
                    .into())
                } else {
                    let mut reqs: InputRequests = BTreeMap::new();
                    reqs.insert(
                        "user_name".to_owned(),
                        make_elicitation_input_request(
                            "What is your name?",
                            &json!({
                                "type": "object",
                                "properties": { "name": { "type": "string" } },
                                "required": ["name"]
                            }),
                        ),
                    );
                    reqs.insert(
                        "capital_question".to_owned(),
                        make_sampling_input_request("What is the capital of France?"),
                    );
                    reqs.insert(
                        "client_roots".to_owned(),
                        InputRequest::ListRoots(ListRootsRequest::default()),
                    );
                    Ok(
                        InputRequiredResult::new(Some(reqs), Some("multi-input-state".to_owned()))
                            .into(),
                    )
                }
            }
            "test_input_required_result_multi_round" => {
                if let Some(resps) = responses.as_ref() {
                    if let Some(step2) = resps.get("step2") {
                        return Ok(CallToolResult::success(vec![text_block(format!(
                            "Multi-round complete: state={request_state:?}, step2={step2}"
                        ))])
                        .into());
                    }
                    if let Some(step1) = resps.get("step1") {
                        let mut reqs: InputRequests = BTreeMap::new();
                        reqs.insert(
                            "step2".to_owned(),
                            make_elicitation_input_request(
                                "Step 2: Confirm your details",
                                &json!({
                                    "type": "object",
                                    "properties": { "confirmed": { "type": "boolean" } },
                                    "required": ["confirmed"]
                                }),
                            ),
                        );
                        return Ok(InputRequiredResult::new(
                            Some(reqs),
                            Some(format!("round-2:{step1}")),
                        )
                        .into());
                    }
                }
                let mut reqs: InputRequests = BTreeMap::new();
                reqs.insert(
                    "step1".to_owned(),
                    make_elicitation_input_request(
                        "Step 1: What is your name?",
                        &json!({
                            "type": "object",
                            "properties": { "name": { "type": "string" } },
                            "required": ["name"]
                        }),
                    ),
                );
                Ok(InputRequiredResult::new(Some(reqs), Some("round-1".to_owned())).into())
            }
            "test_input_required_result_tampered_state" => {
                if let Some(state) = request_state.as_deref() {
                    if state != SIGNED_REQUEST_STATE {
                        return Err(McpError::invalid_params(
                            "requestState integrity check failed",
                            None,
                        ));
                    }
                    return Ok(CallToolResult::success(vec![text_block(
                        "Verified signed requestState.",
                    )])
                    .into());
                }
                let mut reqs: InputRequests = BTreeMap::new();
                reqs.insert(
                    "confirm".to_owned(),
                    make_elicitation_input_request(
                        "Please confirm",
                        &json!({
                            "type": "object",
                            "properties": { "ok": { "type": "boolean" } },
                            "required": ["ok"]
                        }),
                    ),
                );
                Ok(
                    InputRequiredResult::new(Some(reqs), Some(SIGNED_REQUEST_STATE.to_owned()))
                        .into(),
                )
            }
            "test_input_required_result_capabilities" => {
                let mut reqs: InputRequests = BTreeMap::new();
                if client_has_capability(&context, "sampling") {
                    reqs.insert(
                        "sampling_req".to_owned(),
                        make_sampling_input_request("Summarize capability check"),
                    );
                }
                if client_has_capability(&context, "elicitation") {
                    reqs.insert(
                        "elicitation_req".to_owned(),
                        make_elicitation_input_request(
                            "Confirm capability",
                            &json!({
                                "type": "object",
                                "properties": { "ok": { "type": "boolean" } },
                                "required": ["ok"]
                            }),
                        ),
                    );
                }
                if client_has_capability(&context, "roots") {
                    reqs.insert(
                        "roots_req".to_owned(),
                        InputRequest::ListRoots(ListRootsRequest::default()),
                    );
                }
                if reqs.is_empty() {
                    Ok(CallToolResult::success(vec![text_block(
                        "No client capabilities declared.",
                    )])
                    .into())
                } else {
                    Ok(InputRequiredResult::from_input_requests(reqs).into())
                }
            }
            other => Err(McpError::invalid_params(
                format!("unknown tool `{other}`"),
                None,
            )),
        }
    }
}
