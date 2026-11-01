//! The Feature report (ADRs 0072, 0075 and 0094): each Feature's status at
//! one milestone's gate, from the inventory, the declarations, the
//! exceptions, the quarantine list (ADR 0064) and `main`'s latest Release
//! suite run in the results store, with each Product's view by area and by
//! the milestone each Feature is due.
//!
//! - **Due.** A generated Feature is due with its code, so at every gate. A
//!   declared one is due at its own milestone, and a story with
//!   `journey = true` at the later of its own and the earliest milestone of
//!   a variant of a Journey that covers it (ADR 0075). A declared Feature
//!   not due at the gate, or with no milestone yet, is **planned**.
//! - **Tested.** At least one declaring test, and every one passed in every
//!   configuration of the latest Release run and is not quarantined. A
//!   Journey's declaration stands for each of its variants due at the gate,
//!   each proved when the test that carries it passed. A story also needs a
//!   declaring test above the unit level, and a flagged one a proved variant
//!   of a Journey that covers it.
//! - **Untested**, by the first cause that holds: `no-test`; `failed`,
//!   `quarantined` or `not-run` for a declaring test other than a Journey's
//!   variant; `no-test-above-unit-level`; `no-covering-journey` for a
//!   flagged story none of whose covering variants is proved; then
//!   `failed`, `quarantined` or `not-run` for a declaring Journey's variant.
//!   A test is `not-run` when the Release run has no result for it, or only
//!   a skipped or inconclusive one. The store holds no conformance result
//!   document yet, so a conformance case is `not-run`.
//! - **Excepted**, by its evidence kind, when `features/exceptions.toml`
//!   names an untested Feature.
//!
//! "The latest Release run" is the newest commit of `main` the Release
//! suite's test runs name, across its dispatches; for each test and
//! configuration the newest of its runs on that commit counts.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::Milestone;
use goodfetch_protocol::result_document::{self, CaseOutcome, LoadedDocument};
use serde::Serialize;

use super::files::{
    self, Evidence, Exception, ExceptionsFile, Feature, Kind, Status, StoryFile, Unreadable,
};
use super::scan::{Declaration, Level, Source};
use crate::journeys::{self, CatalogVariant, ManualException, VariantOutcome};
use crate::quarantine;
use crate::results::{self, Outcome};

/// What the report reads besides the workspace's `features/` files.
pub(crate) struct Sources<'a> {
    pub(crate) root: &'a Path,
    /// The milestone whose gate the report judges.
    pub(crate) gate: Milestone,
    pub(crate) declarations: &'a [Declaration],
    /// Each active Journey's variants.
    pub(crate) variants: &'a [CatalogVariant],
    /// The tests that carry each variant, from the runners' listings; none
    /// when the report read no listing.
    pub(crate) variant_tests: Option<&'a BTreeMap<String, Vec<(String, String)>>>,
    /// The results store's folder, if the report reads one.
    pub(crate) store: Option<&'a Path>,
}

/// Why a Feature due at the gate is untested, in the order the report
/// tries them.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Cause {
    NoTest,
    Failed,
    Quarantined,
    NotRun,
    NoTestAboveUnitLevel,
    NoCoveringJourney,
}

impl Cause {
    pub(crate) const ALL: [Self; 6] = [
        Self::NoTest,
        Self::Failed,
        Self::Quarantined,
        Self::NotRun,
        Self::NoTestAboveUnitLevel,
        Self::NoCoveringJourney,
    ];

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::NoTest => "no-test",
            Self::Failed => "failed",
            Self::Quarantined => "quarantined",
            Self::NotRun => "not-run",
            Self::NoTestAboveUnitLevel => "no-test-above-unit-level",
            Self::NoCoveringJourney => "no-covering-journey",
        }
    }
}

/// A Feature's status at the gate.
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum State {
    Tested,
    Untested,
    Excepted,
    Planned,
}

/// What one declaring test, or one variant of a declaring Journey, gave on
/// the latest Release run.
#[derive(Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum TestOutcome {
    Passed,
    Failed,
    Quarantined,
    NotRun,
}

impl TestOutcome {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Failed => "failed",
            Self::Quarantined => "quarantined",
            Self::NotRun => "not-run",
        }
    }
}

/// One declaring test of a Feature, with its outcome.
#[derive(Serialize)]
pub(crate) struct TestEntry {
    /// The test as its runner names it; for a Journey, its variant's ID.
    pub(crate) test: String,
    pub(crate) source: &'static str,
    pub(crate) level: &'static str,
    pub(crate) outcome: TestOutcome,
}

