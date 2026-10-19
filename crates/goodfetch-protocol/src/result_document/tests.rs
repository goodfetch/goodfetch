use std::path::PathBuf;

use rstest::rstest;
use serde::Deserialize;
use serde_json::{Map, json};

use super::{
    CapabilityFacts, CaseCheck, CaseEntry, CaseFacts, CaseOutcome, CheckOutcome, CheckResult,
    ConformanceDocument, ConformanceHeader, Evidence, Executable, JourneyDocument, JourneyFacts,
    JourneyOutcome, OracleAnswer, OracleOutcome, Outcome, Platform, ProvingGroundCopy, ReadError,
    ResultDocument, StepOutcome, StepResult, Suite, Target, TargetIdentity, TaskResult, read,
};
use crate::catalog::Product;
use crate::journey::{JourneyId, TaskId};

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
    outcome: JourneyOutcome,
}

/// One hand-written conformance outcome vector of
/// `test-support/fixtures/outcome-vectors/conformance/`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ConformanceVector {
    #[expect(dead_code, reason = "the vector's own explanation, read by people")]
    description: String,
    cases: Vec<VectorCase>,
    case_outcomes: Vec<CaseOutcome>,
    outcome: Outcome,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorCase {
    entries: Vec<VectorEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VectorEntry {
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
        journey_id: JourneyId::try_from("J-002".to_owned()).unwrap(),
        surface: "cli".to_owned(),
        executable: Executable {
            product: Product::Foxhound,
            version: "0.0.0+0000000000000000000000000000000000000000".to_owned(),
        },
        surface_package: None,
        platform: platform(),
        runtime: "rust".to_owned(),
        proving_ground: Some(proving_ground()),
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
                        mode: None,
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

    let document =
        JourneyDocument::new(journey_facts(&vector.steps, &vector.checks, &vector.tasks));

    assert_eq!(document.outcome(), vector.outcome);
}

#[rstest]
fn the_conformance_constructor_gives_each_vectors_outcomes(
    #[files("../../test-support/fixtures/outcome-vectors/conformance/*.json")] path: PathBuf,
) {
    let vector: ConformanceVector = read_vector(&path);

    let document = ConformanceDocument::new(header(), vec![conformance_facts(&vector.cases)]);

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

#[test]
fn the_reader_gives_back_the_document_written() {
    let written = ResultDocument::Journey(JourneyDocument::new(journey_facts(
        &[StepOutcome::Passed],
        &[CheckOutcome::Passed],
        &[Some(OracleOutcome::Verified)],
    )));

    let read = read(&serde_json::to_vec(&written).unwrap()).unwrap();

    assert_eq!(read, written);
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
