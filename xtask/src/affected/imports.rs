//! What a pnpm or uv package's own files import of the other members, read
//! from their text: an import resolves through the root package's hoisted
//! dependencies even when the importer does not declare it, so the
//! selection follows what the files import as well as what the manifests
//! declare (the incremental gate's review, F1).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::Path;

use anyhow::{Context as _, Result};

use crate::package_dependencies;

/// Folders never read: installed packages, build outputs and caches.
const SKIPPED: [&str; 6] = [
    "node_modules",
    "dist",
    "target",
    ".venv",
    "__pycache__",
    "console-build",
];

/// The files under `folder` whose extension is one of `extensions`.
pub(super) fn files(folder: &Path, extensions: &[&str]) -> Result<Vec<std::path::PathBuf>> {
    let mut found = Vec::new();
    walk(folder, extensions, &mut found)?;
    found.sort();
    Ok(found)
}

fn walk(folder: &Path, extensions: &[&str], found: &mut Vec<std::path::PathBuf>) -> Result<()> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", folder.display())),
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let kind = entry.file_type()?;
        if kind.is_dir() {
            if !name.starts_with('.') && !SKIPPED.contains(&name.as_ref()) {
                walk(&entry.path(), extensions, found)?;
            }
        } else if kind.is_file()
            && Path::new(name.as_ref())
                .extension()
                .is_some_and(|extension| extensions.iter().any(|wanted| extension == *wanted))
        {
            found.push(entry.path());
        }
    }
    Ok(())
}

/// The TypeScript and JavaScript extensions a package's files use.
pub(super) const SCRIPT_EXTENSIONS: [&str; 7] = ["ts", "tsx", "mts", "cts", "js", "mjs", "cjs"];

/// The pnpm members of `names` that `text` imports: a string literal that
/// is a member's name, or a path into it (`"@goodfetch/x/sub"`).
pub(super) fn typescript(text: &str, names: &BTreeSet<String>) -> BTreeSet<String> {
    names
        .iter()
        .filter(|name| {
            ['"', '\'', '`'].iter().any(|quote| {
                text.contains(&format!("{quote}{name}{quote}"))
                    || text.contains(&format!("{quote}{name}/"))
            })
        })
        .cloned()
        .collect()
}

/// The uv members of `names` (normalized by PEP 503) whose top-level module
/// `text` imports: `import x`, `import x.y as z, w` or `from x.y import z`.
pub(super) fn python(text: &str, names: &BTreeSet<String>) -> BTreeSet<String> {
    let mut modules = Vec::new();
    for line in text.lines().map(str::trim_start) {
        if let Some(rest) = line.strip_prefix("from ") {
            modules.extend(rest.split_whitespace().next());
        } else if let Some(rest) = line.strip_prefix("import ") {
            modules.extend(
                rest.split(',')
                    .filter_map(|module| module.split_whitespace().next()),
            );
        }
    }
    modules
        .into_iter()
        .filter(|module| !module.starts_with('.'))
        .filter_map(|module| module.split('.').next())
        .map(package_dependencies::pep503)
        .filter(|module| names.contains(module))
        .collect()
}

/// What the files of `folder` with `extensions` import, by `imports`.
pub(super) fn of_folder(
    folder: &Path,
    extensions: &[&str],
    imports: impl Fn(&str) -> BTreeSet<String>,
) -> Result<BTreeSet<String>> {
    let mut found = BTreeSet::new();
    for file in files(folder, extensions)? {
        // A file that is not text imports nothing.
        if let Ok(text) = std::fs::read_to_string(&file) {
            found.extend(imports(&text));
        }
    }
    Ok(found)
}
