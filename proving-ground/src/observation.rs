//! Server-side observations (spec #152): every request a page host receives,
//! recorded as the Proving ground saw it (method, target, headers in the
//! order sent, cookies, HTTP message signature, the connection's TLS
//! `ClientHello` and HTTP/2 settings, pseudo-header order, status and
//! timing), the bytes each
//! request and each host received and sent, and the Effects the site
//! carried out. The oracle and the suites read them, never a tool's report.
//!
//! Bytes are counted twice over, both as they cross the wire:
//!
//! - a request's own bytes are its messages as framed on its connection
//!   (HTTP/1.1's head and body, or the HTTP/2 frames of its stream, frame
//!   headers included), received and sent;
//! - a host's bytes are every byte of the connections to it, TLS records,
//!   handshakes and HTTP/2's connection frames included, and a task's are
//!   those of its host. A connection's host is the server name its TLS
//!   handshake asked for, else the host of its first request.
//!
//! Only the host names the Proving ground serves are observed; its own
//! address (the task list, the oracle, this record) is not. At most
//! [`REQUEST_LIMIT`] requests are kept in detail; later ones are counted in
//! `unrecorded_requests` and in their host's bytes.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;

use serde::Serialize;

use crate::client_hello::ClientHello;
use crate::clock::Stopwatch;
use crate::sync::Mutex;
use crate::task::Effect;

/// The most requests kept in detail (STYLE.md: every buffer is bounded).
const REQUEST_LIMIT: usize = 100_000;

/// The observations of one `serve`.
pub(crate) struct Observations {
    /// Each host name served, with the task it belongs to, if any.
    hosts: BTreeMap<String, Option<String>>,
    stopwatch: Stopwatch,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    connections: u64,
    requests: Vec<Request>,
    unrecorded_requests: u64,
    hosts: BTreeMap<Arc<str>, ByteCount>,
    effects: Vec<EffectRecord>,
    /// Each connection whose host is served, as it was accepted.
    accepted: Vec<Accepted>,
    /// The latest request each host received, by its sequence.
    latest_of_host: HashMap<Arc<str>, usize>,
    /// The latest request of each host, method and target.
    latest_of_target: HashMap<(Arc<str>, String, String), usize>,
}

/// Bytes received from and sent to a client.
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct ByteCount {
    pub(crate) received: u64,
    pub(crate) sent: u64,
}

impl ByteCount {
    fn add(&mut self, other: Self) {
        self.received += other.received;
        self.sent += other.sent;
    }
}

/// The application protocol a request came on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub(crate) enum Protocol {
    #[serde(rename = "http/1.1")]
    Http1,
    #[serde(rename = "h2")]
    Http2,
}

/// What an HTTP/2 connection's client said before its first request: its
/// `SETTINGS` in the order sent, and the increment of its first connection
/// `WINDOW_UPDATE`.
#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Http2Connection {
    pub(crate) settings: Vec<(u16, u32)>,
    pub(crate) window_update: Option<u32>,
}

/// An HTTP/2 request's connection and stream.
#[derive(Debug, Serialize)]
pub(crate) struct Http2Stream {
    #[serde(flatten)]
    pub(crate) connection: Arc<Http2Connection>,
    pub(crate) stream: u32,
}

/// A request's head, as a connection's parser read it.
pub(crate) struct RequestHead {
    pub(crate) protocol: Protocol,
    pub(crate) method: String,
    pub(crate) target: String,
    /// The `Host` header, or HTTP/2's `:authority`, port included.
    pub(crate) authority: String,
    /// HTTP/2's pseudo-header names in the order sent.
    pub(crate) pseudo_header_order: Option<Vec<String>>,
    /// Every other header, in the order sent, names as sent.
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) http2: Option<Http2Stream>,
}

