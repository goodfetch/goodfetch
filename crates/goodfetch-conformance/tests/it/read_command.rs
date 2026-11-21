//! `goodfetch-conformance read <file>`: the command line through which a
//! runner that is not written in Rust, such as the TypeScript and Python
//! helpers' tests, loads a result document with `goodfetch-protocol`'s
//! reader. It exits 0 when the reader accepts the document and 1 when it
//! refuses it.

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::journey::{JourneyId, TaskId};
use goodfetch_protocol::result_document::{
    CheckOutcome, CheckResult, Executable, JourneyDocument, JourneyFacts, OracleAnswer,
    OracleOutcome, ProvingGroundCopy, ResultDocument, StepOutcome, StepResult, TaskResult,
};
use std::process::Output;

use goodfetch_test_support::{TestEnv, command};
use serde_json::{Map, json};

use crate::platform;

const READ: &str = env!("CARGO_BIN_EXE_goodfetch-conformance");

/// A Journey variant whose one task the oracle answers `outcome`.
fn document(outcome: OracleOutcome) -> ResultDocument {
    let task = TaskId::try_from("catalog/2".to_owned()).unwrap();
    ResultDocument::Journey(JourneyDocument::new(JourneyFacts {
        test_errors: Vec::new(),
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

/// J-951, a Journey of two steps whose one check names `catalog/1`.
const J951: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/it/fixtures/journey-run/J-951.toml"
);

const VERSION_STEP: &str = "Ask the executable for its version";
const READ_STEP: &str = "Read every product the catalog lists, from its first listing page";

/// A J-951 variant that recorded `steps`, each passed, its task verified.
fn j951_document(steps: &[&str]) -> ResultDocument {
    let task = TaskId::try_from("catalog/1".to_owned()).unwrap();
    ResultDocument::Journey(JourneyDocument::new(JourneyFacts {
        test_errors: Vec::new(),
        journey_id: JourneyId::try_from("J-951".to_owned()).unwrap(),
        surface: "cli".to_owned(),
        executable: Executable {
            product: Product::Collie,
            version: "0.0.0".to_owned(),
        },
        surface_package: None,
        platform: platform(),
        runtime: "rust".to_owned(),
        // J-951's own seed, as its Journey file pins it.
        proving_ground: Some(ProvingGroundCopy {
            version: "0.0.0".to_owned(),
            seed: 951,
        }),
        steps: steps
            .iter()
            .map(|text| StepResult {
                text: (*text).to_owned(),
                outcome: StepOutcome::Passed,
            })
            .collect(),
        checks: vec![CheckResult {
            text: "The oracle verifies the catalog's Records".to_owned(),
            task: Some(task.clone()),
            outcome: CheckOutcome::Passed,
        }],
        tasks: vec![TaskResult {
            id: task.clone(),
            answer: Some(OracleAnswer {
                task,
                outcome: OracleOutcome::Verified,
                details: Map::new(),
            }),
            observations: Vec::new(),
        }],
    }))
}

/// Writes `document` into the test's folder and runs `read` on it, then `arguments`.
fn read_with(env: &TestEnv, document: &ResultDocument, arguments: &[&str]) -> Output {
    let file = env.folder().join("journey.json");
    std::fs::write(&file, serde_json::to_vec(document).unwrap()).unwrap();
    command(env, READ)
        .arg("read")
        .arg(&file)
        .args(arguments)
        .output()
        .unwrap()
}

#[test]
fn read_with_its_journey_refuses_a_document_that_left_a_step_out() {
    let env = TestEnv::new();
    let document = j951_document(&[READ_STEP]);

    let read = read_with(&env, &document, &["--journey", J951]);

    assert_eq!(read.status.code(), Some(1), "{read:?}");
}

#[test]
fn read_with_its_journey_accepts_a_document_holding_its_steps() {
    let env = TestEnv::new();
    let document = j951_document(&[VERSION_STEP, READ_STEP]);

    let read = read_with(&env, &document, &["--journey", J951]);

    assert_eq!(read.status.code(), Some(0), "{read:?}");
}
