//! Each runner's own listing of the workspace's tests, as quarantine IDs with
//! where each test is defined. The JavaScript and Python runners run from the
//! workspace's installed packages (`pnpm install`, `uv sync`), as the suite
//! commands run them. A listing is read from the runner's own output, so a
//! saved output reads as a fresh one (`xtask stress`).

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{Runner, TestId};

/// Every test `runner` lists in the workspace at `root`.
pub(super) fn tests(
    runner: Runner,
    root: &Path,
    cargo: &OsStr,
    cargo_profile: Option<&str>,
) -> Result<BTreeSet<TestId>> {
    let output = output(runner, root, cargo, cargo_profile, None, None)?;
    Ok(parse(runner, root, &output)?
        .into_iter()
        .map(|listed| listed.test)
        .collect())
}

/// One listed test and where it is defined.
pub(crate) struct Listed {
    pub(crate) test: TestId,
    pub(crate) location: Location,
}

/// Where a listed test is defined.
pub(crate) enum Location {
    /// A Rust test binary, which names no file: its package, its target's
    /// kind (`lib`, `test`, `bin`…) and its target's name.
    Target {
        package: String,
        kind: String,
        name: String,
    },
    /// A file, as a `/`-separated path from the workspace's folder; `None`
    /// when it lies outside it.
    File(Option<String>),
}

/// What `runner` prints when it lists the workspace's tests. With
/// `changed_since`, Vitest lists only the tests `--changed` reaches and
/// Playwright those `--only-changed` reaches: the tests of the files changed
/// since that commit and of the files that import them. nextest builds into
/// `target_directory` when given, else where Cargo resolves.
pub(crate) fn output(
    runner: Runner,
    root: &Path,
    cargo: &OsStr,
    cargo_profile: Option<&str>,
    target_directory: Option<&Path>,
    changed_since: Option<&str>,
) -> Result<Vec<u8>> {
    let output = match runner {
        Runner::Nextest => {
            let mut command = crate::child_cargo::command(cargo);
            command.args(["nextest", "list", "--workspace", "--message-format", "json"]);
            if let Some(profile) = cargo_profile {
                command.args(["--cargo-profile", profile]);
            }
            if let Some(folder) = target_directory {
                command.env("CARGO_TARGET_DIR", folder);
            }
            run(command.current_dir(root), "cargo nextest list", &[0])?
        }
        Runner::Vitest => {
            let program = installed(root, "node_modules/.bin/vitest", "pnpm install")?;
            let mut command = Command::new(program);
            // Collected by loading each file, as a run collects it: read
            // statically, a `test.for` or `test.each` test is one test under
            // its unformatted title (`reads %s`), not one per case under the
            // name it runs as.
            command.args(["list", "--json", "--staticParse=false"]);
            if let Some(commit) = changed_since {
                command.arg(format!("--changed={commit}"));
            }
            run(command.current_dir(root), "vitest list", &[0])?
        }
        Runner::Playwright => {
            let program = installed(root, "node_modules/.bin/playwright", "pnpm install")?;
            let mut command = Command::new(program);
            // No test is no failure: a workspace may hold none yet.
            command.args(["test", "--list", "--reporter=json", "--pass-with-no-tests"]);
            if let Some(commit) = changed_since {
                command.arg(format!("--only-changed={commit}"));
            }
            run(command.current_dir(root), "playwright test --list", &[0])?
        }
        Runner::Pytest => {
            let program = installed(root, ".venv/bin/pytest", "uv sync")?;
            // Exit code 5: no test collected.
            run(
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
            )?
        }
    };
    Ok(output.stdout)
}

/// The tests in `output`, what `runner` printed listing the workspace at `root`.
pub(crate) fn parse(runner: Runner, root: &Path, output: &[u8]) -> Result<Vec<Listed>> {
    match runner {
        Runner::Nextest => nextest(output),
        Runner::Vitest => vitest(root, output),
        Runner::Playwright => playwright(root, output),
        Runner::Pytest => pytest(output),
    }
}

