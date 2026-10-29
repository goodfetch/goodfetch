//! A Client's Connection through the gateway (ADR 0096): the Console's
//! WebSocket, answered as Goodfetch Cloud's gateway answers it. The gateway
//! answers the Client's `hello` with the Product it needs and its own
//! engine instance, `run.list` from its own index, and a start (`run.start`
//! or a crawl's) by starting the run on a new Attached engine of the Product
//! the routing rule picks (`product_for_run`): the Client's for a Client,
//! and for the Console Foxhound's when the start's operation or a key of its
//! settings is one only Foxhound carries. A start that names a Run ID its index holds goes to that run's
//! engine. Every other message goes, byte for byte, to the engine that owns
//! the run its params name, or to an engine of the Client's Product when it
//! names none, each over a Connection of the Client's own to that engine's
//! private endpoint; whatever an engine sends back reaches the Client as it
//! is. A Collie Client's call of an operation only Foxhound carries is
//! refused by the gateway with `not_in_product`, as a Collie engine refuses
//! it, even on a run of Foxhound's engine, and a gateway given no Foxhound
//! executable refuses a Foxhound Client at `hello` the same way.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::io;
use std::sync::Arc;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_engine::transport::ConnectionWriter;
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{
    CANCEL, EngineInstance, Form, HELLO, HelloParams, HelloResult, NOT_IN_PRODUCT,
    NotInProductDetails, Surface,
};
use goodfetch_protocol::foxhound_names;
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_protocol::log::LogLevel;
use goodfetch_protocol::routing::{RunStart, Starter, Target, product_for_run};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt as _, AsyncRead, AsyncWrite, AsyncWriteExt as _};
use tokio::sync::{Mutex, mpsc};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::engines::{Engine, Engines, version};
use crate::index::{RunIndex, setting_keys};

/// The operation the gateway answers from its own index.
const RUN_LIST: &str = "run.list";
/// The operations that start a run, each on the engine the routing rule
/// picks from the operation and the settings it names: `run.start`, and a
/// crawl's start, which only Foxhound carries.
const STARTS: [&str; 2] = ["run.start", "crawl.start"];
/// JSON-RPC's code for a failure inside the gateway.
const INTERNAL_ERROR: i64 = -32603;
/// JSON-RPC's code for a Connection that does not open with `hello`.
const INVALID_REQUEST: i64 = -32600;
/// How many messages wait for a Client's socket before senders wait.
const OUTGOING: usize = 256;

/// The gateway's engines and its own index.
pub(crate) struct Gateway {
    engines: Engines,
    index: Mutex<RunIndex>,
    /// The engine of each Product that answers what names no run.
    defaults: Mutex<Vec<Arc<Engine>>>,
    /// The gateway process, as its `hello` names it.
    instance: EngineInstance,
}

impl Gateway {
    /// A gateway over `engines`, its instance drawn from `entropy`.
    pub(crate) fn new(engines: Engines, entropy: &mut Entropy) -> Self {
        Self {
            engines,
            index: Mutex::default(),
            defaults: Mutex::default(),
            instance: EngineInstance::new(format!("stub-gateway-{:016x}", entropy.next_u64())),
        }
    }

    /// Kills its engines, and returns once each has exited and been reaped.
    pub(crate) async fn stop(&self) {
        self.engines.stop().await;
    }

