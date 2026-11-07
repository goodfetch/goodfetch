//! Running chosen tests a set number of times, whatever each run's result
//! (ADR 0064): the diagnostic run's 20 runs of a failed test, the daily
//! test-health job's 50 runs of each of the 10 oldest quarantined tests, and
//! the stress run's 20 runs of each new or changed test. This is no retry:
//! every run counts, and a pass changes no verdict. Each runner gets the
//! arguments that select the tests and repeat them; the `justfile`'s
//! `repeat` recipe appends them to the runner's command:
//! - nextest: `--stress-count`, with filtersets of the tests, of which a
//!   test matches any, and `--ignore-default-filter` when they hold LazyFS
//!   power cuts (`lazyfs::`), which the profiles' default filter leaves out
//!   (ADR 0060): they run only where `GOODFETCH_LAZYFS` names a LazyFS
//!   build, and elsewhere the selection fails rather than drop them;
//! - Vitest: `--repeats`, one less than the count, since it counts the runs
//!   after the first, and a name pattern. Its JUnit file reports the test
//!   once, with one `<failure>` per failed run;
//! - Playwright: `--repeat-each`, with a pattern of the tests;
//! - pytest: pytest-repeat's `--count`, with the tests' node IDs (a
//!   parameterized case's, one per run).
//!
//! The stress run stops at the end of its first red iteration (ADR 0064's
//! amendment of 2026-10-28): with `--until-red`, nextest's commands run one
//! iteration each (`--stress-count=1`), and the recipe runs them `count`
//! times, stopping after a red one. nextest itself has no such stop: its
//! `--max-fail` ends a stress run in the middle of the iteration that
//! reaches it.
//!
//! A runner's command runs in batches when its arguments would pass the
//! operating system's limits ([`Limits`]): each batch has the same counts
//! and a part of the tests, and every test is in exactly one batch.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::num::NonZeroU32;
use std::path::Path;
use std::process::{Command, ExitCode};

use anyhow::{Context as _, Result, bail, ensure};

use crate::environment::Environment;
use crate::output;
use crate::quarantine;
use crate::runners::{self, Runner, TestId, alternation};

/// The tests to repeat: the given ones, or the oldest quarantined ones.
pub(crate) enum Selection {
    Tests(Vec<(Runner, TestId)>),
    /// The quarantine list's oldest entries, at most this many.
    Quarantined(usize),
}

/// What one runner command's arguments may take, so that the operating
/// system starts it (`execve` fails with "Argument list too long" beyond).
#[derive(Clone, Copy)]
pub(crate) struct Limits {
    /// The bytes one argument may hold.
    pub(crate) argument: usize,
    /// The bytes one command's arguments may take in all, each counted with
    /// its terminating NUL and its pointer ([`footprint`]).
    pub(crate) command: usize,
}

impl Limits {
    /// Half of Linux's limit on one argument, 128 KiB (`MAX_ARG_STRLEN`;
    /// macOS has none), and half of what `getconf ARG_MAX` leaves after this
    /// process's environment, whose variables take `environment` bytes. The
    /// other halves cover the command's own words and the variables that
    /// Cargo, pnpm and uv add before the runner starts.
    fn of_host(environment: usize) -> Result<Self> {
        let output = Command::new("getconf")
            .arg("ARG_MAX")
            .output()
            .context("running `getconf ARG_MAX`")?;
        ensure!(
            output.status.success(),
            "`getconf ARG_MAX` exited with {}",
            output.status
        );
        let maximum: usize = String::from_utf8_lossy(&output.stdout)
            .trim()
            .parse()
            .context("reading `getconf ARG_MAX`")?;
        Ok(Self {
            argument: 64 * 1024,
            command: maximum.saturating_sub(environment) / 2,
        })
    }
}

/// The bytes `argument` takes in the area `execve` copies: its text, its
/// terminating NUL and its pointer.
fn footprint(argument: &str) -> usize {
    argument.len() + 1 + size_of::<usize>()
}

/// How a runner's arguments select its tests.
enum Selecting<'a> {
    /// Patterns, each `before`, the tests' alternatives joined by
    /// `separator`, then `after`; a test runs when it matches one. A command
    /// takes at most `per_command` of them (nextest takes several
    /// filtersets, Vitest and Playwright one pattern).
    Patterns {
        before: &'a str,
        separator: &'static str,
        after: &'static str,
        per_command: Option<usize>,
    },
    /// Each alternative is an argument (pytest's node IDs).
    Each,
}

