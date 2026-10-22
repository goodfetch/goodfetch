//! `JourneyRun`, the Rust Journey helper (ADRs 0063, 0066 and 0069): it reads
//! a Journey file, starts the variant's Proving ground and home, records each
//! step, check and oracle answer in the file's order, and writes the
//! document a constructor it is given makes of them. These tests give it a
//! stub constructor only, so test support depends on no Goodfetch crate,
//! not even here; the document's own constructor and reader are tested with
//! it in `goodfetch-conformance` (Seam C).

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use goodfetch_test_support::{
    ENGINE_VARIABLE, JourneyRun, JourneyRunError, RESULT_DOCUMENTS_VARIABLE, SEED_VARIABLE,
    TOOLS_VARIABLE, TestEnv, Tool, UnfinishedJourneyRun, Variables, VariantNotProved, tools,
};
use rstest::rstest;
use serde_json::{Value, json};

use crate::probe;

/// The folder of the fixture Journeys: J-901, a Collie Journey that touches
/// no page, J-902, a Foxhound Journey on seed 902 naming `catalog/1`, and
/// J-903, one on seed 903 whose two checks name `catalog/1`.
fn journey(id: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/it/fixtures/journey-run")
        .join(format!("{id}.toml"))
}

/// The commit's executable of a Product, from the test tool manifest.
fn executable(tool: Tool) -> PathBuf {
    tools().unwrap().path(tool).unwrap().to_owned()
}

/// A stub of `goodfetch-protocol`'s constructor: the facts it is handed,
/// as a `journey` document whose outcome is `outcome`.
fn stub(outcome: &'static str) -> impl FnOnce(Value) -> Value {
    move |mut facts| {
        facts["kind"] = json!("journey");
        facts["outcome"] = json!(outcome);
        facts
    }
}

/// The document a finished run wrote, read back from its file.
fn written(path: &Path) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

fn outcomes(document: &Value, list: &str) -> Vec<String> {
    document[list]
        .as_array()
        .unwrap()
        .iter()
        .map(|entry| format!("{} {}", entry["outcome"].as_str().unwrap(), entry["text"]))
        .collect()
}

#[test]
fn steps_after_a_failed_step_are_not_reached_and_the_manual_step_stays_manual() {
    let mut run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub("failed"),
    )
    .unwrap();
    let mut ran_after_the_failure = false;

    run.step("Ask the executable for its version", |run| {
        run.command().arg("--version").assert().success();
    });
    run.step("Ask the executable for its help", |run| {
        run.command().arg("--no-such-flag").assert().success();
    });
    run.step("Ask the executable for its version once more", |_| {
        ran_after_the_failure = true;
    });
    run.check("The version names the Product", |_| {
        ran_after_the_failure = true;
    });
    let finished = run.finish().unwrap();

    let document = written(finished.path());
    assert!(!ran_after_the_failure);
    assert_eq!(
        outcomes(&document, "steps"),
        [
            r#"passed "Ask the executable for its version""#,
            r#"manual "Read the version aloud""#,
            r#"failed "Ask the executable for its help""#,
            r#"not_reached "Ask the executable for its version once more""#,
        ]
    );
    assert_eq!(
        outcomes(&document, "checks"),
        [r#"not_reached "The version names the Product""#]
    );
}

#[test]
fn a_complete_run_records_every_step_and_check_in_the_journeys_order() {
    let mut run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub("proved"),
    )
    .unwrap();

    for step in [
        "Ask the executable for its version",
        "Ask the executable for its help",
        "Ask the executable for its version once more",
    ] {
        run.step(step, |run| {
            run.command().arg("--help").assert().success();
        });
    }
    run.check("The version names the Product", |_| Ok::<_, String>(()));
    let finished = run.finish().unwrap();

    let document = written(finished.path());
    assert_eq!(
        outcomes(&document, "steps"),
        [
            r#"passed "Ask the executable for its version""#,
            r#"manual "Read the version aloud""#,
            r#"passed "Ask the executable for its help""#,
            r#"passed "Ask the executable for its version once more""#,
        ]
    );
    assert_eq!(
        outcomes(&document, "checks"),
        [r#"passed "The version names the Product""#]
    );
    finished.assert_proved();
}

