//! The drift check (ADR 0071), run with the daily test-health job: it reads
//! only the results store and GitHub. `main`'s latest counted figures on
//! each platform, from the Release suite's runs on `main` and from the merge
//! queue, are compared with the figures recorded on the last release's
//! commit. A figure up more than 10% since that release opens or updates one
//! `needs-triage` issue, catching accepted steps under 5% that add up; so
//! does a resident-memory peak within 10% of its ceiling. Before the first
//! release, no figure is compared with a release.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use jiff::Timestamp;
use serde::Deserialize;

use crate::github::Api;
use crate::issues::{Issues, Label};
use crate::output;
use crate::results::{self, Kept};

use super::budget;
use super::figure::Figure;
use super::figures::{self, Recorded};

/// The branch whose figures are `main`'s, and the event of the merge
/// queue's runs, which test what `main` becomes.
const MAIN: &str = "main";
const MERGE_GROUP: &str = "merge_group";

#[derive(Deserialize)]
struct Release {
    tag_name: String,
}

#[derive(Deserialize)]
struct Commit {
    sha: String,
}

/// One platform's figures from one attempt.
struct Recording {
    started_at: Timestamp,
    commit: String,
    figures: BTreeMap<String, Recorded>,
}

pub(crate) fn check(api: &Api<'_>, repository: &str, store: &Path) -> Result<ExitCode> {
    let kept = results::read_kept(store, figures::FILE_NAME)?;
    let main =
        latest_by_platform(kept.iter().filter(|kept| {
            kept.run.branch.as_deref() == Some(MAIN) || kept.run.event == MERGE_GROUP
        }))?;
    let release = match api
        .find::<Release>(&format!("/repos/{repository}/releases/latest"))
        .context("reading the latest release")?
    {
        Some(release) => {
            let commit: Commit = api
                .get(&format!("/repos/{repository}/commits/{}", release.tag_name))
                .with_context(|| format!("reading the commit of {}", release.tag_name))?;
            Some((
                release.tag_name,
                latest_by_platform(kept.iter().filter(|kept| kept.run.commit == commit.sha))?,
            ))
        }
        None => None,
    };
    let mut issues = Issues::new(api, repository);
    for (platform, recording) in &main {
        for recorded in recording.figures.values() {
            if let Some(ceiling) = near_ceiling(recorded) {
                let key = format!("perf-ceiling {platform} {}", recorded.id());
                let title = format!(
                    "Resident memory near its ceiling: {} on {platform}",
                    recorded.subject
                );
                let body = format!(
                    "`main`'s resident-memory peak of {} on {platform} is within {}% of its ceiling (ADR 0071).\n\n\
                     | Peak | Ceiling | Commit |\n|---:|---:|---|\n| {} | {ceiling} | {} |\n",
                    recorded.subject,
                    budget::CEILING_MARGIN_PERCENT,
                    recorded.value,
                    recording.commit
                );
                issues.keep(
                    Label::NeedsTriage,
                    &key,
                    &title,
                    &body,
                    recording.started_at,
                )?;
            }
            let Some((tag, released)) = release.as_ref().and_then(|(tag, recordings)| {
                let released = recordings.get(platform)?.figures.get(&recorded.id())?;
                Some((tag, released))
            }) else {
                continue;
            };
            if drifted(released.value, recorded.value) {
                let key = format!("perf-drift {platform} {}", recorded.id());
                let title = format!(
                    "Performance drift since {tag}: {} on {platform}",
                    recorded.id()
                );
                let mut body = format!(
                    "`main`'s {} on {platform} is up more than {}% since {tag} (ADR 0071's drift check).\n\n",
                    recorded.id(),
                    budget::DRIFT_PERCENT
                );
                let _ = write!(
                    body,
                    "| {tag} | `main` | Change | Commit |\n|---:|---:|---:|---|\n| {} | {} | {} | {} |\n",
                    released.value,
                    recorded.value,
                    change(released.value, recorded.value),
                    recording.commit
                );
                issues.keep(
                    Label::NeedsTriage,
                    &key,
                    &title,
                    &body,
                    recording.started_at,
                )?;
            }
        }
    }
    for action in issues.actions() {
        output::line(action);
    }
    if release.is_none() {
        output::line("No release yet: no figure is compared with a release (ADR 0071).");
    }
    Ok(ExitCode::SUCCESS)
}

/// The latest recording of each platform, `<os>-<arch>`, among `kept`.
fn latest_by_platform<'a>(
    kept: impl Iterator<Item = &'a Kept>,
) -> Result<BTreeMap<String, Recording>> {
    let mut latest: BTreeMap<String, Recording> = BTreeMap::new();
    for kept in kept {
        let platform = format!("{}-{}", kept.configuration.os, kept.configuration.arch);
        if latest
            .get(&platform)
            .is_some_and(|recording| recording.started_at > kept.run.started_at)
        {
            continue;
        }
        let figures = figures::parse(&kept.contents)
            .with_context(|| format!("in run {} attempt {}", kept.run.run, kept.run.attempt))?;
        latest.insert(
            platform,
            Recording {
                started_at: kept.run.started_at,
                commit: kept.run.commit.clone(),
                figures: figures
                    .figures
                    .into_iter()
                    .map(|recorded| (recorded.id(), recorded))
                    .collect(),
            },
        );
    }
    Ok(latest)
}

/// A resident-memory peak's ceiling, when the peak is within the margin.
fn near_ceiling(recorded: &Recorded) -> Option<u64> {
    let ceiling = recorded.ceiling?;
    (recorded.figure == Figure::ResidentMemory
        && u128::from(recorded.value) * 100
            >= u128::from(ceiling) * u128::from(100 - budget::CEILING_MARGIN_PERCENT))
    .then_some(ceiling)
}

fn drifted(released: u64, value: u64) -> bool {
    u128::from(value) * 100 > u128::from(released) * u128::from(100 + budget::DRIFT_PERCENT)
}

fn change(released: u64, value: u64) -> String {
    if released == 0 {
        return "—".to_owned();
    }
    let hundredths = (i128::from(value) - i128::from(released)) * 10_000 / i128::from(released);
    let sign = if hundredths < 0 { '-' } else { '+' };
    let hundredths = hundredths.unsigned_abs();
    format!("{sign}{}.{:02}%", hundredths / 100, hundredths % 100)
}
