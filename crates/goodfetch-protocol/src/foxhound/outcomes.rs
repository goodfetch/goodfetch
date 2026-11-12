//! A crawl's endings, read from its committed state (ADRs 0079, 0093, 0102,
//! 0103 and 0117): each crawl request ends in one `CrawlRequestOutcome`,
//! committed with the work item's last transition, in commit order.
//! `run.outcomes` pages them from the first, or from the next commit with
//! `after: "now"`, and waits for the next commit while the Run is live, so
//! a reader that stops reading never slows the crawl; its filters run in
//! the engine. `run.failed_requests` reads the requests whose latest
//! outcome is `failed` (with `blocked`, also `blocked`): exactly those
//! `run.retry_failed` re-admits with the same option. `run.retry_failed`
//! re-admits them in the same Run and its Datasets, each keeping its ID,
//! Label, Payload and depth with a fresh `crawl.retry.max`, and answers how
//! many; on an ended Run it returns the Run to `running` first (ADR 0102).
//! Foxhound's own.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::block::BlockCause;
use crate::catalog::{
    Entries, Operation, OperationClass, OperationKind, ProtocolVersion, Reach, Stability, schema_of,
};
use crate::foxhound::frontier::NotAddedReason;
use crate::run::{PointInTime, RunId};
use crate::settings::ProjectSettings;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The most outcomes one page of `run.outcomes` carries.
pub const OUTCOMES_PAGE: usize = 256;

/// How a crawl request ended (ADR 0079).
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize, JsonSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum OutcomeKind {
    /// Its handler returned, and what it staged was committed.
    Succeeded,
    /// Its last attempt failed, and no retry is left.
    Failed,
    /// A decision ended it: a skip is never retried.
    Skipped,
    /// The site refused it, with a Block cause.
    Blocked,
}

impl OutcomeKind {
    /// Its name on the wire and in run state.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::Failed => "failed",
            Self::Skipped => "skipped",
            Self::Blocked => "blocked",
        }
    }
}

/// Why a crawl request was skipped (ADRs 0079, 0093, 0107 and 0119).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SkipReason {
    /// A redirect left the crawl's scope.
    OutOfScope,
    /// A redirect or a Hook's rewrite reached a host the domain policy
    /// refuses.
    DomainPolicy,
    /// A URL rule merged it with a request already admitted.
    Duplicate,
    /// The site's robots.txt refuses it.
    RobotsTxt,
    /// The site's Crawl-delay could not be kept.
    CrawlDelay,
    /// An AI-use signal refuses the Run's Purpose.
    UseSignals,
    /// A before-request Hook skipped it.
    Hook,
    /// Its handler skipped it.
    Handler,
    /// The Run's Model allowance was reached.
    ModelAllowance,
    /// Its response's content type is not one its Label takes.
    ContentType,
}

/// The crawl request an outcome ends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OutcomeRequest {
    /// Its ID within the Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// Its URL.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Its Label.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub label: String,
    /// How many links away from a seed it is: 0 for a seed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub depth: u32,
}

/// The error an attempt failed with.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AttemptError {
    /// Which attempt it was, from 1.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempt: u32,
    /// Its Error code.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: String,
    /// What it said.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// The error's own details, as its Error code defines them, such as the
    /// time a site's `Retry-After` asked for and a `fix`; left out when it
    /// has none.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub details: Map<String, Value>,
}

/// What a succeeded request's commit did with its Records in one Dataset
/// (ADR 0103).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct DatasetRecords {
    /// The Dataset's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dataset: String,
    /// How many Records it committed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub committed: u64,
    /// How many a Hook dropped.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub dropped: u64,
    /// How many the Dataset's schema rejected.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub rejected: u64,
}

/// Why a succeeded request committed no Record (ADR 0103).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum NothingCommitted {
    /// Its handler pushed none.
    NonePushed,
    /// Every Record it pushed was dropped or rejected, as its `records`
    /// count.
    NoneKept,
    /// `dataset.records` is off, so the crawl keeps no Record (ADR 0138).
    RecordsOff,
}

