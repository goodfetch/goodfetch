//! What a crawl's Run report adds to every Run's (ADR 0102): how its
//! requests ended, its Records per Dataset, its Sinks, its Frontier's
//! learned URL rules and its sitemaps; the warnings Foxhound adds to the
//! warning table, such as `trap_links_off`, `learning_off` and the
//! warnings of a per-page feature switched off (ADR 0138); and the
//! Completeness reasons a crawl adds to every Run's (ADRs 0005, 0045,
//! 0048, 0107, 0118 and 0120), each named by its setting or its cause.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ProtocolVersion, SharedType, SharedTypeValue};
use crate::run::RunReport;
use crate::run::completeness::reason;

use super::crawl::RESPONSE_UNAVAILABLE;
use super::frontier::{
    CRAWL_MAX_DEPTH, CRAWL_MAX_PATH_SEGMENTS, CRAWL_MAX_REQUESTS, CRAWL_MAX_SEGMENT_REPEATS,
    CRAWL_MAX_SITEMAP_NESTING, CRAWL_MAX_URL_LENGTH, CRAWL_PATH_BUDGETS,
};
use super::outcomes::DatasetRecords;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The warning that a crawl ran with its trap-link detection off.
pub const TRAP_LINKS_OFF: &str = "trap_links_off";

/// The warning that a crawl ran with `frontier.learning` off, so its
/// Frontier learned no rule (ADR 0138).
pub const LEARNING_OFF: &str = "learning_off";

/// The warning that a crawl ran with `provenance.log` off, so its Run trace
/// holds no Provenance log (ADR 0138).
pub const PROVENANCE_OFF: &str = "provenance_off";

/// The warning that a crawl ran with `dataset.records` off, so it kept no
/// Record (ADR 0138).
pub const RECORDS_OFF: &str = "records_off";

/// The warning that a crawl ran with `fetch.block_classification` off, so
/// no document was read for a Block cause (ADR 0138).
pub const BLOCK_CLASSIFICATION_OFF: &str = "block_classification_off";

/// The warning that a Session replacing one a block retired found every
/// block of the Run's local addresses blocked on its domain, so it rotated
/// within one (ADR 0044).
pub const LOCAL_ADDRESS_ROTATED_WITHIN_BLOCK: &str = "local_address_rotated_within_block";
/// The warning that the installed browser's version could not be read, so
/// a `standard` Session's own HTTP transport presented no browser identity
/// (ADR 0021).
pub const BROWSER_VERSION_UNREAD: &str = "browser_version_unread";

/// The Completeness reason of the URLs learned traps refused (ADR 0005).
pub const TRAP_PATH: &str = "trap_path";
/// The Completeness reason of listings paged only by an offset (ADR 0005).
pub const OFFSET_PAGINATION: &str = "offset_pagination";
/// The Completeness reason of the offset URLs learned cursors refused
/// (ADR 0005).
pub const CURSOR_PAGINATION: &str = "cursor_pagination";
/// The Completeness reason of the URLs a learned rule merged before a
/// contradicting pair stopped it, which the crawl did not get back (ADR
/// 0005).
pub const LEARNED_RULE_STOPPED: &str = "learned_rule_stopped";
/// The Completeness reason of the requests the Model allowance skipped
/// (ADR 0107).
pub const MODEL_ALLOWANCE: &str = "model_allowance";
/// The Completeness reason of the requests that ended `failed` with an
/// Error code other than `response_unavailable`, which has its own (ADRs
/// 0005 and 0102).
pub const FAILED_REQUESTS: &str = "failed_requests";
/// The Completeness reason of the pages left `blocked` with
/// `payment_required` (ADR 0045).
pub const PAYMENT_REQUIRED: &str = "payment_required";
/// The Completeness reason of the pages left `blocked` with
/// `consent_required` (ADR 0048).
pub const CONSENT_REQUIRED: &str = "consent_required";

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
    /// Each sitemap seed's sitemaps, as its progress gives them.
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
    /// The form it merges, for an `equivalent_form`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub form: Option<EquivalentForm>,
    /// What it was learned from, naming the URLs.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub evidence: String,
    /// The pair of pages that contradicted it, naming their URLs: both
    /// forms fetched, neither redirected, and their page signatures
    /// differ. From then on it merges nothing more; absent while it holds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub contradiction: Option<String>,
    /// Each URL it merged with another or refused, in the order the crawl
    /// met them, a pending request it ended `skipped` with `duplicate`
    /// included.
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
    /// Two forms of the host's URLs serve one page, so a URL in one form
    /// is a `duplicate` of an admitted one in the other.
    EquivalentForm,
    /// A listing pages by a cursor as well as by an offset: its offset
    /// URLs not yet fetched are not added, with `duplicate`, and the cursor
    /// is followed.
    CursorPagination,
}

/// Two spellings of one host's URLs a learned `equivalent_form` merges
/// (ADR 0120), each learned on evidence: a redirect from one to the
/// other, pairs of pages of matching signatures, or pages whose
/// `rel=canonical` differs from their URL only by it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EquivalentForm {
    /// One leading `www.` label against the bare host, the rule's host.
    Www,
    /// `http` against `https`.
    Scheme,
    /// A path with a trailing slash against the same path without it.
    TrailingSlash,
    /// A final `index.html` against its folder.
    IndexHtml,
    /// A final `index.php` against its folder.
    IndexPhp,
}

/// How many sitemaps a sitemap seed could not read [`SitemapReport`] lists
/// one by one; the rest are counted.
pub const SITEMAP_FAILURES_LISTED: usize = 20;

