//! A suite's declared case list: its name and version, the capabilities it
//! declares, and each capability's cases with the Features each proves
//! (ADRs 0033 and 0072). What a suite declares and did not reach stays
//! `unproved` in its result document.

use goodfetch_protocol::journey::FeatureId;
use goodfetch_protocol::result_document::Suite;
use serde::Deserialize;

/// A suite's declared case list.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CaseList {
    /// The suite's name and the list's version.
    pub suite: Suite,
    /// Each capability the suite declares, in the order its document lists
    /// them.
    #[serde(rename = "capability")]
    pub capabilities: Vec<DeclaredCapability>,
}

/// One capability a suite declares.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DeclaredCapability {
    /// Its name, unique in the suite.
    pub name: String,
    /// Its cases, every one of which must pass for it to be `proved`.
    #[serde(rename = "case")]
    pub cases: Vec<DeclaredCase>,
}

/// One case a suite declares. A case of the Client conformance case list
/// (`conformance/clients/cases.toml`) also names its `surfaces`, `modes`,
/// `platforms`, `product`, `script` or `task`, which `xtask clients check`
/// and the runners read and the writer leaves aside.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub struct DeclaredCase {
    /// Its ID, unique in the suite.
    pub id: String,
    /// The Features it proves.
    pub covers: Vec<FeatureId>,
}
