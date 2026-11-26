//! The `journey` result document (ADR 0063) matches its schema. Its outcome
//! and the reader that recomputes it and compares it with its Journey
//! (Seam C) are `goodfetch-protocol`'s `result_document` tests, over its
//! committed vectors.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::{JourneyId, TaskId};
use goodfetch_protocol::result_document::{
    CheckOutcome, CheckResult, Executable, JourneyDocument, JourneyFacts, OracleAnswer,
    OracleOutcome, ProvingGroundCopy, ResultDocument, StepOutcome, StepResult, SurfacePackage,
    TaskResult,
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
        test_errors: Vec::new(),
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
