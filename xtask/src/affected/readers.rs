//! The reads of a changed file that no build, import or member graph
//! shows, found by what files name, which both the selection and the gate's
//! evidence add (ADR 0135):
//! - `literal_readers`: the members whose own files hold a string literal
//!   naming a changed path no member owns, such as a test reading
//!   `TESTING.md`. A literal on a comment line names the file without
//!   reading it, and a document the Quick suite alone checks is read only
//!   where a literal names the file itself (#2041). A member whose code also
//!   names heading lines of a changed Markdown document, in string literals,
//!   reads only those parts of it (`parts`), so it is a reader only when one
//!   of them changed.
//! - `tests_read_elsewhere`: the crates a changed file of their `tests/`
//!   folder reaches beyond their tests, since a file outside that folder
//!   may name it (#2238). That search must miss no reader, so a file that
//!   might name it counts (#2240). Its known limits, each because counting
//!   it would give every `tests/` change its crate's whole reach back (ADR
//!   0135's note of 2026-11-22): a folder walk in a file that names neither
//!   the crate's folder nor its name; a computed value whose crate only
//!   another file names, since no value is followed from file to file and
//!   the tree joins such values to `tests` for other purposes (test
//!   support's `project / "tests"`); a glob that names no crate
//!   (`crates/*/tests/…`), since no glob is matched against paths and the
//!   root Vitest config holds `**/tests/**`; and a Markdown table with the
//!   crate and the path in two columns, as Markdown counts only through the
//!   crate's folder or name just before `tests`.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Result;

use super::selection::{TESTS_FOLDER, needs_no_tests, tests_folder};
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

/// A changed file of a crate's `tests/` folder, as the search for what names
/// it takes it.
struct TestsFile<'a> {
    path: &'a RepoPath,
    krate: &'a str,
    folder: &'a str,
    tests_folder: String,
    /// Its path from the `tests/` folder.
    within: RepoPath,
    /// The crate's folder name and its name, as a path may name the crate.
    names: [&'a str; 2],
}

impl<'a> TestsFile<'a> {
    fn new(members: &'a WorkspaceMembers, path: &'a RepoPath) -> Option<Self> {
        let (krate, folder) = members.tests_owner(path)?;
        let tests_folder = tests_folder(folder);
        let within = path
            .as_str()
            .strip_prefix(&tests_folder)?
            .strip_prefix('/')?;
        Some(Self {
            path,
            krate,
            folder,
            within: RepoPath::new(within),
            tests_folder,
            names: [folder.rsplit('/').next().unwrap_or(folder), krate],
        })
    }

    /// Whether `after`, what follows a `tests` segment, leaves the path at
    /// the folder or goes on into this file or a folder holding it.
    fn holds(&self, after: &str) -> bool {
        let Some(rest) = after.strip_prefix('/') else {
            return true;
        };
        let named: String = rest.chars().take_while(|c| in_a_path(*c)).collect();
        let named = named.trim_end_matches('/');
        named.is_empty() || self.within.is_in(named)
    }
}

/// Where a file that may name a crate's `tests/` files stands.
#[derive(Clone, Copy)]
struct Naming {
    /// A file of the crate itself, outside its `tests/` folder.
    own: bool,
    /// Markdown, which computes no path.
    prose: bool,
    /// xtask's, whose commands run at the repository's root.
    xtasks: bool,
}

