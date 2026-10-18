//! nextest's configuration: `retries = 0` and `flaky-result = "fail"` hold in
//! every profile and per-test override. Profiles inherit from `default`, so
//! `default` states the flaky-result lock that nextest leaves at `pass`.

use std::path::Path;

use anyhow::{Context as _, Result};
use toml::{Table, Value};

use crate::check::Finding;

const CONFIG: &str = ".config/nextest.toml";

pub(super) fn check(root: &Path) -> Result<Vec<Finding>> {
    let path = root.join(CONFIG);
    let config: Table = if path.is_file() {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    } else {
        Table::new()
    };
    let empty = Table::new();
    let profiles = config
        .get("profile")
        .and_then(Value::as_table)
        .unwrap_or(&empty);

    let mut findings = Vec::new();
    let default_states_the_lock = profiles
        .get("default")
        .and_then(|profile| profile.get("flaky-result"))
        .is_some();
    if !default_states_the_lock {
        findings.push(Finding::new(
            "flaky-result-not-fail",
            CONFIG,
            "the default profile does not set `flaky-result = \"fail\"`, and nextest's own default is `pass`",
        ));
    }
    for (name, profile) in profiles {
        let Some(profile) = profile.as_table() else {
            continue;
        };
        settings(&format!("profile `{name}`"), profile, &mut findings);
        let overrides = profile.get("overrides").and_then(Value::as_array);
        for entry in overrides.into_iter().flatten().filter_map(Value::as_table) {
            settings(
                &format!("a per-test override of profile `{name}`"),
                entry,
                &mut findings,
            );
        }
    }
    Ok(findings)
}

fn settings(place: &str, table: &Table, findings: &mut Vec<Finding>) {
    if let Some(retries) = table.get("retries")
        && !no_retries(retries)
    {
        findings.push(Finding::new(
            "retry-setting",
            CONFIG,
            format!("{place} retries a failed test; tests are never retried (ADR 0064)"),
        ));
    }
    if let Some(flaky_result) = table.get("flaky-result")
        && flaky_result.as_str() != Some("fail")
    {
        findings.push(Finding::new(
            "flaky-result-not-fail",
            CONFIG,
            format!("{place} sets `flaky-result` to something other than \"fail\""),
        ));
    }
}

/// `retries = 0`, or a backoff table whose `count` is 0.
fn no_retries(retries: &Value) -> bool {
    let count = match retries {
        Value::Table(table) => table.get("count"),
        other => Some(other),
    };
    count.and_then(Value::as_integer) == Some(0)
}
