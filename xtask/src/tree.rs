//! The repository's own files, walked from a folder: never version control,
//! hidden folders, build outputs, installed packages, fixtures (samples the
//! checks run on purpose) or the frozen `rnd/` archive (ADR 0032).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

/// Folders never entered, wherever they are.
const SKIPPED: [&str; 5] = ["target", "node_modules", "fixtures", "dist", "__pycache__"];

/// The folders of samples the checks run on purpose, one of [`SKIPPED`].
const FIXTURES: &str = "fixtures";

/// The frozen R&D archive at the root (ADR 0032).
const FROZEN: &str = "rnd";

/// Every file under `folder` (relative to `root`), as sorted `/`-separated
/// paths relative to `root`; nothing when `folder` does not exist.
pub(crate) fn files(root: &Path, folder: &str) -> Result<Vec<String>> {
    walked(root, folder, None)
}

/// Every file under `folder` as [`files`] gives them, fixtures folders
/// included but those at the paths `except` names, such as a check's own
/// case files: a check of what a test may import reads a fixture module too.
pub(crate) fn files_with_fixtures(
    root: &Path,
    folder: &str,
    except: &[&str],
) -> Result<Vec<String>> {
    walked(root, folder, Some(except))
}

fn walked(root: &Path, folder: &str, fixtures: Option<&[&str]>) -> Result<Vec<String>> {
    let mut files = Vec::new();
    let start = root.join(folder);
    if start.is_dir() {
        walk(root, &start, fixtures, &mut files)?;
    }
    files.sort();
    Ok(files)
}

/// Walks `folder`; `fixtures` enters fixtures folders but those it names
/// (paths from `root`), and `None` enters none.
fn walk(
    root: &Path,
    folder: &Path,
    fixtures: Option<&[&str]>,
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
            let frozen = folder == root && name == FROZEN;
            let relative = path
                .strip_prefix(root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let entered_fixtures = name == FIXTURES
                && fixtures.is_some_and(|except| !except.contains(&relative.as_str()));
            let skipped = SKIPPED.contains(&name.as_ref()) && !entered_fixtures;
            if !(name.starts_with('.') || skipped || frozen) {
                walk(root, &path, fixtures, files)?;
            }
        } else if kind.is_file() {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            files.push(relative.to_string_lossy().replace('\\', "/"));
        }
    }
    Ok(())
}