/// A recorded request, as `GET /observations` lists it.
#[derive(Serialize)]
struct Request {
    sequence: usize,
    connection: u64,
    task: Option<String>,
    host: String,
    protocol: Protocol,
    method: String,
    target: String,
    pseudo_header_order: Option<Vec<String>>,
    headers: Vec<(String, String)>,
    cookies: Vec<(String, String)>,
    message_signature: Option<SignatureHeaders>,
    tls: Option<Arc<ClientHello>>,
    http2: Option<Http2Stream>,
    status: Option<u16>,
    timing: Timing,
    bytes: ByteCount,
    /// The earlier request of the same host, method and target this one
    /// repeats, the latest of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    repeats: Option<usize>,
    /// The next request the same host received, and how long after this
    /// one's answer (or its arrival, if it had none) it arrived.
    #[serde(skip_serializing_if = "Option::is_none")]
    next_request: Option<NextRequest>,
    /// What the raw server sent of the answer, for the hosts it answers.
    #[serde(skip_serializing_if = "Option::is_none")]
    delivery: Option<Delivery>,
}

/// The request that followed another to its host.
#[derive(Serialize)]
struct NextRequest {
    sequence: usize,
    after_microseconds: u64,
}

/// What the raw server sent of an answer: the body length its head
/// declared, the body bytes it sent, and how the answer ended.
#[derive(Debug, Serialize)]
pub(crate) struct Delivery {
    /// The answer's `Content-Length`, if its head had one.
    pub(crate) declared_length: Option<u64>,
    /// The body's bytes sent, before any transfer coding or framing.
    pub(crate) body_sent: u64,
    pub(crate) end: End,
}

/// How an answer ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum End {
    /// Its whole message was sent.
    Complete,
    /// The connection was closed after what was sent.
    ConnectionClosed,
    /// The connection was reset (TCP `RST`) after what was sent.
    ConnectionReset,
    /// Its HTTP/2 stream was reset with `RST_STREAM`.
    StreamReset { error_code: &'static str },
    /// The connection's `GOAWAY` named a last stream below the request's,
    /// so the request was never processed.
    GoAway {
        last_stream: u32,
        error_code: &'static str,
    },
}

/// A connection the Proving ground accepted for a host it serves.
#[derive(Serialize)]
struct Accepted {
    connection: u64,
    host: Arc<str>,
    /// The client's address family; `null` when it was not read.
    family: Option<Family>,
    accepted_microseconds: u64,
}

/// An address family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Family {
    Ipv4,
    Ipv6,
}

/// What the Proving ground found when it checked a request's Web Bot Auth
/// signature against the key directory its `Signature-Agent` names, as the
/// request arrived.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Verification {
    pub(crate) verified: bool,
    /// The key that signed it, when verified.
    pub(crate) key_id: Option<String>,
    /// Why it was refused, when it was.
    pub(crate) error: Option<String>,
}

/// A task's request, as the oracle reads it.
pub(crate) struct Seen {
    pub(crate) target: String,
    /// `None` when unsigned; else whether its signature was verified.
    pub(crate) signature: Option<bool>,
    /// The key that signed it, when its signature was verified.
    pub(crate) key_id: Option<String>,
}

/// The HTTP Message Signatures headers of a request (RFC 9421), as Web Bot
/// Auth sends them.
#[derive(Serialize)]
struct SignatureHeaders {
    signature_agent: Option<String>,
    signature_input: Option<String>,
    signature: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    verification: Option<Verification>,
}

/// When a request's head arrived and its answer's head left, in
/// microseconds since `serve` started.
#[derive(Serialize)]
struct Timing {
    arrived_microseconds: u64,
    answered_microseconds: Option<u64>,
}

/// An Effect the site carried out, with the task of the host it came on.
#[derive(Serialize)]
struct EffectRecord {
    task: Option<String>,
    #[serde(flatten)]
    effect: Effect,
}

/// A recorded request's place in the record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestId(usize);

