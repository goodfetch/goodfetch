//! Retries written in test sources and in the configurations beside them:
//! Vitest's `retry` and Playwright's `retries` (its `test.describe.configure`
//! included), pytest's `flaky` marker, and a rerun plugin in `uv.lock`.

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use super::File;
use crate::check::Finding;

const LOCKFILE: &str = "uv.lock";

/// pytest plugins that run a failed test again.
const RERUN_PLUGINS: [&str; 3] = ["pytest-rerunfailures", "flaky", "pytest-retry"];

/// Vitest's option, then Playwright's.
const RETRY_OPTIONS: [&str; 2] = ["retry", "retries"];

/// Foxhound's crawl setting, as the formatter writes it: it retries a
/// crawl's request, not the test. Its whole object, so a runner's own key
/// beside it (Vitest's `count`) is still found.
const CRAWL_SETTING: &str = "retry: { handlerFailed: true }";

#[derive(Deserialize)]
struct Lockfile {
    #[serde(rename = "package", default)]
    packages: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
}

pub(super) fn check_lockfile(root: &Path) -> Result<Vec<Finding>> {
    let path = root.join(LOCKFILE);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let lockfile: Lockfile =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(lockfile
        .packages
        .iter()
        .filter(|package| RERUN_PLUGINS.contains(&package.name.as_str()))
        .map(|package| {
            Finding::new(
                "retry-plugin",
                LOCKFILE,
                format!("`{}` reruns failed tests (ADR 0064)", package.name),
            )
        })
        .collect())
}

/// An object property `retry` or `retries` whose value is anything but `0`
/// or Foxhound's crawl setting.
pub(super) fn check_javascript(file: &File, findings: &mut Vec<Finding>) {
    for line in file.text.lines() {
        let code = line.split("//").next().unwrap_or(line);
        let code = code.replace(CRAWL_SETTING, "");
        for option in RETRY_OPTIONS {
            if let Some(value) = property_value(&code, option)
                && value != "0"
            {
                findings.push(Finding::new(
                    "retry-setting",
                    &file.path,
                    format!("`{option}: {value}` retries a failed test (ADR 0064)"),
                ));
            }
        }
    }
}

/// The value of `<name>: <value>` in a line of code, the name standing alone
/// (quoted or not) and the value running to the next `,`, `}` or line end.
fn property_value<'a>(code: &'a str, name: &str) -> Option<&'a str> {
    let mut from = 0;
    while let Some(found) = code[from..].find(name) {
        let start = from + found;
        let end = start + name.len();
        from = end;
        let before = code[..start].chars().next_back();
        if before.is_some_and(|character| {
            character.is_alphanumeric() || matches!(character, '_' | '$' | '.')
        }) {
            continue;
        }
        let after = code[end..].trim_start_matches(['"', '\'']).trim_start();
        let Some(value) = after.strip_prefix(':') else {
            continue;
        };
        let value = value.split([',', '}']).next().unwrap_or(value).trim();
        return Some(value);
    }
    None
}

/// A `RetryOnFailure` result in a typ or web-test expectation (ADR 0074).
pub(super) fn check_expectations(file: &File, findings: &mut Vec<Finding>) {
    for line in file.text.lines() {
        if super::commands::words(line)
            .iter()
            .any(|word| word == "RetryOnFailure")
        {
            findings.push(Finding::new(
                "retry-expectation",
                &file.path,
                format!("`{}` retries a failed test (ADR 0074)", line.trim()),
            ));
        }
    }
}

/// pytest's `flaky` marker, and the `flaky` plugin's decorator.
pub(super) fn check_python(file: &File, findings: &mut Vec<Finding>) {
    for line in file.text.lines() {
        let code = line.split('#').next().unwrap_or(line).trim();
        if code.contains("mark.flaky") || code.starts_with("@flaky") {
            findings.push(Finding::new(
                "flaky-marker",
                &file.path,
                format!("`{code}` reruns a failed test (ADR 0064)"),
            ));
        }
    }
}
