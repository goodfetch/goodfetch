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
//! whose stored outcome disagrees. A `journey` document then goes through
//! [`read_journey_document`], the only way to a [`JourneyDocument`] from
//! stored bytes, which refuses one whose steps, checks and named Proving
//! ground tasks are not exactly its Journey's, in order (ADR 0063).
//!
//! The rules, which the TypeScript and Python helpers also follow and which
//! the outcome vectors of `test-support/fixtures/outcome-vectors/` state as
//! data:
//! - a case is `not_reached` with no entry, `passed` when every entry has at
//!   least one check and every check passed, and `failed` otherwise;
//! - in gate mode (ADR 0047), and only there, each mode's entries are a pair, one with
//!   Filter-list blocking on and one with it off: the mode passed when its
//!   entry with blocking on passed, failed when that entry failed and the
//!   one with blocking off passed, and is otherwise `inconclusive`, as a
//!   vendor's scoring change moves both; a case is `failed` when any mode
//!   failed, `inconclusive` when none failed and one is inconclusive, and
//!   `passed` otherwise;
//! - a capability is `failed` when any case failed, `proved` when it has
//!   cases and every one passed, and `unproved` otherwise, so a capability
//!   the suite did not reach, or one with an inconclusive case, is never
//!   `proved` (ADR 0033);
//! - a document blocks its run when a case failed, unless it is an online
//!   result outside gate mode, which only reports (ADR 0043);
//! - a Journey variant is `proved` when every automated step and every check
//!   passed, every named task is `verified` and no error failed its test
//!   outside them (`test_errors`), a `manual` step aside, and `failed`
//!   otherwise (ADR 0063).
//!
//! The oracle's answers and the server-side observations are the Proving
//! ground's own JSON (it depends on no Goodfetch crate); this module reads
//! only an answer's task and outcome and keeps the rest as it came.
//!
//! Size: one contract, the result document's shape with the rules that
//! derive its outcomes and the reader that refuses a stored document breaking
//! them, kept whole so every rule sits beside the types it constrains.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use schemars::JsonSchema;

use crate::catalog::Product;
use crate::journey::{Check, FeatureId, Journey, JourneyId, TaskId};
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

impl ResultDocument {
    /// The `journey` document of a variant's facts, its outcome derived:
    /// the constructor a Rust Journey test passes to
    /// `goodfetch-test-support`'s `JourneyRun`, which records the facts.
    pub fn journey(facts: JourneyFacts) -> Self {
        Self::Journey(JourneyDocument::new(facts))
    }
}

/// Loads a result document, recomputing every outcome it stores. A
/// `journey` document comes back not yet compared with its Journey: only
/// [`read_journey_document`] turns it into a [`JourneyDocument`].
///
/// # Errors
///
/// [`ReadError::Malformed`] when the bytes are not a result document, and
/// [`ReadError::CaseOutcome`], [`ReadError::CapabilityOutcome`] or
/// [`ReadError::JourneyOutcome`] when a stored outcome is not the one its
/// document's contents give.
pub fn read(bytes: &[u8]) -> Result<LoadedDocument, ReadError> {
    match serde_json::from_slice(bytes)? {
        StoredDocument::Conformance(stored) => stored.verify().map(LoadedDocument::Conformance),
        StoredDocument::Journey(stored) => stored
            .verify()
            .map(|document| LoadedDocument::Journey(UncomparedJourneyDocument(document))),
    }
}

/// What [`read`] loads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadedDocument {
    /// A `conformance` document, its outcomes recomputed.
    Conformance(ConformanceDocument),
    /// A `journey` document, its outcome recomputed, still to be compared
    /// with its Journey.
    Journey(UncomparedJourneyDocument),
}

/// A `journey` document whose outcome [`read`] recomputed but which is not
/// compared with its Journey yet. A reader can only ask which Journey it
/// names and pass both to [`read_journey_document`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UncomparedJourneyDocument(JourneyDocument);

impl UncomparedJourneyDocument {
    /// The Journey the document says it ran.
    pub fn journey_id(&self) -> &JourneyId {
        &self.0.facts.journey_id
    }
}

/// The surface of a Collie Journey's `cli` variant run on Foxhound's
/// executable (ADR 0094).
const FOXHOUND_CLI: &str = "foxhound-cli";