/// What `GET /observations` serves.
#[derive(Serialize)]
struct Document<'a> {
    requests: &'a [Request],
    connections: &'a [Accepted],
    unrecorded_requests: u64,
    effects: &'a [EffectRecord],
    tasks: BTreeMap<&'a str, ByteCount>,
    hosts: &'a BTreeMap<Arc<str>, ByteCount>,
}

impl Observations {
    /// Observes the host names in `hosts`, each with its task.
    pub(crate) fn new(hosts: BTreeMap<String, Option<String>>) -> Self {
        Self {
            hosts,
            stopwatch: Stopwatch::start(),
            state: Mutex::default(),
        }
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock()
    }

    /// Starts observing a new connection whose client's address is not
    /// known.
    #[cfg(test)]
    pub(crate) fn connection(self: &Arc<Self>) -> Arc<Connection> {
        self.connection_with(None)
    }

    /// Starts observing a new connection from `peer`.
    pub(crate) fn connection_from(self: &Arc<Self>, peer: SocketAddr) -> Arc<Connection> {
        self.connection_with(Some(if peer.ip().to_canonical().is_ipv4() {
            Family::Ipv4
        } else {
            Family::Ipv6
        }))
    }

    fn connection_with(self: &Arc<Self>, family: Option<Family>) -> Arc<Connection> {
        let id = {
            let mut state = self.state();
            state.connections += 1;
            state.connections - 1
        };
        Arc::new(Connection {
            id,
            observations: Arc::clone(self),
            state: Mutex::new(ConnectionState {
                family,
                accepted_microseconds: self.stopwatch.microseconds(),
                ..ConnectionState::default()
            }),
        })
    }

    /// The served host a `Host` header or server name names, if it is one.
    fn served(&self, authority: &str) -> Option<(&str, &Option<String>)> {
        self.hosts
            .get_key_value(host_name(authority).as_str())
            .map(|(host, task)| (host.as_str(), task))
    }

    fn count(&self, host: &Arc<str>, bytes: ByteCount) {
        self.state()
            .hosts
            .entry(Arc::clone(host))
            .or_default()
            .add(bytes);
    }

    /// Records that the site carried out `effect` on a request to
    /// `authority`.
    pub(crate) fn effect(&self, authority: &str, effect: Effect) {
        let task = self.served(authority).and_then(|(_, task)| task.clone());
        self.state().effects.push(EffectRecord { task, effect });
    }

    /// The Effects carried out for `task`, in order.
    pub(crate) fn effects_of(&self, task: &str) -> Vec<Effect> {
        self.state()
            .effects
            .iter()
            .filter(|record| record.task.as_deref() == Some(task))
            .map(|record| record.effect.clone())
            .collect()
    }

    /// The requests recorded for `task`, in order.
    pub(crate) fn requests_of(&self, task: &str) -> Vec<Seen> {
        self.state()
            .requests
            .iter()
            .filter(|request| request.task.as_deref() == Some(task))
            .map(|request| {
                let verification = request
                    .message_signature
                    .as_ref()
                    .map(|signature| signature.verification.as_ref());
                Seen {
                    target: request.target.clone(),
                    signature: verification.map(|found| found.is_some_and(|found| found.verified)),
                    key_id: verification
                        .flatten()
                        .and_then(|found| found.key_id.clone()),
                }
            })
            .collect()
    }

    /// The record as JSON: the requests, the Effects, and the bytes per task
    /// and per host.
    pub(crate) fn document(&self) -> Vec<u8> {
        let state = self.state();
        let mut tasks = BTreeMap::new();
        for (host, bytes) in &state.hosts {
            if let Some(Some(task)) = self.hosts.get(&**host) {
                tasks
                    .entry(task.as_str())
                    .or_insert_with(ByteCount::default)
                    .add(*bytes);
            }
        }
        serde_json::to_vec(&Document {
            requests: &state.requests,
            connections: &state.accepted,
            unrecorded_requests: state.unrecorded_requests,
            effects: &state.effects,
            tasks,
            hosts: &state.hosts,
        })
        .expect("invariant: the observations serialize")
    }

