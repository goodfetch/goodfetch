//! What a Run states about itself when it ends, its Run report (ADR 0102):
//! the three axes with the reasons it is not `verified` or not `complete`
//! (ADR 0029), its Usage with the Limits that apply to it (ADRs 0041 and
//! 0101), the settings that weakened a safeguard (ADR 0028), the parts
//! each area supplies (the Plan, the Plugins, the Response recording, the
//! uncertain Effects), its warnings, and what a crawl adds, kept as the
//! engine sent it so that a Collie reader keeps a crawl's parts.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::settings::{ExplainedSetting, SettingLimit, WeakenedSetting};

use super::{Checkpoint, Completeness, Lifecycle, LimitScope, Period, PointInTime, RunId};
use super::{StopReason, Verdict};

/// What a Run states about itself when it ends (ADR 0102). A part no area
/// supplied is absent, never zero.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunReport {
    /// The Run's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Where it ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub lifecycle: Lifecycle,
    /// Why it stopped, when it did.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stop_reason: Option<StopReason>,
    /// Whether its own checks prove what it committed.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub verdict: Verdict,
    /// Why it is not `verified`, when it is not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub verdict_reasons: Vec<AxisReason>,
    /// Whether its results cover its scope.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub completeness: Completeness,
    /// Why it is not `complete`, when it is not.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub completeness_reasons: Vec<AxisReason>,
    /// When it started.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub started_at: PointInTime,
    /// When it ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended_at: PointInTime,
    /// What it used, as the Usage ledger charged it, with the Limits that
    /// apply to it (ADR 0101).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage: RunUsage,
    /// Every setting that weakened a safeguard for it (ADR 0028).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub weakened: Vec<WeakenedSetting>,
    /// What it did with its Plan file (ADR 0091).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plan: Option<PlanReport>,
    /// The version of the Operator policy it ran under, when one applied
    /// (ADR 0031).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub policy_version: Option<String>,
    /// Each effective value it ran with, the layer that set it and the one
    /// that locked it, as `config.explain` gives them (ADR 0028).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: Option<Vec<ExplainedSetting>>,
    /// Each Plugin that ran (ADRs 0016 and 0033).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub plugins: Option<Vec<PluginRun>>,
    /// What the areas warn of, each with a code and a message.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub warnings: Vec<RunWarning>,
    /// What it did with the Response recording its start named (ADR 0105).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recording: Option<RecordingReport>,
    /// The last Checkpoint written of it in this engine, when one was.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub checkpoint: Option<Checkpoint>,
    /// Why the call that answered it did nothing, when it did nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub nothing_done: Option<NothingDone>,
    /// Each Effect it left uncertain, until a person reconciles it (ADR
    /// 0106).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub uncertain_effects: Option<Vec<UncertainEffect>>,
    /// The error the call that ends it raises: the one that failed a
    /// `failed` Run, or a write at its end that failed, such as its Plan
    /// file's (ADR 0116).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: Option<EndingError>,
    /// What its kind adds, such as a crawl's `requests`, `records`,
    /// `sinks`, `learned_rules` and `sitemaps` (Foxhound's
    /// `CrawlRunReport`), kept as the engine sent them.
    #[serde(flatten)]
    pub kind_parts: Map<String, Value>,
}

impl RunReport {
    /// Gives it the Verdict `needs_review` for `reason`, which it lists
    /// (ADR 0029). A `rejected` Verdict stays: a failed Success check
    /// outranks every other reason, and a reason never raises a Verdict.
    pub fn needs_review(&mut self, reason: AxisReason) {
        if self.verdict == Verdict::Verified {
            self.verdict = Verdict::NeedsReview;
        }
        self.verdict_reasons.push(reason);
    }

    /// Whether it replayed a Response recording (ADR 0105), which marks it
    /// in its Run index row.
    pub fn replayed(&self) -> bool {
        self.recording
            .as_ref()
            .is_some_and(|recording| recording.mode == "replay")
    }
}

