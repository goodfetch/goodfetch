//! The moves that lower the bar in code: a suppression outside the Floor's
//! forms, a stub or an empty catch, a skipped test and a deleted one. Regular
//! expressions on the diff's lines, on purpose: the guard catches the cheap road
//! to green, never a determined author, and a shape it cannot read is left to
//! `code-review`, which also checks the Floor.

use std::collections::BTreeSet;
use std::sync::LazyLock;

use anyhow::Result;
use regex::Regex;

use super::diff::{FileChange, Line};
use crate::check::Finding;

/// How many lines an attribute or an empty catch may span.
const LOOK_AHEAD: usize = 12;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Language {
    Rust,
    /// TypeScript and JavaScript, which share their comments and test runners.
    Script,
    Python,
    Shell,
}

fn language(path: &str) -> Option<Language> {
    let (_, extension) = path.rsplit_once('.')?;
    match extension {
        "rs" => Some(Language::Rust),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Some(Language::Script),
        "py" => Some(Language::Python),
        "sh" => Some(Language::Shell),
        _ => None,
    }
}

/// Code nobody here writes: vendored or generated, or R&D material, which sets
/// nothing (AGENTS.md).
fn is_scanned(path: &str) -> bool {
    !(path.starts_with("vendor/")
        || path.starts_with("rnd/")
        || path.split('/').any(|folder| folder == "generated"))
}

/// A test's file, where the Floor's type-test forms hold. An SDK type test is
/// `*.test-d.ts`.
fn is_test_file(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or(path);
    path.split('/')
        .any(|folder| matches!(folder, "tests" | "test"))
        || name.contains(".test.")
        || name.contains(".test-d.")
        || name.contains(".spec.")
        || name.starts_with("test_")
        || name.ends_with("_test.py")
        || name == "conftest.py"
}

fn pattern(source: &str) -> Regex {
    Regex::new(source).expect("invariant: the guard's patterns are valid")
}

struct Patterns {
    rust_allow: Regex,
    rust_expect: Regex,
    rust_reason: Regex,
    rust_ignore: Regex,
    rust_child_test: Regex,
    rust_stub: Regex,
    rust_empty_arm: Regex,
    rust_test: Regex,
    rust_case: Regex,
    rust_function: Regex,
    script_banned: Regex,
    script_oxlint: Regex,
    script_oxlint_form: Regex,
    script_expect_error: Regex,
    script_expect_error_form: Regex,
    script_stub: Regex,
    script_empty_catch: Regex,
    script_open_catch: Regex,
    script_skip: Regex,
    script_test: Regex,
    python_banned: Regex,
    python_type_ignore: Regex,
    python_type_ignore_form: Regex,
    python_pyright: Regex,
    python_pyright_form: Regex,
    python_stub: Regex,
    python_empty_except: Regex,
    python_open_except: Regex,
    python_skip: Regex,
    python_test: Regex,
    shell_banned: Regex,
}

