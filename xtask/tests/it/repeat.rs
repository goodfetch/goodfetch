//! Running chosen tests many times (ADR 0064): the diagnostic run's 20 runs
//! of a failed test and the daily test-health job's 50 runs of the oldest
//! quarantined tests. `xtask repeat` gives each runner the arguments that
//! select those tests and repeat them; each runner then runs the fixture
//! workspace's tests for real, and its JUnit file shows every run.
//! Covers: story:1-113

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
        .map(|(_, rest)| rest.split_once('\t').expect("a batch, then an argument"))
        .map(|(batch, argument)| {
            assert_eq!(batch, "1", "a few tests run in one command");
            argument
        })
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

/// A parameterized pytest test, whose cases pytest-repeat renames: it adds
/// each run's `<run>-<count>` to the case's parameter ID.
const PARAMETERIZED: &str = r#"import pytest


@pytest.mark.parametrize("name", ["a.log", "b"])
def test_reads(name: str) -> None:
    assert name
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
#[case::pytest_parameterized(
    "pytest",
    "tests/test_params.py::test_reads[a.log]",
    vec![
        ("test_reads[a.log-1-3]".to_owned(), 0),
        ("test_reads[a.log-2-3]".to_owned(), 0),
        ("test_reads[a.log-3-3]".to_owned(), 0),
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
    workspace.write("tests/test_params.py", PARAMETERIZED.as_bytes());
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

/// Each command `xtask repeat` has a runner run, in order: the lines'
/// arguments grouped by what comes before each one's last tab (the runner
/// and the batch).
fn invocations(stdout: &str) -> Vec<Vec<String>> {
    let mut invocations: Vec<(&str, Vec<String>)> = Vec::new();
    for (command, argument) in stdout.lines().filter_map(|line| line.rsplit_once('\t')) {
        match invocations.iter_mut().find(|(key, _)| *key == command) {
            Some((_, arguments)) => arguments.push(argument.to_owned()),
            None => invocations.push((command, vec![argument.to_owned()])),
        }
    }
    invocations
        .into_iter()
        .map(|(_, arguments)| arguments)
        .collect()
}

/// Starts a program with `arguments`, as the `repeat` recipe starts each
/// runner, so the operating system's limits on an argument list apply.
fn starts(arguments: &[String]) -> std::io::Result<()> {
    std::process::Command::new("true")
        .args(arguments)
        .status()
        .map(drop)
}

/// The gate's stress run of 2026-10-22: 3,224 new or changed nextest tests
/// made one filterset of 420 KB, over Linux's 128 KiB for one argument,
/// and `cargo` never started ("Argument list too long").
#[test]
fn nextest_starts_when_the_filterset_is_longer_than_one_argument_may_be() {
    let workspace = Workspace::from_fixture("quarantine");
    let tests: Vec<String> = (0..1_500)
        .map(|index| {
            format!(
                "--test=nextest=goodfetch-engine::it lifecycle::a_long_descriptive_test_name_that_says_what_the_engine_does_{index:05}"
            )
        })
        .collect();
    let mut arguments = vec!["repeat", "--count", "20"];
    arguments.extend(tests.iter().map(String::as_str));

    let outcome = workspace.xtask(&arguments);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    for invocation in invocations(&outcome.stdout) {
        starts(&invocation).expect("the runner starts with its arguments");
    }
}

/// The host's `getconf ARG_MAX`: the bytes one command's arguments and
/// environment may take in all.
fn arg_max() -> usize {
    let output = std::process::Command::new("getconf")
        .arg("ARG_MAX")
        .output()
        .unwrap();
    String::from_utf8(output.stdout)
        .unwrap()
        .trim()
        .parse()
        .unwrap()
}

/// The tests that one command's arguments select, as their runner's
/// pattern alternatives or node IDs (the tests' names hold no character a
/// pattern escapes), and whether it repeats each 20 times.
fn selected(runner: &str, arguments: &[String]) -> (Vec<String>, bool) {
    let (count, before, separator, after) = match runner {
        "nextest" => ("--stress-count=20", "--filterset=", " | ", ""),
        "vitest" => ("--repeats=19", "--testNamePattern=^(?:", "|", ")$"),
        "playwright" => ("--repeat-each=20", "--grep=(?:^| )(?:", "|", ")(?: |$)"),
        "pytest" => ("--count=20", "", "", ""),
        _ => panic!("no runner {runner}"),
    };
    let tests = arguments
        .iter()
        .filter(|argument| !argument.starts_with("--") || argument.starts_with(before))
        .filter(|argument| *argument != count && *argument != "--no-fail-fast")
        .flat_map(|argument| {
            let pattern = argument
                .strip_prefix(before)
                .and_then(|rest| rest.strip_suffix(after))
                .unwrap_or_else(|| panic!("`{argument}` is not a {runner} selection"));
            if separator.is_empty() {
                vec![pattern.to_owned()]
            } else {
                pattern.split(separator).map(str::to_owned).collect()
            }
        })
        .collect();
    (tests, arguments.iter().any(|argument| argument == count))
}

/// The stress run hands `just repeat` every new or changed test, and a
/// change can bring more than one command line holds: the selection comes
/// in a file, and each runner runs it in as many commands as the operating
/// system's limits need, every test in exactly one, each repeated.
#[rstest]
#[case::nextest("nextest", |index: usize| format!("goodfetch-engine::it lifecycle::{}_{index:05}", "a_long_descriptive_name".repeat(8)), |test: &str| {
    let (binary, name) = test.split_once(' ').unwrap();
    format!("(binary_id(={binary}) & test(={name}))")
})]
#[case::vitest("vitest", |index: usize| format!("a page {} {index:05}", "keeps its state across a long descriptive title".repeat(4)), str::to_owned)]
#[case::playwright("playwright", |index: usize| format!("chromium e2e/console_spec_ts the console {} {index:05}", "shows a long descriptive title".repeat(5)), str::to_owned)]
#[case::pytest("pytest", |index: usize| format!("tests/test_sample.py::test_{}_{index:05}", "a_long_descriptive_name".repeat(8)), str::to_owned)]
fn a_selection_longer_than_a_command_line_runs_every_test_in_batches(
    #[case] runner: &str,
    #[case] test: fn(usize) -> String,
    #[case] alternative: fn(&str) -> String,
) {
    let workspace = Workspace::from_fixture("quarantine");
    let mut tests = Vec::new();
    let mut length = 0;
    while length <= arg_max() {
        let id = test(tests.len());
        length += id.len();
        tests.push(id);
    }
    let lines: Vec<String> = tests.iter().map(|id| format!("{runner}={id}")).collect();
    workspace.write("selection.txt", lines.join("\n").as_bytes());

    let outcome = workspace.xtask(&["repeat", "--count", "20", "--tests-file", "selection.txt"]);

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let commands = invocations(&outcome.stdout);
    assert!(commands.len() > 1, "{} commands", commands.len());
    let mut ran = Vec::new();
    for command in &commands {
        starts(command).expect("the runner starts with its arguments");
        let (tests, repeated) = selected(runner, command);
        assert!(repeated, "every command repeats its tests");
        ran.extend(tests);
    }
    let expected: Vec<String> = tests.iter().map(|id| alternative(id)).collect();
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

/// nextest's default filter leaves the LazyFS power cuts out (ADR 0060), and
/// a stress filterset only narrows it: on a host with LazyFS, `repeat`
/// lifts the default filter so a selected `lazyfs::` test runs every time.
#[test]
fn a_lazyfs_test_is_repeated_past_the_default_filter_where_lazyfs_is() {
    let workspace = Workspace::from_fixture("quarantine");

    let outcome = workspace.xtask_with_env(
        &[
            "repeat",
            "--count",
            "20",
            "--test",
            "nextest=sample::it lazyfs::an_unsynced_write_is_lost",
        ],
        &[("GOODFETCH_LAZYFS", std::ffi::OsStr::new("/opt/lazyfs"))],
    );

    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    assert!(
        outcome
            .stdout
            .lines()
            .any(|line| line == "nextest\t1\t--ignore-default-filter"),
        "{}",
        outcome.stdout
    );
}

/// Without LazyFS, a selected `lazyfs::` test fails the run rather than
/// being dropped by the default filter, so its stress runs are never
/// silently skipped (TESTING.md, Before merge).
#[test]
fn a_lazyfs_test_fails_the_repeat_where_lazyfs_is_missing() {
    let workspace = Workspace::from_fixture("quarantine");

    let outcome = workspace.xtask(&[
        "repeat",
        "--count",
        "20",
        "--test",
        "nextest=sample::it lazyfs::an_unsynced_write_is_lost",
    ]);

    assert_eq!(outcome.code, 2, "{}", outcome.stdout);
    assert!(
        outcome.stderr.contains("GOODFETCH_LAZYFS"),
        "{}",
        outcome.stderr
    );
}

/// A nextest test that fails on its second run only, so the last run passes.
const FAILS_ON_ITS_SECOND_RUN: &str = r#"#[cfg(test)]
mod tests {
    #[test]
    fn flaky() {
        assert_ne!(std::env::var("NEXTEST_STRESS_CURRENT").as_deref(), Ok("1"));
    }
}
"#;

/// The gate's stress run of 2026-10-22: `just repeat 20` exited 0 with "19
/// passed, 1 failed", since nextest before 0.9.148 gave a stress run the last
/// run's exit code when fail-fast was off (nextest-rs/nextest#3624), so a
/// failed run went unnoticed. `.config/nextest.toml` requires a nextest that
/// counts every run.
#[test]
fn nextest_fails_when_one_run_of_a_repeated_test_fails() {
    let workspace = Workspace::from_fixture("quarantine");
    workspace.write("src/lib.rs", FAILS_ON_ITS_SECOND_RUN.as_bytes());
    workspace.write(
        ".config/nextest.toml",
        b"[profile.default]\nfail-fast = false\n\n[profile.default.junit]\npath = \"junit.xml\"\n",
    );
    let outcome = workspace.xtask(&[
        "repeat",
        "--count",
        "3",
        "--test",
        "nextest=sample tests::flaky",
    ]);
    assert_eq!(outcome.code, 0, "{}", outcome.stderr);
    let arguments = invocations(&outcome.stdout).concat();

    let status = workspace
        .command("cargo")
        .args(["nextest", "run"])
        .args(&arguments)
        .status()
        .unwrap();

    let report = workspace.read_text("target/nextest/default/junit.xml");
    assert_eq!(
        (
            report.matches("<testcase ").count(),
            report.matches("<failure").count()
        ),
        (3, 1),
        "{report}"
    );
    assert!(!status.success(), "nextest exited with {status}");
}
