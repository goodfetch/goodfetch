//! The wording check (ADR 0035's wording policy, as ADR 0097 amends it): no
//! text published about either Product uses a term of `wording.toml`. It
//! matches whole words, whatever their case, in prose only, and skips code.
//!
//! It reads the docs pages (`docs/` but `docs/adr/` and `docs/agents/`, and
//! the site's own pages in `website/`), every README and `CHANGELOG.md`, the
//! skills (`skills/`), the descriptions, titles and messages of the generated
//! catalog and JSON Schema bundle (the settings schema and the Error codes'
//! messages among them), and every package's description and keywords (Cargo,
//! npm and PyPI manifests). Frozen `rnd/` and fixtures are never read.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde_json::Value;

use crate::check::Finding;
use crate::tree;

/// The one list of terms (ADR 0097).
const LIST: &str = "wording.toml";

/// Folders of `docs/` that publish nothing: the decisions and the agents'
/// configuration.
const UNPUBLISHED: [&str; 2] = ["docs/adr/", "docs/agents/"];

/// The generated contract files whose texts are published (ADR 0038).
const CONTRACTS: [&str; 2] = [
    "crates/goodfetch-protocol/generated/catalog.json",
    "crates/goodfetch-protocol/generated/bundle.json",
];

/// The keys of the contract files that hold prose.
const PROSE_KEYS: [&str; 3] = ["description", "message", "title"];

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct List {
    term: Vec<TermEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct TermEntry {
    words: String,
    #[serde(default)]
    every_form: bool,
}

/// One term: its words in a row, lowercased, and whether its last word also
/// matches every word it starts.
struct Term {
    text: String,
    words: Vec<String>,
    every_form: bool,
}

impl Term {
    /// Whether the term starts at `tokens[0]`.
    fn starts(&self, tokens: &[Token]) -> bool {
        if tokens.len() < self.words.len() {
            return false;
        }
        let last = self.words.len() - 1;
        self.words
            .iter()
            .zip(tokens)
            .enumerate()
            .all(|(index, (word, token))| {
                if index == last && self.every_form {
                    token.word.starts_with(word.as_str())
                } else {
                    token.word == *word
                }
            })
    }
}

/// A word of prose, lowercased, with the line it is on (from 1).
struct Token {
    word: String,
    line: usize,
}

/// Where a text sits, for its finding's subject.
enum Place<'a> {
    /// A file read as Markdown: findings name the line.
    Lines(&'a str),
    /// One field of a structured file, by its JSON Pointer.
    Field(&'a str, &'a str),
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let terms = read_terms(root)?;
    let mut findings = Vec::new();
    for file in tree::files(root, "")? {
        let Some(kind) = kind(&file) else {
            continue;
        };
        let text =
            std::fs::read_to_string(root.join(&file)).with_context(|| format!("reading {file}"))?;
        match kind {
            Kind::Markdown => {
                scan(
                    &terms,
                    &markdown_prose(&text),
                    &Place::Lines(&file),
                    &mut findings,
                );
            }
            Kind::Contract => {
                let value: Value =
                    serde_json::from_str(&text).with_context(|| format!("reading {file}"))?;
                for (pointer, text) in strings(&value, "", &|key| PROSE_KEYS.contains(&key)) {
                    let prose = markdown_prose(&text);
                    scan(
                        &terms,
                        &prose,
                        &Place::Field(&file, &pointer),
                        &mut findings,
                    );
                }
            }
            Kind::Manifest(section) => {
                let value = manifest(&file, &text)?;
                let Some(section) = section.map_or(Some(&value), |name| value.get(name)) else {
                    continue;
                };
                let prefix = section_prefix(&file);
                for (pointer, text) in strings(section, &prefix, &|_| true)
                    .into_iter()
                    .filter(|(pointer, _)| is_published_field(&prefix, pointer))
                {
                    scan(&terms, &text, &Place::Field(&file, &pointer), &mut findings);
                }
            }
        }
    }
    Ok(findings)
}

fn read_terms(root: &Path) -> Result<Vec<Term>> {
    let text =
        std::fs::read_to_string(root.join(LIST)).with_context(|| format!("reading {LIST}"))?;
    let list: List = toml::from_str(&text).with_context(|| format!("reading {LIST}"))?;
    let mut terms = Vec::new();
    for entry in list.term {
        let words: Vec<String> = tokens(&entry.words)
            .into_iter()
            .map(|token| token.word)
            .collect();
        if words.is_empty() {
            bail!("{LIST}: a term has no word");
        }
        terms.push(Term {
            text: entry.words,
            words,
            every_form: entry.every_form,
        });
    }
    Ok(terms)
}

enum Kind {
    Markdown,
    /// The catalog or the JSON Schema bundle: only their prose keys.
    Contract,
    /// A package manifest; the table holding its description and keywords,
    /// or `None` for npm's top level.
    Manifest(Option<&'static str>),
}

/// What a file holds for the check, or `None` when it publishes nothing.
fn kind(file: &str) -> Option<Kind> {
    if UNPUBLISHED.iter().any(|folder| file.starts_with(folder)) {
        return None;
    }
    let name = file.rsplit('/').next().unwrap_or(file);
    let lowercase = name.to_ascii_lowercase();
    let extension = extension(file);
    let markdown = matches!(extension.as_deref(), Some("md" | "mdx"));
    if markdown
        && (["docs/", "website/", "skills/"]
            .iter()
            .any(|folder| file.starts_with(folder))
            || lowercase.starts_with("readme")
            || lowercase.starts_with("changelog"))
    {
        return Some(Kind::Markdown);
    }
    if CONTRACTS.contains(&file) {
        return Some(Kind::Contract);
    }
    match name {
        "Cargo.toml" => Some(Kind::Manifest(Some("package"))),
        "pyproject.toml" => Some(Kind::Manifest(Some("project"))),
        "package.json" => Some(Kind::Manifest(None)),
        _ => None,
    }
}

/// A file's extension, lowercased.
fn extension(file: &str) -> Option<String> {
    Path::new(file)
        .extension()
        .map(|extension| extension.to_string_lossy().to_ascii_lowercase())
}

/// A manifest as JSON values, whatever its format.
fn manifest(file: &str, text: &str) -> Result<Value> {
    if extension(file).as_deref() == Some("json") {
        return serde_json::from_str(text).with_context(|| format!("reading {file}"));
    }
    let table: toml::Table = toml::from_str(text).with_context(|| format!("reading {file}"))?;
    serde_json::to_value(table).with_context(|| format!("reading {file}"))
}

/// The JSON Pointer of a manifest's section.
fn section_prefix(file: &str) -> String {
    match file.rsplit('/').next() {
        Some("Cargo.toml") => "/package".to_owned(),
        Some("pyproject.toml") => "/project".to_owned(),
        _ => String::new(),
    }
}

/// Whether a manifest's string is its description or one of its keywords.
fn is_published_field(prefix: &str, pointer: &str) -> bool {
    let Some(field) = pointer.strip_prefix(prefix) else {
        return false;
    };
    field == "/description"
        || field
            .strip_prefix("/keywords/")
            .is_some_and(|index| index.bytes().all(|byte| byte.is_ascii_digit()))
}

/// Every string under `value` whose key `wanted` accepts (an array's items
/// take their array's key), with its JSON Pointer.
fn strings(value: &Value, pointer: &str, wanted: &dyn Fn(&str) -> bool) -> Vec<(String, String)> {
    fn walk(
        value: &Value,
        pointer: &str,
        key: Option<&str>,
        wanted: &dyn Fn(&str) -> bool,
        found: &mut Vec<(String, String)>,
    ) {
        match value {
            Value::String(text) => {
                if key.is_none_or(wanted) {
                    found.push((pointer.to_owned(), text.clone()));
                }
            }
            Value::Array(items) => {
                for (index, item) in items.iter().enumerate() {
                    walk(item, &format!("{pointer}/{index}"), key, wanted, found);
                }
            }
            Value::Object(fields) => {
                for (name, item) in fields {
                    let escaped = name.replace('~', "~0").replace('/', "~1");
                    walk(
                        item,
                        &format!("{pointer}/{escaped}"),
                        Some(name),
                        wanted,
                        found,
                    );
                }
            }
            Value::Null | Value::Bool(_) | Value::Number(_) => {}
        }
    }
    let mut found = Vec::new();
    walk(value, pointer, None, wanted, &mut found);
    found
}

/// Adds a finding for each term in `text`.
fn scan(terms: &[Term], text: &str, place: &Place<'_>, findings: &mut Vec<Finding>) {
    let tokens = tokens(text);
    for start in 0..tokens.len() {
        for term in terms.iter().filter(|term| term.starts(&tokens[start..])) {
            let subject = match place {
                Place::Lines(file) => format!("{file}:{}", tokens[start].line),
                Place::Field(file, pointer) => format!("{file}#{pointer}"),
            };
            findings.push(Finding::new(
                "wording",
                subject,
                format!(
                    "\"{}\" is on the wording list ({LIST}, ADRs 0035 and 0097); rewrite the sentence",
                    term.text
                ),
            ));
        }
    }
}

/// The words of a text, lowercased: runs of letters and digits.
fn tokens(text: &str) -> Vec<Token> {
    let mut tokens = Vec::new();
    for (index, line) in text.lines().enumerate() {
        for word in line.split(|character: char| !character.is_alphanumeric()) {
            if !word.is_empty() {
                tokens.push(Token {
                    word: word.to_lowercase(),
                    line: index + 1,
                });
            }
        }
    }
    tokens
}

/// A Markdown text with its code blanked out, line for line: fenced code
/// blocks, code spans, link destinations, autolinks and bare URLs.
fn markdown_prose(text: &str) -> String {
    let mut fence: Option<(char, usize)> = None;
    let mut lines = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim_start_matches(' ');
        let indent = line.len() - trimmed.len();
        let opener = trimmed
            .chars()
            .next()
            .filter(|&c| indent < 4 && (c == '`' || c == '~'));
        let run = opener.map_or(0, |c| trimmed.chars().take_while(|&d| d == c).count());
        match fence {
            Some((character, length)) => {
                if opener == Some(character) && run >= length && trimmed[run..].trim().is_empty() {
                    fence = None;
                }
                lines.push(String::new());
            }
            None if run >= 3 => {
                fence = opener.map(|character| (character, run));
                lines.push(String::new());
            }
            None => lines.push(line.to_owned()),
        }
    }
    let joined = lines.join("\n");
    blank_urls(&blank_links(&blank_code_spans(&joined)))
}

/// Blanks the contents of each code span (a run of backticks closed by a run
/// of the same length), keeping line breaks.
fn blank_code_spans(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut output = characters.clone();
    let run_at = |index: usize| {
        characters[index..]
            .iter()
            .take_while(|&&c| c == '`')
            .count()
    };
    let mut index = 0;
    while index < characters.len() {
        if characters[index] != '`' {
            index += 1;
            continue;
        }
        let length = run_at(index);
        let content = index + length;
        let mut search = content;
        let mut close = None;
        while search < characters.len() {
            if characters[search] == '`' {
                let run = run_at(search);
                if run == length {
                    close = Some(search);
                    break;
                }
                search += run;
            } else {
                search += 1;
            }
        }
        let Some(close) = close else {
            index = content;
            continue;
        };
        for character in &mut output[content..close] {
            if *character != '\n' {
                *character = ' ';
            }
        }
        index = close + length;
    }
    output.into_iter().collect()
}

/// Blanks each link destination, `[text](destination)`, and each autolink,
/// `<scheme:…>`.
fn blank_links(text: &str) -> String {
    let characters: Vec<char> = text.chars().collect();
    let mut output = characters.clone();
    let mut index = 0;
    while index < characters.len() {
        let closing = match characters[index] {
            '(' if index > 0 && characters[index - 1] == ']' => ')',
            '<' if is_autolink(&characters[index + 1..]) => '>',
            _ => {
                index += 1;
                continue;
            }
        };
        let end = characters[index + 1..]
            .iter()
            .position(|&c| c == closing || c == '\n')
            .map(|offset| index + 1 + offset);
        match end {
            Some(end) if characters[end] == closing => {
                for character in &mut output[index + 1..end] {
                    *character = ' ';
                }
                index = end + 1;
            }
            _ => index += 1,
        }
    }
    output.into_iter().collect()
}

/// Whether text after a `<` opens an autolink: a scheme, then a colon.
fn is_autolink(after: &[char]) -> bool {
    let scheme = after
        .iter()
        .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
        .count();
    scheme >= 2 && after.get(scheme) == Some(&':')
}

/// Blanks each bare `http://` or `https://` URL, up to the next whitespace.
fn blank_urls(text: &str) -> String {
    let mut output = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = ["http://", "https://"]
        .iter()
        .filter_map(|scheme| rest.find(scheme))
        .min()
    {
        output.push_str(&rest[..start]);
        let url = &rest[start..];
        let end = url.find(char::is_whitespace).unwrap_or(url.len());
        output.extend(std::iter::repeat_n(' ', url[..end].chars().count()));
        rest = &url[end..];
    }
    output.push_str(rest);
    output
}