#[test]
fn a_step_never_called_is_not_reached() {
    let mut run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub("failed"),
    )
    .unwrap();

    run.step("Ask the executable for its version", |_| ());
    let finished = run.finish().unwrap();

    let document = written(finished.path());
    assert_eq!(
        outcomes(&document, "steps"),
        [
            r#"passed "Ask the executable for its version""#,
            r#"manual "Read the version aloud""#,
            r#"not_reached "Ask the executable for its help""#,
            r#"not_reached "Ask the executable for its version once more""#,
        ]
    );
}

#[test]
fn a_step_called_out_of_the_journeys_order_fails_without_running() {
    let mut run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub("failed"),
    )
    .unwrap();
    let mut ran = false;

    run.step("Ask the executable for its version", |_| ());
    run.step("Ask the executable for its version once more", |_| {
        ran = true;
    });
    let finished = run.finish().unwrap();

    let document = written(finished.path());
    assert!(!ran);
    assert_eq!(
        outcomes(&document, "steps"),
        [
            r#"passed "Ask the executable for its version""#,
            r#"manual "Read the version aloud""#,
            r#"failed "Ask the executable for its version once more""#,
            r#"not_reached "Ask the executable for its help""#,
        ]
    );
}

#[test]
fn a_journey_with_no_proving_ground_table_starts_none() {
    let run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub("failed"),
    )
    .unwrap();

    let started = run.proving_ground().is_some();
    let finished = run.finish().unwrap();

    assert!(!started);
    assert_eq!(written(finished.path()).get("proving_ground"), None);
}

#[test]
fn a_journey_with_a_proving_ground_table_starts_one_on_its_own_seed() {
    let run = JourneyRun::start_file(
        &journey("J-902"),
        "cli",
        &executable(Tool::Foxhound),
        stub("failed"),
    )
    .unwrap();

    let seed = run.proving_ground().unwrap().description().seed;
    let version = run.proving_ground().unwrap().description().version.clone();
    let finished = run.finish().unwrap();

    assert_eq!(seed, 902);
    assert_eq!(
        written(finished.path())["proving_ground"],
        json!({ "version": version, "seed": 902 })
    );
}

#[test]
fn the_test_seed_variable_does_not_change_the_journeys_seed() {
    let env = TestEnv::new();
    let variables = Variables::from_process();
    let mut command = probe(&env, "probe::runs_journey_902");
    command
        .env(SEED_VARIABLE, "4242")
        .env(TOOLS_VARIABLE, variables.get(TOOLS_VARIABLE).unwrap())
        .env(RESULT_DOCUMENTS_VARIABLE, env.folder());

    let status = env.spawn(&mut command).unwrap().close_stdin_and_wait();

    assert!(status.unwrap().success());
    let document = written(&env.folder().join("J-902@cli.json"));
    assert_eq!(document["proving_ground"]["seed"], json!(902));
}

/// `just journeys --engine <path>` sets `GOODFETCH_TEST_ENGINE`: a skew run
/// drives that executable, whatever executable the test passes (ADR 0066).
#[test]
fn a_skew_run_drives_the_engine_variable_in_place_of_the_executable_passed() {
    let env = TestEnv::new();
    let variables = Variables::from_process();
    let mut command = probe(&env, "probe::finishes_journey_901_on_collie");
    command
        .env(ENGINE_VARIABLE, executable(Tool::Foxhound))
        .env(TOOLS_VARIABLE, variables.get(TOOLS_VARIABLE).unwrap())
        .env(RESULT_DOCUMENTS_VARIABLE, env.folder());

    let status = env.spawn(&mut command).unwrap().close_stdin_and_wait();

    assert!(status.unwrap().success());
    let document = written(&env.folder().join("J-901@cli.json"));
    assert_eq!(document["executable"]["product"], json!("foxhound"));
}

/// Under `just test`, every test's runs write into one folder, where two
/// tests running one variant, as these tests and Seam C's do, write one
/// file. Each run's own document is the one its test reads; the folder
/// keeps the last.
#[test]
fn a_runs_document_stays_its_own_when_another_run_of_its_variant_finishes_after_it() {
    let env = TestEnv::new();
    let variables = Variables::from_process();
    let mut command = probe(&env, "probe::runs_journey_901_twice");
    command
        .env(TOOLS_VARIABLE, variables.get(TOOLS_VARIABLE).unwrap())
        .env(RESULT_DOCUMENTS_VARIABLE, env.folder());

    let status = env.spawn(&mut command).unwrap().close_stdin_and_wait();

    assert!(status.unwrap().success());
    let kept = written(&env.folder().join("J-901@cli.json"));
    assert_eq!(kept["outcome"], json!("failed"));
}

