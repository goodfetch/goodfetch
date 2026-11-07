//! The Proving ground's interface as one described format (ADRs 0032 and
//! 0063). The serve line, the routes and the tool status are the Proving
//! ground's own source, `proving-ground/src/interface.rs`, compiled here
//! too; the requests `GET /observations` lists and what a test submits to
//! the oracle are read and written here, and the Proving ground's own tests
//! check every observations document they read against them. `xtask
//! codegen` commits their JSON Schemas and the route list, which the
//! TypeScript and Python test support read.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "../../../proving-ground/src/interface.rs"]
mod interface;

pub use interface::{FixtureAddresses, ProxyCredentials, ROUTES, Routes, ServeLine, ToolStatus};

/// What a test submits to `POST /oracle` for one task: the tool's status
/// and the Records it returned. The Proving ground's oracle reads more
/// fields, the files a tool downloaded, the Access dimensions and a failing
/// tool's error, which a tool that reports them adds.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct OracleSubmission {
    /// The task's ID, such as `catalog/2`.
    pub task: String,
    /// What the tool said of its result.
    pub status: ToolStatus,
    /// The Records the tool returned, each with its fields by name.
    #[serde(default)]
    pub records: Vec<Value>,
    /// The files the tool downloaded.
    #[serde(default)]
    pub files: Vec<Value>,
}

/// One request a page host received, as `GET /observations` lists it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ObservedRequest {
    /// Its place among every request, from 0.
    pub sequence: u64,
    /// The connection it came on.
    pub connection: u64,
    /// The task of the host it came to, if the host has one.
    pub task: Option<String>,
    /// The host name it named, without the port.
    pub host: String,
    /// `http/1.1` or `h2`.
    pub protocol: String,
    /// What it opens: `websocket`, `event_stream` or `request`.
    pub kind: String,
    /// Its method.
    pub method: String,
    /// Its target: the path and query.
    pub target: String,
    /// HTTP/2's pseudo-header names in the order sent.
    pub pseudo_header_order: Option<Vec<String>>,
    /// Every other header, in the order sent, names as sent.
    pub headers: Vec<(String, String)>,
    /// Its cookies, in the order sent.
    pub cookies: Vec<(String, String)>,
    /// Its HTTP Message Signatures headers (RFC 9421), when signed.
    pub message_signature: Option<Value>,
    /// The TLS `ClientHello` of its connection, over TLS.
    pub tls: Option<Value>,
    /// Its HTTP/2 connection's settings and its stream, over HTTP/2.
    pub http2: Option<Value>,
    /// Its answer's status, once the answer's head left.
    pub status: Option<u16>,
    /// When it arrived and was answered.
    pub timing: RequestTiming,
    /// The bytes received and sent for it.
    pub bytes: Value,
    /// The latest earlier request of the same host, method and target
    /// this one repeats.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeats: Option<u64>,
    /// The next request the same host received, and how long after.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next_request: Option<Value>,
    /// What the raw server sent of the answer, for the hosts it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub delivery: Option<Value>,
}

impl ObservedRequest {
    /// The value of its first header named `name`, compared without case.
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(header, _)| header.eq_ignore_ascii_case(name))
            .map(|(_, value)| value.as_str())
    }
}

/// When a request's head arrived and its answer's head left, in
/// microseconds since `serve` started.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RequestTiming {
    /// When its head arrived.
    pub arrived_microseconds: u64,
    /// When its answer's head left; none while it has not.
    pub answered_microseconds: Option<u64>,
}

/// What `GET /observations` serves: the requests, typed, and the rest of
/// the record (the Effects, the reports, the findings, the bytes per task
/// and per host, the TLS handshakes and the connection record) by name.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct Observations {
    /// Every request its page hosts received, in arrival order.
    pub requests: Vec<ObservedRequest>,
    /// How many requests came past the record's bound.
    pub unrecorded_requests: u64,
    /// Every other part of the record, by name.
    #[serde(flatten)]
    pub record: BTreeMap<String, Value>,
}

#[cfg(test)]
mod tests;
