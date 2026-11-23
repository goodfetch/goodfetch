use std::collections::BTreeMap;
use std::path::PathBuf;

use rstest::rstest;
use serde::Deserialize;
use serde_json::{Map, json};

use super::{
    Blocking, CapabilityFacts, CaseCheck, CaseEntry, CaseFacts, CaseOutcome, CheckOutcome,
    CheckResult, ConformanceDocument, ConformanceHeader, Difference, Evidence, Executable,
    JourneyDocument, JourneyFacts, JourneyOutcome, JourneyPart, LoadedDocument, OracleAnswer,
    OracleOutcome, Outcome, Platform, ProvingGroundCopy, ReadError, ResultDocument, StepOutcome,
    StepResult, Suite, Target, TargetIdentity, TaskResult, read, read_journey_document,
};
use crate::catalog::Product;
use crate::journey::{
    Check, Journey, JourneyId, Milestone, PinnedTask, ProvingGround, Status, Step, TaskId,
};
use crate::releases::Sha256;

/// One hand-written Journey outcome vector of
/// `test-support/fixtures/outcome-vectors/journey/`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct JourneyVector {
    #[expect(dead_code, reason = "the vector's own explanation, read by people")]
    description: String,
    steps: Vec<StepOutcome>,
    checks: Vec<CheckOutcome>,
    tasks: Vec<Option<OracleOutcome>>,
    #[serde(default)]
    test_errors: Vec<String>,
    outcome: JourneyOutcome,
}

/// One hand-written conformance outcome vector of
/// `test-support/fixtures/outcome-vectors/conformance/`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceVector {
    #[expect(dead_code, reason = "the vector's own explanation, read by people")]
    description: String,
    /// Whether the document is a gate run's (ADR 0047).
    #[serde(default)]
    gate: bool,
    cases: Vec<VectorCase>,
    case_outcomes: Vec<CaseOutcome>,
    outcome: Outcome,
}

