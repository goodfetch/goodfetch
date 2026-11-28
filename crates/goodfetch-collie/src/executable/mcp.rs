//! `goodfetch-collie mcp` (and `goodfetch-foxhound mcp`): serves the Model
//! Context Protocol over stdio by default, or over Streamable HTTP with
//! `--http [ADDRESS]` (ADRs 0011, 0017, 0024, 0041, 0067, 0070, 0090;
//! spec #608). Connects to a running or configured Daemon when available, or
//! runs the engine in-process (`--attached` and `--daemon` force either).
//! Over stdio, stdout carries only JSON-RPC lines; over `--http`, stdout
//! carries only the single `{ address, token }` startup record printed by the
//! CLI's writer; logs always go to stderr through `tracing`.

use std::future::Future;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::pin::Pin;
use std::process::ExitCode;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::audit::AuditLog;
use goodfetch_engine::daemon::{DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::endpoint::Refusals;
use goodfetch_engine::ops::Ops;
use goodfetch_engine::settings::get_or_default;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::transport::{self, InMemoryDialer, Network};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_mcp::engine::{EngineConnector, EngineLink, EngineLinkConfig, EngineLinkError};
use goodfetch_mcp::profiles::{ProfileName, ResolvedProfile};
use goodfetch_mcp::resources::{EmbeddedDoc, EmbeddedSkillFile, ResourceStore};
use goodfetch_mcp::server::{McpServer, ServerOptions};
use goodfetch_mcp::transports::{
    self, BoundHttpServer, DEFAULT_HTTP_BIND, HttpAuth, HttpRefusalRecorder, HttpServerConfig,
};
use goodfetch_protocol::audit::{Listener, RefusalReason};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::{DaemonEndpoint, DaemonToken, TLS_CERT_FILE, TLS_KEY_FILE};
use goodfetch_protocol::mcp::PROFILE;
use secrecy::SecretString;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::daemon::Machine;
use super::engine_choice::{Choice, EngineFlags};
use super::output;
use super::skills::Skills;
use crate::runtime::{self, Threads};

/// Product documentation pages embedded into the executable and served as
/// `goodfetch://docs/<page>` resources (ADRs 0070, 0097; issue 1371).
const EMBEDDED_DOC_PAGES: &[(&str, &str)] = &[
    ("cache.md", include_str!("../../../../docs/cache.md")),
    ("install.md", include_str!("../../../../docs/install.md")),
    (
        "operations.md",
        include_str!("../../../../docs/operations.md"),
    ),
    ("recipes.md", include_str!("../../../../docs/recipes.md")),
    ("runs.md", include_str!("../../../../docs/runs.md")),
    (
        "start-a-project.md",
        include_str!("../../../../docs/start-a-project.md"),
    ),
    (
        "throughput.md",
        include_str!("../../../../docs/throughput.md"),
    ),
];

fn embedded_docs(product: Product) -> Vec<EmbeddedDoc> {
    let executable = product.executable_name();
    let product_name = product.name();
    EMBEDDED_DOC_PAGES
        .iter()
        .map(|&(page, raw)| {
            let markdown = raw
                .replace("{{executable}}", executable)
                .replace("{{product}}", product_name);
            let title = markdown
                .lines()
                .find_map(|line| line.strip_prefix("# "))
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or(page)
                .to_owned();
            EmbeddedDoc {
                page: page.to_owned(),
                title,
                markdown,
            }
        })
        .collect()
}

/// Arguments for the `mcp` command.
#[derive(Clone, Debug, clap::Args)]
pub(super) struct Arguments {
    /// Tool profile to serve (`browser`, `agent`, `run`, `code` or `full`).
    /// Overrides `mcp.profile` (`browser` by default).
    #[arg(long, value_name = "NAME", value_parser = ["browser", "agent", "run", "code", "full"])]
    pub(super) profile: Option<String>,
    /// Serve MCP over Streamable HTTP instead of stdio, binding `ADDRESS`
    /// (`127.0.0.1:0` by default). Prints one `{ address, token }` JSON record
    /// on stdout before serving.
    #[arg(
        long,
        value_name = "ADDRESS",
        num_args = 0..=1,
        default_missing_value = DEFAULT_HTTP_BIND
    )]
    pub(super) http: Option<String>,
    #[command(flatten)]
    pub(super) engine: EngineFlags,
}

