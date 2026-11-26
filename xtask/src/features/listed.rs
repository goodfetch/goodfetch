//! The declarations the runners' listings carry (ADR 0072), read from the
//! folder `xtask journeys listings` writes:
//! - `vitest.json`: each test's tags; a tag with a `:` is a Feature ID (a
//!   Journey's or a surface's tag has none), declared in `vitest.config.ts`
//!   from the inventory, so Vitest itself refuses an unknown one;
//! - `playwright.json`: each test's `covers` annotations, from `--list` with
//!   the JSON reporter, once per test however many projects list it;
//! - `pytest.json`: each test's `covers` marker values, from the
//!   test-support plugin's `--marker-listing`.
//!
//! A test's level comes from its file's path; its package is the nearest
//! `package.json`'s or `pyproject.toml`'s name.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Deserialize;

use super::scan::{Declaration, Level, Source};
use crate::journeys::listing_file;
use crate::runners::Runner;

/// The type of a Playwright annotation that declares a Feature.
const COVERS: &str = "covers";

/// Every declaration of the listings in `folder`, of the workspace at `root`.
pub(crate) fn read(root: &Path, folder: &Path) -> Result<Vec<Declaration>> {
    let root = Root::new(root);
    let [vitest_file, playwright_file, pytest_file] =
        [Runner::Vitest, Runner::Playwright, Runner::Pytest].map(listing_file);
    let mut declarations = Vec::new();
    declarations.extend(vitest(&root, &listing(folder, vitest_file)?).context(vitest_file)?);
    declarations
        .extend(playwright(&root, &listing(folder, playwright_file)?).context(playwright_file)?);
    declarations.extend(pytest(&root, &listing(folder, pytest_file)?).context(pytest_file)?);
    Ok(declarations)
}

fn listing(folder: &Path, name: &str) -> Result<Vec<u8>> {
    let path = folder.join(name);
    std::fs::read(&path).with_context(|| {
        format!(
            "reading {}; `xtask journeys listings` writes it",
            path.display()
        )
    })
}

/// The workspace's folder, as given and resolved, which listings' absolute
/// paths start with.
struct Root {
    given: PathBuf,
    canonical: PathBuf,
}

impl Root {
    fn new(root: &Path) -> Self {
        Self {
            given: root.to_path_buf(),
            canonical: root.canonicalize().unwrap_or_else(|_| root.to_path_buf()),
        }
    }

    /// `file` from the workspace's folder, `/`-separated; a relative path is
    /// taken as relative to it already.
    fn relative(&self, file: &Path) -> String {
        [&self.given, &self.canonical]
            .into_iter()
            .find_map(|root| file.strip_prefix(root).ok())
            .unwrap_or(file)
            .to_string_lossy()
            .replace('\\', "/")
    }

    /// The name in the nearest manifest named `manifest` above `file`, read
    /// by `name`.
    fn package(
        &self,
        file: &str,
        manifest: &str,
        name: fn(&str) -> Option<String>,
    ) -> Option<String> {
        Path::new(file).ancestors().skip(1).find_map(|folder| {
            let text = std::fs::read_to_string(self.given.join(folder).join(manifest)).ok()?;
            name(&text)
        })
    }
}

/// A test's declaration of `feature`, at the level its file's path gives.
fn declaration(
    feature: &str,
    source: Source,
    test: String,
    path: String,
    package: Option<String>,
) -> Declaration {
    Declaration {
        feature: feature.to_owned(),
        level: Level::of(Path::new(&path)),
        source,
        test,
        package,
        path,
    }
}

fn npm_name(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Manifest {
        name: Option<String>,
    }
    serde_json::from_str::<Manifest>(text).ok()?.name
}

fn python_name(text: &str) -> Option<String> {
    #[derive(Deserialize)]
    struct Manifest {
        project: Option<Project>,
    }
    #[derive(Deserialize)]
    struct Project {
        name: String,
    }
    Some(toml::from_str::<Manifest>(text).ok()?.project?.name)
}