/// Compares a loaded `journey` document with its Journey (ADR 0063). The
/// document must name the Journey and hold exactly its steps, its checks
/// and the Proving ground tasks it names, each in the Journey's order, with
/// a step recorded `manual` exactly when the Journey marks it manual. Each
/// check naming a task is judged on its own submission, so a task is held
/// once per check that names it, the answers in the checks' order, and
/// once when no check names it. A
/// helper that leaves a failed or unreached step out of its document
/// therefore writes one every reader refuses.
///
/// # Errors
///
/// [`ReadError::OtherJourney`] when the document names another Journey,
/// [`ReadError::UnlistedSurface`] when its surface is not one of the
/// Journey's variants, [`ReadError::OtherSeed`] when its Proving ground
/// seed is not the Journey's, [`ReadError::AnswerOfAnotherTask`] when a
/// task holds an answer naming another task,
/// [`ReadError::NotTheJourneys`] when a step, check or task is missing,
/// extra or out of order, and [`ReadError::ManualStep`] when a step is
/// recorded `manual` against its Journey's word, or the other way round.
pub fn read_journey_document(
    document: UncomparedJourneyDocument,
    journey: &Journey,
) -> Result<JourneyDocument, ReadError> {
    let UncomparedJourneyDocument(document) = document;
    let facts = &document.facts;
    if facts.journey_id != journey.id {
        return Err(ReadError::OtherJourney {
            document: facts.journey_id.clone(),
            journey: journey.id.clone(),
        });
    }
    // A Collie Journey's `cli` variant also runs on Foxhound's executable
    // as `foxhound-cli` (ADR 0094).
    let listed = journey.surfaces.contains_key(&facts.surface)
        || (facts.surface == FOXHOUND_CLI
            && journey.product == Product::Collie
            && journey.surfaces.contains_key("cli"));
    if !listed {
        return Err(ReadError::UnlistedSurface {
            surface: facts.surface.clone(),
        });
    }
    let seeds = (
        facts.proving_ground.as_ref().map(|copy| copy.seed),
        journey.proving_ground.as_ref().map(|table| table.seed),
    );
    if seeds.0 != seeds.1 {
        return Err(ReadError::OtherSeed {
            document: seeds.0,
            journey: seeds.1,
        });
    }
    compare(
        JourneyPart::Step,
        &journey
            .steps
            .iter()
            .map(|step| step.text.as_str())
            .collect::<Vec<_>>(),
        &facts
            .steps
            .iter()
            .map(|step| step.text.as_str())
            .collect::<Vec<_>>(),
        |text| (*text).to_owned(),
    )?;
    for (index, (expected, found)) in journey.steps.iter().zip(&facts.steps).enumerate() {
        let recorded_manual = found.outcome == StepOutcome::Manual;
        if expected.manual.is_some() != recorded_manual {
            return Err(ReadError::ManualStep {
                number: index + 1,
                text: found.text.clone(),
                recorded_manual,
            });
        }
    }
    // The variant's checks: those its Journey scopes to other variants'
    // surfaces are not its own.
    let checks: Vec<&Check> = journey
        .expect
        .iter()
        .filter(|check| check.belongs_to(&facts.surface))
        .collect();
    compare(
        JourneyPart::Check,
        &checks
            .iter()
            .map(|check| (check.text.as_str(), check.task.as_ref()))
            .collect::<Vec<_>>(),
        &facts
            .checks
            .iter()
            .map(|check| (check.text.as_str(), check.task.as_ref()))
            .collect::<Vec<_>>(),
        |(text, _)| (*text).to_owned(),
    )?;
    // Each check naming a task is judged on its own submission, so a task
    // two checks name is held once per check; a task no check names, once.
    let named: Vec<&TaskId> = journey
        .proving_ground
        .iter()
        .flat_map(|proving_ground| proving_ground.tasks.iter().map(|task| &task.id))
        .flat_map(|id| {
            let naming = checks
                .iter()
                .filter(|check| check.task.as_ref() == Some(id))
                .count();
            std::iter::repeat_n(id, naming.max(1))
        })
        .collect();
    compare(
        JourneyPart::Task,
        &named,
        &facts.tasks.iter().map(|task| &task.id).collect::<Vec<_>>(),
        |id| id.as_str().to_owned(),
    )?;
    if let Some((task, answer)) = facts.tasks.iter().find_map(|task| {
        task.answer
            .as_ref()
            .filter(|answer| answer.task != task.id)
            .map(|answer| (task, answer))
    }) {
        return Err(ReadError::AnswerOfAnotherTask {
            task: task.id.clone(),
            answered: answer.task.clone(),
        });
    }
    Ok(document)
}

