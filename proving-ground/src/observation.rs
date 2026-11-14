//! Server-side observations (spec #152): every request a page host receives,
//! recorded as the Proving ground saw it (method, target, headers in the
//! order sent, cookies, HTTP message signature, the connection's TLS
//! `ClientHello` and HTTP/2 settings, pseudo-header order, status and
//! timing), every TLS handshake's `ClientHello` whether or not it
//! completed, the bytes each
//! request and each host received and sent, the Effects the site
//! carried out, what its pages' own scripts reported, and the findings of
//! the Crawl families (a page fetched twice, a trap followed, a honeypot
//! hit, a `Retry-After` ignored). The oracle and the suites read them,
//! never a tool's report. The connection record beside them lists every
//! connection the Proving ground's listeners accepted and every relay its
//! forward proxies carried out, each entry with the time step it came at,
//! which `PUT /time-step` sets; and `GET /network-fixtures` serves the
//! addresses of the network fixtures `serve` reported, for a harness that
//! knows only the Proving ground's address.
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
//! [`RECORDED_REQUEST_LIMIT`] requests are kept in detail; later ones are counted in
//! `unrecorded_requests` and in their host's bytes, and at most
//! [`FINDING_LIMIT`] findings, later ones counted in `unrecorded_findings`.

use std::collections::{BTreeMap, HashMap};
use std::net::SocketAddr;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::client_hello::ClientHello;
use crate::clock::Stopwatch;
use crate::connection_record::{self, ConnectionRecord, Stamp};
use crate::sync::Mutex;
use crate::task::{Effect, Finding};

/// The most reports and TLS handshakes kept (STYLE.md: every buffer is bounded).
const REQUEST_LIMIT: usize = 100_000;

/// The most requests kept in detail: twice [`REQUEST_LIMIT`], so a crawl of
/// a 100,000-page site, as `just compare`'s growth cell runs, is kept
/// whole with its robots.txt, retries and duplicates (STYLE.md: every
/// buffer is bounded).
const RECORDED_REQUEST_LIMIT: usize = 200_000;

/// The most findings kept (STYLE.md: every buffer is bounded).
const FINDING_LIMIT: usize = 10_000;

/// The observations of one `serve`.
pub(crate) struct Observations {
    /// Each host name served, with the task it belongs to, if any.
    hosts: BTreeMap<String, Option<String>>,
    stopwatch: Stopwatch,
    /// The Proving ground's time step, as `PUT /time-step` sets it.
    time_step: Mutex<u64>,
    state: Mutex<State>,
    record: ConnectionRecord,
    /// What `serve` reported of its network fixtures, once bound.
    network_fixtures: Mutex<Option<serde_json::Value>>,
    /// Whether the `limited` proxy holds each request it counts until a
    /// harness releases it (`PUT /limited-proxy-hold`).
    limited_hold: tokio::sync::watch::Sender<bool>,
}

#[derive(Default)]
struct State {
    requests: Vec<Request>,
    unrecorded_requests: u64,
    hosts: BTreeMap<Arc<str>, ByteCount>,
    effects: Vec<EffectRecord>,
    /// The latest request each host received, by its sequence.
    latest_of_host: HashMap<Arc<str>, usize>,
    /// The latest request of each host, method and target.
    latest_of_target: HashMap<(Arc<str>, String, String), usize>,
    reports: Vec<ReportRecord>,
    findings: Vec<FindingRecord>,
    unrecorded_findings: u64,
    /// The host name of each handshake that presented a client certificate,
    /// in order.
    client_certificates: Vec<String>,
    /// Each TLS handshake's `ClientHello`, in order, whether or not the
    /// handshake completed.
    tls_handshakes: Vec<Handshake>,
}

/// A TLS handshake a connection began: its `ClientHello`, recorded before
/// the server answers, so a client that refuses the Proving ground's
/// certificate, as a browser trusting only its own roots does, still shows
/// its fingerprint.
#[derive(Serialize)]
struct Handshake {
    connection: u64,
    #[serde(flatten)]
    hello: Arc<ClientHello>,
}

/// The most handshakes that presented a client certificate kept (STYLE.md:
/// every buffer is bounded).
const CLIENT_CERTIFICATE_LIMIT: usize = 10_000;

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

/// Which part of a request's messages counted bytes are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    /// Heads and framing: an HTTP/1.1 head or chunk line, an HTTP/2 frame
    /// header or a frame that carries no body.
    Framing,
    /// The body's data: an HTTP/1.1 body's, or an HTTP/2 `DATA` frame's
    /// payload.
    Body,
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
    /// What the request opens: a WebSocket, a server-sent events stream,
    /// or an ordinary request.
    kind: RequestKind,
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
    /// Every byte of its messages, as framed.
    bytes: ByteCount,
    /// Its messages' body bytes alone, which no client's framing or header
    /// compression changes.
    body: ByteCount,
    /// The earlier request of the same host, method and target this one
    /// repeats, the latest of them.
    #[serde(skip_serializing_if = "Option::is_none")]
    repeats: Option<usize>,
    /// The next request the same host received, and how long after this
    /// one's answer (or its arrival, if it had none) it arrived.
    #[serde(rename = "next_request", skip_serializing_if = "Option::is_none")]
    followed_by: Option<NextRequest>,
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
    /// Its HTTP/2 stream was reset with `RST_STREAM`.
    StreamReset { error_code: &'static str },
    /// The connection's `GOAWAY` named a last stream below the request's,
    /// so the request was never processed.
    GoAway {
        last_stream: u32,
        error_code: &'static str,
    },
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

/// What a request opens, as its headers ask: a WebSocket (`Upgrade:
/// websocket`), a server-sent events stream (`Accept: text/event-stream`),
/// or an ordinary request.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum RequestKind {
    Websocket,
    EventStream,
    Request,
}