/// One reason a Run is not `verified` or not `complete` (ADR 0029).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AxisReason {
    /// What it is, such as `stopped` or `failed`; a reason this reader does
    /// not know is kept as it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: String,
    /// What it says, for people.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// How many it counts, when it counts some: the URLs a bound refused,
    /// the pages left at a Block cause, or the requests a stop left.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: Option<u64>,
    /// The AI task it is about, by its `callId`, when it is about one (ADR
    /// 0100).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub call_id: Option<String>,
}

/// An Error code a Run's ending raises, with its message and details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EndingError {
    /// The Error code.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: String,
    /// What it says, for people.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// The code's own details.
    #[serde(default, skip_serializing_if = "Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub details: Map<String, Value>,
}

/// Who set a Limit, which decides whether it allows paying (ADR 0045).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum LimitSetter {
    /// A person, in the Kennel or an interactive CLI prompt.
    Person,
    /// A managed Operator policy.
    ManagedPolicy,
    /// A Client, which only adds a Limit of its own.
    Client,
}

impl LimitSetter {
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Person, Self::ManagedPolicy, Self::Client];

    /// Its name on the wire and in the Usage ledger.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// What a Run used, as the Usage ledger charged it, and the Limits that
/// apply to it (ADRs 0041 and 0101): what `run.usage` answers and the
/// report carries.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunUsage {
    /// Its model calls.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub model: ModelUsage,
    /// How long it held a browser, in milliseconds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub browser_time: u64,
    /// The bytes it sent and received through a proxy.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_bytes: u64,
    /// The bytes it sent and received through a Proxy provider's proxies,
    /// by proxy kind (ADR 0021), cheapest kind first.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_bytes_by_kind: Vec<ProxyKindBytes>,
    /// The price of those bytes, by currency, at each provider's configured
    /// price (ADR 0101).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub proxy_price: Vec<UsageAmount>,
    /// The requests it sent.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub requests: u64,
    /// Its paid solves, by currency (ADR 0035).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub paid_solves: Vec<UsageAmount>,
    /// Its Paid fetches, by currency (ADR 0045).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub paid_fetches: Vec<UsageAmount>,
    /// What it used of a Usage kind this engine does not name, kept as the
    /// ledger holds it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub other: Vec<KindUsage>,
    /// Each Limit that applies to it, with what is left of it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limits: Vec<AppliedLimit>,
    /// Its Model allowance, one entry per kind set (ADR 0107): not a Limit,
    /// so apart from `limits`; empty when none is set.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub model_allowance: Vec<ModelAllowance>,
    /// Whether it spent, or reserved, money in a currency no Limit holding
    /// it caps (ADR 0041): *uncapped*.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub uncapped: bool,
}

/// One kind of a Run's Model allowance (ADR 0107): the most model Usage
/// of that kind the Run may spend, past which its model calls are refused
/// while it goes on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelAllowance {
    /// Its kind: `price`, `tokens` or `calls`; a kind this reader does not
    /// know is kept as it is.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// The currency of a `price` allowance, an ISO 4217 code.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// What it allows: a count, or millionths of the currency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub value: u64,
    /// What remains of it, spent and reserved taken off.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub remains: u64,
    /// When a model call was first refused under it; none until then.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reached_at: Option<PointInTime>,
    /// The setting that set it, its value and its layer.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub source: SettingLimit,
}

/// A Run's model calls (ADR 0101).
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ModelUsage {
    /// The calls, retries and Repairs included.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub calls: u64,
    /// The tokens, input and output.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub tokens: u64,
}

/// The bytes a Run sent and received through proxies of one proxy kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ProxyKindBytes {
    /// The proxy kind, such as `residential`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub kind: String,
    /// The bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub bytes: u64,
}

/// An amount of a Usage kind counted in a currency, or as a count.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UsageAmount {
    /// The currency, an ISO 4217 code; none for a count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// A count, or millionths of the currency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: u64,
}

/// What a Run used of one Usage kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct KindUsage {
    /// The Usage kind, a value of a type both Products share.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// The currency, an ISO 4217 code; none for a count.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// A count, or millionths of the currency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: u64,
}

