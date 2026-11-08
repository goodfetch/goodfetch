//! The gateway's engines (ADR 0096): each an Attached engine started from
//! the executable of its Product as an SDK starts one (`attached-engine`),
//! in a Goodfetch home of its own under the gateway's state, and held for
//! the gateway's life through its stdio, where the gateway starts its runs.
//! A Client's Connection reaches it on its private endpoint with the token
//! of its token file. Every `hello` the gateway sends an engine names the
//! surface `cloud`, which tells it that it sits behind the gateway, needs
//! that engine's own Product (ADR 0095), and names in `client_surface` the
//! surface of the Client it serves, whose rules the engine then holds the
//! Connection to, a model's included (ADR 0092).
//!
//! Each Connection to an engine is a [`Line`]: a task writes what a bounded
//! channel gives it, and a task reads the engine's lines, handing each
//! answer to the gateway's own call its ID names and every other line to
//! the Client. When the engine's side ends, every call still waiting fails,
//! so a call never waits on an engine that exited.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use goodfetch_engine::children::Children;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::transport;
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{
    Form, HELLO, HelloParams, HelloResult, PrivateEndpoint, ProductVersion, Surface,
};
use goodfetch_protocol::log::LogLevel;
use serde_json::{Value, json};
use tokio::io::{AsyncBufRead, AsyncBufReadExt as _, AsyncWrite, AsyncWriteExt as _, BufReader};
use tokio::process::Command;
use tokio::sync::{Mutex, mpsc, oneshot};

/// The command that runs a Product's executable as an Attached engine.
const ATTACHED_ENGINE: &str = "attached-engine";

/// The variable that names an engine's Goodfetch home.
const HOME_VARIABLE: &str = "GOODFETCH_HOME";

/// How long the gateway waits for an engine to answer its `hello`: a
/// loaded machine's first start of a fresh build can take this long, as
/// the conformance harness allows a process it just started.
const HELLO_DEADLINE: Duration = Duration::from_secs(30);

/// How many lines wait for an engine's input before their senders wait.
const QUEUED: usize = 256;

/// The prefix of the IDs of the gateway's own calls on a Connection, which
/// tells their answers from those of the Client the Connection serves.
const CALL_ID: &str = "goodfetch-stub-gateway-";

/// The Products' executables, of the same commit: Collie's always, and
/// Foxhound's on a gateway that can run Foxhound's engine.
pub(crate) struct Executables {
    pub(crate) collie: PathBuf,
    pub(crate) foxhound: Option<PathBuf>,
}

/// What a relayed Connection keeps of its Client's `hello`.
#[derive(Clone, Copy)]
pub(crate) struct Relayed {
    /// The Client's surface, which the engine holds the Connection to.
    pub(crate) surface: Surface,
    pub(crate) form: Option<Form>,
    pub(crate) log_level: Option<LogLevel>,
}

/// The `hello` the gateway sends an engine of `product`: as the gateway,
/// needing that engine's own Product, for the Client `relayed` names, or
/// for none on a Connection of the gateway's own.
pub(crate) fn hello(
    product: Product,
    token: Option<String>,
    relayed: Option<Relayed>,
) -> HelloParams {
    HelloParams {
        form: relayed.and_then(|relayed| relayed.form),
        token,
        needs: Some(product),
        log_level: relayed.and_then(|relayed| relayed.log_level),
        client_surface: relayed.map(|relayed| relayed.surface),
        ..HelloParams::new(Surface::Cloud, version())
    }
}

/// The gateway's own Product version: this commit's.
pub(crate) fn version() -> ProductVersion {
    ProductVersion::parse(env!("CARGO_PKG_VERSION"))
        .expect("invariant: the workspace's version is a Product version")
}

/// Starts engines, each in its own home.
pub(crate) struct Engines {
    executables: Executables,
    environment: Vec<(OsString, OsString)>,
    state: PathBuf,
    children: Mutex<Children>,
    /// One for each engine started, sent once its process has exited and
    /// been reaped.
    exits: Mutex<Vec<oneshot::Receiver<()>>>,
    started: Mutex<u64>,
    owner: Arc<TaskOwner>,
}

impl Engines {
    /// Engines of `executables`, run with `environment` and their homes
    /// under `state`, whose tasks `owner` holds.
    ///
    /// # Errors
    ///
    /// When the OS refuses what holds the children.
    pub(crate) fn new(
        executables: Executables,
        environment: Vec<(OsString, OsString)>,
        state: PathBuf,
        owner: Arc<TaskOwner>,
    ) -> io::Result<Self> {
        Ok(Self {
            executables,
            environment,
            state,
            children: Mutex::new(Children::new()?),
            exits: Mutex::default(),
            started: Mutex::new(0),
            owner,
        })
    }

