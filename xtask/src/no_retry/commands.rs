//! Commands, wherever they are written: the flags and variables that undo a
//! runner's locks, a gtest run without Chromium's retry limit, and a shell
//! loop that runs a test again. A command is a logical line of a script
//! (continuations joined) or one string of a structured file, a list of
//! strings read as one command. A flag that only one runner has counts on a
//! line that names that runner, or in that runner's own configuration file,
//! so a downloader's flag of the same name (`curl --retry`) is left alone.

use anyhow::{Context as _, Result};
use serde_json::Value;

use super::File;
use crate::check::Finding;

/// A test runner whose name, on a command line, scopes its flags.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Runner {
    Nextest,
    Vitest,
    Playwright,
    Pytest,
    /// wptrunner, run as `wpt run`.
    Wpt,
}

impl Runner {
    fn word(self) -> &'static str {
        match self {
            Self::Nextest => "nextest",
            Self::Vitest => "vitest",
            Self::Playwright => "playwright",
            Self::Pytest => "pytest",
            Self::Wpt => "wpt",
        }
    }
}

/// Where a flag means a retry.
enum Scope {
    /// On any command: the flag's name is the runner's alone.
    Anywhere,
    /// On a command of this runner.
    Runner(Runner),
}

/// The values a flag or variable may take without retrying a test.
enum Allowed {
    /// Only `0`.
    Zero,
    /// `0` or `1`: one run in all.
    AtMostOne,
    /// Only `fail`.
    Fail,
    /// The flag itself retries, whatever follows it.
    Never,
}

struct Flag {
    /// The flag without its leading `--`.
    name: &'static str,
    scope: Scope,
    allowed: Allowed,
}

const fn flag(name: &'static str, scope: Scope, allowed: Allowed) -> Flag {
    Flag {
        name,
        scope,
        allowed,
    }
}

/// ADR 0064's runner flags, then ADR 0074's for the Chromium build gate.
const FLAGS: [Flag; 14] = [
    flag("retries", Scope::Runner(Runner::Nextest), Allowed::Zero),
    flag(
        "flaky-result",
        Scope::Runner(Runner::Nextest),
        Allowed::Fail,
    ),
    flag("retry", Scope::Runner(Runner::Vitest), Allowed::Zero),
    flag("retries", Scope::Runner(Runner::Playwright), Allowed::Zero),
    // pytest-rerunfailures and pytest-retry.
    flag("reruns", Scope::Runner(Runner::Pytest), Allowed::Zero),
    flag("retries", Scope::Runner(Runner::Pytest), Allowed::Zero),
    // Chromium's test launcher and `run_web_tests.py`.
    flag("test-launcher-retry-limit", Scope::Anywhere, Allowed::Zero),
    flag("num-retries", Scope::Anywhere, Allowed::Zero),
    // wptrunner.
    flag("retry-unexpected", Scope::Anywhere, Allowed::Zero),
    flag("rerun", Scope::Runner(Runner::Wpt), Allowed::AtMostOne),
    flag("repeat-until-unexpected", Scope::Anywhere, Allowed::Never),
    // typ, under `run_gpu_integration_test.py`.
    flag("retry-limit", Scope::Anywhere, Allowed::Zero),
    flag(
        "isolated-script-test-launcher-retry-limit",
        Scope::Anywhere,
        Allowed::Zero,
    ),
    flag(
        "retry-only-retry-on-failure-tests",
        Scope::Anywhere,
        Allowed::Never,
    ),
];

/// nextest's variables for its two locks.
const VARIABLES: [(&str, Allowed); 2] = [
    ("NEXTEST_RETRIES", Allowed::Zero),
    ("NEXTEST_FLAKY_RESULT", Allowed::Fail),
];

/// Chromium's test launcher flag that every gtest run states.
const LAUNCHER_RETRY_LIMIT: &str = "--test-launcher-retry-limit";

/// How a file's strings are read.
#[derive(Clone, Copy)]
pub(super) struct Reading<'a> {
    /// Runners whose flags count without their name, in their own files.
    pub(super) runners: &'a [Runner],
    /// Whether the strings are shell scripts, whose loops are read.
    pub(super) scripts: bool,
}