    fn update(&self, request: RequestId, change: impl FnOnce(&mut Request)) {
        if let Some(record) = self.state().requests.get_mut(request.0) {
            change(record);
        }
    }
}

/// A host name without its port or a trailing dot, in lowercase.
pub(crate) fn host_name(authority: &str) -> String {
    let name = if let Some(rest) = authority.strip_prefix('[') {
        rest.split(']').next().unwrap_or_default()
    } else {
        authority
            .rsplit_once(':')
            .map_or(authority, |(name, _)| name)
    };
    name.trim_end_matches('.').to_ascii_lowercase()
}

/// One connection's observer, shared by the layers that read its bytes: the
/// wire below TLS and the parser above it.
pub(crate) struct Connection {
    id: u64,
    observations: Arc<Observations>,
    state: Mutex<ConnectionState>,
}

#[derive(Default)]
struct ConnectionState {
    host: Host,
    /// Bytes counted before the host was known.
    pending: ByteCount,
    tls: Option<Arc<ClientHello>>,
    family: Option<Family>,
    accepted_microseconds: u64,
    /// The latest request read on each HTTP/2 stream, or on the connection
    /// for HTTP/1.1 (stream 0).
    latest: BTreeMap<u32, RequestId>,
}

/// Whose bytes a connection's are.
#[derive(Default)]
enum Host {
    /// Not known yet: its bytes wait until it is.
    #[default]
    Unknown,
    Served(Arc<str>),
    /// A host the Proving ground does not serve, such as its own address.
    Unserved,
}