    /// Serves one Client's WebSocket until it closes, its tasks `owner`'s.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the Client's Connections to its engines.
    pub(crate) async fn serve<S>(self: Arc<Self>, socket: WebSocketStream<S>, owner: TaskOwner)
    where
        S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
    {
        let (mut sink, mut stream) = socket.split();
        let (outgoing, mut sending) = mpsc::channel::<String>(OUTGOING);
        owner.spawn(async move {
            while let Some(text) = sending.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    return;
                }
            }
            let _ = sink.close().await;
        });
        let Some(client) = opened(&mut stream, &outgoing, &self.instance, &self.engines).await
        else {
            return;
        };
        let mut links: BTreeMap<u64, ConnectionWriter> = BTreeMap::new();
        while let Some(Ok(message)) = stream.next().await {
            let text = match message {
                Message::Text(text) => text.to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            let Ok(request) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            let id = request.get("id").cloned();
            match (request["method"].as_str(), id) {
                (Some(RUN_LIST), Some(id)) => {
                    let (gateway, outgoing) = (Arc::clone(&self), outgoing.clone());
                    owner.spawn(async move {
                        let answer = gateway.run_list(request["params"].clone()).await;
                        let _ = outgoing.send(replied(answer, &id)).await;
                    });
                }
                (Some(method), Some(id)) if STARTS.contains(&method) => {
                    let (gateway, outgoing) = (Arc::clone(&self), outgoing.clone());
                    let method = method.to_owned();
                    owner.spawn(async move {
                        let answer = gateway
                            .start(&method, request["params"].clone(), client.starter)
                            .await;
                        let _ = outgoing.send(replied(answer, &id)).await;
                    });
                }
                (Some(CANCEL), _) => {
                    for writer in links.values_mut() {
                        let _ = write_line(writer, &text).await;
                    }
                }
                (Some(method), Some(id))
                    if client.needs == Product::Collie
                        && foxhound_names::OPERATIONS.contains(&method) =>
                {
                    let _ = outgoing
                        .send(not_in_product(&id, client.needs, Some(method)))
                        .await;
                }
                (_, id) => {
                    let forwarded = self
                        .forward(&request, &text, &client, &mut links, &outgoing, &owner)
                        .await;
                    if let (Err(error), Some(id)) = (forwarded, id) {
                        let _ = outgoing
                            .send(failure(&id, INTERNAL_ERROR, &error.to_string()))
                            .await;
                    }
                }
            }
        }
    }

    /// Sends `text` to the engine `request` belongs to, over the Client's
    /// own Connection to it, opened the first time.
    async fn forward(
        &self,
        request: &Value,
        text: &str,
        client: &Client,
        links: &mut BTreeMap<u64, ConnectionWriter>,
        outgoing: &mpsc::Sender<String>,
        owner: &TaskOwner,
    ) -> io::Result<()> {
        let named = named_run(&request["params"]);
        let owning = match named {
            Some(run_id) => self.index.lock().await.owner(run_id),
            None => None,
        };
        let engine = match owning {
            Some(engine) => engine,
            None => self.default_engine(client.needs).await?,
        };
        let writer = match links.entry(engine.number()) {
            Entry::Occupied(link) => link.into_mut(),
            Entry::Vacant(slot) => {
                let (mut reader, writer) = engine.link(client.form, client.log_level).await?;
                let outgoing = outgoing.clone();
                owner.spawn(async move {
                    let mut line = String::new();
                    while matches!(reader.read_line(&mut line).await, Ok(read) if read > 0) {
                        if outgoing.send(line.trim_end().to_owned()).await.is_err() {
                            return;
                        }
                        line.clear();
                    }
                });
                slot.insert(writer)
            }
        };
        write_line(writer, text).await
    }

    /// The engine of `product` that answers what names no run, started the
    /// first time.
    async fn default_engine(&self, product: Product) -> io::Result<Arc<Engine>> {
        let mut defaults = self.defaults.lock().await;
        if let Some(engine) = defaults.iter().find(|engine| engine.product() == product) {
            return Ok(Arc::clone(engine));
        }
        let engine = self.engines.start(product).await?;
        defaults.push(Arc::clone(&engine));
        Ok(engine)
    }

    /// A start, `method` with `params`: on the engine its Run ID's run has,
    /// or on a new engine of the Product the routing rule picks.
    async fn start(&self, method: &str, params: Value, starter: Starter) -> io::Result<Value> {
        let known = match params["run_id"].as_str() {
            Some(run_id) => self.index.lock().await.owner(run_id),
            None => None,
        };
        let keys = params["settings"]
            .as_object()
            .map(setting_keys)
            .unwrap_or_default();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let product = product_for_run(&RunStart {
            known_run: known.as_ref().map(|engine| engine.product()),
            starter,
            recipe: None,
            target: Target {
                operations: &[method],
                setting_keys: &keys,
                ..Target::default()
            },
        });
        let engine = match known {
            Some(engine) => engine,
            None => self.engines.start(product).await?,
        };
        let answered = engine.call(method, params).await?;
        if let Some(run_id) = answered["result"]["run_id"].as_str() {
            self.index.lock().await.insert(run_id, &engine);
            tracing::info!(run_id, product = product.name(), "a run started");
        }
        Ok(answered)
    }

    /// `run.list`: the rows of every run the index holds, from the engines
    /// that own them, filtered as `params` asks.
    async fn run_list(&self, params: Value) -> io::Result<Value> {
        let engines = self.index.lock().await.engines();
        let mut rows = Vec::new();
        for engine in engines {
            let answered = engine.call(RUN_LIST, params.clone()).await?;
            if answered.get("error").is_some() {
                return Ok(answered);
            }
            let index = self.index.lock().await;
            rows.extend(
                answered["result"]["runs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter(|row| row["run_id"].as_str().is_some_and(|id| index.holds(id)))
                    .cloned(),
            );
        }
        Ok(json!({"jsonrpc": "2.0", "result": {"runs": rows}}))
    }
}