/// The patterns that join `alternatives`, in order, each at most `room`
/// bytes long.
fn patterns(
    alternatives: Vec<String>,
    (before, separator, after): (&str, &str, &str),
    room: usize,
) -> Result<Vec<String>> {
    let mut patterns = Vec::new();
    let mut open = String::new();
    for alternative in alternatives {
        if !open.is_empty() {
            if open.len() + separator.len() + alternative.len() + after.len() <= room {
                open.push_str(separator);
                open.push_str(&alternative);
                continue;
            }
            open.push_str(after);
            patterns.push(std::mem::take(&mut open));
        }
        if before.len() + alternative.len() + after.len() > room {
            bail!("`{alternative}` alone passes the {room} bytes one argument may hold");
        }
        open.push_str(before);
        open.push_str(&alternative);
    }
    if !open.is_empty() {
        open.push_str(after);
        patterns.push(open);
    }
    Ok(patterns)
}

/// A runner's commands' arguments: each `fixed`, then the arguments that
/// select a part of the tests, every test in exactly one, within `limits`.
fn batches(
    fixed: &[String],
    alternatives: Vec<String>,
    selecting: &Selecting<'_>,
    limits: Limits,
) -> Result<Vec<Vec<String>>> {
    let base: usize = fixed.iter().map(|argument| footprint(argument)).sum();
    let (selecting_arguments, per_command) = match *selecting {
        Selecting::Each => (alternatives, None),
        Selecting::Patterns {
            before,
            separator,
            after,
            per_command,
        } => {
            let room = limits
                .argument
                .min(limits.command.saturating_sub(base + 1 + size_of::<usize>()));
            (
                patterns(alternatives, (before, separator, after), room)?,
                per_command,
            )
        }
    };
    let mut commands = Vec::new();
    let mut current: Vec<String> = Vec::new();
    let mut size = base;
    for argument in selecting_arguments {
        let full = per_command.is_some_and(|most| current.len() == most);
        if !current.is_empty() && (full || size + footprint(&argument) > limits.command) {
            commands.push([fixed, &current].concat());
            current.clear();
            size = base;
        }
        if size + footprint(&argument) > limits.command {
            bail!("`{argument}` alone passes what one command's arguments may take");
        }
        size += footprint(&argument);
        current.push(argument);
    }
    if !current.is_empty() {
        commands.push([fixed, &current].concat());
    }
    Ok(commands)
}

/// How nextest's commands run a count's iterations.
#[derive(Clone, Copy)]
pub(crate) enum Iterations {
    /// All in one command, `--stress-count=<count>`.
    InOneCommand,
    /// One a command, `--stress-count=1`, for the recipe to run `count`
    /// times and stop at the end of the first red one (the stress run's).
    OneACommand,
}

/// Each runner's commands, by its name, in the order they run: each
/// command's arguments, a batch of its tests; none for a runner with no test
/// chosen. `lazyfs` says whether the host has a LazyFS build.
pub(crate) fn arguments(
    root: &Path,
    selection: Selection,
    count: NonZeroU32,
    iterations: Iterations,
    limits: Limits,
    lazyfs: bool,
) -> Result<BTreeMap<&'static str, Vec<Vec<String>>>> {
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
    let (playwright_before, playwright_after) = runners::PLAYWRIGHT_AROUND;
    let playwright_before = format!("--grep={playwright_before}");
    let runners: [(&'static str, Vec<String>, Vec<String>, Selecting<'_>); 4] = [
        (
            "nextest",
            nextest_fixed(&of(Runner::Nextest), count, iterations, lazyfs)?,
            of(Runner::Nextest)
                .iter()
                .map(|test| {
                    format!(
                        "(binary_id(={}) & test(={}))",
                        test.nextest_binary(),
                        test.nextest_name()
                    )
                })
                .collect(),
            Selecting::Patterns {
                before: "--filterset=",
                separator: " | ",
                after: "",
                per_command: None,
            },
        ),
        (
            "vitest",
            vec![format!("--repeats={}", count.get() - 1)],
            of(Runner::Vitest)
                .iter()
                .map(|test| alternation(std::iter::once(test.as_str().to_owned())))
                .collect(),
            Selecting::Patterns {
                before: "--testNamePattern=^(?:",
                separator: "|",
                after: ")$",
                per_command: Some(1),
            },
        ),
        (
            "playwright",
            vec![format!("--repeat-each={count}")],
            of(Runner::Playwright)
                .iter()
                .map(|test| runners::playwright_alternative(test))
                .collect(),
            Selecting::Patterns {
                before: &playwright_before,
                separator: "|",
                after: playwright_after,
                per_command: Some(1),
            },
        ),
        (
            "pytest",
            vec![format!("--count={count}")],
            of(Runner::Pytest)
                .iter()
                .flat_map(|test| pytest_node_ids(test.as_str(), count))
                .collect(),
            Selecting::Each,
        ),
    ];

    let mut commands = BTreeMap::new();
    for (runner, fixed, alternatives, selecting) in runners {
        let runner_commands = if alternatives.is_empty() {
            Vec::new()
        } else {
            batches(&fixed, alternatives, &selecting, limits)
                .with_context(|| format!("dividing {runner}'s tests into commands"))?
        };
        commands.insert(runner, runner_commands);
    }
    Ok(commands)
}