impl Connection {
    fn state(&self) -> std::sync::MutexGuard<'_, ConnectionState> {
        self.state.lock()
    }

    /// Counts bytes that crossed the wire, TLS included.
    pub(crate) fn wire(&self, bytes: ByteCount) {
        let mut state = self.state();
        match &mut state.host {
            Host::Served(host) => self.observations.count(host, bytes),
            Host::Unknown => state.pending_add(bytes),
            Host::Unserved => {}
        }
    }

    /// Decides the connection's host, once: the bytes counted so far become
    /// its own.
    fn decide_host(&self, state: &mut ConnectionState, authority: &str) {
        if !matches!(state.host, Host::Unknown) {
            return;
        }
        let pending = state.take_pending();
        state.host = match self.observations.served(authority) {
            Some((host, _)) => {
                let host: Arc<str> = Arc::from(host);
                self.observations.count(&host, pending);
                self.observations.state().accepted.push(Accepted {
                    connection: self.id,
                    host: Arc::clone(&host),
                    family: state.family,
                    accepted_microseconds: state.accepted_microseconds,
                });
                Host::Served(host)
            }
            None => Host::Unserved,
        };
    }

    /// Records the connection's `ClientHello`, whose server name decides its
    /// host.
    pub(crate) fn client_hello(&self, hello: ClientHello) {
        let mut state = self.state();
        if let Some(name) = hello.sni.clone() {
            self.decide_host(&mut state, &name);
        }
        state.tls = Some(Arc::new(hello));
    }

    /// Records a request whose head has arrived, if its host is served; a
    /// connection's first request decides its host when TLS did not.
    pub(crate) fn request(&self, head: RequestHead) -> Option<RequestId> {
        let tls = {
            let mut state = self.state();
            self.decide_host(&mut state, &head.authority);
            state.tls.clone()
        };
        let (host, task) = self.observations.served(&head.authority)?;
        let stream = head.http2.as_ref().map_or(0, |http2| http2.stream);
        let arrived = self.observations.stopwatch.microseconds();
        let mut state = self.observations.state();
        if state.requests.len() >= REQUEST_LIMIT {
            state.unrecorded_requests += 1;
            return None;
        }
        let sequence = state.requests.len();
        let host_key: Arc<str> = Arc::from(host);
        if let Some(previous) = state.latest_of_host.insert(Arc::clone(&host_key), sequence)
            && let Some(record) = state.requests.get_mut(previous)
        {
            let since = record
                .timing
                .answered_microseconds
                .unwrap_or(record.timing.arrived_microseconds);
            record.next_request = Some(NextRequest {
                sequence,
                after_microseconds: arrived.saturating_sub(since),
            });
        }
        let repeats = state.latest_of_target.insert(
            (host_key, head.method.clone(), head.target.clone()),
            sequence,
        );
        state.requests.push(Request {
            sequence,
            connection: self.id,
            task: task.clone(),
            host: host.to_owned(),
            protocol: head.protocol,
            cookies: cookies(&head.headers),
            message_signature: message_signature(&head.headers),
            method: head.method,
            target: head.target,
            pseudo_header_order: head.pseudo_header_order,
            headers: head.headers,
            tls,
            http2: head.http2,
            status: None,
            timing: Timing {
                arrived_microseconds: arrived,
                answered_microseconds: None,
            },
            bytes: ByteCount::default(),
            repeats,
            next_request: None,
            delivery: None,
        });
        drop(state);
        self.state().latest.insert(stream, RequestId(sequence));
        Some(RequestId(sequence))
    }

    /// The latest request read on HTTP/2 stream `stream`, or on the
    /// connection for HTTP/1.1 (stream 0).
    pub(crate) fn latest_request(&self, stream: u32) -> Option<RequestId> {
        self.state().latest.get(&stream).copied()
    }

    /// Records what the raw server sent of a request's answer.
    pub(crate) fn delivered(&self, request: RequestId, delivery: Delivery) {
        self.observations
            .update(request, |record| record.delivery = Some(delivery));
    }

    /// Records the check of a request's Web Bot Auth signature.
    pub(crate) fn verified(&self, request: RequestId, verification: Verification) {
        self.observations.update(request, |record| {
            if let Some(signature) = &mut record.message_signature {
                signature.verification = Some(verification);
            }
        });
    }

    /// Counts bytes of a request's own messages.
    pub(crate) fn request_bytes(&self, request: RequestId, bytes: ByteCount) {
        self.observations
            .update(request, |record| record.bytes.add(bytes));
    }

    /// Records the status of a request's final answer, as its head leaves.
    pub(crate) fn answered(&self, request: RequestId, status: u16) {
        let answered = self.observations.stopwatch.microseconds();
        self.observations.update(request, |record| {
            record.status = Some(status);
            record.timing.answered_microseconds = Some(answered);
        });
    }
}

impl ConnectionState {
    fn pending_add(&mut self, bytes: ByteCount) {
        self.pending.add(bytes);
    }

    fn take_pending(&mut self) -> ByteCount {
        std::mem::take(&mut self.pending)
    }
}

/// Every `Cookie` header's pairs, in order.
fn cookies(headers: &[(String, String)]) -> Vec<(String, String)> {
    headers
        .iter()
        .filter(|(name, _)| name.eq_ignore_ascii_case("cookie"))
        .flat_map(|(_, value)| value.split(';'))
        .filter_map(|pair| {
            let pair = pair.trim();
            (!pair.is_empty()).then(|| {
                let (name, value) = pair.split_once('=').unwrap_or((pair, ""));
                (name.to_owned(), value.to_owned())
            })
        })
        .collect()
}

/// The HTTP Message Signatures headers (RFC 9421), if the request carries any.
fn message_signature(headers: &[(String, String)]) -> Option<SignatureHeaders> {
    let header = |wanted: &str| {
        headers
            .iter()
            .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
            .map(|(_, value)| value.clone())
    };
    let signature = SignatureHeaders {
        signature_agent: header("signature-agent"),
        signature_input: header("signature-input"),
        signature: header("signature"),
        verification: None,
    };
    (signature.signature_agent.is_some()
        || signature.signature_input.is_some()
        || signature.signature.is_some())
    .then_some(signature)
}
