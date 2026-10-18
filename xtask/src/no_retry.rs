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

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};

use crate::check::Finding;

/// Folders the walk never enters, wherever they are: version control, build
/// outputs, installed packages, and fixtures, which are data that other
/// checks' tests break on purpose.
const SKIPPED: [&str; 6] = [
    ".git",
    "target",
    "node_modules",
    ".venv",
    "__pycache__",
    "fixtures",
];

/// Folders under the root the walk never enters: the frozen R&D archive
/// (ADR 0032), agents' worktrees, and the proofs' broken fixtures.
const SKIPPED_FROM_ROOT: [&str; 3] = ["rnd", ".claude", "ci/proofs"];

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
        }
    }
    Ok(findings)
}

/// What a file holds, as far as retries go.
enum Kind {
    /// A Vitest or Playwright configuration, or a test source they run.
    JavaScriptTest,
    /// A pytest test source or `conftest.py`.
    PythonTest,
    /// A file that holds pytest's options.
    PytestConfig,
}

fn kind(path: &str) -> Option<Kind> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, extension) = name.rsplit_once('.')?;
    let javascript = matches!(
        extension,
        "ts" | "tsx" | "mts" | "cts" | "js" | "jsx" | "mjs" | "cjs"
    );
    if javascript
        && ([
            "vitest.config",
            "vitest.workspace",
            "vite.config",
            "playwright.config",
        ]
        .contains(&stem)
            || stem
                .rsplit_once('.')
                .is_some_and(|(_, role)| role == "test" || role == "spec"))
    {
        return Some(Kind::JavaScriptTest);
    }
    if extension == "py"
        && (stem == "conftest" || stem.starts_with("test_") || stem.ends_with("_test"))
    {
        return Some(Kind::PythonTest);
    }
    if ["pyproject.toml", "pytest.ini", "tox.ini", "setup.cfg"].contains(&name) {
        return Some(Kind::PytestConfig);
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
            let relative = path.strip_prefix(root).unwrap_or(&path);
            let from_root = SKIPPED_FROM_ROOT
                .iter()
                .any(|skipped| relative == Path::new(skipped));
            if !SKIPPED.contains(&name) && !from_root {
                walk(root, &path, files)?;
            }
        } else if metadata.is_file() {
            files.push(path);
        }
    }
    Ok(())
}