/// Startup record printed once on stdout by `mcp --http` before serving
/// (ADRs 0011, 0017, 0067; issue 1365).
#[derive(Serialize)]
struct HttpStartupRecord {
    address: String,
    token: String,
}

/// Resolves the MCP profile from `--profile`, `mcp.profile`, or the `browser` default.
fn resolve_profile(
    flag: Option<&str>,
    settings: Option<&goodfetch_engine::settings::Resolved>,
) -> ProfileName {
    flag.and_then(ProfileName::parse).unwrap_or_else(|| {
        let configured: &str = get_or_default(settings, &PROFILE);
        ProfileName::parse(configured).unwrap_or_default()
    })
}

fn build_resources(product: Product, version: &str, skills: Skills) -> ResourceStore {
    let skill_files = skills
        .files()
        .iter()
        .map(|file| EmbeddedSkillFile {
            path: file.path.to_owned(),
            contents: file.contents.to_vec(),
        })
        .collect();
    ResourceStore::from_embedded(product, version, embedded_docs(product), skill_files)
}

fn build_server_options(
    arguments: &Arguments,
    engine: &Engine,
    settings: Option<&goodfetch_engine::settings::Resolved>,
    skills: Skills,
) -> ServerOptions {
    let product = engine.identity().product;
    let version = engine.identity().product_version.to_string();
    let profile = ResolvedProfile::for_product(
        product,
        resolve_profile(arguments.profile.as_deref(), settings),
    );
    let resources = build_resources(product, &version, skills);
    ServerOptions {
        product,
        version,
        profile,
        resources,
    }
}

/// Parses an `--http` bind address (`127.0.0.1:0`, `[::1]:8080`, or `localhost:<port>`).
fn parse_http_bind(raw: &str) -> Result<SocketAddr, String> {
    if let Ok(addr) = raw.parse::<SocketAddr>() {
        return Ok(addr);
    }
    if let Some(port_str) = raw
        .strip_prefix("localhost:")
        .or_else(|| raw.strip_prefix("LOCALHOST:"))
        && let Ok(port) = port_str.parse::<u16>()
    {
        return Ok(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), port));
    }
    Err(format!(
        "setting_invalid: `--http` address `{raw}` is not a valid `host:port` socket address"
    ))
}

struct DaemonConnector {
    endpoint: DaemonEndpoint,
    config: EngineLinkConfig,
}

impl EngineConnector for DaemonConnector {
    fn connect(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<EngineLink, EngineLinkError>> + Send + '_>> {
        Box::pin(async move {
            let connection = transport::connect_daemon(&self.endpoint)
                .await
                .map_err(|err| {
                    EngineLinkError::Transport(format!(
                        "the Daemon at {} does not answer: {err}",
                        self.endpoint
                    ))
                })?;
            let (read, write) = connection.split();
            let (link, _hello) =
                EngineLink::connect_split(read, write, self.config.clone()).await?;
            Ok(link)
        })
    }
}

struct InMemoryConnector {
    dialer: InMemoryDialer,
    config: EngineLinkConfig,
}

impl EngineConnector for InMemoryConnector {
    fn connect(
        &self,
    ) -> Pin<Box<dyn Future<Output = Result<EngineLink, EngineLinkError>> + Send + '_>> {
        Box::pin(async move {
            let connection = self.dialer.connect().await.map_err(|err| {
                EngineLinkError::Transport(format!("in-process engine took no connection: {err}"))
            })?;
            let (read, write) = connection.split();
            let (link, _hello) =
                EngineLink::connect_split(read, write, self.config.clone()).await?;
            Ok(link)
        })
    }
}

struct AuditRefusalRecorder {
    refusals: Refusals,
}

