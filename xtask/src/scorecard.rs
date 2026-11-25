//! The Open Source Security Foundation's Scorecard, a nightly Live check
//! (ADRs 0032 and 0064; TESTING.md, Live checks): `xtask results scorecard` reads the JSON results the
//! Scorecard action wrote and holds each check to its floor in
//! `ci/scorecard.toml`. A check under its floor fails; a check Scorecard
//! could not run (score -1) above a floor of -1, a floored check missing
//! from the results, or missing results make the Live check inconclusive;
//! a check with no floor only reports.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::output;

/// Each check's floor, by Scorecard's name for it.
pub(crate) const FLOORS: &str = "ci/scorecard.toml";

/// Scorecard's score of a check it could not run.
const NOT_RUN: i8 = -1;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Floors {
    floor: BTreeMap<String, i8>,
}

/// The part of Scorecard's JSON results the floors read.
#[derive(Deserialize)]
struct Results {
    checks: Vec<Check>,
}

#[derive(Deserialize)]
struct Check {
    name: String,
    score: i8,
    reason: String,
}

/// The Live check's result.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum Verdict {
    Passed,
    Inconclusive,
    Failed,
}

/// Holds the results at `results` to the floors of the workspace at `root`,
/// printing one line per check.
pub(crate) fn judge(root: &Path, results: &Path) -> Result<Verdict> {
    let path = root.join(FLOORS);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let floors: Floors =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    if !results.exists() {
        output::line(&format!(
            "inconclusive: Scorecard wrote no results at {}",
            results.display()
        ));
        return Ok(Verdict::Inconclusive);
    }
    let text = std::fs::read_to_string(results)
        .with_context(|| format!("reading {}", results.display()))?;
    let results: Results =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", results.display()))?;
    let mut verdict = Verdict::Passed;
    for check in &results.checks {
        let floor = floors.floor.get(&check.name).copied();
        let ending = match floor {
            None => "reported",
            Some(floor) if check.score == NOT_RUN && floor > NOT_RUN => {
                verdict = verdict.max(Verdict::Inconclusive);
                "inconclusive, not run"
            }
            Some(floor) if check.score < floor => {
                verdict = Verdict::Failed;
                "failed, under its floor"
            }
            Some(_) => "passed",
        };
        let floor = floor.map_or_else(|| "none".to_owned(), |floor| floor.to_string());
        output::line(&format!(
            "{}: {} (floor {floor}) {ending}: {}",
            check.name, check.score, check.reason
        ));
    }
    for name in floors.floor.keys() {
        if !results.checks.iter().any(|check| &check.name == name) {
            verdict = verdict.max(Verdict::Inconclusive);
            output::line(&format!("{name}: missing from the results, inconclusive"));
        }
    }
    Ok(verdict)
}
