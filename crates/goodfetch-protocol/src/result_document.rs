//! The result document (ADRs 0063, 0069 and 0072): what one conformance
//! suite proved against one target, or one Journey variant proved on one
//! surface, written as one JSON document whose `kind` is `conformance` or
//! `journey`.
//!
//! Outcomes are derived, never set. [`ConformanceDocument::new`] derives each
//! case's and each capability's outcome from the checks its entries hold,
//! and [`JourneyDocument::new`] the variant's from its steps, checks and
//! oracle answers; neither type can be deserialized. Every reader (the
//! catalog report, the Feature check, the release gate) loads a document
//! through [`read`], which recomputes each outcome and refuses a document
//! whose stored outcome disagrees.
//!
//! The rules, which the TypeScript and Python helpers also follow and which
//! the outcome vectors of `test-support/fixtures/outcome-vectors/` state as
//! data:
//! - a case is `not_reached` with no entry, `passed` when every entry has at
//!   least one check and every check passed, and `failed` otherwise;
//! - a capability is `failed` when any case failed, `proved` when it has
//!   cases and every one passed, and `unproved` otherwise, so a capability
//!   the suite did not reach is never `proved` (ADR 0033);
//! - a Journey variant is `proved` when every automated step and every check
//!   passed and every named task is `verified`, a `manual` step aside, and
//!   `failed` otherwise (ADR 0063).
//!
//! The oracle's answers and the server-side observations are the Proving
//! ground's own JSON (it depends on no Goodfetch crate); this module reads
//! only an answer's task and outcome and keeps the rest as it came.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use schemars::JsonSchema;

use crate::catalog::Product;
use crate::journey::{FeatureId, JourneyId, TaskId};
use crate::releases::Sha256;

/// One result document, of either kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResultDocument {
    /// What a conformance suite proved against a target.
    Conformance(ConformanceDocument),
    /// What a Journey variant proved on a surface.
    Journey(JourneyDocument),
}

/// Loads a result document, recomputing every outcome it stores.
///
/// # Errors
///
/// [`ReadError::Malformed`] when the bytes are not a result document, and
/// the other variants when a stored outcome is not the one its document's
/// contents give.
pub fn read(bytes: &[u8]) -> Result<ResultDocument, ReadError> {
    match serde_json::from_slice(bytes)? {
        StoredDocument::Conformance(stored) => stored.verify().map(ResultDocument::Conformance),
        StoredDocument::Journey(stored) => stored.verify().map(ResultDocument::Journey),
    }
}

/// Why [`read`] refused a document.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum ReadError {
    /// Not a result document of either kind.
    #[error("not a result document: {0}")]
    Malformed(#[from] serde_json::Error),
    /// A case's stored outcome is not the one its entries give.
    #[error("case `{case}` is stored as `{}` but its entries give `{}`", stored.as_str(), derived.as_str())]
    CaseOutcome {
        /// The case's ID.
        case: String,
        /// The outcome the document stores.
        stored: CaseOutcome,
        /// The outcome its entries give.
        derived: CaseOutcome,
    },
    /// A capability's stored outcome is not the one its cases give.
    #[error("capability `{capability}` is stored as `{}` but its cases give `{}`", stored.as_str(), derived.as_str())]
    CapabilityOutcome {
        /// The capability's name.
        capability: String,
        /// The outcome the document stores.
        stored: Outcome,
        /// The outcome its cases give.
        derived: Outcome,
    },
    /// A Journey variant's stored outcome is not the one its steps, checks
    /// and oracle answers give.
    #[error("the Journey variant is stored as `{}` but its steps, checks and tasks give `{}`", stored.as_str(), derived.as_str())]
    JourneyOutcome {
        /// The outcome the document stores.
        stored: JourneyOutcome,
        /// The outcome its contents give.
        derived: JourneyOutcome,
    },
}

/// The operating system and architecture a document's tests ran on.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Platform {
    /// The operating system, as Rust's `std::env::consts::OS` names it
    /// (`linux`, `macos`, `windows`).
    pub os: String,
    /// The architecture, as Rust's `std::env::consts::ARCH` names it
    /// (`x86_64`, `aarch64`).
    pub arch: String,
}

/// The Proving ground a document's tests ran against: its version, which
/// pins its generator, and the seed it served (ADR 0063).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ProvingGroundCopy {
    /// The workspace version and the commit it was built from.
    pub version: String,
    /// The seed it served.
    pub seed: u64,
}