impl HttpRefusalRecorder for AuditRefusalRecorder {
    fn record(&self, reason: RefusalReason) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        Box::pin(async move {
            self.refusals.refused(Listener::McpHttp, reason).await;
        })
    }
}

struct InProcessSuccession;

impl Succession for InProcessSuccession {
    fn is_pick(&self) -> bool {
        true
    }

    fn begin_handover(&self) -> Box<dyn Send> {
        Box::new(())
    }
}

/// Runs the MCP server over stdio (default) or Streamable HTTP (`--http`).
pub(super) fn run(
    arguments: &Arguments,
    engine: &Engine,
    dependencies: Dependencies,
    machine: &Machine<'_>,
    skills: Skills,
) -> ExitCode {
    let settings = machine.settings().ok();
    let choice = arguments.engine.choice();
    let product = engine.identity().product;
    let product_version = engine.identity().product_version.clone();
    let server_options = build_server_options(arguments, engine, settings, skills);

    let daemon = match choice {
        Choice::Attached => None,
        Choice::Daemon => match machine.daemon_for_mcp(true) {
            Ok(Some(found)) => Some(found),
            Ok(None) => {
                tracing::error!("no Daemon could be started or reached");
                return ExitCode::FAILURE;
            }
            Err(failure) => return failure.report(),
        },
        Choice::Auto => machine.daemon_for_mcp(false).ok().flatten(),
    };

    if let Some(raw_bind) = arguments.http.as_deref() {
        return run_http(
            raw_bind,
            choice,
            daemon,
            server_options,
            engine,
            dependencies,
            settings,
            product,
            product_version,
        );
    }

    run_stdio(
        choice,
        daemon,
        server_options,
        engine,
        dependencies,
        settings,
        product,
        product_version,
    )
}

