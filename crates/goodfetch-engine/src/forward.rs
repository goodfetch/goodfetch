//! Live actions forwarded to the engine that owns a Run (ADR 0031): an
//! engine that cannot take a Run's writer lock asks the owner the Run index
//! names. A Daemon of the engine's major is reached with the token of its
//! runtime file, and an endpoint no running Daemon of the major publishes
//! gets no call; an Attached engine is reached at its private endpoint,
//! with the token its token file holds. An owner that does not own the Run
//! answers `run_not_owned`. A read of a Run another engine works goes to
//! that engine the same way, and its answer, a result or a failure, is the
//! read's (ADR 0102).

use std::future::Future;
use std::io;
use std::pin::Pin;
use std::time::Duration;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::daemon::DaemonEndpoint;
use goodfetch_protocol::jsonrpc::{ErrorObject, META, Message, Request, RequestId, Version};
use goodfetch_protocol::run::{RUN_ABORT, RunAbortParams, RunId, RunOwner, RunReport};
use goodfetch_protocol::settings::TIMEOUT_CLOSE;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tracing::Instrument as _;

use crate::attached::read_token;
use crate::clock::Clock;
use crate::daemon::files::DaemonFiles;
use crate::deadline::Deadline;
use crate::home::Home;
use crate::runs::{Forwarder, ReadAnswer};
use crate::settings::Settings;
use crate::tasks::TaskOwner;
use crate::transport::Connection;
use crate::{trace, transport};

#[cfg(test)]
mod tests;

/// How long the owner has to answer `hello`: a live owner answers at once.
const HELLO_DEADLINE: Duration = Duration::from_secs(5);

/// Why a forwarded call came to nothing.
#[derive(Debug, thiserror::Error)]
enum NoAnswer {
    /// The owner could not be reached.
    #[error("no Connection: {0}")]
    Unconnected(#[source] io::Error),
    /// The owner refused the Connection.
    #[error("`hello` answered `{0}`")]
    HelloRefused(String),
    /// The owner did not answer `hello` within [`HELLO_DEADLINE`].
    #[error("no answer to `hello`")]
    HelloUnanswered,
    /// The owner did not answer the call within its deadline.
    #[error("no answer to `{0}`")]
    CallUnanswered(String),
    /// A call could not be written.
    #[error("`{method}` not sent: {error}")]
    NotSent {
        /// The call's method.
        method: String,
        /// The OS's error.
        #[source]
        error: io::Error,
    },
    /// An answer could not be read.
    #[error("{0}")]
    Unread(#[source] io::Error),
    /// The Connection ended before the answer.
    #[error("the Connection closed")]
    Closed,
    /// An abort answered with the failure of this code.
    #[error("answered `{0}`")]
    Failed(String),
    /// An abort's answer is no Run report.
    #[error("no Run report: {0}")]
    NoReport(#[source] serde_json::Error),
}

/// Forwards to the owner the Run index names: a Daemon of one major, or an
/// Attached engine.
pub(crate) struct ToOwner {
    /// The Goodfetch home, which holds the Daemon's runtime file.
    pub(crate) home: Home,
    /// This engine's Product version, which names the major.
    pub(crate) version: ProductVersion,
    /// The engine's clock, which times the `hello` and the call.
    pub(crate) clock: Clock,
    /// The machine's settings, whose `timeout.close` bounds the call.
    pub(crate) settings: Settings,
}

impl ToOwner {
    /// How long the owner has to answer the forwarded call once it accepted
    /// `hello`: what the machine's `timeout.close` gives a stop, its default
    /// when the machine's settings cannot be read, since an abort waits for
    /// the Run's next safe point. This engine holds none of the settings of
    /// a Run another engine works. An owner that answers nothing by then
    /// counts as one that did not take the action, so the abort retries the
    /// writer lock rather than waiting forever.
    async fn answer_deadline(&self) -> Duration {
        let product = self.settings.product();
        let resolved = self
            .settings
            .machine()
            .await
            .and_then(|machine| machine.resolve(product))
            .inspect_err(|error| {
                tracing::warn!(%error, "`timeout.close` cannot be read, so a forwarded call waits for its default");
            })
            .ok();
        Deadline::of_resolved(resolved.as_ref(), &TIMEOUT_CLOSE).duration()
    }

    /// Where `owner` listens and the token its `hello` takes; `None` when
    /// it cannot be reached.
    async fn reach(&self, owner: &RunOwner, tasks: &TaskOwner) -> Option<(DaemonEndpoint, String)> {
        match owner {
            RunOwner::Daemon(endpoint) => {
                let files = DaemonFiles::of(&self.home, &self.version);
                let running = tasks.blocking(move || files.running()).await.ok()??;
                (&running.endpoint == endpoint)
                    .then(|| (running.endpoint, running.token.expose().to_owned()))
            }
            RunOwner::Attached(private) => {
                let token_file = private.token_file.clone();
                let token = tasks.blocking(move || read_token(&token_file)).await.ok()?;
                Some((private.address.clone(), token))
            }
        }
    }
}

impl ToOwner {
    /// The `hello` that opens a Connection to an owner whose token is
    /// `token`.
    fn hello(&self, token: String, needs: Product) -> HelloParams {
        HelloParams {
            token: Some(token),
            needs: Some(needs),
            ..HelloParams::new(Surface::Rust, self.version.clone())
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
            let hello = self.hello(token, Product::Collie);
            let answered = self.call(&endpoint, &hello, RUN_ABORT.name, &params).await;
            let report = match answered {
                Ok(Ok(report)) => serde_json::from_value(report).map_err(NoAnswer::NoReport),
                Ok(Err(failure)) => Err(NoAnswer::Failed(failure.data.code)),
                Err(reason) => Err(reason),
            };
            match report {
                Ok(report) => Some(report),
                Err(reason) => {
                    tracing::debug!(%run_id, %owner, %reason, "the Run's owner did not stop it");
                    None
                }
            }
        })
    }

    fn read<'a>(
        &'a self,
        owner: &'a RunOwner,
        needs: Product,
        method: &'static str,
        params: Value,
        tasks: &'a TaskOwner,
    ) -> ReadAnswer<'a> {
        Box::pin(async move {
            let (endpoint, token) = self.reach(owner, tasks).await?;
            let hello = self.hello(token, needs);
            match self.call(&endpoint, &hello, method, &params).await {
                Ok(answer) => Some(answer),
                Err(reason) => {
                    tracing::debug!(%owner, method, %reason, "the Run's owner did not answer a read");
                    None
                }
            }
        })
    }
}