#[test]
fn the_oracles_answer_to_a_submission_is_its_tasks_answer() {
    let mut run = JourneyRun::start_file(
        &journey("J-902"),
        "cli",
        &executable(Tool::Foxhound),
        stub("failed"),
    )
    .unwrap();

    run.step("Read the catalog from its first listing page", |_| ());
    let answer = run
        .expect_task(
            "The oracle verifies the catalog's Records",
            &json!({ "status": "failure", "records": [] }),
        )
        .cloned();
    let finished = run.finish().unwrap();

    let document = written(finished.path());
    assert_eq!(answer.unwrap()["outcome"], json!("failed"));
    assert_eq!(document["tasks"][0]["id"], json!("catalog/1"));
    assert_eq!(document["tasks"][0]["answer"]["outcome"], json!("failed"));
    assert_eq!(
        outcomes(&document, "checks"),
        [r#"failed "The oracle verifies the catalog's Records""#]
    );
}

/// Each check naming a task is judged on its own submission, so two
/// checks naming one task take two answers, written in the checks' order;
/// after the first answer that is not `verified`, nothing more goes to the
/// oracle.
#[test]
fn each_check_naming_a_task_is_judged_on_its_own_submission() {
    let mut run = JourneyRun::start_file(
        &journey("J-903"),
        "cli",
        &executable(Tool::Foxhound),
        stub("failed"),
    )
    .unwrap();
    let failure = json!({ "status": "failure", "records": [] });

    run.step(
        "Read the catalog from its first listing page, twice",
        |_| (),
    );
    let first = run
        .expect_task("The oracle verifies the first read's Records", &failure)
        .cloned();
    let second = run
        .expect_task("The oracle verifies the second read's Records", &failure)
        .cloned();
    let finished = run.finish().unwrap();

    let document = written(finished.path());
    let first = first.unwrap();
    assert_eq!(first["outcome"], json!("failed"));
    assert_eq!(second, None);
    assert_eq!(
        outcomes(&document, "checks"),
        [
            r#"failed "The oracle verifies the first read's Records""#,
            r#"not_reached "The oracle verifies the second read's Records""#,
        ]
    );
    assert_eq!(
        document["tasks"],
        json!([{ "id": "catalog/1", "answer": first }, { "id": "catalog/1" }])
    );
}

#[rstest]
#[case::a_check_naming_no_task("The oracle verifies nothing", 1)]
#[case::a_check_judged_twice("The oracle verifies the first read's Records", 2)]
fn a_submission_for_a_check_that_names_no_task_or_was_judged_fails_the_run(
    #[case] check: &str,
    #[case] calls: usize,
) {
    let mut run = JourneyRun::start_file(
        &journey("J-903"),
        "cli",
        &executable(Tool::Foxhound),
        stub("failed"),
    )
    .unwrap();

    run.step(
        "Read the catalog from its first listing page, twice",
        |_| (),
    );
    for _ in 1..calls {
        run.expect_task(check, &json!({ "status": "failure", "records": [] }));
    }
    let answer = run
        .expect_task(check, &json!({ "status": "success", "records": [] }))
        .cloned();
    let finished = run.finish().unwrap();

    // The last call adds its own failure: the first call, when there is
    // one, failed on the oracle's answer.
    assert_eq!(answer, None);
    assert_eq!(
        finished.failures().len(),
        calls,
        "{:?}",
        finished.failures()
    );
}

/// A check the Journey scopes to other variants' surfaces is not the
/// variant's: J-901's `sdk-rust` check is neither run nor recorded in its
/// `cli` variant's document.
#[test]
fn a_check_of_other_variants_is_not_the_variants_own() {
    let mut run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub("failed"),
    )
    .unwrap();

    run.check("The Rust crate reports the executable's version", |_| ());
    let finished = run.finish().unwrap();

    let document = written(finished.path());
    assert_eq!(
        outcomes(&document, "checks"),
        [
            r#"not_reached "The version names the Product""#,
            r#"failed "The Rust crate reports the executable's version""#,
        ]
    );
    assert_eq!(finished.failures().len(), 1, "{:?}", finished.failures());
}