/// An oracle's answer for one Proving ground task: the task, the outcome,
/// and the rest of the Proving ground's answer as it came (what is missing,
/// extra or wrong, the tool's status, the oracle's time).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct OracleAnswer {
    /// The task judged.
    pub task: TaskId,
    /// The oracle's outcome.
    pub outcome: OracleOutcome,
    /// The answer's other fields.
    #[serde(flatten)]
    pub details: Map<String, Value>,
}

/// The outcome the Proving ground's oracle gives a submitted result, whatever
/// status the tool reported (ADR 0025).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum OracleOutcome {
    /// Correct and complete.
    Verified,
    /// Correct but missing Records, without claiming success.
    Incomplete,
    /// Claimed success with wrong or missing data.
    FalseSuccess,
    /// Neither correct nor a claimed success.
    Failed,
}

impl OracleOutcome {
    /// The outcome as a document writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Verified => "verified",
            Self::Incomplete => "incomplete",
            Self::FalseSuccess => "false_success",
            Self::Failed => "failed",
        }
    }
}

/// What a test saw, beside its checks.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Evidence {
    /// The oracle's answers.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub oracle_answers: Vec<OracleAnswer>,
    /// What the Proving ground observed on the server side, as it reported
    /// it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<Value>,
    /// The scripted messages exchanged, for a Client conformance case (ADR
    /// 0069).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub messages: Vec<Value>,
}

// The `conformance` kind.

/// What a `conformance` document records besides its capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct ConformanceHeader {
    /// The suite.
    pub suite: Suite,
    /// What the suite ran against.
    pub target: Target,
    /// Where it ran.
    pub platform: Platform,
    /// The runtime the cases ran on, such as `rust`, `node-24` or
    /// `cpython-3.14`.
    pub runtime: String,
    /// The Proving ground the cases ran against.
    pub proving_ground: ProvingGroundCopy,
    /// Whether the Proving ground was the online copy, whose results report
    /// and never block a release.
    pub online: bool,
}

/// A conformance suite.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Suite {
    /// Its name, such as `self-test`.
    pub name: String,
    /// The version of its case list.
    pub version: String,
}

/// What a suite ran against: an engine over the protocol, the Browser host,
/// a process Plugin, Goodfetch Cloud's engine or, for the self-test, the
/// Proving ground itself.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct Target {
    /// Its name.
    pub name: String,
    /// Its version or digest.
    #[serde(flatten)]
    pub identity: TargetIdentity,
    /// For an engine, the Product its `hello` reply reports (ADR 0095).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub product: Option<Product>,
}

/// How a document names which build of its target it ran against.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum TargetIdentity {
    /// The version it reports, for an engine its Product version.
    Version(String),
    /// The SHA-256 digest of its artifact, for a target with no version.
    Digest(Sha256),
}

/// One case of a capability before its outcome is derived: what the case
/// list declares of it and the entries the case log holds for it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct CaseFacts {
    /// The case's ID in its suite.
    pub id: String,
    /// The Features the case proves (ADR 0072).
    pub covers: Vec<FeatureId>,
    /// One entry per time the case ran, such as once per mode; none when it
    /// was not reached.
    pub entries: Vec<CaseEntry>,
}

/// One time a case ran.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseEntry {
    /// The mode it ran in, such as `attached` or `daemon`, when the case
    /// runs in several.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// Each check's result.
    pub checks: Vec<CaseCheck>,
    /// What it saw.
    #[serde(default)]
    pub evidence: Evidence,
}

/// One check of a case.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseCheck {
    /// What it checks.
    pub name: String,
    /// Whether it held.
    pub passed: bool,
}

/// One declared capability before its outcome is derived.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CapabilityFacts {
    /// Its name, such as a fetch backend's `http2` (ADR 0033).
    pub name: String,
    /// Every case the suite declares for it.
    pub cases: Vec<CaseFacts>,
}

/// A `conformance` document: each declared capability's outcome, derived
/// from its cases.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct ConformanceDocument {
    #[serde(flatten)]
    header: ConformanceHeader,
    capabilities: Vec<CapabilityResult>,
}

impl ConformanceDocument {
    /// Derives the outcome of every case and capability.
    pub fn new(header: ConformanceHeader, capabilities: Vec<CapabilityFacts>) -> Self {
        Self {
            header,
            capabilities: capabilities
                .into_iter()
                .map(CapabilityResult::new)
                .collect(),
        }
    }

    /// The suite, target, platform, runtime and Proving ground.
    pub fn header(&self) -> &ConformanceHeader {
        &self.header
    }

    /// Each declared capability, in the case list's order.
    pub fn capabilities(&self) -> &[CapabilityResult] {
        &self.capabilities
    }
}

/// One capability of a `conformance` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CapabilityResult {
    name: String,
    outcome: Outcome,
    cases: Vec<CaseResult>,
}

