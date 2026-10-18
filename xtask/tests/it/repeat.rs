//! Running chosen tests many times (ADR 0064): the diagnostic run's 20 runs
//! of a failed test and the daily test-health job's 50 runs of the oldest
//! quarantined tests. `xtask repeat` gives each runner the arguments that
//! select those tests and repeat them; each runner then runs the fixture
//! workspace's tests for real, and its JUnit file shows every run.
//! Covers: story:1-65

use rstest::rstest;

use crate::workspace::Workspace;

/// Each runner's JUnit file and the command that runs it, before the
/// arguments `xtask repeat` gives (as the `justfile`'s `repeat` recipe runs it).
fn command(runner: &str) -> (&'static str, Vec<&'static str>, &'static str) {
    match runner {
        "nextest" => (
            "cargo",
            vec!["nextest", "run"],
            "target/nextest/default/junit.xml",
        ),
        "vitest" => (
            "node_modules/.bin/vitest",
            vec!["run", "--reporter=junit", "--outputFile=junit/vitest.xml"],
            "junit/vitest.xml",
        ),
        "playwright" => (
            "node_modules/.bin/playwright",
            vec!["test", "--reporter=junit"],
            "junit/playwright.xml",
        ),
        "pytest" => (
            ".venv/bin/pytest",
            vec!["-p", "no:randomly", "--junitxml=junit/pytest.xml"],
            "junit/pytest.xml",
        ),
        _ => panic!("no runner {runner}"),
    }
}

/// Runs `runner` with the arguments `xtask repeat <selection>` gave it, and
/// returns each test case it ran (skipped ones left out) with its failures.
fn repeated(workspace: &Workspace, runner: &str, selection: &[&str]) -> Vec<(String, usize)> {
    let outcome = workspace.xtask(&[&["repeat"], selection].concat());
    assert_eq!(outcome.code, 0);
    let arguments: Vec<&str> = outcome
        .stdout
        .lines()
        .filter_map(|line| line.split_once('\t'))
        .filter(|(line_runner, _)| *line_runner == runner)
        .map(|(_, argument)| argument)
        .collect();
    let (program, base, junit) = command(runner);
    let program = if program == "cargo" {
        program.into()
    } else {
        workspace.path().join(program)
    };
    workspace
        .command(program)
        .args(base)
        .args(&arguments)
        .env("PLAYWRIGHT_JUNIT_OUTPUT_FILE", "junit/playwright.xml")
        .output()
        .unwrap();
    let report = String::from_utf8(workspace.read(junit).unwrap()).unwrap();
    report
        .split("<testcase ")
        .skip(1)
        .filter_map(|case| {
            let body = case.split("</testcase>").next().unwrap_or(case);
            let body = body.split("<testcase").next().unwrap_or(body);
            if body.contains("<skipped") {
                return None;
            }
            let name = format!(" {case}");
            let name = name.split(" name=\"").nth(1)?.split('"').next()?;
            let name = name.replace("&gt;", ">");
            Some((name, body.matches("<failure").count()))
        })
        .collect()
}

fn runs(name: &str, count: usize) -> Vec<(String, usize)> {
    vec![(name.to_owned(), 0); count]
}

/// A Vitest test that passes on its first run only, so that its JUnit test
/// case (Vitest reports a repeated test once) counts the runs after it.
const PASSES_ONCE: &str = r#"import { expect, test } from "vitest";

let runs = 0;

test("passes on its first run only", () => {
  runs += 1;
  expect(runs).toBe(1);
});
"#;

#[rstest]
#[case::nextest("nextest", "sample tests::flaky", runs("tests::flaky", 3))]
#[case::playwright(
    "playwright",
    "e2e/sample.spec.ts › sample › flaky",
    runs("sample › flaky", 3)
)]
#[case::pytest(
    "pytest",
    "tests/test_sample.py::test_flaky",
    vec![
        ("test_flaky[1-3]".to_owned(), 0),
        ("test_flaky[2-3]".to_owned(), 0),
        ("test_flaky[3-3]".to_owned(), 0),
    ]
)]
#[case::vitest(
    "vitest",
    "passes on its first run only",
    vec![("passes on its first run only".to_owned(), 2)]
)]
fn chosen_test_runs_the_given_number_of_times_and_no_other_test_runs(
    #[case] runner: &str,
    #[case] id: &str,
    #[case] expected: Vec<(String, usize)>,
) {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.link_installed_runners();
    workspace.write("web/once.test.ts", PASSES_ONCE.as_bytes());
    workspace.write(
        ".config/nextest.toml",
        b"[profile.default.junit]\npath = \"junit.xml\"\n",
    );

    let ran = repeated(
        &workspace,
        runner,
        &["--count", "3", "--test", &format!("{runner}={id}")],
    );

    assert_eq!(ran, expected);
}

#[test]
fn oldest_quarantined_tests_are_the_ones_repeated() {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.link_installed_runners();
    workspace.write(
        "quarantine.toml",
        b"[[test]]\nrunner = \"pytest\"\nid = \"tests/test_sample.py::test_steady\"\nissue = 1602\ndate = 2026-10-10\n\n\
          [[test]]\nrunner = \"pytest\"\nid = \"tests/test_sample.py::test_flaky\"\nissue = 1601\ndate = 2026-10-01\n",
    );

    let ran = repeated(
        &workspace,
        "pytest",
        &["--count", "2", "--quarantined", "1"],
    );

    assert_eq!(
        ran,
        vec![
            ("test_flaky[1-2]".to_owned(), 0),
            ("test_flaky[2-2]".to_owned(), 0),
        ]
    );
}