/// pytest's configuration files: pytest's flags count there without its name.
pub(super) fn check_pytest_config(file: &File, findings: &mut Vec<Finding>) -> Result<()> {
    let reading = Reading {
        runners: &[Runner::Pytest],
        scripts: false,
    };
    if std::path::Path::new(&file.path)
        .extension()
        .is_some_and(|extension| extension == "toml")
    {
        let document: Value =
            toml::from_str(&file.text).with_context(|| format!("parsing {}", file.path))?;
        check_document(&file.path, &document, reading, findings);
    } else {
        check_text(&file.path, &file.text, reading, findings);
    }
    Ok(())
}

/// Every string of a structured file, and the values of the variables it sets.
pub(super) fn check_document(
    path: &str,
    value: &Value,
    reading: Reading<'_>,
    findings: &mut Vec<Finding>,
) {
    match value {
        Value::String(text) => check_text(path, text, reading, findings),
        Value::Array(items) if items.iter().all(Value::is_string) => {
            let words: Vec<&str> = items.iter().filter_map(Value::as_str).collect();
            check_text(path, &words.join(" "), reading, findings);
        }
        Value::Array(items) => {
            for item in items {
                check_document(path, item, reading, findings);
            }
        }
        Value::Object(entries) => {
            for (key, value) in entries {
                // `.cargo/config.toml` may set `{ value = "…", force = true }`.
                let set = value.get("value").unwrap_or(value);
                check_variable(path, key, scalar(set).as_deref(), findings);
                check_document(path, value, reading, findings);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) => {}
    }
}

fn scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(text) => Some(text.clone()),
        Value::Number(number) => Some(number.to_string()),
        Value::Bool(flag) => Some(flag.to_string()),
        Value::Null | Value::Array(_) | Value::Object(_) => None,
    }
}

/// A script or free text, one logical line at a time, then its loops.
pub(super) fn check_text(
    path: &str,
    text: &str,
    reading: Reading<'_>,
    findings: &mut Vec<Finding>,
) {
    for line in logical_lines(text) {
        let words = words(&line);
        check_flags(path, &line, &words, reading.runners, findings);
        for (index, word) in words.iter().enumerate() {
            if let Some((name, value)) = assignment(&words, index) {
                check_variable(path, name, value, findings);
            }
            if is_gtest(word)
                && !words
                    .iter()
                    .any(|other| other.starts_with(LAUNCHER_RETRY_LIMIT))
            {
                findings.push(Finding::new(
                    "gtest-without-retry-limit",
                    path,
                    format!(
                        "`{word}` runs without `{LAUNCHER_RETRY_LIMIT}=0`, so the launcher retries a failure once (ADR 0074)"
                    ),
                ));
            }
        }
    }
    if reading.scripts {
        check_loops(path, text, findings);
    }
}

fn check_flags(
    path: &str,
    line: &str,
    words: &[String],
    runners: &[Runner],
    findings: &mut Vec<Finding>,
) {
    let named = |runner: Runner| {
        runners.contains(&runner)
            || words
                .iter()
                .any(|word| word.to_ascii_lowercase().contains(runner.word()))
    };
    for (index, word) in words.iter().enumerate() {
        let Some(flag_text) = word.strip_prefix("--") else {
            continue;
        };
        let (name, inline) = match flag_text.split_once('=') {
            Some((name, value)) => (name, Some(value)),
            None => (flag_text, None),
        };
        let value = inline.or_else(|| {
            words
                .get(index + 1)
                .map(String::as_str)
                .filter(|next| !next.starts_with('-'))
        });
        let refused = FLAGS.iter().any(|flag| {
            flag.name == name
                && match flag.scope {
                    Scope::Anywhere => true,
                    Scope::Runner(runner) => named(runner),
                }
                && !allowed(&flag.allowed, value)
        });
        if refused {
            findings.push(Finding::new(
                "retry-flag",
                path,
                format!("`{word}` retries a failed test (ADR 0064): {}", line.trim()),
            ));
        }
    }
}

/// `NAME=value`, `NAME: value` or `NAME = value`, starting at `words[index]`.
fn assignment(words: &[String], index: usize) -> Option<(&str, Option<&str>)> {
    let word = words[index].as_str();
    let next = |offset: usize| words.get(index + offset).map(String::as_str);
    if let Some((name, value)) = word.split_once('=') {
        return Some((name, Some(value)));
    }
    if let Some(name) = word.strip_suffix(':') {
        return Some((name, next(1)));
    }
    match next(1) {
        Some("=" | ":") => Some((word, next(2))),
        _ => None,
    }
}

