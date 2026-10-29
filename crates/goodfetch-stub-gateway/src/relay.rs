//! A Client's Connection through the gateway (ADR 0096), answered as
//! Goodfetch Cloud's gateway answers it. The gateway accepts Clients per
//! Product as an engine does, under the Daemon's version rule (ADR 0066),
//! and signs a program in by the token its `hello` presents, a browser by
//! its sign-in cookie (spec #586's duty 8). It answers the Client's `hello`
//! with the Product it needs and its own engine instance, `run.list` from
//! its own index, and a start (`run.start` or a crawl's) by starting the
//! run on a new Attached engine of the Product the routing rule picks
//! (`product_for_run`): the Client's for a Client, and for the Console
//! Foxhound's when the start's operation or a key of its settings is one
//! only Foxhound carries. A start that names a Run ID its index holds, or
//! one another start is starting, goes to that run's engine. Every other
//! message goes, byte for byte, to the engine that owns the run its params
//! name, or to an engine of the Client's Product when it names none, each
//! over a Connection of the Client's own to that engine's private endpoint;
//! whatever an engine sends back reaches the Client as it is, so a call the
//! run's engine lacks is refused by that engine (ADRs 0095 and 0096). Only
//! a gateway given no Foxhound executable refuses a Foxhound Client itself,
//! at `hello`, with `not_in_product`.

use std::collections::BTreeMap;
use std::collections::btree_map::Entry;
use std::io;
use std::sync::Arc;

use futures_util::{SinkExt as _, StreamExt as _};
use goodfetch_engine::endpoint::same_token;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::rpc::version_refusal;
use goodfetch_engine::tasks::TaskOwner;
use goodfetch_protocol::catalog::{ErrorCodeDefinition, PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{
    CANCEL, EngineInstance, HELLO, HelloParams, HelloResult, NOT_IN_PRODUCT, NotInProductDetails,
    PROTOCOL_VERSION_UNSUPPORTED, Surface,
};
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_protocol::routing::{RunStart, Starter, Target, product_for_run};
use serde::Serialize;
use serde_json::{Value, json};
use tokio::io::{AsyncRead, AsyncWrite};
use tokio::sync::{Mutex, OnceCell, mpsc};
use tokio_tungstenite::WebSocketStream;
use tokio_tungstenite::tungstenite::Message;

use crate::engines::{Engine, Engines, Line, Relayed, version};
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

/// How a Client's WebSocket was signed in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Door {
    /// A browser, by its sign-in cookie.
    Cookie,
    /// A program, by the token its `hello` presents.
    Hello,
}

/// The gateway's engines and its own index.
pub(crate) struct Gateway {
    engines: Engines,
    index: Mutex<RunIndex>,
    /// The engine of each Product that answers what names no run: Collie's
    /// and Foxhound's, each started the first time.
    defaults: [OnceCell<Arc<Engine>>; 2],
    /// The token a program's `hello` presents.
    token: String,
    /// The gateway process, as its `hello` names it.
    instance: EngineInstance,
}

impl Gateway {
    /// A gateway over `engines`, signing programs in with `token`, its
    /// instance drawn from `entropy`.
    pub(crate) fn new(engines: Engines, token: String, entropy: &mut Entropy) -> Self {
        Self {
            engines,
            index: Mutex::default(),
            defaults: Default::default(),
            token,
            instance: EngineInstance::new(format!("stub-gateway-{:016x}", entropy.next_u64())),
        }
    }

