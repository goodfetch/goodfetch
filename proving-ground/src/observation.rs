//! Server-side observations (spec #152): every request a page host receives,
//! recorded as the Proving ground saw it (method, target, headers in the
//! order sent, cookies, HTTP message signature, the connection's TLS
//! `ClientHello` and HTTP/2 settings, pseudo-header order, status and
//! timing), the bytes each
//! request and each host received and sent, the Effects the site
//! carried out, and what its pages' own scripts reported. The oracle and the
//! suites read them, never a tool's report. The connection record beside
//! them lists every connection the Proving ground's listeners accepted and
//! every relay its forward proxies carried out, each entry with the time
//! step it came at, which `PUT /time-step` sets.
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

use std::collections::BTreeMap;
use std::net::SocketAddr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::client_hello::ClientHello;
use crate::clock::Stopwatch;
use crate::connection_record::{self, ConnectionRecord, Stamp};
use crate::sync::Mutex;
use crate::task::Effect;

/// The most requests kept in detail (STYLE.md: every buffer is bounded).
const REQUEST_LIMIT: usize = 100_000;

/// The observations of one `serve`.
pub(crate) struct Observations {
    /// Each host name served, with the task it belongs to, if any.
    hosts: BTreeMap<String, Option<String>>,
    stopwatch: Stopwatch,
    /// The Proving ground's time step, as `PUT /time-step` sets it.
    time_step: Mutex<u64>,
    state: Mutex<State>,
    record: ConnectionRecord,
}

#[derive(Default)]
struct State {
    requests: Vec<Request>,
    unrecorded_requests: u64,
    hosts: BTreeMap<Arc<str>, ByteCount>,
    effects: Vec<EffectRecord>,
    reports: Vec<ReportRecord>,
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
}

/// One request as a check of its client reads it.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) target: String,
    /// `None` while its answer has not left.
    pub(crate) status: Option<u16>,
    pub(crate) fingerprint: Fingerprint,
}

/// What a request shows of the client that sent it, beyond its content:
/// what a site's bot protection compares between requests.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Fingerprint {
    pub(crate) protocol: Protocol,
    /// The connection's JA4 without what a resumed handshake adds (its
    /// `ja4_base`); `None` over plain text.
    pub(crate) ja4: Option<String>,
    /// HTTP/2's settings, first window update and pseudo-header order.
    pub(crate) http2: Option<Http2Fingerprint>,
    /// The header names in the order sent, in lowercase.
    pub(crate) header_order: Vec<String>,
    pub(crate) user_agent: Option<String>,
    /// The `Sec-CH-UA` headers (client hints), names in lowercase, in the
    /// order sent.
    pub(crate) client_hints: Vec<(String, String)>,
}

/// What an HTTP/2 request shows of its client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub(crate) struct Http2Fingerprint {
    pub(crate) settings: Vec<(u16, u32)>,
    pub(crate) window_update: Option<u32>,
    pub(crate) pseudo_header_order: Vec<String>,
}

impl Request {
    fn seen(&self) -> Seen {
        let header = |wanted: &str| {
            self.headers
                .iter()
                .find(|(name, _)| name.eq_ignore_ascii_case(wanted))
                .map(|(_, value)| value.clone())
        };
        Seen {
            method: self.method.clone(),
            target: self.target.clone(),
            status: self.status,
            fingerprint: Fingerprint {
                protocol: self.protocol,
                ja4: self.tls.as_ref().map(|hello| hello.ja4_base.clone()),
                http2: self.http2.as_ref().map(|stream| Http2Fingerprint {
                    settings: stream.connection.settings.clone(),
                    window_update: stream.connection.window_update,
                    pseudo_header_order: self.pseudo_header_order.clone().unwrap_or_default(),
                }),
                header_order: self
                    .headers
                    .iter()
                    .map(|(name, _)| name.to_ascii_lowercase())
                    .collect(),
                user_agent: header("user-agent"),
                client_hints: self
                    .headers
                    .iter()
                    .filter(|(name, _)| name.to_ascii_lowercase().starts_with("sec-ch-ua"))
                    .map(|(name, value)| (name.to_ascii_lowercase(), value.clone()))
                    .collect(),
            },
        }
    }
}

