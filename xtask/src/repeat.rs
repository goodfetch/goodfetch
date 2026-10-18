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
//! - pytest: pytest-repeat's `--count`, with the tests' node IDs.

use std::num::NonZeroU32;
use std::path::Path;

use anyhow::{Context as _, Result};

use crate::quarantine::{self, Runner, TestId, alternation};

/// The tests to repeat: the given ones, or the oldest quarantined ones.
pub(crate) enum Selection {
    Tests(Vec<(Runner, TestId)>),
    /// The quarantine list's oldest entries, at most this many.
    Quarantined(usize),
}

/// A `--test` value, `<runner>=<test ID>`.
pub(crate) fn parse_test(value: &str) -> Result<(Runner, TestId)> {
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

    // Playwright matches its pattern against the project, the file and the
    // titles joined by spaces.
    let playwright = of(Runner::Playwright);
    if !playwright.is_empty() {
        let paths = alternation(
            playwright
                .iter()
                .map(|test| test.as_str().replace(" › ", " ")),
        );
        arguments.push(("playwright", format!("--repeat-each={count}")));
        arguments.push(("playwright", format!("--grep=(?:^| )(?:{paths})(?: |$)")));
    }

    let pytest = of(Runner::Pytest);
    if !pytest.is_empty() {
        arguments.push(("pytest", format!("--count={count}")));
        for test in pytest {
            arguments.push(("pytest", test.as_str().to_owned()));
        }
    }
    Ok(arguments)
}