    /// Serves one Client's WebSocket, signed in through `door`, until it
    /// closes, its tasks `owner`'s.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future closes the Client's Connections to its engines.
    pub(crate) async fn serve<S>(
        self: Arc<Self>,
        socket: WebSocketStream<S>,
        door: Door,
        owner: TaskOwner,
    ) where
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
        let Some(client) = self.opened(&mut stream, &outgoing, door).await else {
            return;
        };
        let mut links: BTreeMap<u64, Line> = BTreeMap::new();
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
                            .start(&method, request["params"].clone(), client)
                            .await;
                        let _ = outgoing.send(replied(answer, &id)).await;
                    });
                }
                (Some(CANCEL), _) => {
                    for link in links.values() {
                        let _ = link.send(&text).await;
                    }
                }
                (_, id) => {
                    let forwarded = self
                        .forward(&request, &text, client, &mut links, &outgoing)
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

    /// The Client's `hello`, answered; `None` when the Connection did not
    /// open with one, or the gateway refused it.
    async fn opened<S>(
        &self,
        stream: &mut futures_util::stream::SplitStream<WebSocketStream<S>>,
        outgoing: &mpsc::Sender<String>,
        door: Door,
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
        // A program signs in with the token its `hello` presents; one that
        // presents none or another is closed unanswered, as an engine
        // closes it (ADR 0024).
        if door == Door::Hello {
            let reason = match hello.token.as_deref() {
                None => Some("token_missing"),
                Some(token) if !same_token(token, &self.token) => Some("token_wrong"),
                Some(_) => None,
            };
            if let Some(reason) = reason {
                tracing::warn!(
                    listener = "console",
                    reason,
                    "the gateway refused a Connection"
                );
                return None;
            }
        }
        if let Some((message, details)) = version_refusal(hello.protocol_version, false) {
            let _ = outgoing
                .send(refusal(
                    &id,
                    &PROTOCOL_VERSION_UNSUPPORTED,
                    message,
                    &details,
                ))
                .await;
            return None;
        }
        if !self.engines.runs(needs) {
            let _ = outgoing.send(no_foxhound(&id)).await;
            return None;
        }
        let result = HelloResult {
            protocol_version: PROTOCOL_VERSION.min(hello.protocol_version),
            product: needs,
            product_version: version(),
            engine_instance: self.instance.clone(),
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
            relayed: Relayed {
                surface: hello.surface,
                form: hello.form,
                log_level: hello.log_level,
            },
        })
    }

    /// Sends `text` to the engine `request` belongs to, over the Client's
    /// own Connection to it, opened the first time.
    async fn forward(
        &self,
        request: &Value,
        text: &str,
        client: Client,
        links: &mut BTreeMap<u64, Line>,
        outgoing: &mpsc::Sender<String>,
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
        let link = match links.entry(engine.number()) {
            Entry::Occupied(link) => link.into_mut(),
            Entry::Vacant(slot) => {
                slot.insert(engine.link(client.relayed, outgoing.clone()).await?)
            }
        };
        link.send(text).await
    }

    /// The engine of `product` that answers what names no run, started the
    /// first time; a concurrent first call waits for that start alone.
    async fn default_engine(&self, product: Product) -> io::Result<Arc<Engine>> {
        let slot = match product {
            Product::Collie => &self.defaults[0],
            Product::Foxhound => &self.defaults[1],
        };
        slot.get_or_try_init(|| self.engines.start(product, None))
            .await
            .map(Arc::clone)
    }

    /// A start, `method` with `params` from `client`: on the engine of the
    /// run its Run ID names, or of the start of that ID already under way,
    /// or else on a new engine of the Product the routing rule picks.
    async fn start(&self, method: &str, params: Value, client: Client) -> io::Result<Value> {
        let named = params["run_id"].as_str().map(ToOwned::to_owned);
        let slot = match &named {
            Some(run_id) => Some(self.index.lock().await.reserve(run_id)),
            None => None,
        };
        let keys = params["settings"]
            .as_object()
            .map(setting_keys)
            .unwrap_or_default();
        let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
        let product = product_for_run(&RunStart {
            known_run: slot
                .as_ref()
                .and_then(|slot| slot.get())
                .map(|engine| engine.product()),
            starter: client.starter,
            recipe: None,
            target: Target {
                operations: &[method],
                setting_keys: &keys,
                ..Target::default()
            },
        });
        let start = || self.engines.start(product, Some(client.relayed));
        let engine = match &slot {
            Some(slot) => slot.get_or_try_init(start).await.map(Arc::clone),
            None => start().await,
        };
        let answered = match engine {
            Ok(engine) => engine
                .call(method, params)
                .await
                .map(|answered| (engine, answered)),
            Err(error) => Err(error),
        };
        let mut index = self.index.lock().await;
        let started = answered
            .as_ref()
            .ok()
            .and_then(|(engine, answered)| Some((engine, answered["result"]["run_id"].as_str()?)));
        match (started, &named, &slot) {
            (Some((engine, run_id)), _, _) => {
                index.insert(run_id, engine);
                tracing::info!(run_id, product = product.name(), "a run started");
            }
            (None, Some(run_id), Some(slot)) => index.release(run_id, slot),
            (None, _, _) => {}
        }
        drop(index);
        answered.map(|(_, answered)| answered)
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
    relayed: Relayed,
}

/// `not_in_product` to a Foxhound Client's `hello` on a gateway that cannot
/// run Foxhound's engine, as its own Product, Collie, refuses it.
fn no_foxhound(id: &Value) -> String {
    let details = NotInProductDetails {
        product: Product::Collie,
        product_version: version(),
        needs: Product::Foxhound,
        requested: None,
        fix: "Connect needing Collie, or to a gateway that runs Foxhound's engine.".to_owned(),
        install: None,
        handover: false,
    };
    refusal(
        id,
        &NOT_IN_PRODUCT,
        "This gateway runs no Foxhound engine, and this Client needs Foxhound.".to_owned(),
        &details,
    )
}

/// The gateway's own refusal of the request `id` with `code`.
fn refusal(
    id: &Value,
    code: &ErrorCodeDefinition,
    message: String,
    details: &impl Serialize,
) -> String {
    let details = serde_json::to_value(details)
        .ok()
        .and_then(|details| match details {
            Value::Object(details) => Some(details),
            _ => None,
        })
        .expect("invariant: an Error code's details serialize to an object");
    let error = ErrorObject {
        code: code.group.json_rpc_class(),
        message,
        data: ErrorData {
            code: code.name.to_owned(),
            retryable: code.retryable,
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
