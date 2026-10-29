//! The members that may read a changed file at run time: those whose own
//! files hold a string literal naming a changed path no member owns, such
//! as a test reading `TESTING.md`. No build or import shows such a read, so
//! both the selection and the gate's evidence add these members (ADR 0135).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use super::{RepoPath, WorkspaceMembers};

/// The extensions of the files whose string literals may name a file a test
/// reads at run time.
const SOURCE_EXTENSIONS: [&str; 7] = ["rs", "ts", "tsx", "mts", "js", "mjs", "py"];

/// The members whose own files hold a string literal naming one of the
/// `changed` paths no member owns (`TESTING.md"`, `"docs/adr`…): a test may
/// read such a file at run time, which no build or import shows.
pub(crate) fn literal_readers(
    root: &Path,
    members: &WorkspaceMembers,
    changed: &[RepoPath],
) -> Result<BTreeMap<String, String>> {
    let unowned: Vec<&RepoPath> = changed
        .iter()
        .filter(|path| members.owner(path.as_str()).is_none())
        .collect();
    let mut reached = BTreeMap::new();
    if unowned.is_empty() {
        return Ok(reached);
    }
    let patterns: Vec<(String, &RepoPath)> = unowned
        .iter()
        .flat_map(|path| {
            literals(path)
                .into_iter()
                .map(move |literal| (literal, *path))
        })
        .collect();
    for (member, _) in members.all() {
        let Some(folder) = members.folder(&member) else {
            continue;
        };
        'files: for file in super::source_files(&root.join(&folder), &SOURCE_EXTENSIONS)? {
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            for (literal, path) in &patterns {
                if text.contains(literal.as_str()) {
                    let relative = file.strip_prefix(root).unwrap_or(&file);
                    reached.insert(
                        member.clone(),
                        format!("{} names {}", relative.display(), path.as_str()),
                    );
                    break 'files;
                }
            }
        }
    }
    Ok(reached)
}

/// The string literals that name `path`: its file name ending a literal,
/// and each folder that holds it starting one.
fn literals(path: &RepoPath) -> Vec<String> {
    let parts: Vec<&str> = path.as_str().split('/').collect();
    let mut literals = Vec::new();
    if let Some(name) = parts.last() {
        for quote in ['"', '\'', '`'] {
            literals.push(format!("{name}{quote}"));
        }
    }
    for end in 1..parts.len() {
        let folder = parts[..end].join("/");
        for quote in ['"', '\''] {
            literals.push(format!("{quote}{folder}{quote}"));
            literals.push(format!("{quote}{folder}/"));
        }
    }
    literals
}

#[cfg(test)]
mod tests;