/// One Feature of the report.
#[derive(Serialize)]
pub(crate) struct Entry {
    pub(crate) id: String,
    pub(crate) kind: Kind,
    pub(crate) area: Option<u32>,
    /// Its milestone as the inventory writes it.
    pub(crate) milestone: Option<String>,
    /// The milestone it is due at; none for a generated Feature, due with
    /// its code, and for a declared one with no milestone yet.
    pub(crate) due: Option<Milestone>,
    pub(crate) products: Vec<Product>,
    pub(crate) status: State,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) cause: Option<Cause>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) evidence: Option<&'static str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) link: Option<String>,
    pub(crate) tests: Vec<TestEntry>,
}

/// How many Features have each status, cause and evidence kind.
#[derive(Serialize)]
pub(crate) struct Summary {
    pub(crate) tested: usize,
    pub(crate) untested: BTreeMap<&'static str, usize>,
    pub(crate) excepted: BTreeMap<&'static str, usize>,
    pub(crate) planned: usize,
}

impl Summary {
    fn new() -> Self {
        Self {
            tested: 0,
            untested: Cause::ALL.iter().map(|cause| (cause.as_str(), 0)).collect(),
            excepted: Evidence::ALL
                .iter()
                .map(|evidence| (evidence.as_str(), 0))
                .collect(),
            planned: 0,
        }
    }

    fn add(&mut self, entry: &Entry) {
        match entry.status {
            State::Tested => self.tested += 1,
            State::Planned => self.planned += 1,
            State::Untested => {
                if let Some(cause) = entry.cause {
                    *self.untested.entry(cause.as_str()).or_default() += 1;
                }
            }
            State::Excepted => {
                if let Some(evidence) = entry.evidence {
                    *self.excepted.entry(evidence).or_default() += 1;
                }
            }
        }
    }

    pub(crate) fn untested_count(&self) -> usize {
        self.untested.values().sum()
    }

    pub(crate) fn excepted_count(&self) -> usize {
        self.excepted.values().sum()
    }
}

/// One area's, or one due milestone's, Features.
#[derive(Serialize)]
pub(crate) struct Group<K> {
    pub(crate) key: Option<K>,
    pub(crate) summary: Summary,
}

/// What one Product carries.
#[derive(Serialize)]
pub(crate) struct View {
    pub(crate) product: Product,
    pub(crate) summary: Summary,
    pub(crate) areas: Vec<Group<u32>>,
    pub(crate) milestones: Vec<Group<Milestone>>,
    /// The IDs of the Features it carries.
    pub(crate) features: Vec<String>,
}

/// The whole report, as `features.json` holds it.
#[derive(Serialize)]
pub(crate) struct Report {
    /// The milestone whose gate it judges.
    pub(crate) milestone: Milestone,
    /// The commit of `main`'s latest Release run it read; none when the
    /// store holds none.
    pub(crate) release_commit: Option<String>,
    /// Whether it read the runners' listings, so the Vitest, Playwright and
    /// pytest declarations and the tests that carry each Journey variant.
    pub(crate) listings: bool,
    pub(crate) features: Vec<Entry>,
    pub(crate) views: Vec<View>,
    /// The milestone gate's state (ADR 0072).
    pub(crate) gate: Gate,
}

/// The milestone gate of ADR 0072, over both Products: ready only when the
/// store holds a Release run of `main` and no Feature due at the milestone
/// is untested on it.
#[derive(Serialize)]
pub(crate) struct Gate {
    pub(crate) ready: bool,
    /// The untested Features' IDs, in the inventory's order.
    pub(crate) untested: Vec<String>,
}

impl Report {
    /// The gate's state in one sentence, as stdout and the page give it.
    pub(crate) fn gate_line(&self) -> String {
        let milestone = format!("{:?}", self.milestone);
        match (&self.release_commit, self.gate.untested.len()) {
            (None, _) => {
                format!("{milestone} gate: not ready, since the store holds no Release run of main")
            }
            (Some(commit), 0) => {
                format!("{milestone} gate: ready on main's latest Release run, of {commit}")
            }
            (Some(commit), untested) => format!(
                "{milestone} gate: not ready, {untested} Features untested on main's latest Release run, of {commit}"
            ),
        }
    }
}