/// What the gateway keeps of a Client's `hello`.
#[derive(Clone, Copy)]
struct Client {
    needs: Product,
    starter: Starter,
    form: Option<Form>,
    log_level: Option<LogLevel>,
}

/// The Client's `hello`, answered; `None` when the Connection did not open
/// with one.
async fn opened<S>(
    stream: &mut futures_util::stream::SplitStream<WebSocketStream<S>>,
    outgoing: &mpsc::Sender<String>,
    instance: &EngineInstance,
    engines: &Engines,
) -> Option<Client>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let text = loop {
        match stream.next().await? {
            Ok(Message::Text(text)) => break text.to_string(),
            Ok(Message::Close(_)) | Err(_) => return None,
            Ok(_) => {}
        }
    };
    let request: Value = serde_json::from_str(&text).ok()?;
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let hello = (request["method"] == HELLO)
        .then(|| serde_json::from_value::<HelloParams>(request["params"].clone()).ok())
        .flatten();
    let Some((hello, needs)) = hello.and_then(|hello| hello.needs.map(|needs| (hello, needs)))
    else {
        let _ = outgoing
            .send(failure(
                &id,
                INVALID_REQUEST,
                "a Connection opens with a `hello` that names the Product it needs",
            ))
            .await;
        return None;
    };
    if !engines.runs(needs) {
        let _ = outgoing
            .send(not_in_product(&id, Product::Collie, None))
            .await;
        return None;
    }
    let result = HelloResult {
        protocol_version: PROTOCOL_VERSION.min(hello.protocol_version),
        product: needs,
        product_version: version(),
        engine_instance: instance.clone(),
        private_endpoint: None,
        console_link: None,
    };
    let reply = json!({"jsonrpc": "2.0", "id": id, "result": result});
    outgoing.send(reply.to_string()).await.ok()?;
    Some(Client {
        needs,
        starter: if hello.surface == Surface::Console {
            Starter::Console
        } else {
            Starter::Client { needs }
        },
        form: hello.form,
        log_level: hello.log_level,
    })
}

/// `not_in_product` the gateway answers on a Connection it serves as
/// `product`'s: to a Foxhound Client's `hello` when it cannot run
/// Foxhound's engine (`requested` none), and to a Collie Client's call of an
/// operation only Foxhound carries, as a Collie engine refuses it, whatever
/// the engine of the run it names (ADR 0096).
fn not_in_product(id: &Value, product: Product, requested: Option<&str>) -> String {
    let (message, fix) = match requested {
        None => (
            "This gateway runs no Foxhound engine, and this Client needs Foxhound.".to_owned(),
            "Connect needing Collie, or to a gateway that runs Foxhound's engine.".to_owned(),
        ),
        Some(name) => (
            format!("`{name}` is part of Foxhound, and this Client needs Collie."),
            format!("Only a Client that needs Foxhound calls `{name}`; connect needing Foxhound."),
        ),
    };
    let details = NotInProductDetails {
        product,
        product_version: version(),
        needs: Product::Foxhound,
        requested: requested.map(ToOwned::to_owned),
        fix,
        handover: false,
    };
    let details = match serde_json::to_value(details) {
        Ok(Value::Object(details)) => details,
        _ => serde_json::Map::new(),
    };
    let error = ErrorObject {
        code: NOT_IN_PRODUCT.group.json_rpc_class(),
        message,
        data: ErrorData {
            code: NOT_IN_PRODUCT.name.to_owned(),
            retryable: NOT_IN_PRODUCT.retryable,
            details,
        },
    };
    json!({"jsonrpc": "2.0", "id": id, "error": error}).to_string()
}

/// The Run ID a call's params name, as `run_id` or `run`.
fn named_run(params: &Value) -> Option<&str> {
    params["run_id"].as_str().or_else(|| params["run"].as_str())
}

/// `answer`, an engine's whole reply or the gateway's own, sent as the
/// reply to the Client's request `id`.
fn replied(answer: io::Result<Value>, id: &Value) -> String {
    match answer {
        Ok(mut answered) => {
            answered["id"] = id.clone();
            answered.to_string()
        }
        Err(error) => failure(id, INTERNAL_ERROR, &error.to_string()),
    }
}

/// A JSON-RPC failure the gateway itself answers.
fn failure(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}).to_string()
}

/// Writes `text` and its newline to an engine.
async fn write_line(writer: &mut ConnectionWriter, text: &str) -> io::Result<()> {
    writer.write_all(text.as_bytes()).await?;
    writer.write_all(b"\n").await?;
    writer.flush().await
}
