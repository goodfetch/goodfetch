//! What a crawl's Run report adds to every Run's (ADR 0102): how its
//! requests ended, its Records per Dataset, its Sinks, its Frontier's
//! learned URL rules and its sitemaps; and the warning Foxhound adds to the
//! warning table, `trap_links_off`.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ProtocolVersion, SharedType, SharedTypeValue};
use crate::run::RunReport;

use super::outcomes::DatasetRecords;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The warning that a crawl ran with its trap-link detection off.
pub const TRAP_LINKS_OFF: &str = "trap_links_off";

/// A crawl's Run report (ADR 0102): what a crawl adds to every Run's
/// report, a part no area supplied being absent. On the wire, a crawl's
/// parts sit beside every Run's, which a reader of [`RunReport`] keeps among
/// its `kind_parts`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlRunReport {
    /// How its requests ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub requests: Option<RequestCounts>,
    /// Its Records, per Dataset (ADR 0103).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub records: Option<Vec<DatasetRecords>>,
    /// Each Sink that applies to it (ADRs 0026 and 0103).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sinks: Option<Vec<SinkReport>>,
    /// Its Frontier's learned URL rules (ADR 0005).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub learned_rules: Option<Vec<LearnedRule>>,
    /// Its sitemaps, as its progress gives them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sitemaps: Option<Vec<SitemapReport>>,
    /// What every Run's report holds.
    #[serde(flatten)]
    pub report: RunReport,
}

/// How a crawl's requests ended (ADR 0079).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RequestCounts {
    /// How many succeeded.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub succeeded: u64,
    /// How many failed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failed: u64,
    /// How many were skipped.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub skipped: u64,
    /// How many a site blocked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub blocked: u64,
    /// How many were left pending.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub pending: u64,
    /// The skipped ones, by skip reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by_skip_reason: Vec<CountOf>,
    /// The failed ones, by Error code.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by_error_code: Vec<CountOf>,
    /// The blocked ones, by Block cause.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by_block_cause: Vec<CountOf>,
}

/// How many requests ended for one reason, code or cause.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CountOf {
    /// The reason, code or cause.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// How many.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: u64,
}

/// A Sink that applies to a crawl's Run (ADRs 0026 and 0103).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SinkReport {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// Its Guarantee class.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub guarantee_class: String,
    /// How many deliveries are dead letters.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dead_letters: u64,
    /// How many deliveries fell outside the destination's deduplication
    /// window, and so were at least once.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub outside_dedup_window: u64,
    /// Its delivery status.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: SinkStatus,
    /// For an inactive Sink, the Dataset names it gave that the crawl does
    /// not declare.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub undeclared_datasets: Vec<String>,
}

/// A Sink's delivery status (ADRs 0026 and 0103).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SinkStatus {
    /// Everything was delivered.
    Complete,
    /// Everything was delivered, some of it rejected.
    CompleteWithRejections,
    /// It is still delivering.
    Behind,
    /// It failed.
    Failed,
    /// It names none of the crawl's Datasets and delivers nothing.
    Inactive,
}

/// A URL rule the Frontier learned from the pages it fetched (ADRs 0005
/// and 0120), on one host and so never across a registrable domain, with
/// its evidence and every URL it merged, so each merge can be audited.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct LearnedRule {
    /// What it learned.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: LearnedRuleKind,
    /// The host it holds on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub host: String,
    /// The path it holds on: a trap's path pattern, its numeric,
    /// date-like and hexadecimal segments written `{number}`, `{date}` and
    /// `{hex}`, or the path of a listing whose cursor replaced its offset;
    /// absent for a rule of the whole host.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
    /// The query parameter it is about: one that has no effect, or the
    /// offset parameter a cursor replaced; absent for a trap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub parameter: Option<String>,
    /// What it was learned from, naming the URLs.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub evidence: String,
    /// Each URL it merged with another or refused, in the order the crawl
    /// met them.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub merged: Vec<String>,
}

/// What a learned URL rule learned (ADRs 0005 and 0120).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LearnedRuleKind {
    /// A query parameter has no effect on the host's pages: pairs of
    /// URLs differing only by it served pages of matching signatures, so a
    /// URL that differs from an admitted one only by it is a `duplicate`.
    NoEffectParameter,
    /// A path pattern is a trap: its pages brought no new page signature
    /// and no URL outside it, so its URLs are not added, with
    /// `trap_path`.
    TrapPath,
    /// A listing pages by a cursor as well as by an offset: its offset
    /// URLs not yet fetched are not added, with `duplicate`, and the cursor
    /// is followed.
    CursorPagination,
}

/// A sitemap a crawl read (ADR 0102).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SitemapReport {
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// How many of its entries were added.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub added: u64,
    /// How many of its entries were not.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub not_added: u64,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[SharedTypeValue {
        shared_type: SharedType::RunWarningCode,
        name: TRAP_LINKS_OFF,
        description: "The crawl ran with its trap-link detection off.",
        arrived_in: ARRIVED_IN,
    }],
};