impl CapabilityResult {
    fn new(facts: CapabilityFacts) -> Self {
        let cases: Vec<CaseResult> = facts.cases.into_iter().map(CaseResult::new).collect();
        Self {
            name: facts.name,
            outcome: capability_outcome(&cases),
            cases,
        }
    }

    /// Its name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Its derived outcome.
    pub fn outcome(&self) -> Outcome {
        self.outcome
    }

    /// Its cases.
    pub fn cases(&self) -> &[CaseResult] {
        &self.cases
    }
}

/// One case of a capability, with its derived outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct CaseResult {
    #[serde(flatten)]
    facts: CaseFacts,
    outcome: CaseOutcome,
}

impl CaseResult {
    fn new(facts: CaseFacts) -> Self {
        Self {
            outcome: case_outcome(&facts.entries),
            facts,
        }
    }

    /// Its ID, the Features it covers and its entries.
    pub fn facts(&self) -> &CaseFacts {
        &self.facts
    }

    /// Its derived outcome.
    pub fn outcome(&self) -> CaseOutcome {
        self.outcome
    }
}

/// A capability's outcome (ADR 0033).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    /// Every declared case ran and passed.
    Proved,
    /// No case failed, but not every one ran, or none is declared.
    Unproved,
    /// A case failed.
    Failed,
}

impl Outcome {
    /// The outcome as a document writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proved => "proved",
            Self::Unproved => "unproved",
            Self::Failed => "failed",
        }
    }
}

/// A case's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaseOutcome {
    /// Every entry has checks and every check held.
    Passed,
    /// A check failed, or an entry holds no check.
    Failed,
    /// The case log holds no entry for it.
    NotReached,
}

impl CaseOutcome {
    /// The outcome as a document writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::NotReached => "not_reached",
        }
    }
}

fn case_outcome(entries: &[CaseEntry]) -> CaseOutcome {
    if entries.is_empty() {
        CaseOutcome::NotReached
    } else if entries
        .iter()
        .all(|entry| !entry.checks.is_empty() && entry.checks.iter().all(|check| check.passed))
    {
        CaseOutcome::Passed
    } else {
        CaseOutcome::Failed
    }
}

fn capability_outcome(cases: &[CaseResult]) -> Outcome {
    if cases.iter().any(|case| case.outcome == CaseOutcome::Failed) {
        Outcome::Failed
    } else if !cases.is_empty() && cases.iter().all(|case| case.outcome == CaseOutcome::Passed) {
        Outcome::Proved
    } else {
        Outcome::Unproved
    }
}

// The `journey` kind.

/// What a `journey` document holds before its outcome is derived (ADRs
/// 0063, 0084 and 0090).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct JourneyFacts {
    /// The Journey.
    pub journey_id: JourneyId,
    /// The variant's surface, by its name in ADR 0063, or `foxhound-cli`
    /// for a Collie Journey's `cli` variant run on Foxhound's executable
    /// (ADR 0094).
    pub surface: String,
    /// The executable the variant drove.
    pub executable: Executable,
    /// The surface's package, for a surface that has one (an SDK, the
    /// Console's build, an adapter).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub surface_package: Option<SurfacePackage>,
    /// Where it ran.
    pub platform: Platform,
    /// The runtime it ran on.
    pub runtime: String,
    /// The Proving ground it started, for a Journey whose goal touches a
    /// page.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proving_ground: Option<ProvingGroundCopy>,
    /// Each step of the Journey, in its order.
    pub steps: Vec<StepResult>,
    /// Each check of the Journey's outcome, in its order.
    pub checks: Vec<CheckResult>,
    /// Each Proving ground task the Journey names.
    pub tasks: Vec<TaskResult>,
}

/// The executable a Journey variant drove: its Product and the Product
/// version it reports (ADRs 0084 and 0090).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Executable {
    /// Its Product.
    pub product: Product,
    /// Its Product version.
    pub version: String,
}

/// The package of a Journey variant's surface.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct SurfacePackage {
    /// Its name in its registry.
    pub name: String,
    /// Its version.
    pub version: String,
}

/// One step's outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StepResult {
    /// The step's text in the Journey.
    pub text: String,
    /// What happened.
    pub outcome: StepOutcome,
}

/// What happened to a step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StepOutcome {
    /// It ran and held.
    Passed,
    /// It ran and did not hold.
    Failed,
    /// An earlier failure stopped the variant before it.
    NotReached,
    /// It needs a person; the owner test covers it.
    Manual,
}

