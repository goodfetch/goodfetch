//! The Proving ground's interface as one described format (ADRs 0032 and
//! 0063). The serve line, the routes, the tool status and the requests `GET
//! /observations` lists are the Proving ground's own source,
//! `proving-ground/src/interface.rs`, compiled here too, as the conformance
//! harness compiles it. What a test submits to the oracle and the whole
//! observations document are written here: the oracle has a `Submission` of
//! its own, of which only `ToolStatus` is shared, and the Proving ground's own
//! tests check every observations document they read against this one.
//! `xtask codegen` commits their JSON Schemas and the route list. The
//! TypeScript and Python test support read the route list, and one Python test
//! the serve line's and oracle submission's schemas.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[path = "../../../proving-ground/src/interface.rs"]
#[expect(
    unreachable_pub,
    reason = "the Proving ground's own source: its Secret canary shapes are this crate's, read by `canaries`, never re-exported"
)]
mod interface;

pub(crate) use interface::{
    BASE32_ALPHABET, CANARY_MARK, ENDPOINT_TOKEN_CANARY, SESSION_COOKIE_CANARY, TOTP_CANARY_DRAWN,
    TOTP_CANARY_MARK, TYPED_PASSWORD_CANARY,
};
pub use interface::{
    FixtureAddresses, OWN_TARGETS, ObservedRequest, ProxyCredentials, ROUTES, ReceivedRequests,
    RequestTiming, Routes, ServeLine, ToolStatus,
};

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