#[derive(Deserialize)]
struct NextestListing {
    #[serde(rename = "rust-suites")]
    suites: BTreeMap<String, NextestSuite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct NextestSuite {
    package_name: String,
    kind: String,
    binary_name: String,
    testcases: BTreeMap<String, serde_json::Value>,
}

fn nextest(output: &[u8]) -> Result<Vec<Listed>> {
    let listing: NextestListing =
        serde_json::from_slice(output).context("reading cargo nextest list's output")?;
    Ok(listing
        .suites
        .into_iter()
        .flat_map(|(binary, suite)| {
            let NextestSuite {
                package_name,
                kind,
                binary_name,
                testcases,
            } = suite;
            testcases.into_keys().map(move |name| Listed {
                test: TestId(format!("{binary} {name}")),
                location: Location::Target {
                    package: package_name.clone(),
                    kind: kind.clone(),
                    name: binary_name.clone(),
                },
            })
        })
        .collect())
}

#[derive(Deserialize)]
struct VitestTest {
    name: String,
    file: PathBuf,
}

fn vitest(root: &Path, output: &[u8]) -> Result<Vec<Listed>> {
    let tests: Vec<VitestTest> =
        serde_json::from_slice(output).context("reading vitest list's output")?;
    let root = canonical(root)?;
    Ok(tests
        .into_iter()
        .map(|test| Listed {
            test: TestId(test.name),
            location: Location::File(relative(&root, &test.file)),
        })
        .collect())
}

#[derive(Deserialize)]
struct PlaywrightReport {
    config: PlaywrightConfig,
    #[serde(default)]
    suites: Vec<PlaywrightSuite>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PlaywrightConfig {
    /// The folder the suites' files are relative to.
    root_dir: PathBuf,
}

#[derive(Deserialize)]
struct PlaywrightSuite {
    title: String,
    file: PathBuf,
    #[serde(default)]
    specs: Vec<PlaywrightSpec>,
    #[serde(default)]
    suites: Vec<PlaywrightSuite>,
}

#[derive(Deserialize)]
struct PlaywrightSpec {
    title: String,
}

fn playwright(root: &Path, output: &[u8]) -> Result<Vec<Listed>> {
    let report: PlaywrightReport =
        serde_json::from_slice(output).context("reading Playwright's listing")?;
    let root = canonical(root)?;
    let mut tests = Vec::new();
    // Each top-level suite is a file, titled with its path.
    for file in &report.suites {
        let path = relative(&root, &report.config.root_dir.join(&file.file));
        collect_playwright(file, &file.title, path.as_ref(), &mut tests);
    }
    Ok(tests)
}

fn collect_playwright(
    suite: &PlaywrightSuite,
    title: &str,
    file: Option<&String>,
    tests: &mut Vec<Listed>,
) {
    for spec in &suite.specs {
        tests.push(Listed {
            test: TestId(format!("{title} › {}", spec.title)),
            location: Location::File(file.cloned()),
        });
    }
    for child in &suite.suites {
        collect_playwright(child, &format!("{title} › {}", child.title), file, tests);
    }
}

fn pytest(output: &[u8]) -> Result<Vec<Listed>> {
    let listing = std::str::from_utf8(output).context("reading pytest's listing")?;
    Ok(listing
        .lines()
        .filter(|line| line.contains("::"))
        .map(|line| {
            let id = line.trim();
            Listed {
                test: TestId(id.to_owned()),
                location: Location::File(id.split_once("::").map(|(file, _)| file.to_owned())),
            }
        })
        .collect())
}

fn canonical(path: &Path) -> Result<PathBuf> {
    std::fs::canonicalize(path).with_context(|| format!("resolving {}", path.display()))
}

/// `path` from `root` (canonical), `/`-separated; `None` outside it.
fn relative(root: &Path, path: &Path) -> Option<String> {
    let path = std::fs::canonicalize(path).ok()?;
    let relative = path.strip_prefix(root).ok()?;
    Some(
        relative
            .components()
            .map(|component| component.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/"),
    )
}

fn installed(root: &Path, relative: &str, install: &str) -> Result<PathBuf> {
    let program = root.join(relative);
    if !program.is_file() {
        bail!(
            "{} is missing; run `{install}` to list the workspace's tests",
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