    /// Kills every engine started, and returns once each has exited and
    /// been reaped, so that none outlives the gateway: an engine shares
    /// the gateway's stderr, which a test's runner watches until every
    /// holder has closed it.
    pub(crate) async fn stop(&self) {
        self.children.lock().await.kill_all();
        let exits = std::mem::take(&mut *self.exits.lock().await);
        for exit in exits {
            // An error means the wait's task ended without seeing the exit,
            // which leaves nothing to wait for.
            let _ = exit.await;
        }
    }

    /// Whether the gateway can run an engine of `product`.
    pub(crate) fn runs(&self, product: Product) -> bool {
        product == Product::Collie || self.executables.foxhound.is_some()
    }

    /// A new Attached engine of `product`, once it answered the gateway's
    /// `hello`, which names the Client the gateway starts it for.
    ///
    /// # Errors
    ///
    /// When it cannot start, or refuses or leaves unanswered the `hello`.
    #[expect(
        clippy::disallowed_methods,
        reason = "ADR 0140: a file opened on no counted row yet; the Shared paths audit moves it onto its counted function"
    )]
    pub(crate) async fn start(
        &self,
        product: Product,
        relayed: Option<Relayed>,
    ) -> io::Result<Arc<Engine>> {
        let number = {
            let mut started = self.started.lock().await;
            *started += 1;
            *started
        };
        let home = self.state.join(format!("engine-{number}"));
        tokio::fs::create_dir_all(&home).await?;
        let executable =
            match product {
                Product::Collie => &self.executables.collie,
                Product::Foxhound => self.executables.foxhound.as_ref().ok_or_else(|| {
                    io::Error::other("the gateway was given no Foxhound executable")
                })?,
            };
        let mut command = Command::new(executable);
        command
            .arg(ATTACHED_ENGINE)
            .env_clear()
            .envs(self.environment.iter().cloned())
            .env(HOME_VARIABLE, &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit());
        let mut child = self.children.lock().await.spawn(command)?;
        let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
            return Err(io::Error::other("the engine's stdio is not piped"));
        };
        let own = Line::open(BufReader::new(stdout), stdin, None, &self.owner);
        let (exited, exit) = oneshot::channel();
        self.exits.lock().await.push(exit);
        self.owner.spawn(async move {
            if let Err(error) = child.wait().await {
                tracing::debug!(%error, "an engine's exit was not seen");
            }
            let _ = exited.send(());
        });
        let params = serde_json::to_value(hello(product, None, relayed))?;
        let answered = own.call_within(HELLO, params, HELLO_DEADLINE).await?;
        let result: HelloResult = serde_json::from_value(answered["result"].clone())
            .map_err(|_| io::Error::other(format!("the engine refused the hello: {answered}")))?;
        tracing::info!(number, product = product.name(), "an engine started");
        Ok(Arc::new(Engine {
            number,
            product,
            stdio: own,
            private: result.private_endpoint,
            owner: Arc::clone(&self.owner),
        }))
    }
}

/// One Attached engine.
pub(crate) struct Engine {
    number: u64,
    product: Product,
    /// The gateway's own Connection, on the engine's stdio.
    stdio: Line,
    private: Option<PrivateEndpoint>,
    owner: Arc<TaskOwner>,
}

impl Engine {
    /// Which engine it is, in the order they started.
    pub(crate) fn number(&self) -> u64 {
        self.number
    }

    /// Its Product.
    pub(crate) fn product(&self) -> Product {
        self.product
    }

    /// The whole answer, result or error, to the gateway's own call of
    /// `method` with `params` on the engine's stdio.
    ///
    /// # Errors
    ///
    /// When the engine is gone, or goes before it answers.
    pub(crate) async fn call(&self, method: &str, params: Value) -> io::Result<Value> {
        self.stdio.call(method, params).await
    }

    /// A Connection of its own to the engine's private endpoint for the
    /// Client `relayed` names, after the gateway's `hello` there: every
    /// line the engine sends on it goes to `client`.
    ///
    /// # Errors
    ///
    /// When the endpoint cannot be reached, or the engine refuses or
    /// leaves unanswered the `hello`.
    pub(crate) async fn link(
        &self,
        relayed: Relayed,
        client: mpsc::Sender<String>,
    ) -> io::Result<Line> {
        let Some(private) = &self.private else {
            return Err(io::Error::other("the engine reported no private endpoint"));
        };
        let token = tokio::fs::read_to_string(&private.token_file).await?;
        let connection = transport::connect_daemon(&private.address).await?;
        let (reader, writer) = connection.split();
        let line = Line::open(BufReader::new(reader), writer, Some(client), &self.owner);
        let params = hello(self.product, Some(token.trim().to_owned()), Some(relayed));
        let answered = line
            .call_within(HELLO, serde_json::to_value(params)?, HELLO_DEADLINE)
            .await?;
        if answered.get("result").is_none() {
            return Err(io::Error::other(format!(
                "the engine refused the hello: {answered}"
            )));
        }
        Ok(line)
    }
}