/// The milestone a milestone's name starts with: `M1` for `M1 Core`.
pub(crate) fn milestone_of(text: &str) -> Option<Milestone> {
    use serde::Deserialize as _;
    let word = text.split_whitespace().next()?;
    Milestone::deserialize(serde::de::value::StrDeserializer::<serde::de::value::Error>::new(word))
        .ok()
}

pub(crate) fn build(sources: &Sources<'_>) -> Result<Report> {
    let root = sources.root;
    let inventory =
        files::read_inventory(root).map_err(|error| unreadable(error, files::INVENTORY))?;
    let exceptions: ExceptionsFile = files::read_optional_toml(root, files::EXCEPTIONS)
        .map_err(|error| unreadable(error, files::EXCEPTIONS))?;
    let exceptions: BTreeMap<&str, &Exception> = exceptions
        .exception
        .iter()
        .map(|exception| (exception.feature.as_str(), exception))
        .collect();
    let flagged = flagged_stories(root)?;
    let quarantined: BTreeSet<(String, String)> = quarantine::read(root)?
        .into_iter()
        .map(|entry| {
            (
                entry.runner.name().to_owned(),
                entry.test.as_str().to_owned(),
            )
        })
        .collect();
    let (release_commit, outcomes) = match sources.store {
        Some(store) => release_outcomes(store)?,
        None => (None, BTreeMap::new()),
    };
    // Journey variants as the Journey report judges them, from the result
    // documents the cells kept (ADR 0072's rules 3 and 4).
    let variant_outcomes = match sources.store {
        Some(store) => journeys::variant_outcomes(root, store, sources.variant_tests)?,
        None => BTreeMap::new(),
    };
    let judge = Judge {
        gate: sources.gate,
        variants: sources.variants,
        variant_tests: sources.variant_tests,
        quarantined: &quarantined,
        outcomes: &outcomes,
        variant_outcomes: &variant_outcomes,
    };

    let mut declared: BTreeMap<&str, Vec<&Declaration>> = BTreeMap::new();
    for declaration in sources.declarations {
        declared
            .entry(declaration.feature.as_str())
            .or_default()
            .push(declaration);
    }
    let mut features = Vec::new();
    for feature in &inventory.features {
        if feature.status == Status::Retired {
            continue;
        }
        let declarations = declared
            .get(feature.id.as_str())
            .map_or(&[][..], Vec::as_slice);
        features.push(judge.entry(
            feature,
            declarations,
            flagged.contains(feature.id.as_str()),
            exceptions.get(feature.id.as_str()).copied(),
        )?);
    }
    let views = [Product::Collie, Product::Foxhound]
        .into_iter()
        .map(|product| view(product, &features))
        .collect();
    let untested: Vec<String> = features
        .iter()
        .filter(|entry| entry.status == State::Untested)
        .map(|entry| entry.id.clone())
        .collect();
    let gate = Gate {
        ready: release_commit.is_some() && untested.is_empty(),
        untested,
    };
    Ok(Report {
        milestone: sources.gate,
        release_commit,
        listings: sources.variant_tests.is_some(),
        features,
        views,
        gate,
    })
}

fn unreadable(error: Unreadable, path: &str) -> anyhow::Error {
    match error {
        Unreadable::Io(error) => error,
        Unreadable::Schema(message) => {
            anyhow::anyhow!("{path} breaks its schema ({message}); run `xtask features check`")
        }
    }
}

/// The IDs of the active stories with `journey = true`.
fn flagged_stories(root: &Path) -> Result<BTreeSet<String>> {
    let mut flagged = BTreeSet::new();
    for (area, path) in files::story_files(root)? {
        let Some(area) = area else {
            continue;
        };
        let file: StoryFile =
            files::read_toml(root, &path).map_err(|error| unreadable(error, &path))?;
        flagged.extend(
            file.story
                .iter()
                .filter(|story| story.journey && story.status == Status::Active)
                .map(|story| format!("{}:{area}-{}", Kind::Story, story.id)),
        );
    }
    Ok(flagged)
}

/// A test as the store and the quarantine list name it: its runner and ID.
type TestKey = (String, String);