/// nextest's arguments besides its filtersets for `tests`, which run
/// `count` times in one command or once a command (`iterations`): with
/// `--ignore-default-filter` when they hold a LazyFS power cut (`lazyfs::`),
/// which the profiles' default filter leaves out (ADR 0060), and a failure
/// instead when the host has no LazyFS build (`lazyfs`).
fn nextest_fixed(
    tests: &[&TestId],
    count: NonZeroU32,
    iterations: Iterations,
    lazyfs: bool,
) -> Result<Vec<String>> {
    let mut fixed = vec!["--no-fail-fast".to_owned()];
    if let Some(power_cut) = tests
        .iter()
        .find(|test| test.nextest_name().starts_with("lazyfs::"))
    {
        if !lazyfs {
            bail!(
                "`{}` is a LazyFS power cut, which runs only where GOODFETCH_LAZYFS names a LazyFS build (`just lazyfs-test` on Linux); repeat it there rather than drop it",
                power_cut.as_str()
            );
        }
        fixed.push("--ignore-default-filter".to_owned());
    }
    let count = match iterations {
        Iterations::InOneCommand => count,
        Iterations::OneACommand => NonZeroU32::MIN,
    };
    fixed.push(format!("--stress-count={count}"));
    Ok(fixed)
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

/// `xtask repeat`'s arguments: `--test`, repeatable, `--tests-file` or
/// `--quarantined`, and `--until-red` for the stress run.
#[derive(clap::Args)]
#[command(group(
    clap::ArgGroup::new("tests")
        .required(true)
        .args(["test", "tests_file", "quarantined"])
))]
pub(crate) struct Arguments {
    #[arg(long)]
    count: NonZeroU32,
    /// nextest's commands run one iteration each, for the recipe to run
    /// `count` times and stop at the end of the first red one.
    #[arg(long)]
    until_red: bool,
    /// A test, as `<runner>=<test ID>` (the quarantine list's IDs).
    #[arg(long, value_parser = parse_test)]
    test: Vec<(Runner, TestId)>,
    /// A file of tests, one `<runner>=<test ID>` a line: a selection longer
    /// than a command line holds (the stress run's).
    #[arg(long)]
    tests_file: Option<std::path::PathBuf>,
    /// The quarantine list's oldest entries, at most this many.
    #[arg(long)]
    quarantined: Option<usize>,
}

/// The tests a `--tests-file` lists, one `<runner>=<test ID>` a line.
fn read_tests(path: &Path) -> Result<Vec<(Runner, TestId)>> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("reading the tests in {}", path.display()))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(parse_test)
        .collect()
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

/// Prints each runner's commands as one JSON object: each runner's name and
/// its commands in the order they run, each command its arguments,
/// `{"nextest": [["--stress-count=20", "--filterset=…"]], "vitest": [], …}`.
pub(crate) fn run(arguments: Arguments, environment: &Environment) -> Result<ExitCode> {
    let selection = match (arguments.quarantined, arguments.tests_file) {
        (Some(limit), _) => Selection::Quarantined(limit),
        (None, Some(path)) => Selection::Tests(read_tests(&environment.root.join(path))?),
        (None, None) => Selection::Tests(arguments.test),
    };
    let limits = Limits::of_host(environment.variables_bytes)?;
    let commands = self::arguments(
        &environment.root,
        selection,
        arguments.count,
        if arguments.until_red {
            Iterations::OneACommand
        } else {
            Iterations::InOneCommand
        },
        limits,
        environment.lazyfs,
    )?;
    output::line(&serde_json::to_string(&commands).context("writing the commands as JSON")?);
    Ok(ExitCode::SUCCESS)
}
