//! The gateway's engines (ADR 0096): each an Attached engine started from
//! the executable of its Product as an SDK starts one (`attached-engine`),
//! in a Goodfetch home of its own under the gateway's state, and held for
//! the gateway's life through its stdio, where the gateway starts its runs.
//! A Console's Connection reaches it on its private endpoint with the token
//! of its token file. Every `hello` the gateway sends an engine names the
//! surface `cloud`, which tells it that it sits behind the gateway, and
//! needs that engine's own Product (ADR 0095).

use std::collections::HashMap;
use std::ffi::OsString;
use std::io;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::Arc;

use goodfetch_engine::children::Children;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::transport::{self, ConnectionReader, ConnectionWriter};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{
    Form, HELLO, HelloParams, HelloResult, PrivateEndpoint, ProductVersion, Surface,
};
use goodfetch_protocol::log::LogLevel;
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncWriteExt as _, BufReader};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{Mutex, oneshot};

/// The command that runs a Product's executable as an Attached engine.
const ATTACHED_ENGINE: &str = "attached-engine";

/// The variable that names an engine's Goodfetch home.
const HOME_VARIABLE: &str = "GOODFETCH_HOME";

/// The Products' executables, of the same commit: Collie's always, and
/// Foxhound's on a gateway that can run Foxhound's engine.
pub(crate) struct Executables {
    pub(crate) collie: PathBuf,
    pub(crate) foxhound: Option<PathBuf>,
}

/// The `hello` the gateway sends an engine of `product`: as the gateway,
/// needing that engine's own Product, with the Client's form and log level
/// when it relays a Client's Connection.
pub(crate) fn hello(
    product: Product,
    token: Option<String>,
    form: Option<Form>,
    log_level: Option<LogLevel>,
) -> HelloParams {
    HelloParams {
        protocol_version: PROTOCOL_VERSION,
        surface: Surface::Cloud,
        form,
        token,
        needs: Some(product),
        product_version: version(),
        log_level,
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
    /// `hello`.
    ///
    /// # Errors
    ///
    /// When it cannot start or refuses the `hello`.
    pub(crate) async fn start(&self, product: Product) -> io::Result<Arc<Engine>> {
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
        let pending: Pending = Arc::default();
        self.owner
            .spawn(answers(BufReader::new(stdout), Arc::clone(&pending)));
        let (exited, exit) = oneshot::channel();
        self.exits.lock().await.push(exit);
        self.owner.spawn(async move {
            if let Err(error) = child.wait().await {
                tracing::debug!(%error, "an engine's exit was not seen");
            }
            let _ = exited.send(());
        });
        let mut engine = Engine {
            number,
            product,
            stdin: Mutex::new(stdin),
            pending,
            calls: Mutex::new(0),
            private: None,
        };
        let params = serde_json::to_value(hello(product, None, None, None))?;
        let answered = engine.call(HELLO, params).await?;
        let result: HelloResult = serde_json::from_value(answered["result"].clone())
            .map_err(|_| io::Error::other(format!("the engine refused the hello: {answered}")))?;
        engine.private = result.private_endpoint;
        tracing::info!(number, product = product.name(), "an engine started");
        Ok(Arc::new(engine))
    }
}

/// The calls the gateway made on an engine's stdio, by their ID, awaiting
/// their answers.
type Pending = Arc<Mutex<HashMap<String, oneshot::Sender<Value>>>>;

/// Hands each answer on an engine's stdout to the call that awaits it.
async fn answers(stdout: BufReader<tokio::process::ChildStdout>, pending: Pending) {
    let mut lines = stdout.lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let Ok(message) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(id) = message["id"].as_str() else {
            continue;
        };
        let waiting = pending.lock().await.remove(id);
        if let Some(waiting) = waiting {
            let _ = waiting.send(message);
        }
    }
}

/// One Attached engine.
pub(crate) struct Engine {
    number: u64,
    product: Product,
    stdin: Mutex<ChildStdin>,
    pending: Pending,
    calls: Mutex<u64>,
    private: Option<PrivateEndpoint>,
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
    /// When the engine is gone.
    pub(crate) async fn call(&self, method: &str, params: Value) -> io::Result<Value> {
        let id = {
            let mut calls = self.calls.lock().await;
            *calls += 1;
            format!("gateway-{calls}")
        };
        let (sender, answer) = oneshot::channel();
        self.pending.lock().await.insert(id.clone(), sender);
        let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let mut line = request.to_string().into_bytes();
        line.push(b'\n');
        {
            let mut stdin = self.stdin.lock().await;
            stdin.write_all(&line).await?;
            stdin.flush().await?;
        }
        answer
            .await
            .map_err(|_| io::Error::other("the engine ended before it answered"))
    }

    /// A Connection of its own to the engine's private endpoint, after the
    /// gateway's `hello` there, for a Client whose `hello` named `form` and
    /// `log_level`.
    ///
    /// # Errors
    ///
    /// When the endpoint cannot be reached or the engine refuses the
    /// `hello`.
    pub(crate) async fn link(
        &self,
        form: Option<Form>,
        log_level: Option<LogLevel>,
    ) -> io::Result<(BufReader<ConnectionReader>, ConnectionWriter)> {
        let Some(private) = &self.private else {
            return Err(io::Error::other("the engine reported no private endpoint"));
        };
        let token = tokio::fs::read_to_string(&private.token_file).await?;
        let connection = transport::connect_daemon(&private.address).await?;
        let (reader, mut writer) = connection.split();
        let mut reader = BufReader::new(reader);
        let params = hello(self.product, Some(token.trim().to_owned()), form, log_level);
        let request =
            json!({"jsonrpc": "2.0", "id": "gateway-hello", "method": HELLO, "params": params});
        let mut line = request.to_string().into_bytes();
        line.push(b'\n');
        writer.write_all(&line).await?;
        writer.flush().await?;
        let mut answer = String::new();
        reader.read_line(&mut answer).await?;
        let answered: Value = serde_json::from_str(&answer)?;
        if answered.get("result").is_none() {
            return Err(io::Error::other(format!(
                "the engine refused the hello: {answered}"
            )));
        }
        Ok((reader, writer))
    }
}

#[cfg(test)]
mod tests;
