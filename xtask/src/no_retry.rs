//! The no-retry check (ADRs 0064 and 0074): no test runs again after it
//! failed. It refuses retries in runner configurations, per-test overrides
//! included, in test sources, lockfiles and workflow files, and the flags and
//! variables that undo the runners' locks wherever a command is written.
//! Downloads may retry (ADR 0064), so a downloader's own retry flag, such as
//! `curl --retry`, is left alone.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod commands;
mod nextest;
mod sources;
mod workflows;

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use self::commands::Reading;
use crate::check::Finding;
use crate::tree;

/// One file of the workspace, read.
pub(crate) struct File {
    /// The path from the root, with `/` separators.
    pub(crate) path: String,
    pub(crate) text: String,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = nextest::check(root)?;
    findings.extend(sources::check_lockfile(root)?);
    let mut paths = Vec::new();
    walk(root, root, &mut paths)?;
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let Some(kind) = kind(&relative) else {
            continue;
        };
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let file = File {
            path: relative,
            text,
        };
        match kind {
            Kind::JavaScriptTest => sources::check_javascript(&file, &mut findings),
            Kind::PythonTest => sources::check_python(&file, &mut findings),
            Kind::PytestConfig => commands::check_pytest_config(&file, &mut findings)?,
            Kind::Script => commands::check_text(&file.path, &file.text, SCRIPT, &mut findings),
            Kind::Workflow => workflows::check(&file, &mut findings)?,
            Kind::PackageManifest => {
                let document: serde_json::Value = serde_json::from_str(&file.text)
                    .with_context(|| format!("parsing {}", file.path))?;
                commands::check_document(&file.path, &document, SCRIPT, &mut findings);
            }
            Kind::CargoConfig => {
                let document: serde_json::Value =
                    toml::from_str(&file.text).with_context(|| format!("parsing {}", file.path))?;
                commands::check_document(&file.path, &document, SCRIPT, &mut findings);
            }
            Kind::Chromium => {
                commands::check_text(&file.path, &file.text, SCRIPT, &mut findings);
                sources::check_expectations(&file, &mut findings);
            }
        }
    }
    Ok(findings)
}

/// Strings read as shell scripts, no runner implied.
const SCRIPT: Reading<'static> = Reading {
    runners: &[],
    scripts: true,
};

/// What a file holds, as far as retries go.
enum Kind {
    /// A Vitest or Playwright configuration, or a test source they run.
    JavaScriptTest,
    /// A pytest test source or `conftest.py`.
    PythonTest,
    /// A file that holds pytest's options.
    PytestConfig,
    /// A shell script or the `justfile`, whose recipes are the suite commands.
    Script,
    /// A workflow file or a composite action.
    Workflow,
    /// An npm `package.json`, whose scripts are commands.
    PackageManifest,
    /// Cargo's configuration, which may set variables for every command.
    CargoConfig,
    /// The Chromium build gate's scripts, configurations and expectations.
    Chromium,
}

fn kind(path: &str) -> Option<Kind> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, extension) = name.rsplit_once('.').unwrap_or((name, ""));
    if path.starts_with("chromium/")
        && ["sh", "py", "txt", "cfg", "gn", "gni", "json", "toml"].contains(&extension)
    {
        return Some(Kind::Chromium);
    }
    let yaml = matches!(extension, "yml" | "yaml");
    if (path.starts_with(".github/workflows/") && yaml)
        || (path.starts_with(".github/actions/") && stem == "action" && yaml)
    {
        return Some(Kind::Workflow);
    }
    if matches!(name, "justfile" | "Justfile") || matches!(extension, "sh" | "bash" | "just") {
        return Some(Kind::Script);
    }
    if name == "package.json" {
        return Some(Kind::PackageManifest);
    }
    if path == ".cargo/config.toml" || path.ends_with("/.cargo/config.toml") {
        return Some(Kind::CargoConfig);
    }
    if ["pyproject.toml", "pytest.ini", "tox.ini", "setup.cfg"].contains(&name) {
        return Some(Kind::PytestConfig);
    }
    let javascript = matches!(
        extension,
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs"
    );
    let configuration = [
        "vitest.config",
        "vitest.workspace",
        "vite.config",
        "playwright.config",
    ]
    .contains(&stem);
    let test_source = stem
        .rsplit_once('.')
        .is_some_and(|(_, role)| role == "test" || role == "spec");
    if javascript && (configuration || test_source) {
        return Some(Kind::JavaScriptTest);
    }
    if extension == "py"
        && (stem == "conftest" || stem.starts_with("test_") || stem.ends_with("_test"))
    {
        return Some(Kind::PythonTest);
    }
    None
}

fn walk(root: &Path, folder: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    let mut entries = std::fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<PathBuf>, _>>()?;
    entries.sort();
    for path in entries {
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if metadata.is_dir() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or_default();
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            // Fixtures are data other checks' tests break on purpose, the
            // proofs' among them.
            let fixtures = name == tree::FIXTURES || relative == tree::PROOFS;
            if !(tree::never_entered(&relative) || fixtures) {
                walk(root, &path, files)?;
            }
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}
