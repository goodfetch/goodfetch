//! Live actions forwarded to the engine that owns a Run (ADR 0031): an
//! engine that cannot take a Run's writer lock asks the owner the Run index
//! names. A Daemon of the engine's major is reached with the token of its
//! runtime file, and an endpoint no running Daemon of the major publishes
//! gets no call; an Attached engine is reached at its private endpoint,
//! with the token its token file holds. An owner that does not own the Run
//! answers `run_not_owned`.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use goodfetch_protocol::catalog::PROTOCOL_VERSION;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HelloParams, ProductVersion, Surface};
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::jsonrpc::{Message, Request, RequestId, Version};
use goodfetch_protocol::run::{RunAbortParams, RunId, RunOwner, RunReport};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};

use crate::attached::read_token;
use crate::clock::Clock;
use crate::daemon::files::DaemonFiles;
use crate::home::Home;
use crate::runs::Forwarder;
use crate::tasks::TaskOwner;
use crate::transport;

/// How long the owner has to answer `hello`: a live owner answers at once,
/// and a Run's own abort then takes what its next safe point takes.
const HELLO_DEADLINE: Duration = Duration::from_secs(5);

/// Forwards to the owner the Run index names: a Daemon of one major, or an
/// Attached engine.
pub(crate) struct ToOwner {
    /// The Goodfetch home, which holds the Daemon's runtime file.
    pub(crate) home: Home,
    /// This engine's Product version, which names the major.
    pub(crate) version: ProductVersion,
    /// The engine's clock, which times the `hello`.
    pub(crate) clock: Clock,
}

impl ToOwner {
    /// Where `owner` listens and the token its `hello` takes; `None` when
    /// it cannot be reached.
    async fn reach(&self, owner: &RunOwner, tasks: &TaskOwner) -> Option<(DaemonEndpoint, String)> {
        match owner {
            RunOwner::Daemon(endpoint) => {
                let files = DaemonFiles::of(&self.home, &self.version);
                let running = tasks.blocking(move || files.running()).await.ok()??;
                (&running.endpoint == endpoint).then_some((running.endpoint, running.token))
            }
            RunOwner::Attached(private) => {
                let token_file = private.token_file.clone();
                let token = tasks.blocking(move || read_token(&token_file)).await.ok()?;
                Some((private.address.clone(), token))
            }
        }
    }
}

impl Forwarder for ToOwner {
    fn abort<'a>(
        &'a self,
        owner: &'a RunOwner,
        run_id: &'a RunId,
        tasks: &'a TaskOwner,
    ) -> Pin<Box<dyn Future<Output = Option<RunReport>> + Send + 'a>> {
        Box::pin(async move {
            let (endpoint, token) = self.reach(owner, tasks).await?;
            let params = RunAbortParams {
                run_id: run_id.clone(),
                forwarded: true,
            };
            let hello = HelloParams {
                protocol_version: PROTOCOL_VERSION,
                surface: Surface::Rust,
                token: Some(token),
                needs: Some(Product::Collie),
                product_version: self.version.clone(),
                log_level: None,
            };
            let answered = call(&endpoint, &hello, &params, self.clock).await;
            match answered {
                Ok(report) => Some(report),
                Err(reason) => {
                    tracing::debug!(%run_id, %owner, reason, "the Run's owner did not stop it");
                    None
                }
            }
        })
    }
}

/// Opens a Connection to `owner`, says `hello`, sends the forwarded
/// `run.abort` and reads its report.
async fn call(
    owner: &DaemonEndpoint,
    hello: &HelloParams,
    params: &RunAbortParams,
    clock: Clock,
) -> Result<RunReport, String> {
    let connection = transport::connect_daemon(owner)
        .await
        .map_err(|error| format!("no Connection: {error}"))?;
    let (reader, mut writer) = connection.split();
    let mut lines = BufReader::new(reader).lines();
    send(&mut writer, 0, "hello", hello).await?;
    tokio::select! {
        answer = reply(&mut lines, 0) => { answer?; }
        () = clock.sleep(HELLO_DEADLINE) => return Err("no answer to `hello`".to_owned()),
    }
    send(&mut writer, 1, "run.abort", params).await?;
    let result = reply(&mut lines, 1).await?;
    let _ = writer.shutdown().await;
    serde_json::from_value(result).map_err(|error| format!("no Run report: {error}"))
}

/// The result of the call `id`, skipping every other message.
async fn reply(
    lines: &mut tokio::io::Lines<BufReader<transport::ConnectionReader>>,
    id: i64,
) -> Result<Value, String> {
    loop {
        let line = lines
            .next_line()
            .await
            .map_err(|error| error.to_string())?
            .ok_or("the Connection closed")?;
        match serde_json::from_str::<Message>(&line) {
            Ok(Message::Response(response)) if response.id == RequestId::Number(id) => {
                return Ok(response.result);
            }
            Ok(Message::Error(failure)) if failure.id == Some(RequestId::Number(id)) => {
                return Err(format!("answered `{}`", failure.error.data.code));
            }
            _ => {}
        }
    }
}

/// Sends the call `id` of `method` with `params`.
async fn send(
    writer: &mut transport::ConnectionWriter,
    id: i64,
    method: &str,
    params: &impl serde::Serialize,
) -> Result<(), String> {
    let request = Request {
        jsonrpc: Version,
        id: RequestId::Number(id),
        method: method.to_owned(),
        params: Some(serde_json::to_value(params).expect("invariant: params encode")),
    };
    let mut line = serde_json::to_vec(&request).expect("invariant: a request encodes");
    line.push(b'\n');
    let sent = match writer.write_all(&line).await {
        Ok(()) => writer.flush().await,
        Err(error) => Err(error),
    };
    sent.map_err(|error| format!("`{method}` not sent: {error}"))
}