/// A sitemap seed's sitemaps (ADR 0102): those it read, those it could
/// not, and its entries added and not added, counted, never listed, since
/// a sitemap holds up to 50,000.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SitemapReport {
    /// The sitemap or robots.txt the seed named.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub seed: String,
    /// How many of its sitemaps were read, nested ones included; a
    /// robots.txt is none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub read: u64,
    /// How many of its sitemaps could not be read.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failed: u64,
    /// The first 20 of those, by URL, each with why.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub failures: Vec<SitemapFailure>,
    /// How many of its sitemaps' entries were added.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub added: u64,
    /// How many of its sitemaps' entries were not.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub not_added: u64,
    /// Those not added, by the reason the Frontier gave, `no_rule` for an
    /// entry no rule of the seed matched, sorted by name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub not_added_by_reason: Vec<CountOf>,
}

/// A sitemap a sitemap seed could not read (ADR 0102; pitfall U19).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SitemapFailure {
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Why, as a name: its fetch's Error code (`response_unavailable`,
    /// `response_not_ok` for a status or a file that is no sitemap the
    /// crawl reads, `response_limit_exceeded`), its skip reason
    /// (`robots_txt`), its Block cause, or the reason the Frontier did not
    /// add a nested sitemap (`domain_policy`).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: String,
    /// The status its site answered, when one answered.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: Option<u16>,
    /// Why, in words.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
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
    shared_type_values: &[
        SharedTypeValue {
            shared_type: SharedType::RunWarningCode,
            name: TRAP_LINKS_OFF,
            description: "The crawl ran with its trap-link detection off.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::RunWarningCode,
            name: LEARNING_OFF,
            description: "The crawl ran with `frontier.learning` off: its Frontier learned no URL rule, so `learned_rules` is empty by the setting, not for want of evidence.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::RunWarningCode,
            name: PROVENANCE_OFF,
            description: "The crawl ran with `provenance.log` off: its Run trace holds no Provenance log, while each Record still carries its own Provenance.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::RunWarningCode,
            name: RECORDS_OFF,
            description: "The crawl ran with `dataset.records` off: it kept no Record, so its Datasets are empty by the setting, not for want of pages.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::RunWarningCode,
            name: BLOCK_CLASSIFICATION_OFF,
            description: "The crawl ran with `fetch.block_classification` off: no document was read for a Challenge or a captcha, so a page a site blocked may be among its successes.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::RunWarningCode,
            name: LOCAL_ADDRESS_ROTATED_WITHIN_BLOCK,
            description: "A Session replacing one a block retired found every block of `egress.local_addresses` blocked on its domain, so it took an address within a blocked block; `sessions` names it.",
            arrived_in: ARRIVED_IN,
        },
        SharedTypeValue {
            shared_type: SharedType::RunWarningCode,
            name: BROWSER_VERSION_UNREAD,
            description: "The installed browser's version could not be read, so a `standard` Session's own HTTP transport presented no browser identity.",
            arrived_in: ARRIVED_IN,
        },
        reason(
            CRAWL_MAX_DEPTH.name,
            "`crawl.max_depth` refused URLs the crawl would otherwise have admitted, counted: `incomplete`.",
        ),
        reason(
            CRAWL_MAX_REQUESTS.name,
            "`crawl.max_requests` refused URLs the crawl would otherwise have admitted, counted: `incomplete`.",
        ),
        reason(
            CRAWL_MAX_SITEMAP_NESTING.name,
            "`crawl.max_sitemap_nesting` refused sitemaps nested deeper under their sitemap seed, counted: `incomplete`.",
        ),
        reason(
            CRAWL_PATH_BUDGETS.name,
            "One entry of `crawl.path_budgets` refused URLs past its count, named with its pattern and counted: `incomplete`.",
        ),
        reason(
            MODEL_ALLOWANCE,
            "The Model allowance skipped requests the crawl would otherwise have handled, counted: `incomplete`.",
        ),
        reason(
            FAILED_REQUESTS,
            "Requests ended `failed` with an Error code other than `response_unavailable`, counted by code: `incomplete`, since neither their pages nor the pages only they link are in the results; `retryFailed()` re-admits them.",
        ),
        reason(
            RESPONSE_UNAVAILABLE.name,
            "Requests failed with `response_unavailable`, a site asking to wait longer than `politeness.retry_after.max`, counted: `incomplete`.",
        ),
        reason(
            CRAWL_MAX_URL_LENGTH.name,
            "`crawl.max_url_length` refused URLs the crawl would otherwise have admitted, counted: `unverified`.",
        ),
        reason(
            CRAWL_MAX_PATH_SEGMENTS.name,
            "`crawl.max_path_segments` refused URLs the crawl would otherwise have admitted, counted: `unverified`.",
        ),
        reason(
            CRAWL_MAX_SEGMENT_REPEATS.name,
            "`crawl.max_segment_repeats` refused URLs the crawl would otherwise have admitted, counted: `unverified`.",
        ),
        reason(
            TRAP_PATH,
            "Learned traps refused URLs the crawl would otherwise have admitted, counted: `unverified`.",
        ),
        reason(
            OFFSET_PAGINATION,
            "Listings were paged only by an offset parameter, counted: `unverified`.",
        ),
        reason(
            CURSOR_PAGINATION,
            "Learned cursors refused offset URLs of listings the crawl pages by their cursor instead, counted: `unverified`.",
        ),
        reason(
            LEARNED_RULE_STOPPED,
            "A learned rule that a contradicting pair stopped had merged URLs the crawl did not get back, counted per rule: `unverified`.",
        ),
        reason(
            PAYMENT_REQUIRED,
            "Pages in the crawl's scope were left `blocked` with `payment_required`, counted: `unverified`.",
        ),
        reason(
            CONSENT_REQUIRED,
            "Pages in the crawl's scope were left `blocked` with `consent_required`, counted: `unverified`.",
        ),
    ],
};
