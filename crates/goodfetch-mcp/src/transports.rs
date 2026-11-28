//! Transports for serving [`McpServer`]: stdio (default), generic
//! `AsyncRead + AsyncWrite` (used by in-memory tests), and Streamable HTTP
//! (`http`, ADRs 0017, 0067, 0070; ticket #1365).

pub mod http;

use rmcp::ServiceExt as _;
use tokio::io::{AsyncRead, AsyncWrite};

use crate::server::McpServer;

pub use http::{
    BoundHttpServer, DEFAULT_HTTP_BIND, HttpAuth, HttpAuthorities, HttpOAuthFetcher,
    HttpRefusalRecorder, HttpServerConfig, HttpTls, HttpTlsAcceptor, MCP_HTTP_PATH, REFUSAL_TARGET,
    SharedRefusalRecorder, SharedTlsAcceptor, StaticTlsAcceptor, bearer_token, new_launch_token,
    query_carries_token, same_token,
};

/// Errors serving the MCP server over a transport.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum TransportError {
    /// The MCP service failed to initialize or run over the transport.
    #[error("MCP transport failed: {0}")]
    Serve(String),
}

/// Serves `server` over the process's standard input and standard output
/// (newline-delimited JSON-RPC; stdout carries nothing else).
///
/// # Errors
///
/// Returns [`TransportError::Serve`] if the transport fails to initialize or
/// exits with an error.
///
/// # Cancel safety
///
/// Dropping this future cancels the running MCP service.
pub async fn serve_stdio(server: McpServer) -> Result<(), TransportError> {
    let service = server
        .clone()
        .serve(rmcp::transport::stdio())
        .await
        .map_err(|err| TransportError::Serve(err.to_string()))?;
    let _ = service
        .waiting()
        .await
        .map_err(|err| TransportError::Serve(err.to_string()))?;
    server.shutdown().await;
    Ok(())
}

/// Serves `server` over an arbitrary `(read, write)` async byte stream pair,
/// such as an in-memory duplex channel in tests.
///
/// # Errors
///
/// Returns [`TransportError::Serve`] if the transport fails to initialize or
/// exits with an error.
///
/// # Cancel safety
///
/// Dropping this future cancels the running MCP service.
pub async fn serve_io<R, W>(server: McpServer, read: R, write: W) -> Result<(), TransportError>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    let service = server
        .clone()
        .serve((read, write))
        .await
        .map_err(|err| TransportError::Serve(err.to_string()))?;
    let _ = service
        .waiting()
        .await
        .map_err(|err| TransportError::Serve(err.to_string()))?;
    server.shutdown().await;
    Ok(())
}