/// The HTTP Message Signatures headers of a request (RFC 9421), as Web Bot
/// Auth sends them.
#[derive(Serialize)]
struct SignatureHeaders {
    signature_agent: Option<String>,
    signature_input: Option<String>,
    signature: Option<String>,
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

/// What a page's own script reported to the site: that it ran, or what it
/// found or achieved, such as a probe's signal or an escape that worked. A
/// family's page posts it to its host's `/reports`; the site records it
/// for the host's task.
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Report {
    /// The path of the page whose script reported.
    pub(crate) page: String,
    /// What the script checked, such as `escape`.
    pub(crate) check: String,
    /// What it found, such as `top_navigation`.
    pub(crate) observed: String,
}

/// The `check` of a [`Report`] that says only that the page's script ran.
pub(crate) const RAN: &str = "ran";

/// The longest field of a [`Report`] the site records, in bytes.
const REPORT_FIELD_LIMIT: usize = 256;

impl Report {
    /// The report a page's script posted, from its JSON body; `None` when it
    /// is not one or a field is empty or longer than 256 bytes.
    pub(crate) fn parse(body: &[u8]) -> Option<Self> {
        let report = serde_json::from_slice::<Self>(body).ok()?;
        [&report.page, &report.check, &report.observed]
            .iter()
            .all(|field| !field.is_empty() && field.len() <= REPORT_FIELD_LIMIT)
            .then_some(report)
    }
}

/// What the site recorded of one request beside the request itself.
pub(crate) enum Recorded {
    Effect(Effect),
    Report(Report),
}

/// A report a page's script sent, with the task of the host it came on.
#[derive(Serialize)]
struct ReportRecord {
    task: Option<String>,
    #[serde(flatten)]
    report: Report,
}

/// A recorded request's place in the record.
#[derive(Clone, Copy, Debug)]
pub(crate) struct RequestId(usize);

/// What `GET /observations` serves.
#[derive(Serialize)]
struct Document<'a> {
    requests: &'a [Request],
    unrecorded_requests: u64,
    effects: &'a [EffectRecord],
    reports: &'a [ReportRecord],
    tasks: BTreeMap<&'a str, ByteCount>,
    hosts: &'a BTreeMap<Arc<str>, ByteCount>,
    #[serde(flatten)]
    record: &'a connection_record::Document<'a>,
}

impl Observations {
    /// Observes the host names in `hosts`, each with its task.
    pub(crate) fn new(hosts: BTreeMap<String, Option<String>>) -> Self {
        Self {
            hosts,
            stopwatch: Stopwatch::start(),
            time_step: Mutex::new(0),
            state: Mutex::default(),
            record: ConnectionRecord::default(),
        }
    }

    /// The host names observed, each with its task, if any.
    pub(crate) fn hosts(&self) -> &BTreeMap<String, Option<String>> {
        &self.hosts
    }

    /// The task of the host `authority` names, if the Proving ground serves
    /// it for one.
    pub(crate) fn task_of(&self, authority: &str) -> Option<String> {
        self.served(authority).and_then(|(_, task)| task.clone())
    }

    /// Sets the Proving ground's time step, which scheduled content follows
    /// and every later entry of the connection record is stamped with.
    pub(crate) fn set_time_step(&self, step: u64) {
        *self.time_step.lock() = step;
    }

    /// The Proving ground's time step.
    pub(crate) fn time_step(&self) -> u64 {
        *self.time_step.lock()
    }

    /// The time step and the time now.
    pub(crate) fn stamp(&self) -> Stamp {
        Stamp {
            time_step: self.time_step(),
            at_microseconds: self.stopwatch.microseconds(),
        }
    }

    /// The connection record.
    pub(crate) fn connection_record(&self) -> &ConnectionRecord {
        &self.record
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock()
    }

    /// Starts observing a new connection the site's listener accepted from
    /// `source` on `destination`, which the connection record lists.
    pub(crate) fn connection(
        self: &Arc<Self>,
        source: SocketAddr,
        destination: SocketAddr,
    ) -> Arc<Connection> {
        let id = self
            .record
            .accepted(self.stamp(), "site", source, destination);
        Arc::new(Connection {
            id,
            observations: Arc::clone(self),
            state: Mutex::new(ConnectionState::default()),
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

    /// Records what the site recorded of a request to `authority`: an
    /// Effect it carried out, or a report its page's script sent. At most
    /// [`REQUEST_LIMIT`] reports are kept.
    pub(crate) fn record(&self, authority: &str, recorded: Recorded) {
        match recorded {
            Recorded::Effect(effect) => self.effect(authority, effect),
            Recorded::Report(report) => {
                let task = self.served(authority).and_then(|(_, task)| task.clone());
                let mut state = self.state();
                if state.reports.len() < REQUEST_LIMIT {
                    state.reports.push(ReportRecord { task, report });
                }
            }
        }
    }

    /// The reports the pages of `task` sent, in order.
    pub(crate) fn reports_of(&self, task: &str) -> Vec<Report> {
        self.state()
            .reports
            .iter()
            .filter(|record| record.task.as_deref() == Some(task))
            .map(|record| record.report.clone())
            .collect()
    }

    /// The record as JSON: the requests, the Effects, the reports, and the bytes per task
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
        self.record.with_document(|record| {
            serde_json::to_vec(&Document {
                requests: &state.requests,
                unrecorded_requests: state.unrecorded_requests,
                effects: &state.effects,
                reports: &state.reports,
                tasks,
                hosts: &state.hosts,
                record,
            })
            .expect("invariant: the observations serialize")
        })
    }

    /// What the Proving ground saw of each request to `task`'s host, in
    /// arrival order: its target, status and fingerprint.
    pub(crate) fn requests_of(&self, task: &str) -> Vec<Seen> {
        self.state()
            .requests
            .iter()
            .filter(|request| request.task.as_deref() == Some(task))
            .map(Request::seen)
            .collect()
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
        let arrived = self.observations.stopwatch.microseconds();
        let mut state = self.observations.state();
        if state.requests.len() >= REQUEST_LIMIT {
            state.unrecorded_requests += 1;
            return None;
        }
        let sequence = state.requests.len();
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
        });
        Some(RequestId(sequence))
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
    };
    (signature.signature_agent.is_some()
        || signature.signature_input.is_some()
        || signature.signature.is_some())
    .then_some(signature)
}