/// Fails at the first place a document's list differs from its Journey's:
/// an item the document holds more times than the Journey is extra, one it
/// holds fewer times is missing, and otherwise the two hold the same items
/// in another order.
fn compare<T: PartialEq>(
    part: JourneyPart,
    expected: &[T],
    found: &[T],
    text: impl Fn(&T) -> String,
) -> Result<(), ReadError> {
    let count = |list: &[T], item: &T| list.iter().filter(|other| *other == item).count();
    let differs = |difference: Difference, index: usize, item: &T| ReadError::NotTheJourneys {
        part,
        difference,
        number: index + 1,
        text: text(item),
    };
    for index in 0..expected.len().max(found.len()) {
        match (expected.get(index), found.get(index)) {
            (Some(wanted), Some(held)) if wanted == held => {}
            (_, Some(held)) if count(found, held) > count(expected, held) => {
                return Err(differs(Difference::Extra, index, held));
            }
            (Some(wanted), _) if count(found, wanted) < count(expected, wanted) => {
                return Err(differs(Difference::Missing, index, wanted));
            }
            (_, Some(held)) => return Err(differs(Difference::OutOfOrder, index, held)),
            (Some(wanted), None) => return Err(differs(Difference::Missing, index, wanted)),
            // Below the longer list's length, one of the two is there.
            (None, None) => {}
        }
    }
    Ok(())
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
    /// A `journey` document compared with another Journey than the one it
    /// names.
    #[error("the document is Journey `{document}`'s, not `{journey}`'s")]
    OtherJourney {
        /// The Journey the document names.
        document: JourneyId,
        /// The Journey it was compared with.
        journey: JourneyId,
    },
    /// A `journey` document's steps, checks or tasks are not exactly its
    /// Journey's.
    #[error("{}", difference.describe(*part, *number, text))]
    NotTheJourneys {
        /// The list that differs.
        part: JourneyPart,
        /// How it differs, at the first place it does.
        difference: Difference,
        /// The place, from 1, in the document's list or, for a missing
        /// item, in the Journey's.
        number: usize,
        /// The step's or check's text, or the task's ID.
        text: String,
    },
    /// A named task holding the oracle's answer for another task: the
    /// answer names the task it judged (ADR 0063).
    #[error("task `{task}` holds the oracle's answer for task `{answered}`")]
    AnswerOfAnotherTask {
        /// The task the document holds the answer under.
        task: TaskId,
        /// The task the answer names.
        answered: TaskId,
    },
    /// A document whose Proving ground seed is not its Journey's, or that
    /// records a Proving ground its Journey starts none for, or none for a
    /// Journey that starts one: a Journey's seed is its own, never
    /// `GOODFETCH_TEST_SEED` (ADR 0063).
    #[error(
        "the document's Proving ground seed is {}, its Journey's {}",
        seed_text(*document),
        seed_text(*journey)
    )]
    OtherSeed {
        /// The seed the document records, if any.
        document: Option<u64>,
        /// The Journey's seed, if it starts a Proving ground.
        journey: Option<u64>,
    },
    /// A document of a surface its Journey has no variant on.
    #[error("the Journey has no variant on `{surface}`")]
    UnlistedSurface {
        /// The surface the document names.
        surface: String,
    },
    /// A step recorded `manual` that its Journey does not mark manual, or
    /// one the Journey marks manual recorded as an automated step.
    #[error(
        "step {number} `{text}` is {} in the document but not in its Journey",
        if *recorded_manual { "manual" } else { "automated" }
    )]
    ManualStep {
        /// The step's place, from 1.
        number: usize,
        /// The step's text.
        text: String,
        /// Whether the document records it `manual`.
        recorded_manual: bool,
    },
}

/// A seed as a reader's message writes it.
fn seed_text(seed: Option<u64>) -> String {
    seed.map_or_else(|| "none".to_owned(), |seed| seed.to_string())
}

