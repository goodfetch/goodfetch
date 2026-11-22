//! J-033, "Raise a run's Limit from \"Needs you\" at the CLI's prompt, see
//! the run leave the list, then start the program again with its Run ID and
//! see it resume", on the `cli` surface (ADRs 0031, 0041 and 0063): a
//! person sets a monthly install Limit on requests through Collie's shipped
//! executable, a TypeScript program reads the catalog under a Run ID of its
//! own until the Limit stops its Run and cannot raise the Limit itself, the
//! person reads "Needs you" and raises the Limit at the `usage raise`
//! prompt, and the program, started again with its Run ID, resumes the Run
//! and finishes reading the catalog. The program's steps run the TypeScript
//! SDK's program `tests/programs/catalog-reader.ts` with Node, on the same
//! executable.

use std::path::{Path, PathBuf};

use goodfetch_protocol::result_document::ResultDocument;
use goodfetch_test_support::{JourneyRun, browser_cache_in_home, pty};
use serde_json::{Value, json};

const COLLIE: &str = env!("CARGO_BIN_EXE_goodfetch-collie");

/// The program's Run ID, its own for good.
const RUN_ID: &str = "catalog-reader";

/// The install's monthly requests Limit the person sets: too small for
/// the catalog's 110 pages, ten listing pages of ten products each.
const SMALL: &str = "20";

/// The Limit the person raises it to, which covers the whole catalog
/// again on top of what the first start used.
const RAISED: &str = "1000";

/// The period of the install's Limit, as `usage set` and `usage raise`
/// take it.
const MONTHLY: &str = r#"{"every":"month","time_zone":"UTC"}"#;

/// The TypeScript SDK's J-033 program.
fn program() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../sdks/typescript/collie/tests/programs/catalog-reader.ts")
}

/// What the program printed, its one JSON line, when it runs `args` with
/// Node on `run`'s executable: the Node on the running test's `PATH`,
/// which the program gets on top of the test's cleared environment.
#[expect(
    clippy::disallowed_methods,
    reason = "the test forwards its own PATH to the Node it starts (TESTING.md)"
)]
fn run_program<C>(run: &JourneyRun<C>, args: &[&str]) -> Value {
    let mut command = run.env().process_command("node");
    if let Some(path) = std::env::var_os("PATH") {
        command.env("PATH", path);
    }
    let output = command
        .arg(program())
        .args(args)
        .env("GOODFETCH_COLLIE_ENGINE_PATH", run.engine())
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"))
}

/// The kind, Run and Usage kind of its Limit of each item `needs-you list`
/// prints.
fn needs_you<C>(run: &JourneyRun<C>) -> Vec<Value> {
    let output = run
        .command()
        .args(["needs-you", "list", "--json"])
        .output()
        .unwrap();
    assert!(output.status.success(), "{output:?}");
    let listed: Value =
        serde_json::from_slice(&output.stdout).unwrap_or_else(|_| panic!("{output:?}"));
    listed["items"]
        .as_array()
        .unwrap_or_else(|| panic!("{listed}"))
        .iter()
        .map(|item| {
            json!([
                item["kind"],
                item["run"],
                item["details"]["limit"]["usage_kind"]
            ])
        })
        .collect()
}

/// Covers: story:12-23, story:12-100
#[cfg(unix)]
#[test]
#[expect(
    clippy::too_many_lines,
    reason = "a Journey variant is one test of its every step and check, in its file's order (TESTING.md, Journeys)"
)]
fn cli() {
    let mut run = JourneyRun::start("J-033", "cli", Path::new(COLLIE), ResultDocument::journey)
        .expect("J-033's cli variant starts");
    let proving_ground = run.proving_ground().expect("J-033 has a Proving ground");
    let start = proving_ground.start_url("catalog/1").unwrap();
    run.env()
        .user_configuration(&proving_ground.description().reaching());
    browser_cache_in_home(run.env()).expect("Playwright's Chrome for Testing is installed");
    let mut stopped = Value::Null;
    let mut raised_itself = Value::Null;
    let mut listed = Vec::new();
    let mut raise = None;
    let mut left = Vec::new();
    let mut resumed = Value::Null;

    run.step(
        "Set a monthly Limit on the install's requests too small for reading the whole catalog",
        |run| {
            let output = run
                .command()
                .args([
                    "usage",
                    "set",
                    "--scope",
                    "install",
                    "--usage-kind",
                    "requests",
                ])
                .args(["--period", MONTHLY, "--amount", SMALL])
                .output()
                .unwrap();
            assert!(output.status.success(), "{output:?}");
        },
    );
    run.step(
        "Run a program that reads every product of the catalog under a Run ID of its own, until the Limit stops its run",
        |run| stopped = run_program(run, &["read", &start, RUN_ID]),
    );
    run.step("Have the program ask to raise the Limit itself", |run| {
        raised_itself = run_program(run, &["raise"]);
    });
    run.step("Read \"Needs you\" with `needs-you list`", |run| {
        listed = needs_you(run);
    });
    run.step("Raise the Limit at the `usage raise` prompt", |run| {
        let mut prompt = pty(run.env(), run.engine())
            .args([
                "usage",
                "raise",
                "--scope",
                "install",
                "--usage-kind",
                "requests",
            ])
            .args(["--period", MONTHLY, "--amount", RAISED])
            .spawn()
            .unwrap();
        prompt.expect("Raise the install Limit").unwrap();
        prompt.send_line("y").unwrap();
        raise = Some(prompt.finish().unwrap());
        left = needs_you(run);
    });
    run.step("Start the program again with the run's Run ID", |run| {
        resumed = run_program(run, &["read", &start, RUN_ID]);
    });

    run.check(
        "The Limit stopped the run, `stopped` and `incomplete`, and \"Needs you\" listed it with the Limit that stopped it",
        |_| {
            assert_eq!(
                (
                    &stopped["startedAs"],
                    &stopped["stoppedBy"],
                    &stopped["lifecycle"],
                    &stopped["completeness"],
                    &listed,
                ),
                (
                    &json!("new"),
                    &json!("limit_reached"),
                    &json!("stopped"),
                    &json!("incomplete"),
                    &vec![json!(["run_stopped_by_limit", RUN_ID, "requests"])],
                ),
                "{stopped}"
            );
        },
    );
    run.check(
        "The program could not raise the Limit itself: its call was refused with `person_required`",
        |_| assert_eq!(raised_itself, json!({ "refused": "person_required" })),
    );
    run.check(
        "Once a person answered yes at the prompt, the run left \"Needs you\"",
        |_| {
            let raise = raise.as_ref().expect("the prompt ran");
            assert_eq!(
                (raise.exit, &left),
                (Some(0), &Vec::new()),
                "{}",
                raise.stripped()
            );
        },
    );
    run.check(
        "The program's second start answered `resumed`, and the run finished under the raised Limit",
        |_| {
            assert_eq!(
                (
                    &resumed["startedAs"],
                    &resumed["stoppedBy"],
                    &resumed["lifecycle"]
                ),
                (&json!("resumed"), &Value::Null, &json!("finished")),
                "{resumed}"
            );
        },
    );
    run.expect_task(
        "The oracle verifies the resumed run's result",
        &json!({ "status": "success", "records": resumed["products"] }),
    );
    run.finish()
        .expect("J-033's document is written")
        .assert_proved();
}
