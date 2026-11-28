//! The MCP [`ServerHandler`] implementation shared by Collie and Foxhound
//! (ADRs 0011, 0070, 0080, 0090, 0132; spec #608).
//!
//! The server asks nothing of its host: it advertises neither sampling nor
//! elicitation and never sends either request. `initialize` carries the
//! executable's name (`goodfetch-collie` or `goodfetch-foxhound`) and two
//! sentences of `instructions` naming the active profile and pointing to the
//! docs resources and skills.

use std::borrow::Cow;
use std::sync::Arc;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::jsonrpc::TraceContext;
use rmcp::ErrorData as McpError;
use rmcp::ServerHandler;
use rmcp::model::{
    CallToolRequestParams, CallToolResponse, GetSkillRequestParams, GetSkillResult, Implementation,
    ListResourcesResult, ListSkillsResult, ListToolsResult, PaginatedRequestParams,
    ProgressNotificationParam, ProtocolVersion, ReadResourceRequestParams, ReadResourceResponse,
    ServerCapabilities, ServerConfig, Tool,
};
use rmcp::service::{RequestContext, RoleServer};
use tokio::sync::{OnceCell, mpsc};
use tokio_util::task::TaskTracker;

use crate::engine::{EngineConnector, EngineLink, ProgressUpdate};
use crate::profiles::ResolvedProfile;
use crate::projection::{error_tool_result, success_tool_result};
use crate::resources::ResourceStore;
use crate::revisions;

/// Bounded queue for forwarding engine progress updates to MCP notifications.
const PROGRESS_BUFFER: usize = 32;

/// Static configuration for an [`McpServer`] instance.
#[derive(Clone, Debug)]
pub struct ServerOptions {
    /// The Product this server serves (`Product::Collie` or `Product::Foxhound`).
    pub product: Product,
    /// The executable's version string (reported in `serverInfo.version`).
    pub version: String,
    /// The resolved tool profile to serve.
    pub profile: ResolvedProfile,
    /// Embedded docs resources and rendered skills served over MCP (#1371).
    pub resources: ResourceStore,
}

enum SessionEngine {
    Connected(EngineLink),
    Lazy {
        connector: Arc<dyn EngineConnector>,
        link: OnceCell<EngineLink>,
    },
}

struct SessionState {
    engine: SessionEngine,
    tasks: TaskTracker,
}

impl Drop for SessionState {
    fn drop(&mut self) {
        match &self.engine {
            SessionEngine::Connected(link) => link.close_now(),
            SessionEngine::Lazy { link, .. } => {
                if let Some(connected) = link.get() {
                    connected.close_now();
                }
            }
        }
        self.tasks.close();
    }
}

/// The Goodfetch MCP server handler, backed by one [`EngineLink`] per session.
#[derive(Clone)]
pub struct McpServer {
    options: Arc<ServerOptions>,
    session: Arc<SessionState>,
}

impl McpServer {
    /// Creates an MCP server bound to an open [`EngineLink`].
    #[must_use]
    pub fn new(options: ServerOptions, engine: EngineLink) -> Self {
        Self {
            options: Arc::new(options),
            session: Arc::new(SessionState {
                engine: SessionEngine::Connected(engine),
                tasks: TaskTracker::new(),
            }),
        }
    }

    /// Creates a fresh MCP server session that opens its own dedicated
    /// [`EngineLink`] lazily via `connector` on its first tool call, keeping
    /// per-session Limits and cancellation isolated across concurrent
    /// Streamable HTTP sessions (ADRs 0041, 0070).
    #[must_use]
    pub fn with_connector(options: ServerOptions, connector: Arc<dyn EngineConnector>) -> Self {
        Self {
            options: Arc::new(options),
            session: Arc::new(SessionState {
                engine: SessionEngine::Lazy {
                    connector,
                    link: OnceCell::new(),
                },
                tasks: TaskTracker::new(),
            }),
        }
    }

    /// The two-sentence `instructions` string returned by `initialize` (#1359).
    #[must_use]
    pub fn instructions(&self) -> String {
        format!(
            "Goodfetch {} MCP server running the `{}` profile. Read `goodfetch://docs/llms.txt` for product documentation and use `skills/list` to inspect available skills.",
            self.options.product.name(),
            self.options.profile.name,
        )
    }

