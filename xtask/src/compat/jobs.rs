//! `xtask compat jobs` (ADR 0066): the Release suite's skew jobs, written to
//! [`JOBS`] and held there by `xtask codegen --check`.
//!
//! Each released minor of the current major, at its latest patch, takes every
//! check of `compat/skew-checks.toml` whose harness exists, per Product
//! (ADR 0094), in as few jobs as fit ADR 0061's 45 minutes; with none
//! released, one set of jobs runs the current build against itself. No check
//! is ever dropped: one too long for any job gets a job of its own, whose
//! measured time `xtask budgets` raises. A check without its harness is
//! listed as awaiting its ticket, never among what runs.
//! `compat/skew-exclusions.toml` removes single Journey variants of a minor.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::releases::ReleasesFile;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::check::Finding;
use crate::journeys::{VariantId, is_surface};
use crate::products::Product;

/// The generated jobs' file.
pub(crate) use crate::release_suite::SKEW_JOBS as JOBS;

/// The checks each older minor takes.
const CHECKS: &str = "compat/skew-checks.toml";

/// The Journey variants a minor's checks leave out.
const EXCLUSIONS: &str = "compat/skew-exclusions.toml";

/// What names the jobs that run the current build against itself.
const CURRENT: &str = "current";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChecksFile {
    /// What every job spends before its first check: the checkout, the
    /// fetched artifacts and the builds.
    setup_minutes: u32,
    check: Vec<Check>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Check {
    id: String,
    product: Product,
    minutes: u32,
    journeys: Option<Journeys>,
    /// Whether it runs the older executable on what the current Foxhound
    /// wrote.
    #[serde(default)]
    older_engine: bool,
    /// The ticket whose change gives the check its harness or its tests.
    awaiting: Option<u32>,
}

/// What a check runs.
enum Harness<'a> {
    Journeys(&'a Journeys),
    OlderEngine,
}

impl Check {
    /// What it runs; none while it awaits its ticket.
    fn runnable(&self) -> Option<Harness<'_>> {
        match (self.awaiting, &self.journeys) {
            (Some(_), _) => None,
            (None, Some(journeys)) => Some(Harness::Journeys(journeys)),
            (None, None) => self.older_engine.then_some(Harness::OlderEngine),
        }
    }
}

