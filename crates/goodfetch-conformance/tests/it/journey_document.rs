//! The `journey` result document (ADR 0063): its outcome comes from its
//! constructor, and the reader every other reader goes through recomputes
//! it and refuses a document whose stored outcome disagrees.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::{JourneyId, TaskId};
use goodfetch_protocol::result_document::{
    CheckOutcome, CheckResult, Executable, JourneyDocument, JourneyFacts, JourneyOutcome,
    OracleAnswer, OracleOutcome, ProvingGroundCopy, ReadError, ResultDocument, StepOutcome,
    StepResult, SurfacePackage, TaskResult, read,
};
use serde_json::{Value, json};

use crate::{platform, schema_violations};

fn task_id() -> TaskId {
    TaskId::try_from("catalog/1".to_owned()).unwrap()
}

/// J-002's TypeScript variant as its helper would record it, the oracle
/// answering `outcome` for its one task.
fn facts(outcome: OracleOutcome) -> JourneyFacts {
    let Value::Object(details) = json!({
        "tool_status": "success",
        "missing": ["P-0042"],
        "extra": [],
        "wrong": [],
        "oracle_time_microseconds": 310,
    }) else {
        unreachable!("a JSON object literal")
    };
    JourneyFacts {
        journey_id: JourneyId::try_from("J-002".to_owned()).unwrap(),
        surface: "sdk-typescript".to_owned(),
        executable: Executable {
            product: Product::Foxhound,
            version: "0.0.0+0000000000000000000000000000000000000000".to_owned(),
        },
        surface_package: Some(SurfacePackage {
            name: "@goodfetch/foxhound".to_owned(),
            version: "0.0.0".to_owned(),
        }),
        platform: platform(),
        runtime: "node-24".to_owned(),
        proving_ground: Some(ProvingGroundCopy {
            version: "0.0.0+0000000000000000000000000000000000000000".to_owned(),
            seed: 2,
        }),
        steps: vec![
            StepResult {
                text: "Start a crawl of the catalog's first listing page".to_owned(),
                outcome: StepOutcome::Passed,
            },
            StepResult {
                text: "Wait for the crawl to end".to_owned(),
                outcome: StepOutcome::Passed,
            },
        ],
        checks: vec![CheckResult {
            text: "The crawl's Records are the catalog's products".to_owned(),
            task: Some(task_id()),
            outcome: CheckOutcome::Passed,
        }],
        tasks: vec![TaskResult {
            id: task_id(),
            answer: Some(OracleAnswer {
                task: task_id(),
                outcome,
                details,
            }),
            observations: vec![json!({ "requests": 111, "hostname": "catalog.test" })],
        }],
    }
}

/// Covers: story:2-84
#[test]
fn a_journey_document_matches_its_schema() {
    let document = ResultDocument::Journey(JourneyDocument::new(facts(OracleOutcome::Verified)));

    let violations = schema_violations(&document);

    assert_eq!(violations, Vec::<String>::new());
}

/// Covers: story:2-84
#[test]
fn one_task_answering_false_success_fails_the_journey_variant() {
    let document = JourneyDocument::new(facts(OracleOutcome::FalseSuccess));

    assert_eq!(document.outcome(), JourneyOutcome::Failed);
}

/// Covers: story:2-84
#[test]
fn the_reader_refuses_a_journey_document_stored_as_proved_beside_a_false_success() {
    let mut stored = serde_json::to_value(ResultDocument::Journey(JourneyDocument::new(facts(
        OracleOutcome::FalseSuccess,
    ))))
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

/// Covers: story:2-84
#[test]
fn the_reader_loads_the_document_a_constructor_wrote() {
    let written = ResultDocument::Journey(JourneyDocument::new(facts(OracleOutcome::Verified)));

    let loaded = read(&serde_json::to_vec(&written).unwrap()).unwrap();

    assert_eq!(loaded, written);
}
