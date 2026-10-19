//! The failure summary on a job's page (ADR 0061; TESTING.md, Sharding,
//! caching and reports): every failed test of the job's JUnit files, with
//! its failure's first lines and the `goodfetch-test-seed=<n>` lines it
//! printed, so a contributor reads the failure without opening the logs.
//! Every test job runs it, pass or fail. A JUnit file it cannot read is
//! reported on the page and fails the command, never skipped.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::junit::{self, Ending, TestCase};
use crate::output;

/// How many of a failure's lines the page shows.
const FIRST_LINES: usize = 15;

/// How many failed tests the page lists; a job past it is broken, not
/// unlucky, and GitHub refuses a step summary over 1 MiB.
const LISTED_AT_MOST: usize = 100;

/// The line test support prints when it creates a test's seed.
const SEED_PREFIX: &str = "goodfetch-test-seed=";

/// `xtask junit-summary`'s arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The folder of the job's JUnit files, each `<runner>[-<suffix>].xml`.
    #[arg(long, default_value = "target/junit")]
    junit: PathBuf,
    /// The file the summary is appended to, such as `$GITHUB_STEP_SUMMARY`.
    #[arg(long)]
    summary: PathBuf,
}

/// A failed test, from the JUnit file that holds it.
struct Failed {
    file: String,
    case: TestCase,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let folder = environment.path(&arguments.junit);
    let files = junit_files(&folder)?;
    let mut failed = Vec::new();
    let mut malformed = Vec::new();
    for file in &files {
        let path = folder.join(file);
        match read(&path, file) {
            Ok(cases) => failed.extend(
                cases
                    .into_iter()
                    .filter(|case| case.ending == Ending::Failed)
                    .map(|case| Failed {
                        file: file.clone(),
                        case,
                    }),
            ),
            Err(error) => malformed.push(Finding::new(
                "malformed-junit",
                arguments.junit.join(file).display().to_string(),
                format!("{error:#}"),
            )),
        }
    }
    output::append(
        &environment.path(&arguments.summary),
        &summary(files.len(), &failed, &malformed),
    )?;
    Ok(check::report(&malformed))
}

/// The names of the `.xml` files directly in `folder`, sorted; none when
/// the folder does not exist, as when the job stopped before its tests.
fn junit_files(folder: &Path) -> Result<Vec<String>> {
    if !folder.is_dir() {
        return Ok(Vec::new());
    }
    let mut files = Vec::new();
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("listing {}", folder.display()))?
    {
        let entry = entry.with_context(|| format!("listing {}", folder.display()))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        if entry.path().is_file()
            && Path::new(name)
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("xml"))
        {
            files.push(name.to_owned());
        }
    }
    files.sort();
    Ok(files)
}

fn read(path: &Path, file: &str) -> Result<Vec<TestCase>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    junit::test_cases(junit::runner(file), &text)
}

fn summary(files: usize, failed: &[Failed], malformed: &[Finding]) -> String {
    let mut text = String::from("### Failed tests\n\n");
    if files == 0 {
        text.push_str("The job wrote no JUnit file.\n");
    } else if failed.is_empty() {
        let _ = writeln!(text, "No test failed in {}.", count(files, "JUnit file"));
    } else {
        let _ = writeln!(
            text,
            "{} failed in {}.",
            count(failed.len(), "test"),
            count(files, "JUnit file")
        );
    }
    for failed in failed.iter().take(LISTED_AT_MOST) {
        text.push('\n');
        text.push_str(&section(failed));
    }
    if failed.len() > LISTED_AT_MOST {
        let _ = writeln!(
            text,
            "\n{} are in the JUnit files the job uploads.",
            count(failed.len() - LISTED_AT_MOST, "more failed test")
        );
    }
    if !malformed.is_empty() {
        text.push_str("\n### Malformed JUnit files\n\n");
        for finding in malformed {
            let _ = writeln!(text, "- `{}`: {}", finding.subject, finding.message);
        }
    }
    text
}

/// `n` and `noun`, plural unless `n` is 1.
fn count(n: usize, noun: &str) -> String {
    if n == 1 {
        format!("1 {noun}")
    } else {
        format!("{n} {noun}s")
    }
}

/// One failed test's heading, file, seeds and first lines.
fn section(failed: &Failed) -> String {
    let case = &failed.case;
    let mut text = format!("#### `{}`\n\n", case.test);
    let seeds = seeds(case);
    if seeds.is_empty() {
        let _ = writeln!(text, "In `{}`.", failed.file);
    } else {
        let seeds: Vec<String> = seeds.iter().map(|seed| format!("`{seed}`")).collect();
        let _ = writeln!(text, "In `{}`, with {}.", failed.file, seeds.join(", "));
    }
    let lines = first_lines(case);
    if !lines.is_empty() {
        let fence = fence(&lines);
        let _ = write!(text, "\n{fence}text\n{}\n{fence}\n", lines.join("\n"));
    }
    text
}

/// The seed lines the test printed, each once, in the order printed.
fn seeds(case: &TestCase) -> Vec<&str> {
    let failure = case
        .first_failure
        .as_ref()
        .map_or("", |failure| failure.text.as_str());
    let mut seeds = Vec::new();
    for line in case.output.lines().chain(failure.lines()) {
        let line = line.trim();
        let is_seed = line
            .strip_prefix(SEED_PREFIX)
            .is_some_and(|seed| !seed.is_empty() && seed.bytes().all(|byte| byte.is_ascii_digit()));
        if is_seed && !seeds.contains(&line) {
            seeds.push(line);
        }
    }
    seeds
}

/// The failure's first lines: its message, unless its text's first line
/// already says it, then its text, without the common indent.
fn first_lines(case: &TestCase) -> Vec<String> {
    let Some(failure) = case.first_failure.as_ref() else {
        return Vec::new();
    };
    let body: Vec<&str> = failure.text.lines().map(str::trim_end).collect();
    let start = body
        .iter()
        .position(|line| !line.is_empty())
        .unwrap_or(body.len());
    let end = body
        .iter()
        .rposition(|line| !line.is_empty())
        .map_or(start, |last| last + 1);
    let body = &body[start..end];
    let indent = body
        .iter()
        .filter(|line| !line.is_empty())
        .map(|line| line.len() - line.trim_start().len())
        .min()
        .unwrap_or(0);
    let mut lines = Vec::new();
    if let Some(message) = failure.message.as_deref().map(str::trim)
        && !message.is_empty()
        && !body.first().is_some_and(|first| first.contains(message))
    {
        lines.push(message.to_owned());
    }
    lines.extend(
        body.iter()
            .map(|line| line.get(indent..).unwrap_or("").to_owned()),
    );
    if lines.len() > FIRST_LINES {
        lines.truncate(FIRST_LINES);
        lines.push("…".to_owned());
    }
    lines
}

/// A code fence longer than any run of backticks in `lines`.
fn fence(lines: &[String]) -> String {
    let longest = lines
        .iter()
        .flat_map(|line| line.split(|character| character != '`'))
        .map(str::len)
        .max()
        .unwrap_or(0);
    "`".repeat((longest + 1).max(3))
}