/// A list of a Journey that each of its `journey` documents holds exactly.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JourneyPart {
    /// Its steps.
    Step,
    /// Its checks.
    Check,
    /// The Proving ground tasks it names.
    Task,
}

impl JourneyPart {
    /// The part's name, as the reader's messages write it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Step => "step",
            Self::Check => "check",
            Self::Task => "task",
        }
    }
}

/// How a document's list first differs from its Journey's.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Difference {
    /// The document holds an item of the Journey fewer times than the
    /// Journey does.
    Missing,
    /// The document holds an item the Journey does not, or holds it more
    /// times.
    Extra,
    /// The document holds the Journey's items in another order.
    OutOfOrder,
}

impl Difference {
    fn describe(self, part: JourneyPart, number: usize, text: &str) -> String {
        let part = part.as_str();
        match self {
            Self::Missing => {
                format!("the Journey's {part} {number} `{text}` is missing from the document")
            }
            Self::Extra => {
                format!("the document's {part} {number} `{text}` is not one of the Journey's")
            }
            Self::OutOfOrder => {
                format!("the document's {part} {number} `{text}` is out of the Journey's order")
            }
        }
    }
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
    /// Every value, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::Verified,
        Self::Incomplete,
        Self::FalseSuccess,
        Self::Failed,
    ];

    /// The outcome as a document writes it.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
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
    /// Whether the suite ran in gate mode, each case once with Filter-list
    /// blocking on and once with it off, its failures blocking the job that
    /// asked for it (ADR 0047): the Clean-up rules' daily channel's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub gate: bool,
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
    /// Whether Filter-list blocking was on or off, in gate mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<Blocking>,
    /// What it could not reach while it ran, such as a vendor or the
    /// online Proving ground, each as its runner names it. An entry with
    /// any neither passes nor fails, whatever its checks: an outage is
    /// inconclusive (ADR 0064).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub unreachable: Vec<String>,
    /// Each check's result.
    pub checks: Vec<CaseCheck>,
    /// What it saw.
    #[serde(default)]
    pub evidence: Evidence,
}