/// The commit of `main`'s latest Release run, and each test's outcome on it
/// across its configurations.
fn release_outcomes(store: &Path) -> Result<(Option<String>, BTreeMap<TestKey, TestOutcome>)> {
    let attempts = results::read_all(store)
        .with_context(|| format!("reading the results store at {}", store.display()))?;
    let Some((latest, runs)) = results::latest_on_main(&attempts) else {
        return Ok((None, BTreeMap::new()));
    };
    // The newest run of each test in each configuration, across dispatches.
    let mut newest: BTreeMap<(TestKey, String), &results::TestRun> = BTreeMap::new();
    for run in runs {
        let configuration = format!(
            "{} {} {}",
            run.os,
            run.arch,
            run.runtime.as_deref().unwrap_or("")
        );
        let key = ((run.runner.clone(), run.test.clone()), configuration);
        let order = |run: &results::TestRun| (run.started_at, run.run, run.attempt);
        if newest.get(&key).is_none_or(|kept| order(kept) < order(run)) {
            newest.insert(key, run);
        }
    }
    let mut outcomes: BTreeMap<TestKey, TestOutcome> = BTreeMap::new();
    for ((test, _), run) in newest {
        let outcome = match run.outcome {
            Outcome::Passed => TestOutcome::Passed,
            Outcome::Failed => TestOutcome::Failed,
            Outcome::Skipped | Outcome::Inconclusive => TestOutcome::NotRun,
        };
        let combined = outcomes.entry(test).or_insert(outcome);
        *combined = worst(*combined, outcome);
    }
    for (test, outcome) in conformance_outcomes(store, &latest)? {
        let combined = outcomes.entry(test).or_insert(outcome);
        *combined = worst(*combined, outcome);
    }
    Ok((Some(latest), outcomes))
}

/// Each conformance case's outcome in the `conformance` result documents
/// the cells of `main`'s latest Release run, of `latest`, kept under
/// `result-documents/`, keyed as its declaration names it, `<suite> <case
/// id>`, the worst across cells. Each document goes through
/// `goodfetch-protocol`'s reader, which recomputes its outcomes; one it
/// refuses proves no case.
fn conformance_outcomes(store: &Path, latest: &str) -> Result<BTreeMap<TestKey, TestOutcome>> {
    let kept = results::read_kept(store, |file| {
        file.strip_prefix(DOCUMENTS)
            .is_some_and(|name| !name.contains('/'))
    })?;
    let mut outcomes: BTreeMap<TestKey, TestOutcome> = BTreeMap::new();
    for kept in kept.iter().filter(|kept| {
        kept.run.branch.as_deref() == Some(results::MAIN)
            && kept.configuration.suite == results::RELEASE_SUITE
            && kept.commit == latest
    }) {
        let Ok(LoadedDocument::Conformance(document)) = result_document::read(&kept.contents)
        else {
            continue;
        };
        let suite = &document.header().suite.name;
        for capability in document.capabilities() {
            for case in capability.cases() {
                let outcome = match case.outcome() {
                    CaseOutcome::Passed => TestOutcome::Passed,
                    CaseOutcome::Failed => TestOutcome::Failed,
                    CaseOutcome::NotReached => TestOutcome::NotRun,
                };
                let key = (
                    Source::Conformance.as_str().to_owned(),
                    format!("{suite} {}", case.facts().id),
                );
                let combined = outcomes.entry(key).or_insert(outcome);
                *combined = worst(*combined, outcome);
            }
        }
    }
    Ok(outcomes)
}

/// The folder of a JUnit artifact that holds its result documents.
const DOCUMENTS: &str = "result-documents/";

/// The milestone `feature` is due at: none for a generated one, due with
/// its code, and for a declared one with no milestone yet. A flagged story
/// is due at the later of its own milestone and the earliest of a variant
/// of a Journey that covers it (ADR 0075).
fn due(feature: &Feature, flagged: bool, variants: &[CatalogVariant]) -> Result<Option<Milestone>> {
    if !feature.kind.is_declared() {
        return Ok(None);
    }
    let Some(name) = &feature.milestone else {
        return Ok(None);
    };
    let Some(own) = milestone_of(name) else {
        bail!(
            "{} has milestone `{name}`, which starts with none of M1 to M4",
            feature.id
        );
    };
    let journey = variants
        .iter()
        .filter(|variant| flagged && variant.covers.contains(&feature.id))
        .map(|variant| variant.milestone)
        .min();
    Ok(Some(journey.map_or(own, |journey| journey.max(own))))
}