/// A request an attempt's enqueue, `enqueueLinks` call or seed answered
/// `added`, with its ID, that the attempt's commit did not add (ADR 0093):
/// the commit admits each staged request again on the run file as it then
/// stands, and another attempt's commit may have added the request
/// meanwhile, or a rule learned since refuses it. Its ID names no crawl
/// request.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct NotAddedAtCommit {
    /// The ID its staging answered.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: String,
    /// Its URL, as parsed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub url: String,
    /// Why the commit did not add it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: NotAddedReason,
    /// The crawl request it is a duplicate of, when another request was
    /// admitted under its identity first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub duplicate_of: Option<String>,
}

/// How a crawl request ended, with what that ending carries.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Ending {
    /// Its handler returned, and what it staged was committed.
    Succeeded {
        /// Per Dataset its handler pushed into, the Records committed,
        /// dropped and rejected.
        #[schemars(extend("arrived_in" = "0.1"))]
        records: Vec<DatasetRecords>,
        /// Why it committed no Record, when it committed none.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        nothing_committed: Option<NothingCommitted>,
        /// Each request its attempt's enqueues, `enqueueLinks` calls and
        /// seeds answered `added` that the commit did not add, in order.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[schemars(extend("arrived_in" = "0.1"))]
        not_added: Vec<NotAddedAtCommit>,
    },
    /// Its last attempt failed, and no retry is left.
    Failed {
        /// The last attempt's error.
        #[schemars(extend("arrived_in" = "0.1"))]
        error: AttemptError,
    },
    /// A decision ended it.
    Skipped {
        /// Why.
        #[schemars(extend("arrived_in" = "0.1"))]
        reason: SkipReason,
        /// The handler's or the Hook's own text.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        #[schemars(extend("arrived_in" = "0.1"))]
        message: Option<String>,
    },
    /// The site refused it.
    Blocked {
        /// Why the site refused it.
        #[schemars(extend("arrived_in" = "0.1"))]
        cause: BlockCause,
    },
}

impl Ending {
    /// Its kind.
    pub const fn kind(&self) -> OutcomeKind {
        match self {
            Self::Succeeded { .. } => OutcomeKind::Succeeded,
            Self::Failed { .. } => OutcomeKind::Failed,
            Self::Skipped { .. } => OutcomeKind::Skipped,
            Self::Blocked { .. } => OutcomeKind::Blocked,
        }
    }
}

/// How one crawl request ended (ADRs 0079 and 0102). A request that
/// `run.retry_failed` re-admits ends again, and its later outcome
/// supersedes this one: a reader keys outcomes by `request.id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlRequestOutcome {
    /// Its place in the Run's commit order, from 1, which `run.outcomes`'s
    /// `after` takes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub position: u64,
    /// The request it ends.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub request: OutcomeRequest,
    /// How many attempts it took: retries are a count, not an outcome.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub attempts: u32,
    /// The error of each failed attempt, in order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub errors: Vec<AttemptError>,
    /// When it ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended_at: PointInTime,
    /// How it ended.
    #[serde(flatten)]
    pub ending: Ending,
}

/// The word `now`: from the next commit on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub enum Now {
    /// From the next commit on.
    #[serde(rename = "now")]
    Now,
}

/// Where `run.outcomes` starts: after a position, or from the next commit.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum OutcomesAfter {
    /// After the outcome at this position, which a reader last read.
    Position(u64),
    /// From the next commit on.
    Now(Now),
}

/// What `run.outcomes` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunOutcomesParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Only outcomes of these kinds; absent, every kind.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kinds: Option<Vec<OutcomeKind>>,
    /// Only outcomes of requests of these Labels; absent, every Label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub labels: Option<Vec<String>>,
    /// Only outcomes of requests one of whose failed attempts failed with
    /// one of these Error codes; absent, every outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub codes: Option<Vec<String>>,
    /// Where to start: after this position, or `now` for the next commit
    /// on; absent, from the first outcome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub after: Option<OutcomesAfter>,
}

