//! The Client's wire: JSON-RPC requests and notifications framed one a
//! line, the Kennel's `hello`, and the Daemon's socket.

use std::io;
use std::pin::Pin;

use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{
    DaemonEndpoint, HELLO, HelloParams, PROTOCOL_VERSION_UNSUPPORTED, ProductVersion,
    ProtocolVersionUnsupportedDetails, Surface,
};
use goodfetch_protocol::daemon::DaemonRuntimeFile;
use goodfetch_protocol::foxhound::frontier::RunAddSeedsParams;
use goodfetch_protocol::jsonrpc::{ErrorObject, Notification, Request, RequestId, Version};
use serde::Serialize;
use serde_json::Value;
use tokio::io::{AsyncRead, AsyncWrite, AsyncWriteExt};

use super::{Call, Refusal};

/// The id of the Kennel's `hello`, its first request on each Connection.
pub(super) const HELLO_ID: i64 = 1;

pub(super) type Reader = Pin<Box<dyn AsyncRead + Send>>;
pub(super) type Writer = Pin<Box<dyn AsyncWrite + Send>>;

impl Call {
    /// The request of id `id` that makes it.
    pub(super) fn request(&self, id: i64) -> Request {
        let method = self.method();
        match self {
            Self::RaiseLimit(params) => request(id, method, params),
            Self::AllowConsentDomain(params) => request(id, method, params),
            Self::DenyWallDomain(params) => request(id, method, params),
            Self::RemoveRefetchDomain(params) => request(id, method, params),
            Self::Dismiss(params) => request(id, method, params),
            Self::Stop(params) => request(id, method, params),
            Self::Abort(params) => request(id, method, params),
            Self::Pause(params) => request(id, method, params),
            Self::Resume(params) => request(id, method, params),
            Self::RetryFailed(params) => request(id, method, params),
            Self::AddSeeds(seeds) => request(
                id,
                method,
                &RunAddSeedsParams {
                    run_id: seeds.run_id.clone(),
                },
            ),
        }
    }
}

/// The Kennel's `hello` to the Daemon of `runtime`, which carries the
/// keep-alive flag only when `keep_daemon_alive` is on.
pub(super) fn hello_request(runtime: &DaemonRuntimeFile, keep_daemon_alive: bool) -> Request {
    let hello = HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Kennel,
        form: None,
        token: Some(runtime.token.expose().to_owned()),
        needs: Some(Product::Collie),
        product_version: kennel_version(),
        log_level: None,
        client_surface: None,
        keep_daemon_alive,
    };
    request(HELLO_ID, HELLO, &hello)
}

/// What a refused `hello` says.
pub(super) fn refusal(error: ErrorObject) -> Refusal {
    let details = Value::Object(error.data.details);
    if details.get("handover").and_then(Value::as_bool) == Some(true) {
        return Refusal::HandOver;
    }
    if error.data.code == PROTOCOL_VERSION_UNSUPPORTED.name
        && let Ok(details) = serde_json::from_value::<ProtocolVersionUnsupportedDetails>(details)
    {
        return Refusal::ProtocolVersion {
            engine: details.engine_protocol_version,
            needs: PROTOCOL_VERSION,
        };
    }
    Refusal::Other {
        code: error.data.code,
    }
}

/// The request `id` of `method` with `params`.
pub(super) fn request(id: i64, method: &str, params: &impl Serialize) -> Request {
    Request {
        jsonrpc: Version,
        id: RequestId::Number(id),
        method: method.to_owned(),
        params: Some(serde_json::to_value(params).expect("invariant: a request's params encode")),
    }
}

/// The notification `method` with `params`.
pub(super) fn notification(method: &str, params: &impl Serialize) -> Notification {
    Notification {
        jsonrpc: Version,
        method: method.to_owned(),
        params: Some(
            serde_json::to_value(params).expect("invariant: a notification's params encode"),
        ),
    }
}

/// The Kennel's own version, which its `hello` names (ADR 0141).
fn kennel_version() -> ProductVersion {
    ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the crate's version is a Product version")
}

/// Writes `message` as one line.
pub(super) async fn send(writer: &mut Writer, message: &impl Serialize) -> io::Result<()> {
    let mut line = serde_json::to_vec(message).expect("invariant: a message encodes");
    line.push(b'\n');
    writer.write_all(&line).await?;
    writer.flush().await
}

/// Opens the Daemon's socket: a Unix socket on macOS and Linux, loopback
/// TCP on Windows (ADR 0024).
#[expect(
    clippy::disallowed_types,
    reason = "the Kennel's Client is the one module of the Kennel that opens a socket, its Connection to the local Daemon (ADR 0141)"
)]
pub(super) async fn connect(endpoint: &DaemonEndpoint) -> io::Result<(Reader, Writer)> {
    match endpoint {
        #[cfg(unix)]
        DaemonEndpoint::Unix(path) => {
            let (reader, writer) = tokio::net::UnixStream::connect(path).await?.into_split();
            Ok((Box::pin(reader), Box::pin(writer)))
        }
        #[cfg(not(unix))]
        DaemonEndpoint::Unix(_) => Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "a Unix socket on a host that has none",
        )),
        DaemonEndpoint::Tcp(address) => {
            let stream = tokio::net::TcpStream::connect(address).await?;
            stream.set_nodelay(true)?;
            let (reader, writer) = stream.into_split();
            Ok((Box::pin(reader), Box::pin(writer)))
        }
    }
}