impl ToOwner {
    /// Opens a Connection to `owner`, says `hello`, sends `method` with
    /// `params` and reads its answer: its result, or the failure it
    /// answered; [`NoAnswer`] when it gave neither, or not within its
    /// deadline. The call is a request the engine sends, so it opens a
    /// `CLIENT` span, whose Trace context its `_meta` carries (ADR 0132);
    /// `hello` opens the Connection and carries none.
    async fn call(
        &self,
        owner: &DaemonEndpoint,
        hello: &HelloParams,
        method: &'static str,
        params: &impl serde::Serialize,
    ) -> Result<Result<Value, ErrorObject>, NoAnswer> {
        let span = trace::client_span(method);
        let mut params = serde_json::to_value(params).expect("invariant: params encode");
        if let (Some(meta), Value::Object(members)) = (trace::sent(&span), &mut params) {
            members.insert(
                META.to_owned(),
                serde_json::to_value(meta).expect("invariant: a Trace context encodes as JSON"),
            );
        }
        let answer = async {
            let connection = transport::connect_daemon(owner)
                .await
                .map_err(NoAnswer::Unconnected)?;
            self.exchange(connection, hello, method, &params).await
        }
        .instrument(span.clone())
        .await;
        if let Ok(answered) = &answer {
            trace::record_id(&span, &RequestId::Number(CALL));
            if let Err(failure) = answered {
                trace::record_failure(&span, &failure.data.code, failure.code);
            }
        }
        answer
    }

    /// [`Self::call`]'s exchange on the Connection, under its span.
    async fn exchange(
        &self,
        connection: Connection,
        hello: &HelloParams,
        method: &str,
        params: &Value,
    ) -> Result<Result<Value, ErrorObject>, NoAnswer> {
        let (reader, mut writer) = connection.split();
        let mut lines = BufReader::new(reader).lines();
        send(&mut writer, 0, HELLO, hello).await?;
        tokio::select! {
            answer = reply(&mut lines, 0) => {
                answer?.map_err(|failure| NoAnswer::HelloRefused(failure.data.code))?;
            }
            () = self.clock.sleep(HELLO_DEADLINE) => return Err(NoAnswer::HelloUnanswered),
        }
        let deadline = self.answer_deadline().await;
        send(&mut writer, CALL, method, params).await?;
        let answer = tokio::select! {
            answer = reply(&mut lines, CALL) => answer?,
            () = self.clock.sleep(deadline) => {
                return Err(NoAnswer::CallUnanswered(method.to_owned()));
            }
        };
        let _ = writer.shutdown().await;
        Ok(answer)
    }
}

/// The id the forwarded call goes under, after `hello`'s `0`.
const CALL: i64 = 1;

/// The answer to the call `id`, its result or its failure, skipping every
/// other message.
async fn reply(
    lines: &mut tokio::io::Lines<BufReader<transport::ConnectionReader>>,
    id: i64,
) -> Result<Result<Value, ErrorObject>, NoAnswer> {
    loop {
        let line = lines
            .next_line()
            .await
            .map_err(NoAnswer::Unread)?
            .ok_or(NoAnswer::Closed)?;
        match serde_json::from_str::<Message>(&line) {
            Ok(Message::Response(response)) if response.id == RequestId::Number(id) => {
                return Ok(Ok(response.result));
            }
            Ok(Message::Error(failure)) if failure.id == Some(RequestId::Number(id)) => {
                return Ok(Err(failure.error));
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
) -> Result<(), NoAnswer> {
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
    sent.map_err(|error| NoAnswer::NotSent {
        method: method.to_owned(),
        error,
    })
}