#[derive(Deserialize)]
struct VitestTest {
    /// The full name, its suites and title joined by ` > `.
    name: String,
    file: PathBuf,
    tags: Vec<String>,
}

fn vitest(root: &Root, bytes: &[u8]) -> Result<Vec<Declaration>> {
    let tests: Vec<VitestTest> = serde_json::from_slice(bytes)?;
    let mut declarations = Vec::new();
    for test in tests {
        let path = root.relative(&test.file);
        let package = root.package(&path, "package.json", npm_name);
        for tag in test.tags.iter().filter(|tag| tag.contains(':')) {
            declarations.push(declaration(
                tag,
                Source::Vitest,
                test.name.clone(),
                path.clone(),
                package.clone(),
            ));
        }
    }
    Ok(declarations)
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// A test as one project lists it.
#[derive(Deserialize)]
struct PlaywrightSpec {
    title: String,
    #[serde(default)]
    tests: Vec<PlaywrightTest>,
}

#[derive(Deserialize)]
struct PlaywrightTest {
    #[serde(default)]
    annotations: Vec<PlaywrightAnnotation>,
}

#[derive(Deserialize)]
struct PlaywrightAnnotation {
    #[serde(rename = "type")]
    kind: String,
    description: Option<String>,
}

fn playwright(root: &Root, bytes: &[u8]) -> Result<Vec<Declaration>> {
    let report: PlaywrightReport = serde_json::from_slice(bytes)?;
    let mut declarations = Vec::new();
    // Each top-level suite is a file, titled with its path.
    for file in &report.suites {
        let path = root.relative(&report.config.root_dir.join(&file.file));
        let path = path.strip_prefix("./").unwrap_or(&path).to_owned();
        let package = root.package(&path, "package.json", npm_name);
        let mut tests = Vec::new();
        collect_playwright(file, &file.title, &mut tests);
        for (test, feature) in tests {
            declarations.push(declaration(
                feature,
                Source::Playwright,
                test,
                path.clone(),
                package.clone(),
            ));
        }
    }
    // A test each project lists declares its Features once.
    declarations.sort();
    declarations.dedup();
    Ok(declarations)
}

/// Each test of `suite`, named as the quarantine list names it (its file,
/// then its suites and title, joined by ` › `), with each Feature its
/// `covers` annotations name.
fn collect_playwright<'a>(
    suite: &'a PlaywrightSuite,
    title: &str,
    tests: &mut Vec<(String, &'a str)>,
) {
    for spec in &suite.specs {
        let name = format!("{title} › {}", spec.title);
        for annotation in spec.tests.iter().flat_map(|test| &test.annotations) {
            if let (COVERS, Some(feature)) = (annotation.kind.as_str(), &annotation.description) {
                tests.push((name.clone(), feature.as_str()));
            }
        }
    }
    for child in &suite.suites {
        collect_playwright(child, &format!("{title} › {}", child.title), tests);
    }
}

#[derive(Deserialize)]
struct PytestListing {
    tests: Vec<PytestTest>,
}

#[derive(Deserialize)]
struct PytestTest {
    /// The node ID, relative to the workspace's folder.
    id: String,
    covers: Vec<String>,
}

fn pytest(root: &Root, bytes: &[u8]) -> Result<Vec<Declaration>> {
    let listing: PytestListing = serde_json::from_slice(bytes)?;
    let mut declarations = Vec::new();
    for test in listing.tests {
        let path = test
            .id
            .split_once("::")
            .map_or(test.id.as_str(), |(file, _)| file);
        let package = root.package(path, "pyproject.toml", python_name);
        for feature in &test.covers {
            declarations.push(declaration(
                feature,
                Source::Pytest,
                test.id.clone(),
                path.to_owned(),
                package.clone(),
            ));
        }
    }
    Ok(declarations)
}