/// One hand-written vector of
/// `test-support/fixtures/outcome-vectors/journey-comparison/`: a Journey, a
/// document stored for it, and what reading the document gives.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComparisonVector {
    #[expect(dead_code, reason = "the vector's own explanation, read by people")]
    description: String,
    journey: VectorJourney,
    document: VectorDocument,
    read: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorJourney {
    steps: Vec<Step>,
    expect: Vec<Check>,
    tasks: Vec<TaskId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorDocument {
    steps: Vec<StepResult>,
    checks: Vec<CheckResult>,
    tasks: Vec<VectorTask>,
    outcome: JourneyOutcome,
    /// The Proving ground seed the document records; the Journey's own
    /// when absent.
    #[serde(default)]
    seed: Option<u64>,
    /// The variant's surface; `cli` when absent.
    #[serde(default)]
    surface: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorTask {
    id: TaskId,
    answer: Option<OracleOutcome>,
    /// The task the answer names; the entry's own when absent.
    #[serde(default)]
    answered: Option<TaskId>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorCase {
    entries: Vec<VectorEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorEntry {
    #[serde(default)]
    mode: Option<String>,
    #[serde(default)]
    blocking: Option<Blocking>,
    #[serde(default)]
    unreachable: Vec<String>,
    checks: Vec<bool>,
}

#[expect(
    clippy::disallowed_methods,
    reason = "a synchronous unit test reads its fixture; no async worker runs"
)]
fn read_vector<T: for<'de> Deserialize<'de>>(path: &PathBuf) -> T {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn task(id: &str) -> TaskId {
    TaskId::try_from(id.to_owned()).unwrap()
}

fn answer(task_id: &str, outcome: OracleOutcome) -> OracleAnswer {
    OracleAnswer {
        task: task(task_id),
        outcome,
        details: Map::new(),
    }
}

fn journey_facts(
    steps: &[StepOutcome],
    checks: &[CheckOutcome],
    tasks: &[Option<OracleOutcome>],
) -> JourneyFacts {
    JourneyFacts {
        test_errors: Vec::new(),
        journey_id: JourneyId::try_from("J-002".to_owned()).unwrap(),
        surface: "cli".to_owned(),
        executable: Executable {
            product: Product::Foxhound,
            version: "0.0.0+0000000000000000000000000000000000000000".to_owned(),
        },
        surface_package: None,
        platform: platform(),
        runtime: "rust".to_owned(),
        // J-002's own seed, with a Proving ground only when it names a task,
        // as `journey` gives it.
        proving_ground: (!tasks.is_empty()).then(|| ProvingGroundCopy {
            seed: 2,
            ..proving_ground()
        }),
        steps: steps
            .iter()
            .enumerate()
            .map(|(index, outcome)| StepResult {
                text: format!("step {index}"),
                outcome: *outcome,
            })
            .collect(),
        checks: checks
            .iter()
            .enumerate()
            .map(|(index, outcome)| CheckResult {
                text: format!("check {index}"),
                task: None,
                outcome: *outcome,
            })
            .collect(),
        tasks: tasks
            .iter()
            .enumerate()
            .map(|(index, outcome)| {
                let id = format!("catalog/{}", index + 1);
                TaskResult {
                    answer: outcome.map(|outcome| answer(&id, outcome)),
                    id: task(&id),
                    observations: Vec::new(),
                }
            })
            .collect(),
    }
}

fn step(text: &str, manual: Option<&str>) -> Step {
    Step {
        text: text.to_owned(),
        on: None,
        manual: manual.map(str::to_owned),
    }
}

fn check(text: &str, task_id: Option<&str>) -> Check {
    Check {
        text: text.to_owned(),
        task: task_id.map(task),
        surfaces: Vec::new(),
    }
}

/// J-002, the Journey `journey_facts` documents, with these steps, checks and
/// named tasks; a Proving ground table only when it names a task.
fn journey(steps: Vec<Step>, expect: Vec<Check>, tasks: Vec<TaskId>) -> Journey {
    let digest = Sha256::try_from("0".repeat(64)).unwrap();
    Journey {
        id: JourneyId::try_from("J-002".to_owned()).unwrap(),
        title: "Crawl the catalog".to_owned(),
        goal: "A person crawls the catalog. Its Records MUST be the catalog's products.".to_owned(),
        area: 3,
        product: Product::Foxhound,
        status: Status::Active,
        replaced_by: Vec::new(),
        reason: None,
        covers: Vec::new(),
        proving_ground: (!tasks.is_empty()).then(|| ProvingGround {
            seed: 2,
            tasks: tasks
                .into_iter()
                .map(|id| PinnedTask {
                    id,
                    digest: digest.clone(),
                })
                .collect(),
        }),
        surfaces: BTreeMap::from([("cli".to_owned(), Milestone::M1)]),
        steps,
        expect,
    }
}

/// The document these facts give, stored with this outcome.
fn stored(facts: JourneyFacts, outcome: JourneyOutcome) -> Vec<u8> {
    let mut stored =
        serde_json::to_value(ResultDocument::Journey(JourneyDocument::new(facts))).unwrap();
    stored["outcome"] = json!(outcome);
    serde_json::to_vec(&stored).unwrap()
}

/// Reads a stored `journey` document and compares it with the Journey.
fn read_compared(bytes: &[u8], journey: &Journey) -> Result<JourneyDocument, ReadError> {
    match read(bytes)? {
        LoadedDocument::Journey(document) => read_journey_document(document, journey),
        LoadedDocument::Conformance(document) => {
            panic!("a journey document was read as {document:?}")
        }
    }
}

/// What a reading gives, as the comparison vectors write it.
fn reading(result: &Result<JourneyDocument, ReadError>) -> String {
    match result {
        Ok(_) => "accepted".to_owned(),
        Err(ReadError::JourneyOutcome { .. }) => "journey_outcome".to_owned(),
        Err(ReadError::ManualStep { .. }) => "manual_step".to_owned(),
        Err(ReadError::AnswerOfAnotherTask { .. }) => "answer_of_another_task".to_owned(),
        Err(ReadError::OtherSeed { .. }) => "other_seed".to_owned(),
        Err(ReadError::UnlistedSurface { .. }) => "unlisted_surface".to_owned(),
        Err(ReadError::NotTheJourneys {
            part, difference, ..
        }) => match difference {
            Difference::Missing => format!("missing_{}", part.as_str()),
            Difference::Extra => format!("extra_{}", part.as_str()),
            Difference::OutOfOrder => format!("{}_out_of_order", part.as_str()),
        },
        Err(other) => panic!("no comparison vector gives {other:?}"),
    }
}

fn platform() -> Platform {
    Platform {
        os: "linux".to_owned(),
        arch: "x86_64".to_owned(),
    }
}

fn proving_ground() -> ProvingGroundCopy {
    ProvingGroundCopy {
        version: "0.0.0+0000000000000000000000000000000000000000".to_owned(),
        seed: 169,
    }
}

fn header() -> ConformanceHeader {
    ConformanceHeader {
        suite: Suite {
            name: "self-test".to_owned(),
            version: "1".to_owned(),
        },
        target: Target {
            name: "goodfetch-proving-ground".to_owned(),
            identity: TargetIdentity::Version("0.0.0".to_owned()),
            product: None,
        },
        platform: platform(),
        runtime: "rust".to_owned(),
        proving_ground: proving_ground(),
        online: false,
        gate: false,
    }
}

fn conformance_facts(cases: &[VectorCase]) -> CapabilityFacts {
    CapabilityFacts {
        name: "catalog.verified".to_owned(),
        cases: cases
            .iter()
            .enumerate()
            .map(|(index, case)| CaseFacts {
                id: format!("case-{index}"),
                covers: Vec::new(),
                entries: case
                    .entries
                    .iter()
                    .map(|entry| CaseEntry {
                        mode: entry.mode.clone(),
                        blocking: entry.blocking,
                        unreachable: entry.unreachable.clone(),
                        checks: entry
                            .checks
                            .iter()
                            .map(|passed| CaseCheck {
                                name: "check".to_owned(),
                                passed: *passed,
                            })
                            .collect(),
                        evidence: Evidence::default(),
                    })
                    .collect(),
            })
            .collect(),
    }
}

#[rstest]
fn the_journey_constructor_gives_each_vectors_outcome(
    #[files("../../test-support/fixtures/outcome-vectors/journey/*.json")] path: PathBuf,
) {
    let vector: JourneyVector = read_vector(&path);

    let document = JourneyDocument::new(JourneyFacts {
        test_errors: vector.test_errors,
        ..journey_facts(&vector.steps, &vector.checks, &vector.tasks)
    });

    assert_eq!(document.outcome(), vector.outcome);
}

#[rstest]
fn the_conformance_constructor_gives_each_vectors_outcomes(
    #[files("../../test-support/fixtures/outcome-vectors/conformance/*.json")] path: PathBuf,
) {
    let vector: ConformanceVector = read_vector(&path);

    let document = ConformanceDocument::new(
        ConformanceHeader {
            gate: vector.gate,
            ..header()
        },
        vec![conformance_facts(&vector.cases)],
    );

    let capability = &document.capabilities()[0];
    let case_outcomes: Vec<CaseOutcome> = capability
        .cases()
        .iter()
        .map(super::CaseResult::outcome)
        .collect();
    assert_eq!(
        (capability.outcome(), case_outcomes),
        (vector.outcome, vector.case_outcomes)
    );
}

/// A failed case blocks the run on the local copy and, on the online copy,
/// only in gate mode, where a case failed both ways is inconclusive and
/// blocks nothing (ADRs 0043 and 0047).
#[rstest]
#[case::a_failure_on_the_local_copy(false, false, json!([{ "checks": [false] }]), true)]
#[case::a_pass_on_the_local_copy(false, false, json!([{ "checks": [true] }]), false)]
#[case::an_online_failure_outside_gate_mode(true, false, json!([{ "checks": [false] }]), false)]
#[case::a_failure_marked_blocking_on_outside_gate_mode(
    false,
    false,
    json!([{ "blocking": "on", "checks": [false] }]),
    true
)]
#[case::a_failure_only_with_blocking_on_in_gate_mode(
    true,
    true,
    json!([{ "blocking": "on", "checks": [false] }, { "blocking": "off", "checks": [true] }]),
    true
)]
#[case::a_failure_both_ways_in_gate_mode(
    true,
    true,
    json!([{ "blocking": "on", "checks": [false] }, { "blocking": "off", "checks": [false] }]),
    false
)]
fn a_failed_case_blocks_unless_it_is_an_online_result_outside_gate_mode(
    #[case] online: bool,
    #[case] gate: bool,
    #[case] entries: serde_json::Value,
    #[case] blocks: bool,
) {
    let case: VectorCase = serde_json::from_value(json!({ "entries": entries })).unwrap();

    let document = ConformanceDocument::new(
        ConformanceHeader {
            online,
            gate,
            ..header()
        },
        vec![conformance_facts(&[case])],
    );

    assert_eq!(document.blocks(), blocks);
}