/// A Limit that applies to a Run (ADRs 0041 and 0101): its scope's kind and
/// ID, what it caps, its period, what it allows, what is left of it and who
/// set it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct AppliedLimit {
    /// What it is on; a scope kind this reader does not know is kept.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub scope: LimitScope,
    /// The Usage kind it caps.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub usage_kind: String,
    /// The currency it caps a price in, for a price Limit.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub currency: Option<String>,
    /// Its period; none for a Limit that never starts again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub period: Option<Period>,
    /// When its period starts again; none for a Limit without one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub resets_at: Option<PointInTime>,
    /// What it allows per period: a count, or millionths of the currency.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub amount: u64,
    /// What is left of it in the current period.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub left: u64,
    /// Who set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub set_by: LimitSetter,
}

/// What a Run did with its Plan file (ADR 0091).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PlanReport {
    /// The Plan file's path, or none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub file: Option<String>,
    /// The version it used.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub version: Option<u64>,
    /// What became of its new versions.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub status: PlanStatus,
    /// The steps that changed, each Agent task one entry (ADR 0100).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub steps_changed: Vec<String>,
}

/// What became of a Run's new Plan versions (ADR 0091).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PlanStatus {
    /// Nothing changed.
    Unchanged,
    /// They were written to the Plan file.
    Promoted,
    /// They wait for a person.
    PendingReview,
    /// The Plan file is frozen.
    Frozen,
}

/// A Plugin that ran in a Run (ADRs 0016 and 0033).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PluginRun {
    /// Its name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The network level it ran at.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub network_level: String,
    /// The parts of its confinement the host could not give, under a
    /// reduced confinement the Operator policy allowed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub confinement_missing: Vec<String>,
}

/// What a Run warns of (ADR 0102): a code of the warning table, or of a
/// newer engine's, kept as it is, with its message and what it concerns.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RunWarning {
    /// Its code.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: String,
    /// What it says, for people.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// The Sessions it concerns.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub sessions: Vec<String>,
    /// The folder it concerns.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub folder: Option<String>,
}

/// What a Run did with the Response recording its start named (ADR 0105).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct RecordingReport {
    /// Its mode, such as `record` or `replay`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub mode: String,
    /// Its file.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub file: String,
    /// How many recorded responses it used.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub responses_used: u64,
    /// When the oldest of them was fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recorded_from: Option<PointInTime>,
    /// When the newest of them was fetched.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub recorded_to: Option<PointInTime>,
    /// The engine that wrote them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub written_by: Option<String>,
    /// Each request the recording missed.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub misses: Vec<String>,
}

/// Why a call that ends a Run did nothing (ADR 0102's G1).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "reason", rename_all = "snake_case")]
pub enum NothingDone {
    /// The start found the Run ended.
    AlreadyEnded {
        /// When it ended.
        #[schemars(extend("arrived_in" = "0.1"))]
        ended_at: PointInTime,
    },
    /// No seed reached a handler: each was refused when added or ended
    /// before its handler ran, counted by why.
    NoSeedHandled {
        /// How many seeds each reason kept from a handler: the reason an
        /// enqueue answered, or how the request ended, by its skip reason,
        /// Error code or Block cause, in the order of the reasons.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        #[schemars(extend("arrived_in" = "0.1"))]
        seeds: Vec<SeedsKept>,
    },
    /// A resume or a retry found nothing left.
    NothingPending,
}

/// How many seeds one reason kept from a handler.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SeedsKept {
    /// The reason: an enqueue's not-added reason, a skip reason, an Error
    /// code or a Block cause.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// How many seeds it kept.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub count: u64,
}

/// An Effect a Run left uncertain (ADR 0106), until a person reconciles it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct UncertainEffect {
    /// Its Effect journal entry.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub entry: String,
    /// Its key, for a declared Effect.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: Option<String>,
    /// Its instruction, for an AI action.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub instruction: Option<String>,
    /// The crawl request it ran for, in a crawl.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crawl_request: Option<String>,
    /// Why it is uncertain.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub reason: String,
    /// The first command sent for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub first_command: Option<String>,
    /// Since when it is uncertain.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub since: PointInTime,
}