/// Whether a gate mode's entry ran with Filter-list blocking on or off
/// (ADR 0047).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Blocking {
    /// The Filter lists blocked as the case's settings say.
    On,
    /// Every Filter list was off.
    Off,
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
            capabilities: capabilities
                .into_iter()
                .map(|facts| CapabilityResult::new(facts, header.gate))
                .collect(),
            header,
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

    /// Whether the document blocks the job that ran it: a case failed, and
    /// it is no online result outside gate mode, which only reports (ADRs
    /// 0043 and 0047).
    pub fn blocks(&self) -> bool {
        (!self.header.online || self.header.gate)
            && self
                .capabilities
                .iter()
                .any(|capability| capability.outcome == Outcome::Failed)
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
    /// Derives its outcome, judging gate pairs when `gate`.
    fn new(facts: CapabilityFacts, gate: bool) -> Self {
        let cases: Vec<CaseResult> = facts
            .cases
            .into_iter()
            .map(|facts| CaseResult::new(facts, gate))
            .collect();
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
    /// Derives its outcome, judging gate pairs when `gate`.
    fn new(facts: CaseFacts, gate: bool) -> Self {
        Self {
            outcome: case_outcome(&facts.entries, gate),
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
    /// Every value, in declaration order.
    pub const ALL: [Self; 3] = [Self::Proved, Self::Unproved, Self::Failed];

    /// The outcome as a document writes it.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// A case's outcome.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum CaseOutcome {
    /// Every entry has checks and every check held.
    Passed,
    /// A check failed, or an entry holds no check; in gate mode, it failed
    /// with Filter-list blocking on and passed with it off.
    Failed,
    /// An entry could not reach something it needed, such as a vendor, and
    /// no entry that reached everything failed (ADR 0064); or, in gate
    /// mode, it failed with Filter-list
    /// blocking on and did not pass with it off, as when a vendor's scoring
    /// change moves both (ADR 0047). It neither passes nor blocks.
    Inconclusive,
    /// The case log holds no entry for it.
    NotReached,
}

impl CaseOutcome {
    /// Every value, in declaration order.
    pub const ALL: [Self; 4] = [
        Self::Passed,
        Self::Failed,
        Self::Inconclusive,
        Self::NotReached,
    ];

    /// The outcome as a document writes it.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// A case's outcome from its entries; outside gate mode, an entry's
/// blocking makes no pair, so only an entry that could not reach something
/// is inconclusive.
fn case_outcome(entries: &[CaseEntry], gate: bool) -> CaseOutcome {
    if entries.is_empty() {
        return CaseOutcome::NotReached;
    }
    // Each mode's gate pair: its entries with blocking on, then off.
    let mut pairs: BTreeMap<Option<&str>, (Vec<&CaseEntry>, Vec<&CaseEntry>)> = BTreeMap::new();
    let mut outcome = CaseOutcome::Passed;
    for entry in entries {
        let Some(blocking) = entry.blocking.filter(|_| gate) else {
            let judged = if !reached_everything(entry) {
                CaseOutcome::Inconclusive
            } else if passed(entry) {
                CaseOutcome::Passed
            } else {
                CaseOutcome::Failed
            };
            outcome = worse(outcome, judged);
            continue;
        };
        let (on, off) = pairs.entry(entry.mode.as_deref()).or_default();
        match blocking {
            Blocking::On => on.push(entry),
            Blocking::Off => off.push(entry),
        }
    }
    for (on, off) in pairs.values() {
        let all_passed = |entries: &[&CaseEntry]| {
            !entries.is_empty() && entries.iter().all(|entry| passed(entry))
        };
        // A failure with blocking on is the rules' only when that run
        // reached everything: an outage then is no evidence either way.
        let judged = if all_passed(on) {
            CaseOutcome::Passed
        } else if !on.is_empty()
            && on.iter().all(|entry| reached_everything(entry))
            && all_passed(off)
        {
            CaseOutcome::Failed
        } else {
            CaseOutcome::Inconclusive
        };
        outcome = worse(outcome, judged);
    }
    outcome
}

/// Whether an entry reached everything it needed, holds checks and every
/// one passed: an entry that could not reach something never passes.
fn passed(entry: &CaseEntry) -> bool {
    reached_everything(entry)
        && !entry.checks.is_empty()
        && entry.checks.iter().all(|check| check.passed)
}

/// Whether an entry reached everything it needed while it ran.
fn reached_everything(entry: &CaseEntry) -> bool {
    entry.unreachable.is_empty()
}

/// The worse of two outcomes of a reached case: failed, then inconclusive,
/// then passed.
fn worse(left: CaseOutcome, right: CaseOutcome) -> CaseOutcome {
    let rank = |outcome| match outcome {
        CaseOutcome::Passed | CaseOutcome::NotReached => 0,
        CaseOutcome::Inconclusive => 1,
        CaseOutcome::Failed => 2,
    };
    if rank(right) > rank(left) {
        right
    } else {
        left
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
    /// The surface's package, for a surface that has one (an SDK, an
    /// adapter).
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
    /// Each Proving ground task the Journey names, in its order: once per
    /// check naming it, each with the answer to that check's submission in
    /// the checks' order, or once when no check names it.
    pub tasks: Vec<TaskResult>,
    /// Each error that failed the variant's test outside its steps, checks
    /// and tasks, with its Error code: a page error or an accessibility
    /// violation the test found once its body had ended (ADR 0068). A
    /// variant with one is `failed`, whatever its steps and checks say.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub test_errors: Vec<String>,
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
    /// Every automated step and check passed, every named task is
    /// `verified` and no error failed the test outside them.
    Proved,
    /// Anything else.
    Failed,
}

impl JourneyOutcome {
    /// Every value, in declaration order.
    pub const ALL: [Self; 2] = [Self::Proved, Self::Failed];

    /// The outcome as a document writes it.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
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
    if steps && checks && tasks && facts.test_errors.is_empty() {
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
        let gate = self.header.gate;
        let mut capabilities = Vec::with_capacity(self.capabilities.len());
        for stored in self.capabilities {
            let mut cases = Vec::with_capacity(stored.cases.len());
            for case in stored.cases {
                let derived = CaseResult::new(case.facts, gate);
                if derived.outcome != case.outcome {
                    return Err(ReadError::CaseOutcome {
                        case: derived.facts.id,
                        stored: case.outcome,
                        derived: derived.outcome,
                    });
                }
                cases.push(derived.facts);
            }
            let derived = CapabilityResult::new(
                CapabilityFacts {
                    name: stored.name,
                    cases,
                },
                gate,
            );
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