/// Each exception with evidence `manual` in `features/exceptions.toml`
/// whose Feature is due at `gate`, by its Feature's ID: a generated
/// Feature's at every gate, a declared one's at the milestone it is due.
pub(crate) fn manual_exceptions(
    root: &Path,
    gate: Milestone,
    variants: &[CatalogVariant],
) -> Result<Vec<ManualException>> {
    let inventory =
        files::read_inventory(root).map_err(|error| unreadable(error, files::INVENTORY))?;
    let exceptions: ExceptionsFile = files::read_optional_toml(root, files::EXCEPTIONS)
        .map_err(|error| unreadable(error, files::EXCEPTIONS))?;
    let flagged = flagged_stories(root)?;
    let features: BTreeMap<&str, &Feature> = inventory
        .features
        .iter()
        .filter(|feature| feature.status == Status::Active)
        .map(|feature| (feature.id.as_str(), feature))
        .collect();
    let mut due_at_gate = Vec::new();
    for exception in exceptions.exception {
        if !matches!(exception.evidence, Evidence::Manual) {
            continue;
        }
        let Some(feature) = features.get(exception.feature.as_str()) else {
            continue;
        };
        let at_gate = !feature.kind.is_declared()
            || due(feature, flagged.contains(feature.id.as_str()), variants)? == Some(gate);
        if at_gate {
            due_at_gate.push(ManualException {
                feature: exception.feature,
                reason: exception.reason,
                link: exception.link,
            });
        }
    }
    due_at_gate.sort_by(|left, right| left.feature.cmp(&right.feature));
    Ok(due_at_gate)
}

/// The outcome of two runs of one test taken together.
fn worst(left: TestOutcome, right: TestOutcome) -> TestOutcome {
    let rank = |outcome| match outcome {
        TestOutcome::Passed => 0,
        TestOutcome::NotRun => 1,
        TestOutcome::Quarantined => 2,
        TestOutcome::Failed => 3,
    };
    if rank(right) > rank(left) {
        right
    } else {
        left
    }
}

struct Judge<'a> {
    gate: Milestone,
    variants: &'a [CatalogVariant],
    variant_tests: Option<&'a BTreeMap<String, Vec<(String, String)>>>,
    quarantined: &'a BTreeSet<TestKey>,
    outcomes: &'a BTreeMap<TestKey, TestOutcome>,
    variant_outcomes: &'a BTreeMap<String, VariantOutcome>,
}