/// A call of the gateway's own that waits for its answer.
struct Waiting {
    id: String,
    answer: oneshot::Sender<Value>,
}

/// One line-delimited Connection to an engine.
pub(crate) struct Line {
    writes: mpsc::Sender<Vec<u8>>,
    waits: mpsc::Sender<Waiting>,
    calls: Mutex<u64>,
}

impl Line {
    /// The Connection of `reader` and `writer`, its tasks `owner`'s: every
    /// line it reads that answers no call of the gateway's goes to
    /// `client`, or is dropped without one.
    fn open<R, W>(
        reader: R,
        writer: W,
        client: Option<mpsc::Sender<String>>,
        owner: &TaskOwner,
    ) -> Self
    where
        R: AsyncBufRead + Unpin + Send + 'static,
        W: AsyncWrite + Unpin + Send + 'static,
    {
        let (outgoing, queued) = mpsc::channel(QUEUED);
        let (waits, registered) = mpsc::channel(QUEUED);
        owner.spawn(write(writer, queued));
        owner.spawn(read(reader, registered, client));
        Self {
            writes: outgoing,
            waits,
            calls: Mutex::new(0),
        }
    }

    /// The whole answer to the gateway's own call of `method` with
    /// `params`.
    ///
    /// # Errors
    ///
    /// When the engine's side has ended, or ends before it answers.
    pub(crate) async fn call(&self, method: &str, params: Value) -> io::Result<Value> {
        let id = {
            let mut calls = self.calls.lock().await;
            *calls += 1;
            format!("{CALL_ID}{calls}")
        };
        let (answer, answered) = oneshot::channel();
        self.waits
            .send(Waiting {
                id: id.clone(),
                answer,
            })
            .await
            .map_err(|_| ended())?;
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.send(&request.to_string()).await?;
        answered.await.map_err(|_| ended())
    }

    /// [`Line::call`], failing once `deadline` passes without an answer.
    async fn call_within(
        &self,
        method: &str,
        params: Value,
        deadline: Duration,
    ) -> io::Result<Value> {
        tokio::time::timeout(deadline, self.call(method, params))
            .await
            .map_err(|_| {
                io::Error::new(
                    io::ErrorKind::TimedOut,
                    format!(
                        "the engine answered no `{method}` within {} s",
                        deadline.as_secs()
                    ),
                )
            })?
    }

    /// Sends `text` and its newline to the engine.
    ///
    /// # Errors
    ///
    /// When the engine's side has ended.
    pub(crate) async fn send(&self, text: &str) -> io::Result<()> {
        let mut line = text.as_bytes().to_vec();
        line.push(b'\n');
        self.writes
            .send(line)
            .await
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    }
}

/// The failure of a call on an engine whose side ended.
fn ended() -> io::Error {
    io::Error::other("the engine ended before it answered")
}

/// Writes each line `written` gives to the engine, until the Connection or
/// its writer goes.
async fn write<W: AsyncWrite + Unpin>(mut writer: W, mut written: mpsc::Receiver<Vec<u8>>) {
    while let Some(line) = written.recv().await {
        if writer.write_all(&line).await.is_err() || writer.flush().await.is_err() {
            return;
        }
    }
    let _ = writer.shutdown().await;
}

/// Reads the engine's lines until its side ends or the Connection goes:
/// each answer to a call `waiting` registered goes to that call, every
/// other line to `client`. The calls still waiting when it ends fail.
async fn read<R: AsyncBufRead + Unpin>(
    reader: R,
    mut waiting: mpsc::Receiver<Waiting>,
    client: Option<mpsc::Sender<String>>,
) {
    let mut calls: HashMap<String, oneshot::Sender<Value>> = HashMap::new();
    let mut lines = reader.lines();
    loop {
        tokio::select! {
            // A call registers before it writes its request, so its
            // registration is taken before its answer can be read.
            biased;
            registered = waiting.recv() => match registered {
                Some(Waiting { id, answer }) => {
                    calls.insert(id, answer);
                }
                None => return,
            },
            line = lines.next_line() => {
                let Ok(Some(line)) = line else { return };
                let message = serde_json::from_str::<Value>(&line).ok();
                let call = message
                    .as_ref()
                    .and_then(|message| message["id"].as_str())
                    .and_then(|id| calls.remove(id));
                match (call, message) {
                    (Some(call), Some(message)) => {
                        let _ = call.send(message);
                    }
                    _ => {
                        if let Some(client) = &client
                            && client.send(line).await.is_err()
                        {
                            return;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests;