#[test]
fn the_reader_gives_back_the_document_written() {
    let written = JourneyDocument::new(journey_facts(
        &[StepOutcome::Passed],
        &[CheckOutcome::Passed],
        &[Some(OracleOutcome::Verified)],
    ));
    let journey = journey(
        vec![step("step 0", None)],
        vec![check("check 0", None)],
        vec![task("catalog/1")],
    );

    let read = read_compared(
        &serde_json::to_vec(&ResultDocument::Journey(written.clone())).unwrap(),
        &journey,
    )
    .unwrap();

    assert_eq!(read, written);
}

#[rstest]
fn the_reader_compares_each_vectors_document_with_its_journey(
    #[files("../../test-support/fixtures/outcome-vectors/journey-comparison/*.json")] path: PathBuf,
) {
    let vector: ComparisonVector = read_vector(&path);
    let journey = journey(
        vector.journey.steps,
        vector.journey.expect,
        vector.journey.tasks,
    );
    let seed = vector
        .document
        .seed
        .or_else(|| journey.proving_ground.as_ref().map(|table| table.seed));
    let facts = JourneyFacts {
        steps: vector.document.steps,
        checks: vector.document.checks,
        tasks: vector
            .document
            .tasks
            .into_iter()
            .map(|stored| TaskResult {
                answer: stored.answer.map(|outcome| {
                    answer(
                        stored.answered.as_ref().unwrap_or(&stored.id).as_str(),
                        outcome,
                    )
                }),
                id: stored.id,
                observations: Vec::new(),
            })
            .collect(),
        surface: vector.document.surface.unwrap_or_else(|| "cli".to_owned()),
        proving_ground: seed.map(|seed| ProvingGroundCopy {
            seed,
            ..proving_ground()
        }),
        ..journey_facts(&[], &[], &[])
    };

    let read = read_compared(&stored(facts, vector.document.outcome), &journey);

    assert_eq!(reading(&read), vector.read);
}