/// One page of `run.outcomes`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CrawlRequestOutcomes {
    /// The outcomes that match, in commit order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub outcomes: Vec<CrawlRequestOutcome>,
    /// The position the next page starts after.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub next: u64,
    /// Whether the sequence is over: the Run has ended, or has no live
    /// writer, and nothing committed after `next` matches.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended: bool,
}

/// What `run.failed_requests` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunFailedRequestsParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Also the requests whose latest outcome is `blocked`.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub blocked: bool,
}

/// What `run.failed_requests` answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct FailedRequests {
    /// The latest outcome of each such request, in commit order.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub requests: Vec<CrawlRequestOutcome>,
}

/// What `run.retry_failed` takes.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct RunRetryFailedParams {
    /// The crawl's Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Also the requests whose latest outcome is `blocked`.
    #[serde(default)]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub blocked: bool,
    /// The Run's settings from the retry on, nested by key, as a resume
    /// takes them: read only when the retry returns an ended Run to
    /// `running` (ADRs 0083 and 0102).
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Map<String, Value>,
    /// The project configuration the Client found, layer 3 (ADR 0083),
    /// read as `settings` is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub project: Option<ProjectSettings>,
}

/// What `run.retry_failed` answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RetriedFailed {
    /// How many requests it re-admitted: 0 says there was nothing to retry.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub readmitted: u64,
}

/// `run.outcomes`.
pub const RUN_OUTCOMES: Operation = Operation {
    name: "run.outcomes",
    description: "Reads a crawl's outcomes from its committed state, in commit order: from the first, after a position, or with `after: \"now\"` from the next commit. Its filters, `kinds`, `labels` and `codes` (an Error code any of the request's failed attempts failed with), run in the engine. While the Run is live and nothing new matches, it waits for the next commit; a page says `ended` once the Run has ended, or has no live writer, and nothing more matches. A reader never slows the Run. A request `run.retry_failed` re-admits ends again, and its later outcome supersedes the earlier one by `request.id`.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunOutcomesParams>,
    result: schema_of::<CrawlRequestOutcomes>,
    events: &[],
    cancellable: true,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.failed_requests`.
pub const RUN_FAILED_REQUESTS: Operation = Operation {
    name: "run.failed_requests",
    description: "Reads from a crawl's committed state the requests whose latest outcome is `failed`, and with `blocked` also `blocked`, each with that outcome: exactly those `run.retry_failed` re-admits with the same option. A skip is a decision and is never listed.",
    class: OperationClass::Read,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunFailedRequestsParams>,
    result: schema_of::<FailedRequests>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// `run.retry_failed`.
pub const RUN_RETRY_FAILED: Operation = Operation {
    name: "run.retry_failed",
    description: "Re-admits, in the same Run and its Datasets, every request of a crawl whose latest outcome is `failed`, and with `blocked` also `blocked`: exactly those `run.failed_requests` lists with the same option. Each keeps its ID, Label, Payload and depth; its attempts count on, with a fresh `crawl.retry.max`. A skip is a decision and is never re-admitted. It answers how many it re-admitted, 0 when there was nothing to retry. On a crawl this engine runs, the requests join its work at once, unless the crawl is already ending its Run: the call then waits for that end and re-admits them as on an ended Run. On an ended Run with requests to retry, the Run returns to `running` under `settings`, as a resume does, its Verdict and Completeness given anew when it ends, and the calling Connection serves its handlers. A Run with no writer that has not ended fails with `run_interrupted`: resume it first. It is not sent again after a lost Connection, which fails it with `connection_lost`: `run.failed_requests` then lists the requests still to retry.",
    class: OperationClass::Effect,
    kind: OperationKind::RoundTrip,
    reach: Reach::AllSurfaces,
    params: schema_of::<RunRetryFailedParams>,
    result: schema_of::<RetriedFailed>,
    events: &[],
    cancellable: false,
    input: None,
    bound: None,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[RUN_OUTCOMES, RUN_FAILED_REQUESTS, RUN_RETRY_FAILED],
    events: &[],
    handler_requests: &[],
    error_codes: &[],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
