//! Each runner's own listing of the workspace's tests, as quarantine IDs. The
//! JavaScript and Python runners run from the workspace's installed packages
//! (`pnpm install`, `uv sync`), as the suite commands run them.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::Path;
use std::process::{Command, Output};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{Runner, TestId};

pub(super) fn tests(
    runner: Runner,
    root: &Path,
    cargo: &OsStr,
    cargo_profile: Option<&str>,
) -> Result<BTreeSet<TestId>> {
    match runner {
        Runner::Nextest => nextest(root, cargo, cargo_profile),
        Runner::Vitest => vitest(root),
        Runner::Playwright => playwright(root),
        Runner::Pytest => pytest(root),
    }
}

#[derive(Deserialize)]
struct NextestListing {
    #[serde(rename = "rust-suites")]
    suites: BTreeMap<String, NextestSuite>,
}

#[derive(Deserialize)]
struct NextestSuite {
    testcases: BTreeMap<String, serde_json::Value>,
}

fn nextest(root: &Path, cargo: &OsStr, cargo_profile: Option<&str>) -> Result<BTreeSet<TestId>> {
    let mut command = Command::new(cargo);
    command.args(["nextest", "list", "--workspace", "--message-format", "json"]);
    if let Some(profile) = cargo_profile {
        command.args(["--cargo-profile", profile]);
    }
    let output = run(command.current_dir(root), "cargo nextest list", &[0])?;
    let listing: NextestListing =
        serde_json::from_slice(&output.stdout).context("reading cargo nextest list's output")?;
    Ok(listing
        .suites
        .into_iter()
        .flat_map(|(binary, suite)| {
            suite
                .testcases
                .into_keys()
                .map(move |name| TestId(format!("{binary} {name}")))
        })
        .collect())
}

#[derive(Deserialize)]
struct VitestTest {
    name: String,
}

fn vitest(root: &Path) -> Result<BTreeSet<TestId>> {
    let program = installed(root, "node_modules/.bin/vitest", "pnpm install")?;
    let output = run(
        Command::new(program)
            .args(["list", "--json"])
            .current_dir(root),
        "vitest list",
        &[0],
    )?;
    let tests: Vec<VitestTest> =
        serde_json::from_slice(&output.stdout).context("reading vitest list's output")?;
    Ok(tests.into_iter().map(|test| TestId(test.name)).collect())
}

#[derive(Deserialize)]
struct PlaywrightReport {
    #[serde(default)]
    suites: Vec<PlaywrightSuite>,
}

#[derive(Deserialize)]
struct PlaywrightSuite {
    title: String,
    #[serde(default)]
    specs: Vec<PlaywrightSpec>,
    #[serde(default)]
    suites: Vec<PlaywrightSuite>,
}

#[derive(Deserialize)]
struct PlaywrightSpec {
    title: String,
}

fn playwright(root: &Path) -> Result<BTreeSet<TestId>> {
    let program = installed(root, "node_modules/.bin/playwright", "pnpm install")?;
    let output = run(
        Command::new(program)
            .args(["test", "--list", "--reporter=json"])
            .current_dir(root),
        "playwright test --list",
        &[0],
    )?;
    let report: PlaywrightReport =
        serde_json::from_slice(&output.stdout).context("reading Playwright's listing")?;
    let mut tests = BTreeSet::new();
    // Each top-level suite is a file, titled with its path.
    for file in &report.suites {
        collect_playwright(file, &file.title, &mut tests);
    }
    Ok(tests)
}

fn collect_playwright(suite: &PlaywrightSuite, path: &str, tests: &mut BTreeSet<TestId>) {
    for spec in &suite.specs {
        tests.insert(TestId(format!("{path} › {}", spec.title)));
    }
    for child in &suite.suites {
        collect_playwright(child, &format!("{path} › {}", child.title), tests);
    }
}

fn pytest(root: &Path) -> Result<BTreeSet<TestId>> {
    let program = installed(root, ".venv/bin/pytest", "uv sync")?;
    // Exit code 5: no test collected.
    let output = run(
        Command::new(program)
            .args([
                "--collect-only",
                "-q",
                "-p",
                "no:randomly",
                "-p",
                "no:cacheprovider",
            ])
            .current_dir(root),
        "pytest --collect-only",
        &[0, 5],
    )?;
    let listing = String::from_utf8(output.stdout).context("reading pytest's listing")?;
    Ok(listing
        .lines()
        .filter(|line| line.contains("::"))
        .map(|line| TestId(line.trim().to_owned()))
        .collect())
}

fn installed(root: &Path, relative: &str, install: &str) -> Result<std::path::PathBuf> {
    let program = root.join(relative);
    if !program.is_file() {
        bail!(
            "{} is missing; run `{install}` to list the quarantined tests",
            program.display()
        );
    }
    Ok(program)
}

fn run(command: &mut Command, name: &str, success: &[i32]) -> Result<Output> {
    let output = command
        .output()
        .with_context(|| format!("running {name}"))?;
    if !output
        .status
        .code()
        .is_some_and(|code| success.contains(&code))
    {
        bail!("{name} failed: {}", String::from_utf8_lossy(&output.stderr));
    }
    Ok(output)
}