static PATTERNS: LazyLock<Patterns> = LazyLock::new(|| Patterns {
    rust_allow: pattern(r"^#!?\[\s*(cfg_attr\(.*,\s*)?allow\s*\("),
    rust_expect: pattern(r"^#!?\[\s*(cfg_attr\(.*,\s*)?expect\s*\("),
    rust_reason: pattern(r#"reason\s*=\s*"\s*[^"\s]"#),
    rust_ignore: pattern(r"^#\[\s*(cfg_attr\(.*,\s*)?ignore\b"),
    rust_child_test: pattern(r#"^#\[\s*ignore\s*=\s*"run as a child by \S"#),
    rust_stub: pattern(
        r#"\b(todo|unimplemented)!\s*[(\[{]|\bpanic!\s*\(\s*"(?i:not (yet )?implemented)"#,
    ),
    rust_empty_arm: pattern(r"\bErr\(\s*_\w*\s*\)\s*=>\s*(\{\s*\}|\(\s*\))"),
    rust_test: pattern(r"^\s*#\[\s*(\w+::)*(test|rstest)\b"),
    rust_case: pattern(r"^\s*#\[\s*(?:cfg_attr\(.*,\s*)?case::(\w+)"),
    rust_function: pattern(r"^\s*(pub(\([^)]*\))?\s+)?(async\s+)?fn\s+(\w+)"),
    script_banned: pattern(
        r"@ts-ignore|@ts-nocheck|eslint-disable|biome-ignore|(istanbul|c8|v8) ignore|Stryker disable|nosemgrep|gitleaks:allow",
    ),
    script_oxlint: pattern(r"oxlint-disable"),
    // Its reason is what oxlint's parser takes as a description after `--`.
    script_oxlint_form: pattern(r"oxlint-disable-next-line\s+[@\w/-]+(,?\s+[@\w/-]+)*\s+--\s+\S"),
    script_expect_error: pattern(r"@ts-expect-error"),
    script_expect_error_form: pattern(r"@ts-expect-error\s*(:|--)?\s*[^\s*:-]"),
    script_stub: pattern(r"(?i)throw\s+new\s+\w*Error\(.*not (yet )?implemented"),
    script_empty_catch: pattern(
        r"\bcatch\s*(\([^)]*\))?\s*\{\s*\}|\.catch\(\s*(\([^)]*\)|\w+)?\s*=>\s*\{\s*\}\s*\)",
    ),
    script_open_catch: pattern(r"\bcatch\s*(\([^)]*\))?\s*\{\s*$"),
    script_skip: pattern(
        r"\b(test|it|describe|bench)(\.\w+)*\.(skip|todo|fixme)\b|\bx(it|test|describe)\(",
    ),
    script_test: pattern(r#"^\s*(test|it)(\.\w+)*\(\s*["'`]([^"'`]+)"#),
    python_banned: pattern(
        r"#\s*noqa\b|#\s*ruff:\s*noqa|pragma:\s*no cover|nosemgrep|gitleaks:allow",
    ),
    python_type_ignore: pattern(r"#\s*type:\s*ignore"),
    python_type_ignore_form: pattern(r"#\s*type:\s*ignore\[[^\]\s]+\]"),
    python_pyright: pattern(r"#\s*pyright:\s*\w"),
    python_pyright_form: pattern(r"#\s*pyright:\s*ignore\[[^\]\s]+\]"),
    python_stub: pattern(r"\braise\s+NotImplementedError\b"),
    python_empty_except: pattern(r"^\s*except\b.*:\s*(pass|\.\.\.)\s*(#.*)?$"),
    python_open_except: pattern(r"^\s*except\b.*:\s*(#.*)?$"),
    python_skip: pattern(r"pytest\.mark\.(skip|skipif|xfail)\b|\bpytest\.(skip|xfail)\("),
    python_test: pattern(r"^\s*(async\s+)?def\s+(test_\w+)"),
    shell_banned: pattern(r"#\s*shellcheck\s+disable|gitleaks:allow"),
});

/// The lines after `index` that sit right below it in the file.
fn following(lines: &[Line], index: usize) -> impl Iterator<Item = &Line> {
    let hunk = lines[index].hunk;
    lines[index + 1..]
        .iter()
        .take(LOOK_AHEAD)
        .take_while(move |line| line.hunk == hunk)
}

/// An attribute's whole text, however many added lines it spans.
fn attribute(lines: &[Line], index: usize) -> String {
    let mut text = lines[index].text.trim().to_owned();
    let mut rest = following(lines, index);
    while text.matches('[').count() > text.matches(']').count() {
        let Some(line) = rest.next() else { break };
        text.push(' ');
        text.push_str(line.text.trim());
    }
    text
}

fn next_is(lines: &[Line], index: usize, closes: impl Fn(&str) -> bool) -> bool {
    following(lines, index)
        .next()
        .is_some_and(|line| closes(line.text.trim()))
}

const SUPPRESSION: &str = "suppression";
const STUB: &str = "stub";
const SKIP: &str = "skipped-test";
const NOT_LISTED: &str = "a suppression the Floor does not list";
const STUB_WHY: &str = "an unimplemented stub stands in for the work";
const CATCH_WHY: &str = "an empty catch turns a failure into silence";

/// A move: the finding's kind and why, in the Floor's words.
type Move = (&'static str, &'static str);

fn rust_attribute_move(attribute: &str) -> Option<Move> {
    let patterns = &*PATTERNS;
    if patterns.rust_allow.is_match(attribute) {
        return Some((
            SUPPRESSION,
            "`#[allow]`; the Floor's form is `#[expect(lint, reason = \"…\")]`",
        ));
    }
    if patterns.rust_expect.is_match(attribute) && !patterns.rust_reason.is_match(attribute) {
        return Some((SUPPRESSION, "`#[expect]` with no `reason`"));
    }
    if patterns.rust_ignore.is_match(attribute) && !patterns.rust_child_test.is_match(attribute) {
        return Some((
            SKIP,
            "the Floor's form is `#[ignore = \"run as a child by <test>\"]`",
        ));
    }
    None
}

fn rust_move(lines: &[Line], index: usize) -> Option<Move> {
    let patterns = &*PATTERNS;
    let text = lines[index].text.as_str();
    let trimmed = text.trim_start();
    if trimmed.starts_with("//") {
        return None;
    }
    if trimmed.starts_with("#[") || trimmed.starts_with("#![") {
        return rust_attribute_move(&attribute(lines, index));
    }
    if patterns.rust_stub.is_match(text) {
        return Some((STUB, STUB_WHY));
    }
    if patterns.rust_empty_arm.is_match(text) {
        return Some((STUB, "an error arm that does nothing is an empty catch"));
    }
    None
}

fn script_move(path: &str, lines: &[Line], index: usize) -> Option<Move> {
    let patterns = &*PATTERNS;
    let text = lines[index].text.as_str();
    if patterns.script_banned.is_match(text) {
        return Some((SUPPRESSION, NOT_LISTED));
    }
    if patterns.script_oxlint.is_match(text) && !patterns.script_oxlint_form.is_match(text) {
        return Some((
            SUPPRESSION,
            "the Floor's form is `oxlint-disable-next-line <rule> -- <reason>`",
        ));
    }
    if patterns.script_expect_error.is_match(text)
        && !(is_test_file(path) && patterns.script_expect_error_form.is_match(text))
    {
        return Some((
            SUPPRESSION,
            "`@ts-expect-error` belongs in a type test, with what it refuses",
        ));
    }
    if patterns.script_stub.is_match(text) {
        return Some((STUB, STUB_WHY));
    }
    if patterns.script_empty_catch.is_match(text)
        || (patterns.script_open_catch.is_match(text)
            && next_is(lines, index, |next| next.starts_with('}')))
    {
        return Some((STUB, CATCH_WHY));
    }
    patterns
        .script_skip
        .is_match(text)
        .then_some((SKIP, "a skipped test"))
}

fn python_move(path: &str, lines: &[Line], index: usize) -> Option<Move> {
    let patterns = &*PATTERNS;
    let text = lines[index].text.as_str();
    if patterns.python_banned.is_match(text) {
        return Some((SUPPRESSION, NOT_LISTED));
    }
    if patterns.python_type_ignore.is_match(text)
        && !(is_test_file(path) && patterns.python_type_ignore_form.is_match(text))
    {
        return Some((SUPPRESSION, "`# type: ignore[code]` belongs in a type test"));
    }
    if patterns.python_pyright.is_match(text) && !patterns.python_pyright_form.is_match(text) {
        return Some((SUPPRESSION, "the Floor's form is `# pyright: ignore[rule]`"));
    }
    if patterns.python_stub.is_match(text) {
        return Some((STUB, STUB_WHY));
    }
    if patterns.python_empty_except.is_match(text)
        || (patterns.python_open_except.is_match(text)
            && next_is(lines, index, |next| next == "pass" || next == "..."))
    {
        return Some((STUB, CATCH_WHY));
    }
    patterns
        .python_skip
        .is_match(text)
        .then_some((SKIP, "a skipped test"))
}

/// What one added line of a language does to the bar.
fn added_move(language: Language, path: &str, lines: &[Line], index: usize) -> Option<Move> {
    match language {
        Language::Rust => rust_move(lines, index),
        Language::Script => script_move(path, lines, index),
        Language::Python => python_move(path, lines, index),
        Language::Shell => PATTERNS
            .shell_banned
            .is_match(lines[index].text.as_str())
            .then_some((SUPPRESSION, NOT_LISTED)),
    }
}

/// The names of the tests a file holds. A Rust test is its function's name under
/// a test attribute, and each rstest case its own name.
fn tests_in(language: Language, contents: &str) -> BTreeSet<String> {
    let patterns = &*PATTERNS;
    let mut names = BTreeSet::new();
    let mut awaiting = false;
    for text in contents.lines() {
        match language {
            Language::Rust => {
                if let Some(case) = patterns.rust_case.captures(text) {
                    names.insert(format!("case::{}", &case[1]));
                } else if patterns.rust_test.is_match(text) {
                    awaiting = true;
                } else if awaiting && let Some(function) = patterns.rust_function.captures(text) {
                    names.insert(function[4].to_owned());
                    awaiting = false;
                }
            }
            Language::Script => {
                if let Some(test) = patterns.script_test.captures(text) {
                    names.insert(test[3].to_owned());
                }
            }
            Language::Python => {
                if let Some(test) = patterns.python_test.captures(text) {
                    names.insert(test[2].to_owned());
                }
            }
            Language::Shell => {}
        }
    }
    names
}

/// Every lowered line of the change. `contents` reads a file at the merge base
/// (`false`) or the head (`true`), `None` when it has no such file. A test is
/// deleted when its name left a file and no file of the change holds it, so a
/// moved test passes and a renamed one needs a trailer naming its replacement,
/// as the Floor asks.
pub(super) fn findings(
    files: &[FileChange],
    contents: impl Fn(&str, bool) -> Result<Option<String>>,
) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let scanned: Vec<(&FileChange, Language)> = files
        .iter()
        .filter(|file| is_scanned(&file.path))
        .filter_map(|file| Some((file, language(&file.path)?)))
        .collect();
    let mut remaining = BTreeSet::new();
    for (file, language) in &scanned {
        if let Some(head) = contents(&file.path, true)? {
            remaining.extend(tests_in(*language, &head));
        }
    }
    for (file, language) in scanned {
        for index in 0..file.added.len() {
            if let Some((kind, why)) = added_move(language, &file.path, &file.added, index) {
                let line = file.added[index].text.trim();
                let line: String = line.chars().take(120).collect();
                findings.push(Finding::new(
                    kind,
                    file.path.as_str(),
                    format!("{why}, or a `Quality-change` trailer naming this file: `{line}`"),
                ));
            }
        }
        if !file.existed {
            continue;
        }
        let base = contents(&file.path, false)?.unwrap_or_default();
        let deleted: Vec<String> = tests_in(language, &base)
            .into_iter()
            .filter(|name| !remaining.contains(name))
            .collect();
        if !deleted.is_empty() {
            findings.push(Finding::new(
                "deleted-test",
                file.path.as_str(),
                format!(
                    "{} is gone; a `Quality-change` trailer naming this file names its replacement",
                    deleted.join(", ")
                ),
            ));
        }
    }
    Ok(findings)
}
