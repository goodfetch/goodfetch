//! The drift check (ADR 0071), run with the daily test-health job: it reads
//! only the results store and GitHub. `main`'s latest value of each counted
//! figure on each platform, from the Release suite's runs on `main` and from
//! the merge queue, is compared with the release's: the value recorded by
//! the runs that tested the last release's commit, or, for a figure they did
//! not measure, the last one `main` recorded up to them. A run measures only
//! the benchmarks its change selected, so each figure's latest value is its
//! own. A figure up more than 10% since that release opens or updates one
//! `needs-triage` issue, catching accepted steps under 5% that add up; so
//! does a resident-memory peak within 10% of its ceiling. A new issue about
//! a benchmark goes to the spec whose ticket added the benchmark (ADR 0136),
//! and any other to triage. Before the first release, no figure is compared
//! with a release.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use jiff::Timestamp;
use serde::Deserialize;

use crate::github::Api;
use crate::issues::{Issues, Label, Owner};
use crate::output;
use crate::owners::{Added, Owners};
use crate::results::{self, Kept};

use super::budget;
use super::figure::{Figure, Sign, away_from_zero, hundredths};
use super::figures::{self, Recorded};
use super::gungraun;

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

/// One figure's latest value on one platform, with the run that measured it.
struct Latest {
    started_at: Timestamp,
    commit: String,
    recorded: Recorded,
}

/// A figure on a platform: `(<os>-<arch>, <subject> <figure>)`.
type Key = (String, String);

/// Checks the figures of the store at `store` and keeps their issues through
/// `api` in `repository`, whose history is the checkout at `root`.
pub(crate) fn check(
    root: &Path,
    api: &Api<'_>,
    repository: &str,
    store: &Path,
) -> Result<ExitCode> {
    let kept = results::read_kept(store, |file| {
        file.rsplit('/').next() == Some(figures::FILE_NAME)
    })?;
    let on_main: Vec<&Kept> = kept
        .iter()
        .filter(|kept| kept.run.branch.as_deref() == Some(MAIN) || kept.run.event == MERGE_GROUP)
        .collect();
    let main = latest_figures(on_main.iter().copied())?;
    let release = match api
        .find::<Release>(&format!("/repos/{repository}/releases/latest"))
        .context("reading the latest release")?
    {
        Some(release) => {
            let commit: Commit = api
                .get(&format!("/repos/{repository}/commits/{}", release.tag_name))
                .with_context(|| format!("reading the commit of {}", release.tag_name))?;
            // A run measures only what its change selected, so the release's
            // figure of a benchmark its commit's runs did not measure is the
            // last one `main` recorded up to them.
            let released = on_main
                .iter()
                .filter(|kept| kept.commit == commit.sha)
                .map(|kept| kept.run.started_at)
                .max();
            let figures = match released {
                Some(until) => latest_figures(
                    on_main
                        .iter()
                        .copied()
                        .filter(|kept| kept.run.started_at <= until),
                )?,
                None => BTreeMap::new(),
            };
            Some((release.tag_name, figures))
        }
        None => None,
    };
    let mut issues = Issues::new(api, repository);
    let owners = Owners::new(api, repository);
    for ((platform, id), latest) in &main {
        let recorded = &latest.recorded;
        let owner = || benchmark_owner(&owners, root, &recorded.subject);
        if let Some(ceiling) = near_ceiling(recorded) {
            let key = format!("perf-ceiling {platform} {id}");
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
                latest.commit
            );
            issues.keep(
                Label::NeedsTriage,
                &key,
                &title,
                &body,
                latest.started_at,
                owner,
            )?;
        }
        let Some((tag, released)) = release.as_ref().and_then(|(tag, figures)| {
            let released = figures.get(&(platform.clone(), id.clone()))?;
            Some((tag, &released.recorded))
        }) else {
            continue;
        };
        if budget::drifted(released.value, recorded.value) {
            let key = format!("perf-drift {platform} {id}");
            let title = format!("Performance drift since {tag}: {id} on {platform}");
            let mut body = format!(
                "`main`'s {id} on {platform} is up more than {}% since {tag} (ADR 0071's drift check).\n\n",
                budget::DRIFT_PERCENT
            );
            let _ = write!(
                body,
                "| {tag} | `main` | Change | Commit |\n|---:|---:|---:|---|\n| {} | {} | {} | {} |\n",
                released.value,
                recorded.value,
                change(released.value, recorded.value),
                latest.commit
            );
            issues.keep(
                Label::NeedsTriage,
                &key,
                &title,
                &body,
                latest.started_at,
                owner,
            )?;
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

/// The owner of the benchmark `subject` names: the spec whose ticket added
/// its function to its bench file; none for a subject that names no
/// benchmark (ADR 0136).
fn benchmark_owner(owners: &Owners<'_>, root: &Path, subject: &str) -> Result<Option<Owner>> {
    let Some(source) = gungraun::source(subject) else {
        return Ok(None);
    };
    owners.of_added(
        root,
        &Added {
            text: format!("fn {}(", source.function),
            files: Some(format!(
                ":(glob)**/{}/benches/{}.rs",
                source.package, source.file
            )),
        },
    )
}

/// Each figure's latest value on each platform, `<os>-<arch>`, among `kept`:
/// a run records only the benchmarks its change selected, so one figure's
/// latest value may come from an older run than another's.
fn latest_figures<'a>(kept: impl Iterator<Item = &'a Kept>) -> Result<BTreeMap<Key, Latest>> {
    let mut latest: BTreeMap<Key, Latest> = BTreeMap::new();
    for kept in kept {
        let platform = format!("{}-{}", kept.configuration.os, kept.configuration.arch);
        let figures = figures::parse(&kept.contents)
            .with_context(|| format!("in run {} attempt {}", kept.run.run, kept.run.attempt))?;
        for recorded in figures.figures {
            let key = (platform.clone(), recorded.id());
            if latest
                .get(&key)
                .is_some_and(|known| known.started_at > kept.run.started_at)
            {
                continue;
            }
            latest.insert(
                key,
                Latest {
                    started_at: kept.run.started_at,
                    commit: kept.commit.clone(),
                    recorded,
                },
            );
        }
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

fn change(released: u64, value: u64) -> String {
    if released == 0 {
        return "—".to_owned();
    }
    let change = away_from_zero(
        (i128::from(value) - i128::from(released)) * 10_000,
        i128::from(released),
    );
    format!("{}%", hundredths(change, Sign::Always))
}