/// The crates a changed file of their `tests/` folder may reach beyond
/// their tests, each with why. Such a file reaches the crate's tests alone
/// (#2238), unless a file outside that folder may name it or a folder of the
/// `tests/` folder that holds it: a shared fixture listing it, as the SSRF
/// layers table lists Collie's runners for test support's test to read;
/// the crate's own code including or reading it, or its build script
/// watching it; another member's code naming it through the crate's folder
/// or name, in one literal, in pieces (`.join("goodfetch-collie")
/// .join("tests")`), through a value computed in a file that names the
/// crate, or as a path from `tests` on a line that names the crate. Then
/// it reaches what a change to the crate reaches, as before. Every file is
/// read, those of hidden folders and fixtures too, but the documents no test
/// reads (`needs_no_tests`). A comment line names a file without reading it,
/// unless it is a line of a doctest; Markdown names one only through the
/// crate's folder or name; and xtask's files, whose fixtures name such
/// paths, only through a literal that climbs out of xtask (`../`), as for
/// `literal_readers`.
pub(crate) fn tests_read_elsewhere(
    root: &Path,
    members: &WorkspaceMembers,
    changed: &[RepoPath],
) -> Result<BTreeMap<String, String>> {
    let tests: Vec<TestsFile<'_>> = changed
        .iter()
        .filter_map(|path| TestsFile::new(members, path))
        .collect();
    let mut reached = BTreeMap::new();
    if tests.is_empty() {
        return Ok(reached);
    }
    for file in tree::every_file(root)? {
        let file = RepoPath::new(file);
        if needs_no_tests(&file) {
            continue;
        }
        let pending: Vec<&TestsFile<'_>> = tests
            .iter()
            .filter(|tests_file| {
                !reached.contains_key(tests_file.krate) && !file.is_in(&tests_file.tests_folder)
            })
            .collect();
        if pending.is_empty() {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(root.join(file.as_str())) else {
            continue;
        };
        let code = without_comments(&text, &file);
        let prose = Path::new(file.as_str())
            .extension()
            .is_some_and(|extension| extension.eq_ignore_ascii_case("md"));
        let xtasks = members.owner(file.as_str()).as_deref() == Some(XTASK);
        for tests_file in pending {
            let naming = Naming {
                own: file.is_in(tests_file.folder),
                prose,
                xtasks,
            };
            if names_a_tests_file(&code, naming, tests_file) {
                reached
                    .entry(tests_file.krate.to_owned())
                    .or_insert_with(|| {
                        format!("{} names {}", file.as_str(), tests_file.path.as_str())
                    });
            }
        }
    }
    Ok(reached)
}

/// Whether a character may be part of a path written in a file.
fn in_a_path(character: char) -> bool {
    in_a_name(character) || character == '/'
}

/// Whether a character may be part of a file or folder name.
fn in_a_name(character: char) -> bool {
    character.is_ascii_alphanumeric() || "_-.".contains(character)
}

const QUOTES: [char; 3] = ['"', '\'', '`'];

/// The values a file's own location starts from, so that a `tests` after
/// one, or after an expression that starts from one, is that file's crate's
/// or package's own (`CARGO_MANIFEST_DIR`, `import.meta.dirname`,
/// `Path(__file__).parent`).
const LOCATIONS: [&str; 4] = ["CARGO_MANIFEST_DIR", "__dirname", "__file__", "import.meta"];

/// The steps from a path to the folder holding it, which keep a file's own
/// location its own only in an expression that starts from it: the parent
/// of another value (`COLLIE_MANIFEST.parent`) is a computed value.
const CLIMBS: [&str; 2] = ["parent", "dirname"];

/// The calls that join a path's pieces, which the walk back from `tests`
/// steps over.
const JOINS: [&str; 6] = [
    "path.resolve",
    "path.join",
    ".joinpath",
    ".join",
    ".push",
    ".resolve",
];

/// How the piece before a `tests` segment joins it.
#[derive(PartialEq, Eq)]
enum Joined {
    /// Inside one literal, a folder then `/` (`src/egress/tests`).
    Slash,
    /// A literal piece of its own (`.join("goodfetch-collie").join("tests")`).
    Quote,
    /// A computed value: a variable, a call, an interpolation (`crate_folder
    /// .join("tests")`, `` `${folder}/tests` ``).
    Computed,
    /// Nothing: the path starts at `tests` (`"tests/it"`, `=tests/it`).
    Start,
}

/// The piece of a path just before a `tests` segment, `before` being the
/// text up to it, found by walking back over what joins a path's pieces
/// (quotes, `/`, spaces, commas, brackets, `+`, `.join`…), how it joins, and
/// the expression the piece ends (`Path(__file__).parent`).
fn preceding(before: &str) -> (&str, Joined, &str) {
    let mut rest = before;
    loop {
        let trimmed = rest.trim_end_matches(|c: char| {
            c.is_whitespace() || QUOTES.contains(&c) || "/\\,()+".contains(c)
        });
        let trimmed = JOINS
            .iter()
            .find_map(|join| trimmed.strip_suffix(join))
            .unwrap_or(trimmed);
        if trimmed.len() == rest.len() {
            break;
        }
        rest = trimmed;
    }
    let start = rest
        .char_indices()
        .rev()
        .find(|(_, c)| !in_a_name(*c))
        .map_or(0, |(at, c)| at + c.len_utf8());
    let piece = &rest[start..];
    let joined = if piece.is_empty() {
        if rest.ends_with(['}', ')', ']']) {
            Joined::Computed
        } else {
            Joined::Start
        }
    } else {
        match before[rest.len()..].chars().next() {
            Some('/' | '\\') => Joined::Slash,
            Some(c) if QUOTES.contains(&c) => Joined::Quote,
            _ => Joined::Computed,
        }
    };
    (piece, joined, expression(rest, start))
}

/// The expression whose last piece starts at `start` of `text`: that piece,
/// and while it continues a call or an index (`.parent` after `)`), the
/// call or index it continues, back to the name it starts from
/// (`Path(__file__).resolve().parent`).
fn expression(text: &str, start: usize) -> &str {
    let mut at = start;
    while text[at..].starts_with('.') {
        let Some(opening) = opening(&text[..at]) else {
            break;
        };
        at = text[..opening]
            .char_indices()
            .rev()
            .find(|(_, c)| !in_a_name(*c))
            .map_or(0, |(at, c)| at + c.len_utf8());
    }
    &text[at..]
}

/// Where the bracket that the closing one ending `text` closes opens, if
/// `text` ends with one.
fn opening(text: &str) -> Option<usize> {
    let (open, close) = match text.chars().last()? {
        ')' => ('(', ')'),
        ']' => ('[', ']'),
        _ => return None,
    };
    let mut depth = 0_usize;
    for (at, c) in text.char_indices().rev() {
        if c == close {
            depth += 1;
        } else if c == open {
            depth -= 1;
            if depth == 0 {
                return Some(at);
            }
        }
    }
    None
}

/// Whether `code` holds a name of `names` as a name of its own, not inside a
/// longer one.
fn names_the_crate(code: &str, names: &[&str]) -> bool {
    names.iter().any(|name| {
        code.match_indices(name).any(|(at, _)| {
            let bounded_before = !code[..at].chars().last().is_some_and(in_a_name);
            let bounded_after = !code[at + name.len()..]
                .chars()
                .next()
                .is_some_and(in_a_name);
            bounded_before && bounded_after
        })
    })
}

/// Whether `code`, a file's lines but its comments, may name `tests_file`
/// or a folder of its `tests/` folder that holds it. Each `tests` that is a
/// path segment (after `/`, `\` or `=`, before `/`, or quoted alone; not a
/// word of prose such as `the tests'`), followed by nothing more or a path
/// holding the file, counts when the piece before it is the crate's folder
/// or name; in the crate's own file, unless another folder of the crate is
/// that piece (`src/egress/tests`); in another member's file, when a
/// computed value is that piece and the file names the crate anywhere, but
/// not one of its own location (`LOCATIONS`, and a `CLIMBS` step in an
/// expression that starts from one), or when the path starts at `tests` or
/// `./tests` on a line that names the crate (`{ crate = "goodfetch-collie",
/// test = "tests/it/log.rs" }`). Markdown counts
/// only through the crate's folder or name, and xtask's files only on a line
/// that climbs out of xtask (`../`).
fn names_a_tests_file(code: &str, naming: Naming, tests_file: &TestsFile<'_>) -> bool {
    let mentioned = names_the_crate(code, &tests_file.names);
    code.match_indices(TESTS_FOLDER).any(|(start, _)| {
        let before = &code[..start];
        let after = &code[start + TESTS_FOLDER.len()..];
        let previous = before.chars().last();
        let next = after.chars().next();
        if previous.is_some_and(in_a_name) || next.is_some_and(in_a_name) {
            return false;
        }
        let quoted = |c: Option<char>| c.is_some_and(|c| QUOTES.contains(&c));
        let a_segment = matches!(previous, Some('/' | '\\' | '='))
            || next == Some('/')
            || (quoted(previous) && quoted(next));
        if !a_segment || !tests_file.holds(after) {
            return false;
        }
        let line = before.rsplit('\n').next().unwrap_or_default();
        if naming.xtasks && !naming.own && !line.contains("../") {
            return false;
        }
        let (piece, joined, expression) = preceding(before);
        if tests_file.names.contains(&piece) {
            return true;
        }
        if naming.prose {
            return false;
        }
        if naming.own {
            let another_folder = joined == Joined::Slash && !matches!(piece, "" | "." | "..");
            return !another_folder;
        }
        let from_tests = joined == Joined::Start || (joined == Joined::Slash && piece == ".");
        if from_tests {
            let line_end = after
                .find('\n')
                .map_or(code.len(), |end| start + TESTS_FOLDER.len() + end);
            let whole_line = &code[start - line.len()..line_end];
            return names_the_crate(whole_line, &tests_file.names);
        }
        let last = piece.rsplit('.').next().unwrap_or(piece);
        let anchored = (LOCATIONS.contains(&last) || CLIMBS.contains(&last))
            && starts_from_a_location(expression);
        joined == Joined::Computed && !anchored && mentioned
    })
}

/// Whether `expression` starts from one of a file's own locations
/// (`LOCATIONS`), past the calls that wrap it (`Path(__file__)`,
/// `new(env!("CARGO_MANIFEST_DIR"))`): `COLLIE_MANIFEST.parent` does not.
fn starts_from_a_location(expression: &str) -> bool {
    let mut rest = expression;
    loop {
        let at_a_location = LOCATIONS.iter().any(|location| {
            rest.strip_prefix(location).is_some_and(|after| {
                !after.starts_with(|c: char| c.is_ascii_alphanumeric() || c == '_')
            })
        });
        if at_a_location {
            return true;
        }
        let callee = rest.trim_start_matches(|c: char| in_a_name(c) || c == '!');
        let Some(argument) = callee.strip_prefix('(') else {
            return false;
        };
        rest = argument.trim_start_matches(|c: char| c.is_whitespace() || QUOTES.contains(&c));
    }
}

/// `text` without its whole-line comments, by the marker that opens one in
/// `file`'s language: `#` in Python, shell, `justfile`, TOML and YAML, `//`
/// elsewhere. A line that opens with `*` may be code (`*slot = …`), so it
/// stays, as does any line a comment marker does not open. In Rust, the
/// lines of a fenced example (three backticks or `~~~`) in a `///` or `//!`
/// comment stay too, as a doctest's code, which `cargo test --doc` runs; a
/// fence of another language (`text`) stays as well, since the search must
/// miss no reader.
fn without_comments(text: &str, file: &RepoPath) -> String {
    let name = file.as_str().rsplit('/').next().unwrap_or_default();
    let extension = Path::new(name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or_default();
    let marker =
        if name == "justfile" || ["py", "sh", "bash", "toml", "yaml", "yml"].contains(&extension) {
            "#"
        } else {
            "//"
        };
    let rust = extension == "rs";
    let mut in_example = false;
    text.lines()
        .filter(|line| {
            let line = line.trim_start();
            let doc = ["///", "//!"]
                .iter()
                .find_map(|opening| line.strip_prefix(opening))
                .filter(|doc| rust && !doc.starts_with('/'));
            let Some(doc) = doc else {
                in_example = false;
                return !line.starts_with(marker);
            };
            if doc.trim_start().starts_with("```") || doc.trim_start().starts_with("~~~") {
                in_example = !in_example;
                return false;
            }
            in_example
        })
        .collect::<Vec<_>>()
        .join("\n")
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
/// literals naming a document there are its checks' inputs, which the
/// quick and full gates run in `just lint` and the floor guard, and the docs
/// gate only in the floor guard and `just lint-documents`, without `xtask
/// codegen --check` (`selection::CODEGEN_INPUTS`), or its fixture
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
        for quote in ['"', '\'', '`'] {
            literals.push(format!("{quote}{folder}{quote}"));
            literals.push(format!("{quote}{folder}/"));
        }
    }
    literals
}