impl RequestKind {
    fn of(headers: &[(String, String)]) -> Self {
        let has = |name: &str, value: &str| {
            headers.iter().any(|(header, field)| {
                header.eq_ignore_ascii_case(name)
                    && field
                        .split(',')
                        .any(|part| part.trim().eq_ignore_ascii_case(value))
            })
        };
        if has("upgrade", "websocket") {
            Self::Websocket
        } else if has("accept", "text/event-stream") {
            Self::EventStream
        } else {
            Self::Request
        }
    }
}

/// One request as a check of its client reads it.
#[derive(Clone, Debug, Serialize)]
pub(crate) struct Seen {
    pub(crate) method: String,
    pub(crate) target: String,
    /// `None` while its answer has not left.
    pub(crate) status: Option<u16>,
    pub(crate) fingerprint: Fingerprint,
    /// `None` when unsigned; else whether its Web Bot Auth signature was
    /// verified as it arrived.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) signature: Option<bool>,
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
            signature: self.message_signature.as_ref().map(|signature| {
                signature
                    .verification
                    .as_ref()
                    .is_some_and(|found| found.verified)
            }),
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

/// A finding of a Crawl family, with the task of the host it came on.
#[derive(Serialize)]
struct FindingRecord {
    task: Option<String>,
    #[serde(flatten)]
    finding: Finding,
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
    findings: &'a [FindingRecord],
    unrecorded_findings: u64,
    tasks: BTreeMap<&'a str, ByteCount>,
    hosts: &'a BTreeMap<Arc<str>, ByteCount>,
    client_certificates: &'a [String],
    tls_handshakes: &'a [Handshake],
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
            network_fixtures: Mutex::new(None),
            limited_hold: tokio::sync::watch::Sender::new(false),
        }
    }

    /// Sets whether the `limited` proxy holds each request it counts until
    /// it is released, so a harness that sends two at once knows they meet
    /// there, however late the second arrives.
    pub(crate) fn set_limited_hold(&self, held: bool) {
        self.limited_hold.send_replace(held);
    }

    /// Waits while the `limited` proxy's hold is on.
    pub(crate) async fn limited_released(&self) {
        let mut held = self.limited_hold.subscribe();
        // The sender lives as long as these observations, so the wait ends
        // only once the hold is off.
        let _ = held.wait_for(|held| !*held).await;
    }

    /// Whether the `limited` proxy's hold is on.
    pub(crate) fn limited_held(&self) -> bool {
        *self.limited_hold.borrow()
    }

    /// Keeps what `serve` reports of its network fixtures, which
    /// `GET /network-fixtures` serves.
    pub(crate) fn set_network_fixtures(&self, fixtures: serde_json::Value) {
        *self.network_fixtures.lock() = Some(fixtures);
    }

    /// What `serve` reported of its network fixtures, once they are bound.
    pub(crate) fn network_fixtures(&self) -> Option<serde_json::Value> {
        self.network_fixtures.lock().clone()
    }

    /// The requests received on every observed host so far, recorded or
    /// not: what a harness polls during a crawl, at the cost of one count
    /// instead of the whole record.
    pub(crate) fn request_count(&self) -> u64 {
        let state = self.state();
        state.requests.len() as u64 + state.unrecorded_requests
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

    /// Records that a handshake for the host name `host` presented a client
    /// certificate the host accepted.
    pub(crate) fn client_certificate(&self, host: &str) {
        let mut state = self.state();
        if state.client_certificates.len() < CLIENT_CERTIFICATE_LIMIT {
            state.client_certificates.push(host.to_owned());
        }
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

    /// Records a finding of a Crawl family on a request to `authority`.
    pub(crate) fn finding(&self, authority: &str, finding: Finding) {
        let task = self.served(authority).and_then(|(_, task)| task.clone());
        let mut state = self.state();
        if state.findings.len() >= FINDING_LIMIT {
            state.unrecorded_findings += 1;
        } else {
            state.findings.push(FindingRecord { task, finding });
        }
    }

    /// The findings recorded for `task`, in order.
    pub(crate) fn findings_of(&self, task: &str) -> Vec<Finding> {
        self.state()
            .findings
            .iter()
            .filter(|record| record.task.as_deref() == Some(task))
            .map(|record| record.finding.clone())
            .collect()
    }

    /// The record as JSON: the requests, the Effects, the reports, the
    /// findings, the bytes per task and per host, and the connection record.
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
                findings: &state.findings,
                unrecorded_findings: state.unrecorded_findings,
                client_certificates: &state.client_certificates,
                tls_handshakes: &state.tls_handshakes,
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
        let hello = Arc::new(hello);
        state.tls = Some(Arc::clone(&hello));
        drop(state);
        let mut observations = self.observations.state();
        if observations.tls_handshakes.len() < REQUEST_LIMIT {
            observations.tls_handshakes.push(Handshake {
                connection: self.id,
                hello,
            });
        }
    }

    /// The connection's `ClientHello`; `None` over plain text.
    pub(crate) fn tls(&self) -> Option<Arc<ClientHello>> {
        self.state().tls.clone()
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
        if state.requests.len() >= RECORDED_REQUEST_LIMIT {
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
            record.followed_by = Some(NextRequest {
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
            kind: RequestKind::of(&head.headers),
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
            body: ByteCount::default(),
            repeats,
            followed_by: None,
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

    /// Counts bytes of a request's own messages, and of their bodies when
    /// they are `part` [`Part::Body`].
    pub(crate) fn request_bytes(&self, request: RequestId, bytes: ByteCount, part: Part) {
        self.observations.update(request, |record| {
            record.bytes.add(bytes);
            if part == Part::Body {
                record.body.add(bytes);
            }
        });
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
