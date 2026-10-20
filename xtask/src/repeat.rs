//! Running chosen tests a set number of times, whatever each run's result
//! (ADR 0064): the diagnostic run's 20 runs of a failed test, and the daily
//! test-health job's 50 runs of each of the 10 oldest quarantined tests.
//! This is no retry: every run counts, and a pass changes no verdict. Each
//! runner gets the arguments that select the tests and repeat them; the
//! `justfile`'s `repeat` recipe appends them to the runner's command:
//! - nextest: `--stress-count`, with a filterset of the tests;
//! - Vitest: `--repeats`, one less than the count, since it counts the runs
//!   after the first, and a name pattern. Its JUnit file reports the test
//!   once, with one `<failure>` per failed run;
//! - Playwright: `--repeat-each`, with a pattern of the tests;
//! - pytest: pytest-repeat's `--count`, with the tests' node IDs (a
//!   parameterized case's, one per run).

use std::num::NonZeroU32;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};

use crate::environment::Environment;
use crate::output;
use crate::quarantine::{self, Runner, TestId, alternation};

/// The tests to repeat: the given ones, or the oldest quarantined ones.
pub(crate) enum Selection {
    Tests(Vec<(Runner, TestId)>),
    /// The quarantine list's oldest entries, at most this many.
    Quarantined(usize),
}

/// A `--test` value, `<runner>=<test ID>`.
fn parse_test(value: &str) -> Result<(Runner, TestId)> {
    let (runner, id) = value
        .split_once('=')
        .with_context(|| format!("`{value}` is not `<runner>=<test ID>`"))?;
    let runner = Runner::from_name(runner)
        .with_context(|| format!("`{runner}` is not nextest, vitest, playwright or pytest"))?;
    Ok((runner, TestId::parse(runner, id)?))
}

/// Each runner's arguments, as `(runner, argument)` pairs in order.
pub(crate) fn arguments(
    root: &Path,
    selection: Selection,
    count: NonZeroU32,
) -> Result<Vec<(&'static str, String)>> {
    let tests = match selection {
        Selection::Tests(tests) => tests,
        Selection::Quarantined(limit) => {
            let mut entries = quarantine::read(root)?;
            // The day only, so the text sorts as the date does.
            entries.sort_by_key(|entry| entry.date.to_string());
            entries
                .into_iter()
                .take(limit)
                .map(|entry| (entry.runner, entry.test))
                .collect()
        }
    };
    let of = |runner: Runner| -> Vec<&TestId> {
        tests
            .iter()
            .filter(|(test_runner, _)| *test_runner == runner)
            .map(|(_, test)| test)
            .collect()
    };
    let mut arguments = Vec::new();

    let nextest = of(Runner::Nextest);
    if !nextest.is_empty() {
        let filter = nextest
            .iter()
            .map(|test| {
                format!(
                    "(binary_id(={}) & test(={}))",
                    test.nextest_binary(),
                    test.nextest_name()
                )
            })
            .collect::<Vec<_>>()
            .join(" | ");
        arguments.push(("nextest", "--no-fail-fast".to_owned()));
        arguments.push(("nextest", format!("--stress-count={count}")));
        arguments.push(("nextest", format!("--filterset={filter}")));
    }

    let vitest = of(Runner::Vitest);
    if !vitest.is_empty() {
        let names = alternation(vitest.iter().map(|test| test.as_str().to_owned()));
        arguments.push(("vitest", format!("--repeats={}", count.get() - 1)));
        arguments.push(("vitest", format!("--testNamePattern=^(?:{names})$")));
    }

    let playwright = of(Runner::Playwright);
    if !playwright.is_empty() {
        arguments.push(("playwright", format!("--repeat-each={count}")));
        arguments.push((
            "playwright",
            format!("--grep={}", quarantine::playwright_pattern(&playwright)),
        ));
    }

    let pytest = of(Runner::Pytest);
    if !pytest.is_empty() {
        arguments.push(("pytest", format!("--count={count}")));
        for test in pytest {
            arguments.extend(
                pytest_node_ids(test.as_str(), count)
                    .into_iter()
                    .map(|id| ("pytest", id)),
            );
        }
    }
    Ok(arguments)
}

/// The node IDs that select each of `count` runs of a pytest test. pytest-repeat
/// parameterizes each test by its run, last: a test's node ID selects all
/// of its runs (`test_x[1-20]` …), but a parameterized case's, `test_x[a]`,
/// selects none of `test_x[a-1-20]` …, so each run's is given. With one run
/// it parameterizes nothing.
fn pytest_node_ids(id: &str, count: NonZeroU32) -> Vec<String> {
    match id.strip_suffix(']') {
        Some(case) if count.get() > 1 => (1..=count.get())
            .map(|run| format!("{case}-{run}-{count}]"))
            .collect(),
        _ => vec![id.to_owned()],
    }
}

/// `xtask repeat`'s arguments: `--test`, repeatable, or `--quarantined`.
#[derive(clap::Args)]
#[command(group(clap::ArgGroup::new("tests").required(true).args(["test", "quarantined"])))]
pub(crate) struct Arguments {
    #[arg(long)]
    count: NonZeroU32,
    /// A test, as `<runner>=<test ID>` (the quarantine list's IDs).
    #[arg(long, value_parser = parse_test)]
    test: Vec<(Runner, TestId)>,
    /// The quarantine list's oldest entries, at most this many.
    #[arg(long)]
    quarantined: Option<usize>,
}

pub(crate) fn run(arguments: Arguments, environment: &Environment) -> Result<ExitCode> {
    let selection = match arguments.quarantined {
        Some(limit) => Selection::Quarantined(limit),
        None => Selection::Tests(arguments.test),
    };
    for (runner, argument) in self::arguments(&environment.root, selection, arguments.count)? {
        output::line(&format!("{runner}\t{argument}"));
    }
    Ok(ExitCode::SUCCESS)
}
