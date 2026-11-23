//! The gates every release passes (ADR 0030, as ADRs 0064 and 0072 add to
//! it).

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::quarantine;

/// What `xtask features report`'s `features.json` holds that the release
/// gate reads (ADR 0072).
#[derive(Deserialize)]
struct FeatureReport {
    release_commit: Option<String>,
    gate: FeatureGate,
}

#[derive(Deserialize)]
struct FeatureGate {
    untested: Vec<String>,
}

/// Every release's gates: an empty quarantine list (ADR 0064), and a
/// Feature report, at `report` from the workspace's folder, read on a
/// Release run of `main` with no Feature untested (ADR 0072).
pub(super) fn release(root: &Path, report: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let quarantined = quarantine::read(root)?.len();
    if quarantined > 0 {
        findings.push(Finding::new(
            "quarantine-not-empty",
            quarantine::LIST,
            format!("{quarantined} quarantined tests hold every release until they leave the list (ADR 0064)"),
        ));
    }
    let shown = report.display().to_string();
    let text = std::fs::read_to_string(root.join(report))
        .with_context(|| format!("reading the Feature report {shown}"))?;
    let read: FeatureReport = serde_json::from_str(&text)
        .with_context(|| format!("parsing the Feature report {shown}"))?;
    let Some(commit) = read.release_commit else {
        findings.push(Finding::new(
            "no-release-run",
            shown,
            "the Feature report read no Release run of main, so no Feature is known tested (ADR 0072)",
        ));
        return Ok(findings);
    };
    findings.extend(read.gate.untested.into_iter().map(|feature| {
        Finding::new(
            "feature-untested",
            feature,
            format!("untested on main's latest Release run, of {commit} (ADR 0072)"),
        )
    }));
    Ok(findings)
}