/// One check's outcome.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CheckResult {
    /// The check's text in the Journey.
    pub text: String,
    /// The task whose oracle answer the check reads, when it names one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<TaskId>,
    /// What happened.
    pub outcome: CheckOutcome,
}

/// What happened to a check.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CheckOutcome {
    /// It held.
    Passed,
    /// It did not hold.
    Failed,
    /// An earlier failure stopped the variant before it.
    NotReached,
}

/// One named Proving ground task: the oracle's answer and the server-side
/// observations.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TaskResult {
    /// The task.
    pub id: TaskId,
    /// The oracle's answer; none when the variant never submitted a result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub answer: Option<OracleAnswer>,
    /// What the Proving ground observed for the task.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub observations: Vec<Value>,
}

/// A `journey` document: one Journey variant's outcome, derived from its
/// steps, checks and oracle answers.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
pub struct JourneyDocument {
    #[serde(flatten)]
    facts: JourneyFacts,
    outcome: JourneyOutcome,
}

impl JourneyDocument {
    /// Derives the variant's outcome.
    pub fn new(facts: JourneyFacts) -> Self {
        Self {
            outcome: journey_outcome(&facts),
            facts,
        }
    }

    /// What the document holds besides its outcome.
    pub fn facts(&self) -> &JourneyFacts {
        &self.facts
    }

    /// The derived outcome.
    pub fn outcome(&self) -> JourneyOutcome {
        self.outcome
    }
}

/// A Journey variant's outcome. A declared variant with no document is
/// `unproved`, which the catalog report computes from the document's
/// absence (ADR 0063).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum JourneyOutcome {
    /// Every automated step and check passed and every named task is
    /// `verified`.
    Proved,
    /// Anything else.
    Failed,
}

impl JourneyOutcome {
    /// The outcome as a document writes it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Proved => "proved",
            Self::Failed => "failed",
        }
    }
}

fn journey_outcome(facts: &JourneyFacts) -> JourneyOutcome {
    let steps = facts
        .steps
        .iter()
        .all(|step| matches!(step.outcome, StepOutcome::Passed | StepOutcome::Manual));
    let checks = facts
        .checks
        .iter()
        .all(|check| check.outcome == CheckOutcome::Passed);
    let tasks = facts.tasks.iter().all(|task| {
        task.answer
            .as_ref()
            .is_some_and(|answer| answer.outcome == OracleOutcome::Verified)
    });
    if steps && checks && tasks {
        JourneyOutcome::Proved
    } else {
        JourneyOutcome::Failed
    }
}

// What a stored document holds, before `read` recomputes its outcomes.

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum StoredDocument {
    Conformance(StoredConformance),
    Journey(StoredJourney),
}

#[derive(Deserialize)]
struct StoredConformance {
    #[serde(flatten)]
    header: ConformanceHeader,
    capabilities: Vec<StoredCapability>,
}

#[derive(Deserialize)]
struct StoredCapability {
    name: String,
    outcome: Outcome,
    cases: Vec<StoredCase>,
}

#[derive(Deserialize)]
struct StoredCase {
    #[serde(flatten)]
    facts: CaseFacts,
    outcome: CaseOutcome,
}

#[derive(Deserialize)]
struct StoredJourney {
    #[serde(flatten)]
    facts: JourneyFacts,
    outcome: JourneyOutcome,
}

impl StoredConformance {
    fn verify(self) -> Result<ConformanceDocument, ReadError> {
        let mut capabilities = Vec::with_capacity(self.capabilities.len());
        for stored in self.capabilities {
            let mut cases = Vec::with_capacity(stored.cases.len());
            for case in stored.cases {
                let derived = CaseResult::new(case.facts);
                if derived.outcome != case.outcome {
                    return Err(ReadError::CaseOutcome {
                        case: derived.facts.id,
                        stored: case.outcome,
                        derived: derived.outcome,
                    });
                }
                cases.push(derived.facts);
            }
            let derived = CapabilityResult::new(CapabilityFacts {
                name: stored.name,
                cases,
            });
            if derived.outcome != stored.outcome {
                return Err(ReadError::CapabilityOutcome {
                    capability: derived.name,
                    stored: stored.outcome,
                    derived: derived.outcome,
                });
            }
            capabilities.push(derived);
        }
        Ok(ConformanceDocument {
            header: self.header,
            capabilities,
        })
    }
}

impl StoredJourney {
    fn verify(self) -> Result<JourneyDocument, ReadError> {
        let derived = JourneyDocument::new(self.facts);
        if derived.outcome == self.outcome {
            Ok(derived)
        } else {
            Err(ReadError::JourneyOutcome {
                stored: self.outcome,
                derived: derived.outcome,
            })
        }
    }
}

#[cfg(test)]
mod tests;
