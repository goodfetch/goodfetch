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

use goodfetch_engine::audit::{AuditLog, Origin};
use goodfetch_engine::daemon::{DaemonOptions, Drain, Holds, Succession};
use goodfetch_engine::endpoint::{Refusals, Termination};
use goodfetch_engine::ops::Ops;
use goodfetch_engine::settings::{Resolved, SettingsError, get_or_default};
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::tls::ServingCertificate;
use goodfetch_engine::transport::{self, InMemoryDialer, Network};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_mcp::engine::{
    EngineConnector, EngineLink, EngineLinkConfig, EngineLinkError, SharedEngineConnector,
};
use goodfetch_mcp::oauth::{
    EpochSecondsClock, OAuthConfig, OAuthResourceServer, SharedOAuthMetadataFetcher,
};
use goodfetch_mcp::profiles::{ProfileName, ResolvedProfile};
use goodfetch_mcp::resources::{EmbeddedDoc, EmbeddedSkillFile, ResourceStore};
use goodfetch_mcp::server::{McpServer, ServerOptions};
use goodfetch_mcp::transports::{
    self, BoundHttpServer, DEFAULT_HTTP_BIND, HttpAuth, HttpOAuthFetcher, HttpRefusalRecorder,
    HttpServerConfig, HttpTls, HttpTlsAcceptor, SharedRefusalRecorder, SharedTlsAcceptor,
};
use goodfetch_protocol::audit::{Actor, Listener, RefusalReason, TLS_TERMINATED_UPSTREAM};
use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::daemon::{
    DaemonEndpoint, DaemonToken, TLS_CERT_FILE, TLS_KEY_FILE, TLS_UPSTREAM,
};
use goodfetch_protocol::mcp::{OAUTH_AUDIENCE, OAUTH_ISSUER, PROFILE};
use goodfetch_protocol::settings::SettingsLayer;
use secrecy::SecretString;
use serde::Serialize;
use tokio_util::sync::CancellationToken;

use super::daemon::{Machine, tls as daemon_tls};
use super::engine_choice::{Choice, EngineFlags};
use super::output;
use super::skills::Skills;
use crate::runtime::{self, Threads};

