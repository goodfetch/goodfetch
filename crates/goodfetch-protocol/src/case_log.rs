//! The case log's entry (ADR 0069): one JSON Lines entry per time a runner
//! ran a Client conformance case, written by the SDK and CLI runners and
//! turned into a `conformance` result document by `goodfetch-conformance`'s
//! writer. The entry has no outcome field and refuses one: a runner never
//! sets an outcome, the result document's constructor derives it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::result_document::{Blocking, CaseCheck, Evidence, Platform};

/// One time a case ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseLogEntry {
    /// The case's ID in the declared case list.
    pub case: String,
    /// The capability the case belongs to.
    pub capability: String,
    /// The mode it ran in, such as `attached` or `daemon`, when the case
    /// runs in several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Whether Filter-list blocking was on or off, in the harness's gate
    /// mode (ADR 0047).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<Blocking>,
    /// What it could not reach while it ran, such as a vendor or the
    /// online Proving ground, each as the runner names it; its entry is
    /// then inconclusive (ADR 0064).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreachable: Vec<String>,
    /// Where it ran.
    pub platform: Platform,
    /// The runtime it ran on, such as `node-24` or `cpython-3.14`.
    pub runtime: String,
    /// Each check's result.
    pub checks: Vec<CaseCheck>,
    /// What it saw: the oracle's answers, the scripted messages.
    #[serde(default)]
    pub evidence: Evidence,
}
