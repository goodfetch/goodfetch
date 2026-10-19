//! `goodfetch-conformance read <file>`: the command line through which a
//! runner that is not written in Rust, such as the TypeScript and Python
//! helpers' tests, loads a result document with `goodfetch-protocol`'s
//! reader. It exits 0 when the reader accepts the document and 1 when it
//! refuses it.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::{JourneyId, TaskId};
use goodfetch_protocol::result_document::{
    CheckOutcome, CheckResult, Executable, JourneyDocument, JourneyFacts, OracleAnswer,
    OracleOutcome, ResultDocument, StepOutcome, StepResult, TaskResult,
};
use goodfetch_test_support::{TestEnv, command};
use serde_json::{Map, json};

use crate::platform;

const READ: &str = env!("CARGO_BIN_EXE_goodfetch-conformance");

/// A Journey variant whose one task the oracle answers `outcome`.
fn document(outcome: OracleOutcome) -> ResultDocument {
    let task = TaskId::try_from("catalog/2".to_owned()).unwrap();
    ResultDocument::Journey(JourneyDocument::new(JourneyFacts {
        journey_id: JourneyId::try_from("J-004".to_owned()).unwrap(),
        surface: "sdk-python".to_owned(),
        executable: Executable {
            product: Product::Foxhound,
            version: "0.0.0".to_owned(),
        },
        surface_package: None,
        platform: platform(),
        runtime: "cpython-3.14".to_owned(),
        proving_ground: None,
        steps: vec![StepResult {
            text: "Extract the product page".to_owned(),
            outcome: StepOutcome::Passed,
        }],
        checks: vec![CheckResult {
            text: "The product is the page's".to_owned(),
            task: Some(task.clone()),
            outcome: CheckOutcome::Passed,
        }],
        tasks: vec![TaskResult {
            id: task.clone(),
            answer: Some(OracleAnswer {
                task,
                outcome,
                details: Map::new(),
            }),
            observations: Vec::new(),
        }],
    }))
}

/// Covers: story:2-84
#[test]
fn read_accepts_a_document_the_constructor_wrote() {
    let env = TestEnv::new();
    let file = env.folder().join("journey.json");
    std::fs::write(
        &file,
        serde_json::to_vec(&document(OracleOutcome::Verified)).unwrap(),
    )
    .unwrap();

    let read = command(&env, READ).arg("read").arg(&file).output().unwrap();

    assert_eq!(read.status.code(), Some(0), "{read:?}");
}

/// Covers: story:2-84
#[test]
fn read_refuses_a_document_stored_as_proved_beside_a_false_success() {
    let env = TestEnv::new();
    let file = env.folder().join("journey.json");
    let mut stored = serde_json::to_value(document(OracleOutcome::FalseSuccess)).unwrap();
    stored["outcome"] = json!("proved");
    std::fs::write(&file, serde_json::to_vec(&stored).unwrap()).unwrap();

    let read = command(&env, READ).arg("read").arg(&file).output().unwrap();

    assert_eq!(read.status.code(), Some(1), "{read:?}");
}