#[test]
fn the_reader_names_the_failed_step_a_document_leaves_out() {
    let journey = journey(
        vec![
            step("Start a crawl", None),
            step("Wait for the crawl to end", None),
            step("Export the Records", None),
        ],
        Vec::new(),
        Vec::new(),
    );
    let facts = JourneyFacts {
        steps: vec![
            StepResult {
                text: "Start a crawl".to_owned(),
                outcome: StepOutcome::Passed,
            },
            StepResult {
                text: "Export the Records".to_owned(),
                outcome: StepOutcome::Passed,
            },
        ],
        ..journey_facts(&[], &[], &[])
    };

    let refused = read_compared(&stored(facts, JourneyOutcome::Proved), &journey);

    assert!(
        matches!(
            &refused,
            Err(ReadError::NotTheJourneys {
                part: JourneyPart::Step,
                difference: Difference::Missing,
                number: 2,
                text,
            }) if text == "Wait for the crawl to end"
        ),
        "{refused:?}"
    );
}

#[test]
fn the_reader_refuses_a_document_compared_with_another_journey() {
    let mut other = journey(Vec::new(), Vec::new(), Vec::new());
    other.id = JourneyId::try_from("J-003".to_owned()).unwrap();
    let facts = journey_facts(&[], &[], &[]);

    let refused = read_compared(&stored(facts, JourneyOutcome::Proved), &other);

    assert!(
        matches!(
            &refused,
            Err(ReadError::OtherJourney { document, journey })
                if document.as_str() == "J-002" && journey.as_str() == "J-003"
        ),
        "{refused:?}"
    );
}

