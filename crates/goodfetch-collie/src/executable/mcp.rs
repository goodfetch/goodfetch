//! `goodfetch-collie mcp` (and `goodfetch-foxhound mcp`): serves the Model
//! Context Protocol over stdio by default (ADRs 0011, 0017, 0024, 0070, 0090;
//! spec #608). Connects to a running or configured Daemon when available, or
//! runs the engine in-process (`--attached` and `--daemon` force either).
//! Stdout carries only JSON-RPC lines; logs go to stderr through `tracing`.

use std::process::ExitCode;

use goodfetch_engine::settings::get_or_default;
use goodfetch_engine::transport::{self, Network};
use goodfetch_engine::{Dependencies, Engine};
use goodfetch_mcp::engine::{EngineLink, EngineLinkConfig};
use goodfetch_mcp::profiles::{ProfileName, ResolvedProfile};
use goodfetch_mcp::resources::{EmbeddedDoc, EmbeddedSkillFile, ResourceStore};
use goodfetch_mcp::server::{McpServer, ServerOptions};
use goodfetch_mcp::transports;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::mcp::PROFILE;

use super::daemon::Machine;
use super::engine_choice::{Choice, EngineFlags};
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
    #[command(flatten)]
    pub(super) engine: EngineFlags,
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

/// Runs the MCP server over stdio until the client closes stdin or fails.
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