impl Judge<'_> {
    fn entry(
        &self,
        feature: &Feature,
        declarations: &[&Declaration],
        flagged: bool,
        exception: Option<&Exception>,
    ) -> Result<Entry> {
        let due = self.due(feature, flagged)?;
        let mut tests = Vec::new();
        for declaration in declarations {
            tests.extend(self.tests_of(declaration));
        }
        let planned = feature.kind.is_declared() && due.is_none_or(|due| due > self.gate);
        let (status, cause) = if planned {
            (State::Planned, None)
        } else {
            match self.cause(feature, &tests, flagged) {
                None => (State::Tested, None),
                Some(_) if exception.is_some() => (State::Excepted, None),
                Some(cause) => (State::Untested, Some(cause)),
            }
        };
        let exception = exception.filter(|_| status == State::Excepted);
        Ok(Entry {
            id: feature.id.clone(),
            kind: feature.kind,
            area: feature.area,
            milestone: feature.milestone.clone(),
            due,
            products: feature.products.clone(),
            status,
            cause,
            evidence: exception.map(|exception| exception.evidence.as_str()),
            reason: exception.map(|exception| exception.reason.clone()),
            link: exception.map(|exception| exception.link.clone()),
            tests,
        })
    }

    fn due(&self, feature: &Feature, flagged: bool) -> Result<Option<Milestone>> {
        due(feature, flagged, self.variants)
    }

    /// One entry per runner test, conformance case, and variant of a
    /// declaring Journey due at the gate.
    fn tests_of(&self, declaration: &Declaration) -> Vec<TestEntry> {
        let entry = |test: String, outcome| TestEntry {
            test,
            source: declaration.source.as_str(),
            level: declaration.level.as_str(),
            outcome,
        };
        match declaration.source {
            Source::Nextest | Source::Vitest | Source::Playwright | Source::Pytest => {
                let key = (
                    declaration.source.as_str().to_owned(),
                    declaration.test.clone(),
                );
                vec![entry(declaration.test.clone(), self.outcome(&key))]
            }
            Source::Conformance => {
                let key = (
                    Source::Conformance.as_str().to_owned(),
                    declaration.test.clone(),
                );
                vec![entry(declaration.test.clone(), self.outcome(&key))]
            }
            Source::Journey => self
                .variants
                .iter()
                .filter(|variant| {
                    variant.id.split('@').next() == Some(declaration.test.as_str())
                        && variant.milestone <= self.gate
                })
                .map(|variant| entry(variant.id.clone(), self.variant_outcome(&variant.id)))
                .collect(),
        }
    }

    fn outcome(&self, key: &TestKey) -> TestOutcome {
        if self.quarantined.contains(key) {
            TestOutcome::Quarantined
        } else {
            self.outcomes
                .get(key)
                .copied()
                .unwrap_or(TestOutcome::NotRun)
        }
    }

    /// A variant's outcome is the Journey report's, from its kept result
    /// documents: `proved` passes, `failed` and `rejected` fail, and
    /// `unproved` has not run; one a quarantined test carries is
    /// quarantined.
    fn variant_outcome(&self, variant: &str) -> TestOutcome {
        let carriers = self
            .variant_tests
            .and_then(|tests| tests.get(variant))
            .map_or(&[][..], Vec::as_slice);
        let judged = match self.variant_outcomes.get(variant) {
            Some(VariantOutcome::Proved) => TestOutcome::Passed,
            Some(VariantOutcome::Failed | VariantOutcome::Rejected) => TestOutcome::Failed,
            Some(VariantOutcome::Unproved) | None => TestOutcome::NotRun,
        };
        if carriers.iter().any(|key| self.quarantined.contains(key)) {
            worst(judged, TestOutcome::Quarantined)
        } else {
            judged
        }
    }

    /// The first cause that keeps a Feature due at the gate untested. The
    /// declaring tests' outcomes come first, then the story rules; a flagged
    /// story none of whose covering variants is proved has no covering
    /// Journey, whatever its Journeys' variants gave, and only then do those
    /// outcomes count (ADR 0072's rule 3).
    fn cause(&self, feature: &Feature, tests: &[TestEntry], flagged: bool) -> Option<Cause> {
        let journey = Source::Journey.as_str();
        let (variants, others): (Vec<&TestEntry>, Vec<&TestEntry>) =
            tests.iter().partition(|test| test.source == journey);
        if tests.is_empty() {
            Some(Cause::NoTest)
        } else if let Some(cause) = outcome_cause(&others) {
            Some(cause)
        } else if feature.kind == Kind::Story
            && tests.iter().all(|test| test.level == Level::Unit.as_str())
        {
            Some(Cause::NoTestAboveUnitLevel)
        } else if flagged && !self.proved_covering_variant(&feature.id) {
            Some(Cause::NoCoveringJourney)
        } else {
            outcome_cause(&variants)
        }
    }

    fn proved_covering_variant(&self, feature: &str) -> bool {
        self.variants.iter().any(|variant| {
            variant.covers.iter().any(|covered| covered == feature)
                && self.variant_outcome(&variant.id) == TestOutcome::Passed
        })
    }
}

/// The cause the outcomes of some declaring tests give, if any.
fn outcome_cause(tests: &[&TestEntry]) -> Option<Cause> {
    let any = |outcome| tests.iter().any(|test| test.outcome == outcome);
    if any(TestOutcome::Failed) {
        Some(Cause::Failed)
    } else if any(TestOutcome::Quarantined) {
        Some(Cause::Quarantined)
    } else if any(TestOutcome::NotRun) {
        Some(Cause::NotRun)
    } else {
        None
    }
}

fn view(product: Product, features: &[Entry]) -> View {
    let carried: Vec<&Entry> = features
        .iter()
        .filter(|entry| entry.products.contains(&product))
        .collect();
    let mut summary = Summary::new();
    let mut areas: BTreeMap<Option<u32>, Summary> = BTreeMap::new();
    let mut milestones: BTreeMap<Option<Milestone>, Summary> = BTreeMap::new();
    for entry in &carried {
        summary.add(entry);
        areas
            .entry(entry.area)
            .or_insert_with(Summary::new)
            .add(entry);
        milestones
            .entry(entry.due)
            .or_insert_with(Summary::new)
            .add(entry);
    }
    View {
        product,
        summary,
        areas: areas
            .into_iter()
            .map(|(key, summary)| Group { key, summary })
            .collect(),
        milestones: milestones
            .into_iter()
            .map(|(key, summary)| Group { key, summary })
            .collect(),
        features: carried.iter().map(|entry| entry.id.clone()).collect(),
    }
}