#[expect(
    clippy::too_many_arguments,
    reason = "runs the stdio MCP transport with already-resolved CLI, engine, and Daemon parameters"
)]
fn run_stdio(
    choice: Choice,
    daemon: Option<(DaemonEndpoint, DaemonToken)>,
    server_options: ServerOptions,
    engine: &Engine,
    dependencies: Dependencies,
    settings: Option<&goodfetch_engine::settings::Resolved>,
    product: Product,
    product_version: ProductVersion,
) -> ExitCode {
    let rt = runtime::multi_thread(Threads::of(settings));

    let served = rt.block_on(async {
        if let Some((endpoint, token)) = daemon {
            let config = EngineLinkConfig {
                product,
                product_version: product_version.clone(),
                token: Some(token.expose().to_owned()),
                live_view_base_url: None,
            };
            match transport::connect_daemon(&endpoint).await {
                Ok(connection) => {
                    let (read, write) = connection.split();
                    match EngineLink::connect_split(read, write, config).await {
                        Ok((link, _hello)) => {
                            let server = McpServer::new(server_options, link);
                            return transports::serve_stdio(server)
                                .await
                                .map_err(|err| err.to_string());
                        }
                        Err(error) if choice == Choice::Daemon => {
                            return Err(format!("the Daemon refused MCP hello: {error}"));
                        }
                        Err(error) => {
                            tracing::warn!(%error, "Daemon refused MCP hello; running engine in-process");
                        }
                    }
                }
                Err(error) if choice == Choice::Daemon => {
                    return Err(format!("the Daemon at {endpoint} does not answer: {error}"));
                }
                Err(error) => {
                    tracing::warn!(%error, %endpoint, "Daemon unreachable; running engine in-process");
                }
            }
        }

        let (network, dialer) = transport::in_memory();
        let config = EngineLinkConfig {
            product,
            product_version,
            token: None,
            live_view_base_url: None,
        };
        let serve_task = async {
            let connection = dialer
                .connect()
                .await
                .map_err(|err| format!("in-process engine took no connection: {err}"))?;
            let (read, write) = connection.split();
            let (link, _hello) = EngineLink::connect_split(read, write, config)
                .await
                .map_err(|err| format!("in-process engine refused MCP hello: {err}"))?;
            let server = McpServer::new(server_options, link);
            transports::serve_stdio(server)
                .await
                .map_err(|err| err.to_string())
        };
        let (engine_res, mcp_res) = tokio::join!(
            engine.run_attached(Network::InMemory(network), dependencies),
            serve_task,
        );
        if let Err(error) = engine_res {
            tracing::debug!(%error, "in-process engine stopped");
        }
        mcp_res
    });

    // Tokio's stdin reads on a blocking thread that only new input wakes.
    rt.shutdown_background();
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the MCP server failed");
            ExitCode::FAILURE
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "runs the Streamable HTTP MCP transport with already-resolved CLI, engine, and Daemon parameters"
)]
fn run_http(
    raw_bind: &str,
    choice: Choice,
    daemon: Option<(DaemonEndpoint, DaemonToken)>,
    server_options: ServerOptions,
    engine: &Engine,
    dependencies: Dependencies,
    settings: Option<&goodfetch_engine::settings::Resolved>,
    product: Product,
    product_version: ProductVersion,
) -> ExitCode {
    let bind = match parse_http_bind(raw_bind) {
        Ok(addr) => addr,
        Err(message) => {
            tracing::error!("{message}");
            return ExitCode::FAILURE;
        }
    };
    if !bind.ip().is_loopback() {
        tracing::error!(
            code = "setting_invalid",
            address = %bind,
            "non-loopback `--http` binding requires TLS (`{}` and `{}`) and OAuth 2.1 configuration",
            TLS_CERT_FILE.name,
            TLS_KEY_FILE.name,
        );
        return ExitCode::FAILURE;
    }

    let launch_token = match transports::new_launch_token() {
        Ok(token) => token,
        Err(error) => {
            tracing::error!(%error, "could not generate per-launch MCP HTTP token");
            return ExitCode::FAILURE;
        }
    };

    let rt = runtime::multi_thread(Threads::of(settings));
    let served = rt.block_on(async {
        let audit_owner = TaskOwner::root(dependencies.blocking);
        let audit = AuditLog::of(
            &dependencies.home,
            dependencies.run_state.vfs.clone(),
            Arc::new(audit_owner.child()),
        );
        let refusals = Refusals::new(audit, dependencies.clock, Arc::new(audit_owner.child()));
        let refusal_recorder: Arc<dyn HttpRefusalRecorder> =
            Arc::new(AuditRefusalRecorder { refusals });

        let shutdown = CancellationToken::new();
        let signal_ct = shutdown.clone();
        audit_owner.spawn(async move {
            wait_for_shutdown_signal().await;
            signal_ct.cancel();
        });

        let use_daemon = match daemon {
            Some((endpoint, token)) => {
                // Probe reachability before committing to the Daemon backend in Auto mode.
                match transport::connect_daemon(&endpoint).await {
                    Ok(connection) => {
                        drop(connection);
                        Some((endpoint, token))
                    }
                    Err(error) if choice == Choice::Daemon => {
                        return Err(format!("the Daemon at {endpoint} does not answer: {error}"));
                    }
                    Err(error) => {
                        tracing::warn!(%error, %endpoint, "Daemon unreachable; running engine in-process");
                        None
                    }
                }
            }
            None => None,
        };

        let result = if let Some((endpoint, token)) = use_daemon {
            serve_http_on_daemon(
                bind,
                launch_token,
                endpoint,
                token,
                server_options,
                product,
                product_version,
                refusal_recorder,
                shutdown.clone(),
            )
            .await
        } else {
            serve_http_in_process(
                bind,
                launch_token,
                server_options,
                engine,
                dependencies,
                product,
                product_version,
                refusal_recorder,
                shutdown.clone(),
            )
            .await
        };

        shutdown.cancel();
        audit_owner.stop().await;
        result
    });

    rt.shutdown_background();
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "the MCP HTTP server failed");
            ExitCode::FAILURE
        }
    }
}