fn check_variable(path: &str, name: &str, value: Option<&str>, findings: &mut Vec<Finding>) {
    let refused = VARIABLES
        .iter()
        .any(|(variable, rule)| *variable == name && !allowed(rule, value));
    if refused {
        findings.push(Finding::new(
            "retry-flag",
            path,
            format!(
                "`{name}={}` undoes nextest's lock (ADR 0064)",
                value.unwrap_or_default()
            ),
        ));
    }
}

fn allowed(allowed: &Allowed, value: Option<&str>) -> bool {
    let number = value.and_then(|value| value.parse::<u64>().ok());
    match allowed {
        Allowed::Zero => number == Some(0),
        Allowed::AtMostOne => number.is_some_and(|number| number <= 1),
        Allowed::Fail => value == Some("fail"),
        Allowed::Never => false,
    }
}

/// A Chromium gtest executable, such as `out/Default/base_unittests`.
fn is_gtest(word: &str) -> bool {
    let name = word.rsplit('/').next().unwrap_or(word);
    let identifier = name.chars().all(|character| {
        character.is_ascii_lowercase() || character.is_ascii_digit() || character == '_'
    });
    identifier
        && ["_unittests", "_browsertests", "_tests"]
            .iter()
            .any(|suffix| name.len() > suffix.len() && name.ends_with(suffix))
}

/// Whether a command starting at `words[index]` runs tests.
fn runs_test(words: &[String], index: usize) -> bool {
    let word = words[index].as_str();
    let next = words.get(index + 1).map(String::as_str);
    let program = word.rsplit('/').next().unwrap_or(word);
    matches!(
        program,
        "nextest" | "cargo-nextest" | "vitest" | "pytest" | "wpt"
    ) || is_gtest(word)
        || matches!(
            (program, next),
            ("playwright" | "cargo" | "insta" | "miri", Some("test"))
                | (
                    "just",
                    Some(
                        "test"
                            | "quick"
                            | "pre-merge"
                            | "release"
                            | "repeat"
                            | "_repeat"
                            | "stress"
                            | "miri"
                    )
                )
        )
}

/// Whether a script runs a test anywhere.
pub(super) fn script_runs_test(text: &str) -> bool {
    let words = script_words(text);
    (0..words.len()).any(|index| runs_test(&words, index))
}

/// A loop open in a script.
struct Loop {
    keyword: String,
    runs_test: bool,
    breaks: bool,
}

/// `until` and `while` loops that run a test, and `for` loops that run one
/// and break out (`for i in 1 2 3; do just test quick && break; done`). A
/// `for` loop over packages that runs each one once is no retry.
fn check_loops(path: &str, text: &str, findings: &mut Vec<Finding>) {
    let words = script_words(text);
    let mut open: Vec<Loop> = Vec::new();
    for (index, word) in words.iter().enumerate() {
        match word.as_str() {
            "for" | "while" | "until" => open.push(Loop {
                keyword: word.clone(),
                runs_test: false,
                breaks: false,
            }),
            "break" => {
                if let Some(innermost) = open.last_mut() {
                    innermost.breaks = true;
                }
            }
            "done" => {
                let Some(closed) = open.pop() else {
                    continue;
                };
                if closed.runs_test && (closed.keyword != "for" || closed.breaks) {
                    findings.push(Finding::new(
                        "retry-loop",
                        path,
                        format!("a `{}` loop runs a test again (ADR 0064)", closed.keyword),
                    ));
                }
                if let Some(outer) = open.last_mut() {
                    outer.runs_test |= closed.runs_test;
                }
            }
            _ => {
                if let Some(innermost) = open.last_mut()
                    && runs_test(&words, index)
                {
                    innermost.runs_test = true;
                }
            }
        }
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

/// A script's words, without the patterns of its `case` statements
/// (`nextest) …`), which name a runner without running it.
fn script_words(text: &str) -> Vec<String> {
    text.lines()
        .flat_map(|line| {
            line.split_whitespace()
                .take_while(|word| !word.starts_with('#'))
                .filter(|word| !word.ends_with(')') || word.contains('('))
                .collect::<Vec<_>>()
        })
        .flat_map(words)
        .collect()
}

/// A line's words, quotes and brackets trimmed, up to a `#` comment.
pub(super) fn words(line: &str) -> Vec<String> {
    const TRIMMED: [char; 11] = ['"', '\'', '[', ']', '(', ')', '{', '}', ',', ';', '!'];
    line.split_whitespace()
        .take_while(|word| !word.starts_with('#'))
        .map(|word| word.trim_matches(TRIMMED).to_owned())
        .filter(|word| !word.is_empty())
        .collect()
}
