//! The members that may read a changed file at run time: those whose own
//! files hold a string literal naming a changed path no member owns, such
//! as a test reading `TESTING.md`. No build or import shows such a read, so
//! both the selection and the gate's evidence add these members (ADR 0135).
//! A literal on a comment line names the file without reading it, and a
//! document the Quick suite alone checks is read only where a literal names
//! the file itself (#2041).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use super::selection::needs_no_tests;
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
            let code = code_lines(&text, &file);
            for (literal, path) in &patterns {
                let from_its_folder = member == XTASK && needs_no_tests(path);
                if names(&code, literal, from_its_folder) {
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

/// The member whose commands run at the repository's root, so that its
/// literals naming a document there are its checks' inputs, which `just
/// lint` and the floor guard run in either gate, or its fixture
/// workspaces' files. Its tests reach the repository's own documents from
/// their folder, through `../` (`include_str!("../../../TESTING.md")`).
const XTASK: &str = "xtask";

/// Whether `code` holds `literal` where it ends a string literal, not
/// before a word as in `TESTING.md's`; with `from_its_folder`, only inside
/// a literal that climbs out of the member's folder (`../`).
fn names(code: &str, literal: &str, from_its_folder: bool) -> bool {
    let quote = literal
        .chars()
        .last()
        .filter(|last| ['"', '\'', '`'].contains(last));
    code.match_indices(literal).any(|(start, _)| {
        let after = &code[start + literal.len()..];
        let ends_a_literal =
            quote.is_none() || !after.starts_with(|next: char| next.is_alphanumeric());
        let line = code[..start].rsplit('\n').next().unwrap_or_default();
        let climbs = quote.is_some_and(|quote| {
            line.rsplit_once(quote)
                .is_some_and(|(_, opened)| opened.contains("../"))
        });
        ends_a_literal && (!from_its_folder || climbs)
    })
}

/// The markers that start a comment line in a file of `extension`.
fn comment_markers(extension: &str) -> &'static [&'static str] {
    if extension == "py" {
        &["#"]
    } else {
        &["//", "/*", "*"]
    }
}

/// `text` without its comment lines: a comment names a file, as a doc
/// comment citing `TESTING.md` does, without reading it.
fn code_lines(text: &str, file: &Path) -> String {
    let extension = file
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    let markers = comment_markers(extension);
    text.lines()
        .filter(|line| {
            let line = line.trim_start();
            !markers.iter().any(|marker| line.starts_with(marker))
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The string literals that name `path`: its file name ending a literal,
/// and each folder that holds it starting one. A document the Quick suite
/// alone checks (`docs/`, `rnd/`, top-level Markdown) is named by its file
/// name only: a literal naming one of its folders is a fixture path such as
/// `"docs/adr/0135.md"` or a word such as `"docs" =>`, and xtask's checks
/// that walk those folders run in `just lint`, which the quick gate runs.
fn literals(path: &RepoPath) -> Vec<String> {
    let parts: Vec<&str> = path.as_str().split('/').collect();
    let mut literals = Vec::new();
    if let Some(name) = parts.last() {
        for quote in ['"', '\'', '`'] {
            literals.push(format!("{name}{quote}"));
        }
    }
    if needs_no_tests(path) {
        return literals;
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
