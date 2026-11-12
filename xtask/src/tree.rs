//! The repository's own files, and which folders a walk of the repository
//! never enters: version control, build outputs, installed packages and
//! caches wherever they are, and the frozen `rnd/` archive (ADR 0032) and
//! agents' `.claude/` folder at the root. Every check that walks the
//! repository skips them through [`never_entered`] or [`generated`], and
//! decides alone whether it enters hidden folders, fixtures (samples the
//! checks run on purpose) and the proofs. [`files`] enters none of those.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

/// Folders no walk enters, wherever they are: version control, build
/// outputs, installed packages and caches.
const GENERATED: [&str; 6] = [
    ".git",
    "target",
    "node_modules",
    ".venv",
    "__pycache__",
    "dist",
];

/// Folders under the root no walk enters: the frozen R&D archive (ADR
/// 0032), and agents' worktrees and notes.
const FROM_ROOT: [&str; 2] = ["rnd", ".claude"];

/// The name of the folders of samples the checks run on purpose.
pub(crate) const FIXTURES: &str = "fixtures";

/// The static checks' proofs (`just proofs`), from the root: every file in
/// it is a fixture.
pub(crate) const PROOFS: &str = "ci/proofs";

/// Whether no walk enters a folder named `name`, wherever it lies: for a
/// walk of a folder whose path from the root it does not follow.
pub(crate) fn generated(name: &str) -> bool {
    GENERATED.contains(&name)
}

/// Whether no walk of the repository enters the folder at `relative`, its
/// `/`-separated path from the root.
pub(crate) fn never_entered(relative: &str) -> bool {
    let name = relative.rsplit('/').next().unwrap_or(relative);
    generated(name) || FROM_ROOT.contains(&relative)
}

/// Whether a folder named `name` is hidden: a dot-folder.
pub(crate) fn hidden(name: &str) -> bool {
    name.starts_with('.')
}

/// Every file under `folder` (relative to `root`), as sorted `/`-separated
/// paths relative to `root`; nothing when `folder` does not exist.
pub(crate) fn files(root: &Path, folder: &str) -> Result<Vec<String>> {
    walked(root, folder, None, false)
}

/// Every file of the repository at `root` that a walk may enter: in hidden
/// folders (`.cargo/`, `.config/`, `.github/`) and fixtures folders too, for
/// a search of what may name a file, which must miss no reader.
pub(crate) fn every_file(root: &Path) -> Result<Vec<String>> {
    walked(root, "", Some(&[]), true)
}

/// Every file under `folder` as [`files`] gives them, fixtures folders
/// included but those at the paths `except` names, such as a check's own
/// case files: a check of what a test may import reads a fixture module too.
pub(crate) fn files_with_fixtures(
    root: &Path,
    folder: &str,
    except: &[&str],
) -> Result<Vec<String>> {
    walked(root, folder, Some(except), false)
}

fn walked(
    root: &Path,
    folder: &str,
    fixtures: Option<&[&str]>,
    hidden_too: bool,
) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let start = root.join(folder);
    if start.is_dir() {
        walk(root, &start, (fixtures, hidden_too), &mut files)?;
    }
    files.sort();
    Ok(files)
}

/// Walks `folder`; `fixtures` enters fixtures folders but those it names
/// (paths from `root`), and `None` enters none; `hidden_too` enters hidden
/// folders, those no walk enters still left out.
fn walk(
    root: &Path,
    folder: &Path,
    (fixtures, hidden_too): (Option<&[&str]>, bool),
    files: &mut Vec<String>,
) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry
            .file_type()
            .with_context(|| format!("reading {}", path.display()))?;
        if kind.is_dir() {
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let skipped_fixtures = name == FIXTURES
                && fixtures.is_none_or(|except| except.contains(&relative.as_str()));
            let skipped_hidden = hidden(&name) && !hidden_too;
            if !(skipped_hidden || never_entered(&relative) || skipped_fixtures) {
                walk(root, &path, (fixtures, hidden_too), files)?;
            }
        } else if kind.is_file() {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