#[test]
fn a_run_records_the_executable_it_was_given_with_its_product_version() {
    let run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub("failed"),
    )
    .unwrap();

    let finished = run.finish().unwrap();

    let document = written(finished.path());
    assert_eq!(
        (&document["journey_id"], &document["surface"]),
        (&json!("J-901"), &json!("cli"))
    );
    assert_eq!(
        document["executable"],
        json!({ "product": "collie", "version": env!("CARGO_PKG_VERSION") })
    );
}

#[test]
fn a_collie_journeys_cli_steps_run_on_foxhounds_executable_as_its_foxhound_cli_variant() {
    let mut run = JourneyRun::start_file(
        &journey("J-901"),
        "foxhound-cli",
        &executable(Tool::Foxhound),
        stub("failed"),
    )
    .unwrap();
    let mut reported = String::new();

    run.step("Ask the executable for its version", |run| {
        let output = run.command().arg("--version").output().unwrap();
        reported = String::from_utf8(output.stdout).unwrap();
    });
    let finished = run.finish().unwrap();

    let document = written(finished.path());
    assert!(reported.starts_with("goodfetch-foxhound "), "{reported}");
    assert_eq!(document["surface"], json!("foxhound-cli"));
    assert_eq!(document["executable"]["product"], json!("foxhound"));
    assert_eq!(
        finished.path().file_name().unwrap().to_str(),
        Some("J-901@foxhound-cli.json")
    );
}

/// Each case runs on Collie's executable unless its surface is
/// `foxhound-cli`, which runs on Foxhound's (ADR 0094).
#[rstest]
#[case::a_surface_the_journey_does_not_list("J-901", "sdk-python", false)]
#[case::foxhound_cli_of_a_foxhound_journey("J-902", "foxhound-cli", false)]
#[case::a_listed_surface("J-901", "sdk-rust", true)]
#[case::foxhound_cli_of_a_collie_journey_with_cli("J-901", "foxhound-cli", true)]
fn a_run_starts_only_on_a_surface_its_journey_lists(
    #[case] id: &str,
    #[case] surface: &str,
    #[case] starts: bool,
) {
    let tool = if surface == "foxhound-cli" {
        Tool::Foxhound
    } else {
        Tool::Collie
    };

    let started = JourneyRun::start_file(&journey(id), surface, &executable(tool), stub("failed"));

    match started {
        Ok(run) => {
            assert!(starts, "{id}@{surface} started");
            let _finished = run.finish().unwrap();
        }
        Err(error) => {
            assert!(!starts, "{error}");
            assert!(
                matches!(error, JourneyRunError::UnlistedSurface { .. }),
                "{error}"
            );
        }
    }
}

#[test]
fn a_run_dropped_unfinished_fails_the_test() {
    let run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub("failed"),
    )
    .unwrap();

    let dropped = catch_unwind(AssertUnwindSafe(move || drop(run)));

    let payload = dropped.unwrap_err();
    let unfinished = payload.downcast_ref::<UnfinishedJourneyRun>();
    assert_eq!(
        unfinished.map(|unfinished| unfinished.variant.as_str()),
        Some("J-901@cli")
    );
}

#[rstest]
#[case::proved("proved", None)]
#[case::failed("failed", Some("failed"))]
fn assert_proved_fails_on_a_variant_its_document_does_not_prove(
    #[case] outcome: &'static str,
    #[case] refused: Option<&str>,
) {
    let run = JourneyRun::start_file(
        &journey("J-901"),
        "cli",
        &executable(Tool::Collie),
        stub(outcome),
    )
    .unwrap();
    let finished = run.finish().unwrap();

    let asserted = catch_unwind(AssertUnwindSafe(|| finished.assert_proved()));

    let payload = asserted.err();
    let not_proved = payload
        .as_ref()
        .and_then(|payload| payload.downcast_ref::<VariantNotProved>());
    assert_eq!(
        not_proved.map(|not_proved| not_proved.outcome.as_str()),
        refused
    );
}