    /// Closes the underlying engine connection and waits for active forwarders.
    ///
    /// # Cancel safety
    ///
    /// Safe to cancel; closing is idempotent.
    pub async fn shutdown(&self) {
        match &self.session.engine {
            SessionEngine::Connected(link) => link.close().await,
            SessionEngine::Lazy { link, .. } => {
                if let Some(connected) = link.get() {
                    connected.close().await;
                }
            }
        }
        self.session.tasks.close();
        self.session.tasks.wait().await;
    }

    async fn engine_link(&self) -> Result<&EngineLink, McpError> {
        match &self.session.engine {
            SessionEngine::Connected(link) => Ok(link),
            SessionEngine::Lazy { connector, link } => link
                .get_or_try_init(|| connector.connect())
                .await
                .map_err(|err| McpError::internal_error(err.to_string(), None)),
        }
    }
}

impl ServerHandler for McpServer {
    fn get_info(&self) -> ServerConfig {
        let capabilities = ServerCapabilities::builder()
            .enable_tools()
            .enable_resources()
            .enable_skills()
            .build();
        let mut info = ServerConfig::new(capabilities)
            .with_protocol_version(revisions::DEFAULT_HANDSHAKE_VERSION)
            .with_server_info(Implementation::new(
                self.options.product.executable_name(),
                self.options.version.clone(),
            ));
        info.instructions = Some(self.instructions());
        info
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Borrowed(revisions::SUPPORTED)
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListResourcesResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListResourcesResult::with_all_items(
            self.options.resources.list_resources(),
        )))
    }

    fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ReadResourceResponse, McpError>> + Send + '_ {
        let outcome = match self.options.resources.read_resource(&request.uri) {
            Some(result) => Ok(ReadResourceResponse::from(result)),
            None => Err(McpError::resource_not_found(
                format!("unknown resource `{}`", request.uri),
                None,
            )),
        };
        std::future::ready(outcome)
    }

    fn list_skills(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<ListSkillsResult, McpError>> + Send + '_ {
        std::future::ready(Ok(ListSkillsResult::with_all_items(
            self.options.resources.list_skills(),
        )))
    }

    fn get_skill(
        &self,
        request: GetSkillRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<GetSkillResult, McpError>> + Send + '_ {
        let outcome = match self.options.resources.get_skill(&request.uri) {
            Some(skill) => Ok(GetSkillResult::new(skill)),
            None => Err(McpError::invalid_params(
                format!("unknown skill `{}`", request.uri),
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
        let tools = self.options.profile.list_tools();
        std::future::ready(Ok(ListToolsResult::with_all_items(tools)))
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.options
            .profile
            .get(name)
            .map(|projected| projected.tool.clone())
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let Some(projected) = self.options.profile.get(&request.name) else {
            return Err(McpError::invalid_params(
                format!("unknown tool `{}`", request.name),
                None,
            ));
        };

        // Propagate W3C traceparent and tracestate; never pass baggage (ADR 0132).
        let trace = TraceContext {
            traceparent: context.meta.get_traceparent().map(str::to_owned),
            tracestate: context.meta.get_tracestate().map(str::to_owned),
        };

        let (progress_tx, forwarder) = match context.meta.get_progress_token() {
            Some(token) => {
                let (tx, rx) = mpsc::channel::<ProgressUpdate>(PROGRESS_BUFFER);
                let peer = context.peer.clone();
                let handle = self.session.tasks.spawn(async move {
                    let mut rx = rx;
                    while let Some(update) = rx.recv().await {
                        let mut param =
                            ProgressNotificationParam::new(token.clone(), update.progress)
                                .with_message(update.message);
                        if let Some(total) = update.total {
                            param = param.with_total(total);
                        }
                        if let Err(error) = peer.notify_progress(param).await {
                            tracing::debug!(%error, "MCP progress notification failed");
                            break;
                        }
                    }
                });
                (Some(tx), Some(handle))
            }
            None => (None, None),
        };

        let engine = self.engine_link().await?;
        let arguments = request.arguments.unwrap_or_default();
        let outcome = engine
            .call(
                projected.operation,
                arguments,
                trace,
                progress_tx,
                context.ct.clone(),
            )
            .await;

        if let Some(handle) = forwarder {
            let _ = handle.await;
        }

        let tool_result = match outcome {
            Ok(value) => success_tool_result(value),
            Err(error) => error_tool_result(&error, Some(projected.operation)),
        };
        Ok(CallToolResponse::from(tool_result))
    }
}