/// A check that runs a tag's own Journeys on some surfaces.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Journeys {
    surfaces: Vec<String>,
    /// Whether the SDKs start the executable themselves, through their
    /// Product's engine-path variable (ADR 0090).
    #[serde(default)]
    stdio: bool,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct ExclusionsFile {
    #[serde(default)]
    exclusion: Vec<Exclusion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Exclusion {
    /// The released minor, such as `1.2`.
    minor: String,
    variant: VariantId,
    reason: String,
}

/// The skew jobs.
#[derive(Serialize)]
pub(crate) struct Jobs {
    jobs: Vec<Job>,
    awaiting: Vec<Awaiting>,
}

#[derive(Serialize)]
struct Job {
    name: String,
    /// The release whose Clients run; none against the current build itself.
    version: Option<String>,
    tag: Option<String>,
    /// The estimate of `compat/skew-checks.toml`.
    minutes: u32,
    steps: Vec<Step>,
}

/// One test run of a job.
#[derive(Serialize)]
struct Step {
    checks: Vec<String>,
    #[serde(flatten)]
    run: Run,
}

#[derive(Serialize)]
#[serde(untagged)]
enum Run {
    /// `just journeys`.
    Journeys {
        /// The Product's executable that the Journeys run against.
        executable: &'static str,
        stdio: bool,
        /// The arguments of `just journeys`, the executable's aside.
        journeys: Vec<String>,
    },
    /// `just older-engine`.
    OlderEngine {
        /// The Product whose older executable it runs.
        older_engine: &'static str,
    },
}

#[derive(Serialize)]
struct Awaiting {
    check: String,
    issue: u32,
}

impl Jobs {
    /// The file's text.
    pub(crate) fn text(&self) -> Result<Vec<u8>> {
        let mut text = serde_json::to_string_pretty(self)?;
        text.push('\n');
        Ok(text.into_bytes())
    }

    /// A note for each job whose estimate passes the budget.
    pub(crate) fn over_budget(&self) -> Vec<Finding> {
        self.jobs
            .iter()
            .filter(|job| job.minutes > crate::release_suite::JOB_BUDGET_MINUTES)
            .map(|job| {
                Finding::new(
                    "over-budget",
                    &job.name,
                    format!(
                        "its one check takes {} minutes with the job's set-up, past ADR 0061's {}; it runs all the same, and `xtask budgets` raises its measured time",
                        job.minutes,
                        crate::release_suite::JOB_BUDGET_MINUTES
                    ),
                )
            })
            .collect()
    }
}

/// The jobs of `root`'s releases, checks and exclusions, when its released
/// crates carry `product_version`.
pub(crate) fn jobs(root: &Path, product_version: &str) -> Result<Jobs> {
    let releases = super::releases(root)?;
    let checks: ChecksFile =
        parse(root, CHECKS)?.with_context(|| format!("{CHECKS} is missing"))?;
    refuse_bad_checks(&checks.check).with_context(|| format!("reading {CHECKS}"))?;
    let exclusions: ExclusionsFile = parse(root, EXCLUSIONS)?.unwrap_or_default();
    refuse_bad_exclusions(&exclusions.exclusion, &releases)
        .with_context(|| format!("reading {EXCLUSIONS}"))?;

    let minors = older_minors(&releases, product_version)?;
    let mut jobs = Vec::new();
    for (minor, version) in &minors {
        let excluded: Vec<&VariantId> = exclusions
            .exclusion
            .iter()
            .filter(|exclusion| exclusion.minor == *minor)
            .map(|exclusion| &exclusion.variant)
            .collect();
        jobs.extend(jobs_of(minor, Some(version.as_str()), &excluded, &checks));
    }
    if minors.is_empty() {
        jobs.extend(jobs_of(CURRENT, None, &[], &checks));
    }
    let awaiting = checks
        .check
        .iter()
        .filter_map(|check| {
            check.awaiting.map(|issue| Awaiting {
                check: check.id.clone(),
                issue,
            })
        })
        .collect();
    Ok(Jobs { jobs, awaiting })
}

/// `path` as TOML, when it exists.
fn parse<T: DeserializeOwned>(root: &Path, path: &str) -> Result<Option<T>> {
    let file = root.join(path);
    if !file.exists() {
        return Ok(None);
    }
    let text = std::fs::read_to_string(&file).with_context(|| format!("reading {path}"))?;
    toml::from_str(&text)
        .map(Some)
        .with_context(|| format!("parsing {path}"))
}

fn refuse_bad_checks(checks: &[Check]) -> Result<()> {
    let mut ids = BTreeSet::new();
    for check in checks {
        let id = &check.id;
        if !ids.insert(id) {
            bail!("`{id}` is listed twice");
        }
        if check.minutes == 0 {
            bail!("`{id}` takes no minute");
        }
        match &check.journeys {
            Some(_) if check.older_engine => {
                bail!("`{id}` runs Journeys and an older engine: make them two checks");
            }
            Some(journeys) if journeys.surfaces.is_empty() => {
                bail!("`{id}` runs the Journeys of no surface");
            }
            Some(journeys) => {
                if let Some(unknown) = journeys.surfaces.iter().find(|name| !is_surface(name)) {
                    bail!("`{id}` names `{unknown}`, no surface of ADR 0063, nor `foxhound-cli`");
                }
            }
            None if check.awaiting.is_none() && !check.older_engine => {
                bail!("`{id}` runs nothing and awaits no ticket");
            }
            None => {}
        }
    }
    if !checks.iter().any(|check| check.runnable().is_some()) {
        bail!("no check runs, so no minor would have a job");
    }
    Ok(())
}

fn refuse_bad_exclusions(exclusions: &[Exclusion], releases: &ReleasesFile) -> Result<()> {
    let mut minors = BTreeSet::new();
    for release in &releases.release {
        let version = ProductVersion::parse(&release.version.to_string())?;
        minors.insert(version.major_minor().to_string());
    }
    for Exclusion {
        minor,
        variant,
        reason,
    } in exclusions
    {
        if !minors.contains(minor) {
            bail!("`{variant}` is excluded from {minor}, a minor never released");
        }
        if reason.trim().is_empty() {
            bail!("`{variant}` is excluded from {minor} with no reason");
        }
    }
    Ok(())
}

/// Each released minor of the current major, oldest first, with its latest
/// patch. The current major is the executables' own, once `xtask release
/// prepare` set it on a release branch, else the newest release's, since
/// `main` keeps the workspace's `0.0.x`. A preview has none: each `0.y`
/// counts as a major (ADR 0030).
fn older_minors(
    releases: &ReleasesFile,
    product_version: &str,
) -> Result<Vec<(String, ProductVersion)>> {
    let mut sorted: Vec<_> = releases
        .release
        .iter()
        .map(|entry| &entry.version)
        .collect();
    sorted.sort();
    let versions = sorted
        .into_iter()
        .map(|version| Ok(ProductVersion::parse(&version.to_string())?))
        .collect::<Result<Vec<_>>>()?;
    let own = ProductVersion::parse(product_version)
        .with_context(|| format!("the Product version {product_version} is no semver"))?;
    let unprepared = own.major_minor().to_string() == "0.0";
    let current = match versions.last() {
        Some(newest) if unprepared => newest.major_minor(),
        _ => own.major_minor(),
    };
    let mut minors = BTreeMap::new();
    for version in versions {
        let minor = version.major_minor();
        if current.major > 0 && minor.major == current.major && minor <= current {
            minors.insert(minor, version);
        }
    }
    Ok(minors
        .into_iter()
        .map(|(minor, version)| (minor.to_string(), version))
        .collect())
}

/// The jobs of one minor, or of the current build: its runnable checks, each
/// in the first job it fits.
fn jobs_of(
    minor: &str,
    version: Option<&str>,
    excluded: &[&VariantId],
    checks: &ChecksFile,
) -> Vec<Job> {
    let room = crate::release_suite::JOB_BUDGET_MINUTES.saturating_sub(checks.setup_minutes);
    let mut groups: Vec<(u32, Vec<(&Check, Harness<'_>)>)> = Vec::new();
    for check in &checks.check {
        let Some(harness) = check.runnable() else {
            continue;
        };
        match groups
            .iter_mut()
            .find(|(used, _)| used + check.minutes <= room)
        {
            Some((used, group)) => {
                *used += check.minutes;
                group.push((check, harness));
            }
            None => groups.push((check.minutes, vec![(check, harness)])),
        }
    }
    let alone = groups.len() == 1;
    groups
        .into_iter()
        .enumerate()
        .map(|(index, (minutes, group))| Job {
            name: if alone {
                minor.to_owned()
            } else {
                format!("{minor}-{}", index + 1)
            },
            version: version.map(str::to_owned),
            tag: version.map(tag),
            minutes: checks.setup_minutes + minutes,
            steps: steps(&group, version, excluded),
        })
        .collect()
}

/// The tag of a release.
fn tag(version: &str) -> String {
    format!("v{version}")
}

/// The steps of a job's checks, in their order. Journeys take one per
/// executable and connection, so each Product's tests are built and listed
/// once.
fn steps(
    group: &[(&Check, Harness<'_>)],
    version: Option<&str>,
    excluded: &[&VariantId],
) -> Vec<Step> {
    let mut steps: Vec<Step> = Vec::new();
    let mut kinds: Vec<(Product, bool)> = Vec::new();
    for (check, harness) in group {
        match harness {
            Harness::Journeys(journeys) => {
                let kind = (check.product, journeys.stdio);
                if !kinds.contains(&kind) {
                    kinds.push(kind);
                    steps.push(journeys_step(group, kind, version, excluded));
                }
            }
            Harness::OlderEngine => steps.push(Step {
                checks: vec![check.id.clone()],
                run: Run::OlderEngine {
                    older_engine: check.product.key(),
                },
            }),
        }
    }
    steps
}

/// The step of a job's Journey checks of one executable and connection.
fn journeys_step(
    group: &[(&Check, Harness<'_>)],
    (product, stdio): (Product, bool),
    version: Option<&str>,
    excluded: &[&VariantId],
) -> Step {
    let mut checks = Vec::new();
    let mut surfaces: Vec<&str> = Vec::new();
    for (check, harness) in group {
        let Harness::Journeys(journeys) = harness else {
            continue;
        };
        if (check.product, journeys.stdio) != (product, stdio) {
            continue;
        }
        checks.push(check.id.clone());
        for surface in &journeys.surfaces {
            if !surfaces.contains(&surface.as_str()) {
                surfaces.push(surface);
            }
        }
    }
    let mut journeys = Vec::new();
    let mut flag = |flag: &str, value: String| journeys.extend([flag.to_owned(), value]);
    if let Some(version) = version {
        flag("--tag", tag(version));
    }
    flag("--product", product.key().to_owned());
    for surface in &surfaces {
        flag("--surface", (*surface).to_owned());
    }
    for variant in excluded {
        if surfaces.contains(&variant.surface()) {
            flag("--exclude", variant.to_string());
        }
    }
    Step {
        checks,
        run: Run::Journeys {
            executable: product.executable().package,
            stdio,
            journeys,
        },
    }
}