#[expect(
    clippy::too_many_arguments,
    reason = "binds and serves the Streamable HTTP MCP server backed by an existing Daemon"
)]
async fn serve_http_on_daemon(
    bind: SocketAddr,
    launch_token: String,
    endpoint: DaemonEndpoint,
    daemon_token: DaemonToken,
    server_options: ServerOptions,
    product: Product,
    product_version: ProductVersion,
    refusal_recorder: Arc<dyn HttpRefusalRecorder>,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let connector: Arc<dyn EngineConnector> = Arc::new(DaemonConnector {
        endpoint,
        config: EngineLinkConfig {
            product,
            product_version,
            token: Some(daemon_token.expose().to_owned()),
            live_view_base_url: None,
        },
    });
    let bound = BoundHttpServer::bind(
        HttpServerConfig {
            bind,
            auth: HttpAuth::LaunchToken(SecretString::from(launch_token.clone())),
            refusals: Some(refusal_recorder),
            cancellation: shutdown,
        },
        server_options,
        connector,
    )
    .await
    .map_err(|err| err.to_string())?;
    output::json(&HttpStartupRecord {
        address: bound.url().to_owned(),
        token: launch_token,
    });
    bound.serve().await.map_err(|err| err.to_string())
}

#[expect(
    clippy::too_many_arguments,
    reason = "binds and serves the Streamable HTTP MCP server backed by an in-process multi-session engine"
)]
async fn serve_http_in_process(
    bind: SocketAddr,
    launch_token: String,
    server_options: ServerOptions,
    engine: &Engine,
    dependencies: Dependencies,
    product: Product,
    product_version: ProductVersion,
    refusal_recorder: Arc<dyn HttpRefusalRecorder>,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let internal_token = transports::new_launch_token()
        .map_err(|err| format!("could not generate internal engine token: {err}"))?;
    let (network, dialer) = transport::in_memory();
    let (listener, endpoint) = Network::InMemory(network)
        .listen_at(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .map_err(|err| format!("in-process engine listener failed: {err}"))?;
    let connector: Arc<dyn EngineConnector> = Arc::new(InMemoryConnector {
        dialer,
        config: EngineLinkConfig {
            product,
            product_version,
            token: Some(internal_token.clone()),
            live_view_base_url: None,
        },
    });
    let bound = BoundHttpServer::bind(
        HttpServerConfig {
            bind,
            auth: HttpAuth::LaunchToken(SecretString::from(launch_token.clone())),
            refusals: Some(refusal_recorder),
            cancellation: shutdown.clone(),
        },
        server_options,
        connector,
    )
    .await
    .map_err(|err| err.to_string())?;

    let daemon_options = DaemonOptions {
        token: internal_token,
        endpoint,
        pid: std::process::id(),
        idle_timeout: Duration::from_secs(3600),
        keep_alive: true,
        holds: Holds::new(),
        succession: Arc::new(InProcessSuccession),
        drain: Drain {
            asked: Some(shutdown),
            grace: Duration::from_secs(2),
        },
        install: None,
        ops: Ops {
            listener: None,
            browsers: None,
        },
        websocket: None,
        accounts: None,
        live_view: None,
        http_listener: None,
    };

    output::json(&HttpStartupRecord {
        address: bound.url().to_owned(),
        token: launch_token,
    });

    let (engine_end, http_res) = tokio::join!(
        engine.run_daemon(listener, dependencies, daemon_options),
        bound.serve(),
    );
    tracing::debug!(?engine_end, "in-process MCP engine stopped");
    http_res.map_err(|err| err.to_string())
}

async fn wait_for_shutdown_signal() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).ok();
        let ctrl_c = tokio::signal::ctrl_c();
        match sigterm.as_mut() {
            Some(term) => {
                tokio::select! {
                    _ = ctrl_c => {}
                    _ = term.recv() => {}
                }
            }
            None => {
                let _ = ctrl_c.await;
            }
        }
    }
    #[cfg(not(unix))]
    {
        let _ = tokio::signal::ctrl_c().await;
    }
}
