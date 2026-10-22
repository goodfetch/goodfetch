//! The Rust Journey helper, `goodfetch-test-support`'s `JourneyRun`, with
//! `goodfetch-protocol`'s constructor and reader (Seam C): the document it
//! writes is loaded from its file as every reader loads it, through `read`
//! and then compared with its typed Journey.

use std::path::{Path, PathBuf};

use goodfetch_conformance::{ProvingGround, Record, References};
use goodfetch_protocol::journey::{Journey, TaskId};
use goodfetch_protocol::result_document::{
    JourneyDocument, JourneyFacts, JourneyOutcome, JourneyPart, LoadedDocument, ReadError,
    ResultDocument, read, read_journey_document,
};
use goodfetch_test_support::{Finished, JourneyRun, Tool, tools};
use serde_json::json;

/// J-951, a Collie Journey on seed 951 whose one check names `catalog/1`.
fn journey_file() -> PathBuf {
    fixture("J-951")
}

/// A fixture Journey: J-951, or J-952, on seed 952, whose two checks name
/// `catalog/1`.
fn fixture(id: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/it/fixtures/journey-run")
        .join(format!("{id}.toml"))
}

const VERSION_STEP: &str = "Ask the executable for its version";
const READ_STEP: &str = "Read every product the catalog lists, from its first listing page";
const CATALOG_CHECK: &str = "The oracle verifies the catalog's Records";

/// Starts J-951's `cli` variant on Collie's executable, with the
/// constructor every Rust Journey test passes.
fn start() -> JourneyRun<impl FnOnce(JourneyFacts) -> ResultDocument> {
    start_file(&journey_file())
}

fn start_file(file: &Path) -> JourneyRun<impl FnOnce(JourneyFacts) -> ResultDocument + use<>> {
    let collie = tools().unwrap().path(Tool::Collie).unwrap().to_owned();
    JourneyRun::start_file(file, "cli", &collie, ResultDocument::journey).unwrap()
}

/// The catalog's products, read from its pages the way a tool reads them.
fn catalog_records<C>(run: &JourneyRun<C>) -> Vec<Record> {
    let address = run.proving_ground().unwrap().address().to_string();
    let references = References::read(&ProvingGround::local(&address)).unwrap();
    references.records[&TaskId::try_from("catalog/1".to_owned()).unwrap()].clone()
}

/// Loads a written document as every reader does.
fn read_compared(path: &Path) -> Result<JourneyDocument, ReadError> {
    read_compared_with(path, &journey_file())
}

fn read_compared_with(path: &Path, journey: &Path) -> Result<JourneyDocument, ReadError> {
    let journey: Journey = toml::from_str(&std::fs::read_to_string(journey).unwrap()).unwrap();
    match read(&std::fs::read(path).unwrap())? {
        LoadedDocument::Journey(loaded) => read_journey_document(loaded, &journey),
        LoadedDocument::Conformance(loaded) => panic!("a journey document was read as {loaded:?}"),
    }
}

/// Runs J-951's steps, then submits `records` as a success.
fn run_submitting(records: impl FnOnce(Vec<Record>) -> Vec<Record>) -> Finished<ResultDocument> {
    let mut run = start();
    let mut read_records = Vec::new();
    run.step(VERSION_STEP, |run| {
        run.command().arg("--version").assert().success();
    });
    run.step(READ_STEP, |run| {
        read_records = catalog_records(run);
    });
    let submission = json!({ "status": "success", "records": records(read_records) });
    run.expect_task(CATALOG_CHECK, &submission);
    run.finish().unwrap()
}

#[test]
fn a_run_that_skips_a_step_writes_a_document_the_reader_refuses() {
    let mut run = start();
    let mut records = Vec::new();

    run.step(READ_STEP, |run| {
        records = catalog_records(run);
    });
    run.expect_task(
        CATALOG_CHECK,
        &json!({ "status": "success", "records": records }),
    );
    let finished = run.finish().unwrap();

    let refused = read_compared(finished.path());
    assert!(
        matches!(
            &refused,
            Err(ReadError::NotTheJourneys {
                part: JourneyPart::Step,
                ..
            })
        ),
        "{refused:?}"
    );
}

#[test]
fn a_run_whose_task_the_oracle_answers_false_success_is_failed() {
    let finished = run_submitting(|mut records| {
        records.pop();
        records
    });

    let document = read_compared(finished.path()).unwrap();

    let answer = document.facts().tasks[0].answer.as_ref().unwrap();
    assert_eq!(answer.outcome.as_str(), "false_success");
    assert_eq!(document.outcome(), JourneyOutcome::Failed);
}

#[test]
fn a_complete_passing_run_is_proved_and_accepted() {
    let finished = run_submitting(|records| records);

    let document = read_compared(finished.path());

    assert_eq!(document.unwrap().outcome(), JourneyOutcome::Proved);
    finished.assert_proved();
}

/// J-952's two checks name one task: each is judged on its own read, the
/// document holds one answer per check, and the reader accepts it and
/// derives the variant's outcome from both answers.
#[test]
fn two_checks_naming_one_task_each_take_their_own_answer() {
    let file = fixture("J-952");
    let mut run = start_file(&file);
    let mut records = Vec::new();
    run.step(
        "Read every product the catalog lists, from its first listing page, twice",
        |run| {
            records = catalog_records(run);
        },
    );
    run.expect_task(
        "The oracle verifies the first read's Records",
        &json!({ "status": "success", "records": records }),
    );
    records.pop();
    run.expect_task(
        "The oracle verifies the second read's Records",
        &json!({ "status": "success", "records": records }),
    );
    let finished = run.finish().unwrap();

    let document = read_compared_with(finished.path(), &file).unwrap();

    let answers: Vec<&str> = document
        .facts()
        .tasks
        .iter()
        .map(|task| task.answer.as_ref().unwrap().outcome.as_str())
        .collect();
    assert_eq!(answers, ["verified", "false_success"]);
    assert_eq!(document.outcome(), JourneyOutcome::Failed);
}