#[test]
fn the_reader_refuses_a_journey_stored_as_proved_beside_a_false_success() {
    let mut stored = serde_json::to_value(ResultDocument::Journey(JourneyDocument::new(
        journey_facts(
            &[StepOutcome::Passed],
            &[CheckOutcome::Passed],
            &[Some(OracleOutcome::FalseSuccess)],
        ),
    )))
    .unwrap();
    stored["outcome"] = json!("proved");

    let refused = read(&serde_json::to_vec(&stored).unwrap());

    assert!(
        matches!(
            refused,
            Err(ReadError::JourneyOutcome {
                stored: JourneyOutcome::Proved,
                derived: JourneyOutcome::Failed,
            })
        ),
        "{refused:?}"
    );
}

#[test]
fn the_reader_refuses_a_capability_stored_as_proved_beside_a_case_not_reached() {
    let document = ConformanceDocument::new(
        header(),
        vec![CapabilityFacts {
            name: "catalog.verified".to_owned(),
            cases: vec![CaseFacts {
                id: "catalog-crawl-reference".to_owned(),
                covers: Vec::new(),
                entries: Vec::new(),
            }],
        }],
    );
    let mut stored = serde_json::to_value(ResultDocument::Conformance(document)).unwrap();
    stored["capabilities"][0]["outcome"] = json!("proved");

    let refused = read(&serde_json::to_vec(&stored).unwrap());

    assert!(
        matches!(
            &refused,
            Err(ReadError::CapabilityOutcome {
                capability,
                stored: Outcome::Proved,
                derived: Outcome::Unproved,
            }) if capability == "catalog.verified"
        ),
        "{refused:?}"
    );
}

#[test]
fn the_reader_refuses_a_case_stored_as_passed_beside_a_failed_check() {
    let document = ConformanceDocument::new(
        header(),
        vec![CapabilityFacts {
            name: "catalog.verified".to_owned(),
            cases: vec![CaseFacts {
                id: "catalog-crawl-reference".to_owned(),
                covers: Vec::new(),
                entries: vec![CaseEntry {
                    mode: None,
                    blocking: None,
                    unreachable: Vec::new(),
                    checks: vec![CaseCheck {
                        name: "the oracle answers verified".to_owned(),
                        passed: false,
                    }],
                    evidence: Evidence::default(),
                }],
            }],
        }],
    );
    let mut stored = serde_json::to_value(ResultDocument::Conformance(document)).unwrap();
    stored["capabilities"][0]["cases"][0]["outcome"] = json!("passed");
    stored["capabilities"][0]["outcome"] = json!("proved");

    let refused = read(&serde_json::to_vec(&stored).unwrap());

    assert!(
        matches!(
            &refused,
            Err(ReadError::CaseOutcome {
                case,
                stored: CaseOutcome::Passed,
                derived: CaseOutcome::Failed,
            }) if case == "catalog-crawl-reference"
        ),
        "{refused:?}"
    );
}

#[test]
fn the_reader_refuses_a_document_of_another_kind() {
    let refused = read(br#"{ "kind": "benchmark" }"#);

    assert!(
        matches!(refused, Err(ReadError::Malformed(_))),
        "{refused:?}"
    );
}
