//! The members that may read a changed file at run time: those whose own
//! files hold a string literal naming a changed path no member owns, such
//! as a test reading `TESTING.md`. No build or import shows such a read, so
//! both the selection and the gate's evidence add these members (ADR 0135).
//! A literal on a comment line names the file without reading it, and a
//! document the Quick suite alone checks is read only where a literal names
//! the file itself (#2041). A member whose code also names heading lines of
//! a changed Markdown document, in string literals, reads only those parts
//! of it (`parts`), so it is a reader only when one of them changed.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use super::selection::{TESTS_FOLDER, needs_no_tests};
use super::{ChangedParts, RepoPath, WorkspaceMembers};
use crate::tree;

/// The extensions of the files whose string literals may name a file a test
/// reads at run time.
const SOURCE_EXTENSIONS: [&str; 7] = ["rs", "ts", "tsx", "mts", "js", "mjs", "py"];

/// The members whose own files hold a string literal naming one of the
/// `changed` paths no member owns (`TESTING.md"`, `"docs/adr`…): a test may
/// read such a file at run time, which no build or import shows. Of a
/// document whose `parts` were read, a member that names some of its
/// heading lines reads those parts alone.
pub(crate) fn literal_readers(
    root: &Path,
    members: &WorkspaceMembers,
    changed: &[RepoPath],
    parts: &ChangedParts,
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
        let mut files = Vec::new();
        for file in super::source_files(&root.join(&folder), &SOURCE_EXTENSIONS)? {
            if let Ok(text) = std::fs::read_to_string(&file) {
                let code = code_lines(&text, &file);
                files.push((file, code));
            }
        }
        let read = patterns.iter().find_map(|(literal, path)| {
            let from_its_folder = member == XTASK && needs_no_tests(path);
            let (file, _) = files
                .iter()
                .find(|(_, code)| names(code, literal, from_its_folder))?;
            reads_a_changed_part(&files, path, parts).then_some((file, path))
        });
        if let Some((file, path)) = read {
            let relative = file.strip_prefix(root).unwrap_or(file);
            reached.insert(
                member.clone(),
                format!("{} names {}", relative.display(), path.as_str()),
            );
        }
    }
    Ok(reached)
}

/// The crates a changed file of their `tests/` folder may reach beyond
/// their tests, each with why. Such a file reaches the crate's tests alone
/// (#2238), unless a file outside that folder names it, or a folder of the
/// `tests/` folder that holds it: a shared fixture listing it, as the SSRF
/// layers table lists Collie's runners for test support's test to read;
/// the crate's own code including or reading it; another crate's code
/// naming it through the crate's folder (`../goodfetch-collie/tests/it`).
/// Then it reaches what a change to the crate reaches, as before. Documents
/// the Quick suite alone checks, and comment lines, name files without
/// reading them, so they do not count. A name of the folder through no
/// path, as a walk of every crate's `tests/` does, is not seen: those walks
/// are xtask's checks, which every gate reruns.
pub(crate) fn tests_read_elsewhere(
    root: &Path,
    members: &WorkspaceMembers,
    changed: &[RepoPath],
) -> Result<BTreeMap<String, String>> {
    let tests: Vec<(&RepoPath, &str, &str)> = changed
        .iter()
        .filter_map(|path| {
            members
                .tests_owner(path)
                .map(|(krate, folder)| (path, krate, folder))
        })
        .collect();
    let mut reached = BTreeMap::new();
    if tests.is_empty() {
        return Ok(reached);
    }
    for file in tree::files_with_fixtures(root, "", &[])? {
        let file = RepoPath::new(file);
        if needs_no_tests(&file) {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(file.as_str())) else {
            continue;
        };
        let text = code_lines(&text, Path::new(file.as_str()));
        for (path, krate, folder) in &tests {
            let tests_folder = format!("{folder}/{TESTS_FOLDER}");
            let Some(within) = path
                .as_str()
                .strip_prefix(&tests_folder)
                .and_then(|rest| rest.strip_prefix('/'))
            else {
                continue;
            };
            if reached.contains_key(*krate) || file.is_in(&tests_folder) {
                continue;
            }
            let segment = folder.rsplit('/').next().unwrap_or(folder);
            if names_a_tests_file(&text, segment, file.is_in(folder), within) {
                reached.insert(
                    (*krate).to_owned(),
                    format!("{} names {}", file.as_str(), path.as_str()),
                );
            }
        }
    }
    Ok(reached)
}

/// Whether a character may be part of a path written in a file.
fn in_a_path(character: char) -> bool {
    character.is_ascii_alphanumeric() || "_-./".contains(character)
}

/// Whether `text` names `within`, a path from a crate's `tests/` folder, or
/// a folder holding it: a `tests` path segment that the crate's folder name
/// `segment` precedes (`goodfetch-collie/tests/it`), or, in a file of the
/// crate itself (`own`), no folder, `.` or `..` (`"tests"`, `"/tests/it"`,
/// `"../tests/it/page.html"`), then nothing more or a path `within` lies in.
/// A `tests` that neither a `/` nor a quote opens is a word, not a path.
fn names_a_tests_file(text: &str, segment: &str, own: bool, within: &str) -> bool {
    text.match_indices(TESTS_FOLDER).any(|(start, _)| {
        let before = &text[..start];
        let after = &text[start + TESTS_FOLDER.len()..];
        if after.starts_with(|next: char| in_a_path(next) && next != '/') {
            return false;
        }
        let preceding = match before.chars().last() {
            Some('/') => before[..before.len() - 1]
                .rsplit(|character: char| character == '/' || !in_a_path(character))
                .next()
                .unwrap_or_default(),
            // A word of prose, `the integration tests`, is no path.
            Some('"' | '\'' | '`') => "",
            _ => return false,
        };
        let from_the_crate = preceding == segment || (own && ["", ".", ".."].contains(&preceding));
        if !from_the_crate {
            return false;
        }
        let named: String = after
            .strip_prefix('/')
            .unwrap_or_default()
            .chars()
            .take_while(|character| in_a_path(*character))
            .collect();
        let named = named.trim_end_matches('/');
        named.is_empty() || RepoPath::new(within).is_in(named)
    })
}

/// Whether a member whose `files` name `path` reads a changed part of it:
/// always, unless its parts were read and the files name some of its
/// heading lines in a string literal (`"## Quality register"`), each the
/// part a reader finds by it, and none of those parts changed.
fn reads_a_changed_part(
    files: &[(std::path::PathBuf, String)],
    path: &RepoPath,
    parts: &ChangedParts,
) -> bool {
    let (Some(headings), Some(changed)) = (parts.headings(path), parts.changed(path)) else {
        return true;
    };
    let named: Vec<&String> = headings
        .iter()
        .filter(|heading| {
            ['"', '\'', '`'].iter().any(|quote| {
                let literal = format!("{quote}{heading}{quote}");
                files.iter().any(|(_, code)| code.contains(&literal))
            })
        })
        .collect();
    named.is_empty() || named.iter().any(|heading| changed.contains(*heading))
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
