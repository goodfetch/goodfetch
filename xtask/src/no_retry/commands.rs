//! Commands, wherever they are written: the flags and variables that undo a
//! runner's locks. A command is a logical line of a script (continuations
//! joined) or one string of a structured file; a flag that only one runner
//! has is refused on a line that names that runner, or in that runner's own
//! configuration file, so a downloader's flag of the same name is left alone.

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::File;
use crate::check::Finding;

/// A test runner whose name, on a command line, scopes its flags.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Runner {
    Pytest,
}

impl Runner {
    const ALL: [Self; 1] = [Self::Pytest];

    fn word(self) -> &'static str {
        match self {
            Self::Pytest => "pytest",
        }
    }
}

/// Where a flag means a retry.
enum Scope {
    /// On a command of this runner.
    Runner(Runner),
}

/// The values a flag or variable may take without retrying a test.
enum Allowed {
    /// Only `0`.
    Zero,
}

struct Flag {
    name: &'static str,
    scope: Scope,
    allowed: Allowed,
}

const FLAGS: [Flag; 1] = [
    // pytest-rerunfailures.
    Flag {
        name: "--reruns",
        scope: Scope::Runner(Runner::Pytest),
        allowed: Allowed::Zero,
    },
];

/// pytest's configuration files: pytest's flags count there without its name.
pub(super) fn check_pytest_config(file: &File, findings: &mut Vec<Finding>) -> Result<()> {
    if std::path::Path::new(&file.path)
        .extension()
        .is_some_and(|extension| extension == "toml")
    {
        let document: Value =
            toml::from_str(&file.text).with_context(|| format!("parsing {}", file.path))?;
        check_document(&file.path, &document, &[Runner::Pytest], findings);
    } else {
        check_text(&file.path, &file.text, &[Runner::Pytest], findings);
    }
    Ok(())
}

/// Every string of a structured file, a list of strings read as one command.
fn check_document(path: &str, value: &Value, runners: &[Runner], findings: &mut Vec<Finding>) {
    match value {
        Value::String(text) => check_text(path, text, runners, findings),
        Value::Array(items) if items.iter().all(Value::is_string) => {
            let words: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
            check_text(path, &words.join(" "), runners, findings);
        }
        Value::Array(items) => {
            for item in items {
                check_document(path, item, runners, findings);
            }
        }
        Value::Object(entries) => {
            for value in entries.values() {
                check_document(path, value, runners, findings);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

/// A script or free text, one logical line at a time.
fn check_text(path: &str, text: &str, runners: &[Runner], findings: &mut Vec<Finding>) {
    for line in logical_lines(text) {
        let words = words(&line);
        let on_line = |runner: Runner| {
            runners.contains(&runner)
                || words
                    .iter()
                    .any(|word| word.to_ascii_lowercase().contains(runner.word()))
        };
        let line_runners: Vec<Runner> = Runner::ALL.into_iter().filter(|r| on_line(*r)).collect();
        for (index, word) in words.iter().enumerate() {
            let Some(flag_text) = word.strip_prefix("--") else {
                continue;
            };
            let (name, inline) = match flag_text.split_once('=') {
                Some((name, value)) => (name, Some(value)),
                None => (flag_text, None),
            };
            let Some(flag) = FLAGS.iter().find(|flag| &flag.name[2..] == name) else {
                continue;
            };
            let applies = match flag.scope {
                Scope::Runner(runner) => line_runners.contains(&runner),
            };
            if !applies {
                continue;
            }
            let value = inline.or_else(|| {
                words
                    .get(index + 1)
                    .map(String::as_str)
                    .filter(|next| !next.starts_with('-'))
            });
            if !allowed(&flag.allowed, value) {
                findings.push(Finding::new(
                    "retry-flag",
                    path,
                    format!("`{}` retries a failed test (ADR 0064): {line}", flag.name),
                ));
            }
        }
    }
}

fn allowed(allowed: &Allowed, value: Option<&str>) -> bool {
    let number = value.and_then(|value| value.parse::<i64>().ok());
    match allowed {
        Allowed::Zero => number == Some(0),
    }
}

/// The text's lines, a line ending in `\` joined with the next.
fn logical_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    for line in text.lines() {
        if let Some(start) = line.trim_end().strip_suffix('\\') {
            current.push_str(start);
            current.push(' ');
        } else {
            current.push_str(line);
            lines.push(std::mem::take(&mut current));
        }
    }
    if !current.is_empty() {
        lines.push(current);
    }
    lines
}

/// A line's words, quotes and brackets trimmed, up to a `#` comment.
fn words(line: &str) -> Vec<String> {
    const TRIMMED: [char; 10] = ['"', '\'', '[', ']', '(', ')', '{', '}', ',', ';'];
    line.split_whitespace()
        .take_while(|word| !word.starts_with('#'))
        .map(|word| word.trim_matches(TRIMMED).to_owned())
        .filter(|word| !word.is_empty())
        .collect()
}