/// Product documentation pages embedded into the executable and served as
/// `goodfetch://docs/<page>` resources (ADRs 0070, 0097; issue 1371).
/// Filtered at runtime by YAML `products:` frontmatter and `{/* products: */}`
/// blocks so each Product serves every page that applies to it and none that
/// belongs exclusively to the other Product.
const EMBEDDED_DOC_PAGES: &[(&str, &str)] = &[
    ("cache.md", include_str!("../../../../docs/cache.md")),
    ("index.md", include_str!("../../../../docs/index.md")),
    ("install.md", include_str!("../../../../docs/install.md")),
    (
        "mcp-frameworks.md",
        include_str!("../../../../docs/mcp-frameworks.md"),
    ),
    (
        "operations.md",
        include_str!("../../../../docs/operations.md"),
    ),
    ("recipes.md", include_str!("../../../../docs/recipes.md")),
    (
        "release-notes-template.md",
        include_str!("../../../../docs/release-notes-template.md"),
    ),
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

/// Renders a source doc page for `product`: returns `None` when the YAML
/// frontmatter `products:` list excludes `product`, and strips `{/* products: ... */}`
/// blocks meant for another Product (matching `xtask/src/docs_samples/render.rs`).
fn render_doc_for_product(raw: &str, product: Product) -> Option<String> {
    let slug = match product {
        Product::Collie => "collie",
        Product::Foxhound => "foxhound",
    };
    let mut lines = raw.lines();
    let mut out = Vec::new();
    if lines.next() == Some("---") {
        out.push("---");
        let mut frontmatter = Vec::new();
        for line in lines.by_ref() {
            if line == "---" {
                break;
            }
            if let Some(rest) = line.strip_prefix("products:") {
                let trimmed = rest.trim().trim_start_matches('[').trim_end_matches(']');
                let listed: Vec<&str> = trimmed
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                if !listed.is_empty() && !listed.contains(&slug) {
                    return None;
                }
            }
            frontmatter.push(line);
        }
        out.extend(frontmatter);
        out.push("---");
    } else {
        lines = raw.lines();
    }

    let mut include_block = true;
    for line in lines {
        let trimmed = line.trim();
        if let Some(inner) = trimmed
            .strip_prefix("{/*")
            .and_then(|s| s.strip_suffix("*/}"))
            .map(str::trim)
        {
            if let Some(products_spec) = inner.strip_prefix("products:") {
                let listed: Vec<&str> = products_spec
                    .split(',')
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .collect();
                include_block = listed.contains(&slug);
                continue;
            }
            if inner == "end products" {
                include_block = true;
                continue;
            }
        }
        if include_block {
            out.push(line);
        }
    }

    let executable = product.executable_name();
    let product_name = product.name();
    let mut rendered = out.join("\n");
    if raw.ends_with('\n') {
        rendered.push('\n');
    }
    Some(
        rendered
            .replace("{{executable}}", executable)
            .replace("{{product}}", product_name),
    )
}

fn embedded_docs(product: Product) -> Vec<EmbeddedDoc> {
    EMBEDDED_DOC_PAGES
        .iter()
        .filter_map(|&(page, raw)| {
            let markdown = render_doc_for_product(raw, product)?;
            let title = markdown
                .lines()
                .find_map(|line| line.strip_prefix("# "))
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .unwrap_or(page)
                .to_owned();
            Some(EmbeddedDoc {
                page: page.to_owned(),
                title,
                markdown,
            })
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
/// (ADRs 0011, 0017, 0067; issues 1365 and 1367). Remote OAuth 2.1 servers
/// omit `token` because they hold no shared secret.
#[derive(Serialize)]
struct HttpStartupRecord {
    address: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    token: Option<String>,
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

struct ServingCertificateAcceptor {
    certificate: ServingCertificate,
    owner: Arc<TaskOwner>,
}

impl HttpTlsAcceptor for ServingCertificateAcceptor {
    fn acceptor(&self) -> Pin<Box<dyn Future<Output = tokio_rustls::TlsAcceptor> + Send + '_>> {
        Box::pin(async move { self.certificate.acceptor(&self.owner).await })
    }
}

fn setting_layer(settings: &Resolved, setting: &Setting) -> SettingsLayer {
    settings
        .effective(setting)
        .map_or(SettingsLayer::Default, |effective| effective.layer)
}

fn resolve_http_security(
    bind: SocketAddr,
    settings: &Resolved,
) -> Result<(Termination, Option<OAuthConfig>), SettingsError> {
    let termination = daemon_tls::open_termination(settings)?;
    let issuer = settings
        .get::<Option<&str>>(&OAUTH_ISSUER)
        .map(str::trim)
        .filter(|value| !value.is_empty());
    let audience = settings
        .get::<Option<&str>>(&OAUTH_AUDIENCE)
        .map(str::trim)
        .filter(|value| !value.is_empty());

    if !bind.ip().is_loopback() {
        if matches!(termination, Termination::Plain) {
            return Err(SettingsError::Invalid {
                key: TLS_CERT_FILE.name.to_owned(),
                layer: None,
                reason: format!(
                    "`{bind}` is not a loopback address: remote `mcp --http` requires TLS; set `{}` and `{}`, or `{}` behind an ingress that terminates TLS",
                    TLS_CERT_FILE.name, TLS_KEY_FILE.name, TLS_UPSTREAM.name
                ),
            });
        }
        let Some(issuer) = issuer else {
            return Err(SettingsError::Invalid {
                key: OAUTH_ISSUER.name.to_owned(),
                layer: Some(setting_layer(settings, &OAUTH_ISSUER)),
                reason: format!(
                    "`{bind}` is not a loopback address: remote `mcp --http` requires OAuth 2.1; set `{}` and `{}` in user configuration or an Operator policy",
                    OAUTH_ISSUER.name, OAUTH_AUDIENCE.name
                ),
            });
        };
        let Some(audience) = audience else {
            return Err(SettingsError::Invalid {
                key: OAUTH_AUDIENCE.name.to_owned(),
                layer: Some(setting_layer(settings, &OAUTH_AUDIENCE)),
                reason: format!(
                    "`{bind}` is not a loopback address: remote `mcp --http` requires OAuth 2.1; set `{}` alongside `{}` in user configuration or an Operator policy",
                    OAUTH_AUDIENCE.name, OAUTH_ISSUER.name
                ),
            });
        };
        let oauth = parse_oauth_config(settings, issuer, audience)?;
        return Ok((termination, Some(oauth)));
    }

    match (issuer, audience) {
        (None, None) => Ok((termination, None)),
        (Some(_), None) => Err(SettingsError::Invalid {
            key: OAUTH_AUDIENCE.name.to_owned(),
            layer: Some(setting_layer(settings, &OAUTH_AUDIENCE)),
            reason: format!(
                "`{}` is set without `{}`: set both keys or unset `{}`",
                OAUTH_ISSUER.name, OAUTH_AUDIENCE.name, OAUTH_ISSUER.name
            ),
        }),
        (None, Some(_)) => Err(SettingsError::Invalid {
            key: OAUTH_ISSUER.name.to_owned(),
            layer: Some(setting_layer(settings, &OAUTH_ISSUER)),
            reason: format!(
                "`{}` is set without `{}`: set both keys or unset `{}`",
                OAUTH_AUDIENCE.name, OAUTH_ISSUER.name, OAUTH_AUDIENCE.name
            ),
        }),
        (Some(issuer), Some(audience)) => {
            let oauth = parse_oauth_config(settings, issuer, audience)?;
            Ok((termination, Some(oauth)))
        }
    }
}

fn parse_oauth_config(
    settings: &Resolved,
    issuer: &str,
    audience: &str,
) -> Result<OAuthConfig, SettingsError> {
    OAuthConfig::new(issuer, audience).map_err(|err| {
        let setting = if err.key == OAUTH_ISSUER.name {
            &OAUTH_ISSUER
        } else {
            &OAUTH_AUDIENCE
        };
        SettingsError::Invalid {
            key: err.key.to_owned(),
            layer: Some(setting_layer(settings, setting)),
            reason: err.reason,
        }
    })
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
            machine,
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

fn build_http_auth(
    oauth_config: Option<OAuthConfig>,
    clock: goodfetch_engine::clock::Clock,
) -> Result<(HttpAuth, Option<String>), String> {
    if let Some(oauth_config) = oauth_config {
        let now = EpochSecondsClock::new(move || {
            clock
                .wall()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs()
        });
        let fetcher = SharedOAuthMetadataFetcher::new(HttpOAuthFetcher::new(&[]));
        let oauth = Arc::new(OAuthResourceServer::new(oauth_config, fetcher, now));
        return Ok((HttpAuth::OAuth(oauth), None));
    }
    let launch_token = transports::new_launch_token()
        .map_err(|error| format!("could not generate per-launch MCP HTTP token: {error}"))?;
    Ok((
        HttpAuth::LaunchToken(SecretString::from(launch_token.clone())),
        Some(launch_token),
    ))
}

async fn activate_http_tls(
    termination: Termination,
    bind: SocketAddr,
    audit: &AuditLog,
    clock: goodfetch_engine::clock::Clock,
    audit_owner: &TaskOwner,
) -> HttpTls {
    match termination {
        Termination::Plain => HttpTls::Plain,
        Termination::Tls(certificate) => {
            HttpTls::Direct(SharedTlsAcceptor::new(ServingCertificateAcceptor {
                certificate,
                owner: Arc::new(audit_owner.child()),
            }))
        }
        Termination::Upstream(proxies) => {
            let trusted: Vec<String> = proxies.iter().map(ToString::to_string).collect();
            let origin = Origin {
                actor: Actor::Engine,
                surface: None,
                layer: None,
                run_id: None,
            };
            let details = serde_json::json!({
                "address": bind.to_string(),
                "trusted_proxies": trusted,
            });
            if let Err(error) = audit
                .record(
                    &origin,
                    &TLS_TERMINATED_UPSTREAM,
                    details,
                    clock.timestamp(),
                )
                .await
            {
                tracing::error!(
                    %error,
                    "the Audit log did not record that MCP HTTP TLS is terminated upstream"
                );
            }
            HttpTls::Upstream(proxies)
        }
    }
}

async fn probe_daemon_for_http(
    choice: Choice,
    daemon: Option<(DaemonEndpoint, DaemonToken)>,
) -> Option<(DaemonEndpoint, DaemonToken)> {
    match daemon {
        Some((endpoint, token)) if choice == Choice::Daemon => Some((endpoint, token)),
        Some((endpoint, token)) => match transport::connect_daemon(&endpoint).await {
            Ok(connection) => {
                drop(connection);
                Some((endpoint, token))
            }
            Err(error) => {
                tracing::warn!(%error, %endpoint, "Daemon unreachable; running engine in-process");
                None
            }
        },
        None => None,
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
    machine: &Machine<'_>,
    product: Product,
    product_version: ProductVersion,
) -> ExitCode {
    let bind = match parse_http_bind(raw_bind) {
        Ok(addr) => addr,
        Err(message) => {
            tracing::error!(code = "setting_invalid", "{message}");
            return ExitCode::FAILURE;
        }
    };
    let settings = match machine.settings() {
        Ok(settings) => settings,
        Err(failure) => return failure.report(),
    };
    let (termination, oauth_config) = match resolve_http_security(bind, settings) {
        Ok(resolved) => resolved,
        Err(error) => {
            tracing::error!(code = error.code(), "{error}");
            return ExitCode::FAILURE;
        }
    };
    let (auth, startup_token) = match build_http_auth(oauth_config, dependencies.clock) {
        Ok(built) => built,
        Err(error) => {
            tracing::error!("{error}");
            return ExitCode::FAILURE;
        }
    };

    let rt = runtime::multi_thread(Threads::of(Some(settings)));
    let served = rt.block_on(async {
        let audit_owner = TaskOwner::root(dependencies.blocking);
        let audit = AuditLog::of(
            &dependencies.home,
            dependencies.run_state.vfs.clone(),
            Arc::new(audit_owner.child()),
        );
        let refusals = Refusals::new(
            audit.clone(),
            dependencies.clock,
            Arc::new(audit_owner.child()),
        );
        let refusal_recorder = SharedRefusalRecorder::new(AuditRefusalRecorder { refusals });
        let tls =
            activate_http_tls(termination, bind, &audit, dependencies.clock, &audit_owner).await;

        let shutdown = CancellationToken::new();
        let signal_ct = shutdown.clone();
        audit_owner.spawn(async move {
            wait_for_shutdown_signal(signal_ct.clone()).await;
            signal_ct.cancel();
        });

        let use_daemon = probe_daemon_for_http(choice, daemon).await;

        let result = if let Some((endpoint, token)) = use_daemon {
            serve_http_on_daemon(
                bind,
                auth,
                tls,
                startup_token,
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
                auth,
                tls,
                startup_token,
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
    auth: HttpAuth,
    tls: HttpTls,
    startup_token: Option<String>,
    endpoint: DaemonEndpoint,
    daemon_token: DaemonToken,
    server_options: ServerOptions,
    product: Product,
    product_version: ProductVersion,
    refusal_recorder: SharedRefusalRecorder,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let connector = SharedEngineConnector::new(DaemonConnector {
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
            auth,
            tls,
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
        token: startup_token,
    });
    bound.serve().await.map_err(|err| err.to_string())
}

#[expect(
    clippy::too_many_arguments,
    reason = "binds and serves the Streamable HTTP MCP server backed by an in-process multi-session engine"
)]
async fn serve_http_in_process(
    bind: SocketAddr,
    auth: HttpAuth,
    tls: HttpTls,
    startup_token: Option<String>,
    server_options: ServerOptions,
    engine: &Engine,
    dependencies: Dependencies,
    product: Product,
    product_version: ProductVersion,
    refusal_recorder: SharedRefusalRecorder,
    shutdown: CancellationToken,
) -> Result<(), String> {
    let internal_token = transports::new_launch_token()
        .map_err(|err| format!("could not generate internal engine token: {err}"))?;
    let (network, dialer) = transport::in_memory();
    let (listener, endpoint) = Network::InMemory(network)
        .listen_at(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))
        .await
        .map_err(|err| format!("in-process engine listener failed: {err}"))?;
    let connector = SharedEngineConnector::new(InMemoryConnector {
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
            auth,
            tls,
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
        token: startup_token,
    });

    let (engine_end, http_res) = tokio::join!(
        engine.run_daemon(listener, dependencies, daemon_options),
        bound.serve(),
    );
    tracing::debug!(?engine_end, "in-process MCP engine stopped");
    http_res.map_err(|err| err.to_string())
}

async fn wait_for_shutdown_signal(shutdown: CancellationToken) {
    #[cfg(unix)]
    {
        use tokio::signal::unix::{SignalKind, signal};
        let mut sigterm = signal(SignalKind::terminate()).ok();
        let ctrl_c = tokio::signal::ctrl_c();
        match sigterm.as_mut() {
            Some(term) => {
                tokio::select! {
                    () = shutdown.cancelled() => {}
                    _ = ctrl_c => {}
                    _ = term.recv() => {}
                }
            }
            None => {
                tokio::select! {
                    () = shutdown.cancelled() => {}
                    _ = ctrl_c => {}
                }
            }
        }
    }
    #[cfg(not(unix))]
    {
        tokio::select! {
            () = shutdown.cancelled() => {}
            _ = tokio::signal::ctrl_c() => {}
        }
    }
}
