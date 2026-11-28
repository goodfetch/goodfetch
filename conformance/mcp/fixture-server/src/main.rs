//! Test-only MCP conformance fixture server executable (`publish = false`,
//! never shipped, ADR 0070, ticket #414).
//!
//! Runs `FixtureServer` over stdio (`--stdio`, default) or Streamable HTTP
//! (`--http [ADDRESS]`) through `goodfetch_mcp::transports::http::BoundHttpServer`,
//! exercising the exact same HTTP listener, bearer auth, `Host`/`Origin`
//! enforcement, and `rmcp` transport as `goodfetch-collie mcp`.

mod fixture;

use std::io::Write as _;
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::process::ExitCode;

use clap::Parser;
use goodfetch_mcp::transports::{
    BoundHttpServer, DEFAULT_HTTP_BIND, HttpAuth, HttpServerConfig, HttpTls, new_launch_token,
};
use rmcp::ServiceExt as _;
use secrecy::SecretString;
use serde::Serialize;
use tokio_util::sync::CancellationToken;
use tokio_util::task::TaskTracker;
use tracing_subscriber::EnvFilter;

use crate::fixture::FixtureServer;

#[derive(Debug, Parser)]
#[command(
    name = "goodfetch-mcp-fixture-server",
    about = "Test-only MCP conformance fixture server (ADR 0070)"
)]
struct Cli {
    /// Serve over stdio (default when `--http` is not given).
    #[arg(long)]
    stdio: bool,
    /// Serve over Streamable HTTP on `ADDRESS` (`127.0.0.1:0` by default).
    #[arg(
        long,
        value_name = "ADDRESS",
        num_args = 0..=1,
        default_missing_value = DEFAULT_HTTP_BIND
    )]
    http: Option<String>,
}

#[derive(Serialize)]
struct HttpStartupRecord {
    address: String,
    token: String,
}

fn parse_bind(raw: &str) -> Result<SocketAddr, String> {
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
    Err(format!("invalid --http bind address `{raw}`"))
}

fn main() -> ExitCode {
    #[expect(
        clippy::disallowed_methods,
        reason = "tracing subscriber writes logs to stderr so stdio JSON-RPC stdout stays pure (STYLE.md)"
    )]
    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let runtime = match goodfetch_runtime::multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(error) => {
            tracing::error!(%error, "failed to build tokio runtime");
            return ExitCode::FAILURE;
        }
    };

    match runtime.block_on(run(cli)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            tracing::error!(%error, "fixture server failed");
            ExitCode::FAILURE
        }
    }
}

async fn run(cli: Cli) -> Result<(), String> {
    if let Some(raw_bind) = cli.http {
        let bind = parse_bind(&raw_bind)?;
        let token = new_launch_token().map_err(|err| err.to_string())?;
        let cancellation = CancellationToken::new();
        let config = HttpServerConfig {
            bind,
            auth: HttpAuth::LaunchToken(SecretString::from(token.clone())),
            tls: HttpTls::Plain,
            refusals: None,
            cancellation: cancellation.clone(),
        };
        let bound = BoundHttpServer::bind_service(config, || Ok(FixtureServer::new()))
            .await
            .map_err(|err| err.to_string())?;

        let record = HttpStartupRecord {
            address: bound.url().to_owned(),
            token,
        };
        let mut line = serde_json::to_string(&record).map_err(|err| err.to_string())?;
        line.push('\n');
        #[expect(
            clippy::disallowed_methods,
            reason = "test-only fixture server prints its single HTTP startup JSON record to stdout (ADR 0070)"
        )]
        let mut stdout = std::io::stdout().lock();
        stdout
            .write_all(line.as_bytes())
            .and_then(|()| stdout.flush())
            .map_err(|err| err.to_string())?;
        drop(stdout);

        let signals = TaskTracker::new();
        let cancel_on_sig = cancellation.clone();
        signals.spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            cancel_on_sig.cancel();
        });
        signals.close();

        bound.serve().await.map_err(|err| err.to_string())
    } else {
        let service = FixtureServer::new()
            .serve(rmcp::transport::stdio())
            .await
            .map_err(|err| err.to_string())?;
        let _ = service.waiting().await.map_err(|err| err.to_string())?;
        Ok(())
    }
}
