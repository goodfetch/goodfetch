//! The gates a release passes (ADR 0030, as ADRs 0064 and 0072 add to it),
//! and those a promotion to `x.y.0` and a patch pass on top (ADRs 0032,
//! 0064, 0066 and 0067).

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use jiff::civil::Weekday;
use jiff::tz::TimeZone;
use jiff::{Timestamp, ToSpan as _};
use serde::Deserialize;
use serde::de::IgnoredAny;

use super::{ProductVersion, Stage, Tree, prepare, tagged};
use crate::check::Finding;
use crate::git;
use crate::github::{Api, List};
use crate::issues::Label;
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

/// Where the issues that hold a promotion are read.
pub(super) struct Issues<'a> {
    pub(super) api: Api<'a>,
    pub(super) repository: &'a str,
    /// The private fuzzing repository (ADR 0067).
    pub(super) fuzz_repository: &'a str,
    /// The time the two business days are counted to.
    pub(super) at: Timestamp,
}

/// The gates of `version` on top of every release's, built from `commit`:
/// none for a candidate; for a promotion to `x.y.0`, a version commit over
/// the last candidate, no unexplained Live-check failure and no open fuzz
/// crash; for a patch, the formats as `x.y.0` released them.
pub(super) fn version(
    root: &Path,
    version: &ProductVersion,
    commit: &str,
    issues: &Issues<'_>,
) -> Result<Vec<Finding>> {
    match version.stage {
        Stage::Nightly(_) => bail!(
            "a nightly, {}, is published from main and passes no release gate",
            version.semver()
        ),
        Stage::Candidate(_) => Ok(Vec::new()),
        Stage::Stable if version.patch == 0 => {
            let mut findings = stable_commit(root, version, commit)?;
            findings.extend(live_checks(issues)?);
            findings.extend(fuzz_crashes(issues)?);
            Ok(findings)
        }
        Stage::Stable => patch(root, commit, version),
    }
}

/// For a patch `x.y.z`, `z` from 1, each format whose version or schema at
/// `commit` differs from `x.y.0`'s (ADR 0066); none for any other version.
pub(super) fn patch(root: &Path, commit: &str, version: &ProductVersion) -> Result<Vec<Finding>> {
    if version.stage != Stage::Stable || version.patch == 0 {
        return Ok(Vec::new());
    }
    let minor = ProductVersion {
        patch: 0,
        ..version.clone()
    };
    formats(root, commit, &minor.tag())
}

/// A stable `x.y.0` is the last candidate's commit plus one commit that only
/// sets versions, `prepare`'s change and the generated files it changes,
/// and adds the changelog, the release notes page in `website/` and
/// `bench/results/<version>/` (ADR 0032).
fn stable_commit(root: &Path, version: &ProductVersion, commit: &str) -> Result<Vec<Finding>> {
    let commit = git::commit(root, commit)?;
    let parent = git::commit(root, &format!("{commit}^"))?;
    let prefix = format!("{}-rc.", version.tag());
    let candidates = git::text(root, &["tag", "--list", &format!("{prefix}*")])?;
    let last = candidates
        .lines()
        .filter_map(|tag| Some((tag.strip_prefix(&prefix)?.parse::<u64>().ok()?, tag)))
        .max();
    let Some((_, last)) = last else {
        return Ok(vec![Finding::new(
            "no-candidate",
            version.tag(),
            format!(
                "no candidate {prefix}N is tagged; a stable version is promoted from the last (ADR 0032)"
            ),
        )]);
    };
    if git::commit(root, last)? != parent {
        return Ok(vec![Finding::new(
            "not-the-last-candidate",
            last,
            format!(
                "{commit}'s parent is not the last candidate's commit, which a stable version is promoted from (ADR 0032)"
            ),
        )]);
    }
    let prepared: BTreeMap<String, String> = prepare(
        Tree::Commit {
            root,
            commit: &parent,
        },
        version,
    )?
    .into_iter()
    .collect();
    let results = format!("bench/results/{}/", version.semver());
    let mut findings = Vec::new();
    let mut pages = Vec::new();
    let changes = git::text(
        root,
        &["diff", "--name-status", "--no-renames", &parent, &commit],
    )?;
    for change in changes.lines() {
        let Some((status, path)) = change.split_once('\t') else {
            continue;
        };
        let allowed = match prepared.get(path) {
            Some(text) => {
                status == "M" && git::text(root, &["show", &format!("{commit}:{path}")])? == *text
            }
            None => match status {
                "A" | "M" if path == "CHANGELOG.md" => true,
                // The release runs `xtask codegen` after `prepare`, and the
                // pull request's `codegen --check` holds what it wrote.
                "M" => is_generated(path),
                "A" if is_site_page(path) => {
                    pages.push(path);
                    continue;
                }
                "A" => path.starts_with(&results),
                _ => false,
            },
        };
        if !allowed {
            findings.push(not_allowed(path));
        }
    }
    if pages.len() > 1 {
        findings.extend(pages.into_iter().map(not_allowed));
    }
    Ok(findings)
}

/// A path the stable version commit may not change as it does.
fn not_allowed(path: &str) -> Finding {
    Finding::new(
        "stable-commit-changes",
        path,
        "the stable version commit may only set versions and add the changelog, one release notes page and the version's benchmark results (ADR 0032)",
    )
}

/// Whether `path` is in a `generated/` folder, which only `xtask codegen`
/// writes.
fn is_generated(path: &str) -> bool {
    path.split('/')
        .rev()
        .skip(1)
        .any(|folder| folder == "generated")
}

/// Whether `path` is a page of the documentation site, which the release
/// notes page a stable version commit adds is (ADR 0032).
fn is_site_page(path: &str) -> bool {
    path.starts_with("website/")
        && Path::new(path)
            .extension()
            .is_some_and(|extension| extension == "md" || extension == "mdx")
}

#[derive(Deserialize)]
struct LiveCheckIssue {
    number: u64,
    created_at: Timestamp,
    pull_request: Option<IgnoredAny>,
}

#[derive(Deserialize)]
struct Comment {
    #[serde(default)]
    body: String,
}

/// Each open `live-check` issue whose cause no comment states, a comment
/// starting `Cause:`, two business days after it opened (ADR 0064).
fn live_checks(issues: &Issues<'_>) -> Result<Vec<Finding>> {
    let repository = issues.repository;
    let label = Label::LiveCheck.name();
    let open: Vec<LiveCheckIssue> = issues.api.get_all(
        &format!("/repos/{repository}/issues?labels={label}&state=open"),
        List::Array,
    )?;
    let mut findings = Vec::new();
    for issue in open.iter().filter(|issue| issue.pull_request.is_none()) {
        let deadline = two_business_days_after(issue.created_at)?;
        if issues.at <= deadline {
            continue;
        }
        let comments: Vec<Comment> = issues.api.get_all(
            &format!("/repos/{repository}/issues/{}/comments", issue.number),
            List::Array,
        )?;
        let stated = comments
            .iter()
            .any(|comment| comment.body.trim_start().starts_with("Cause:"));
        if !stated {
            findings.push(Finding::new(
                "live-check-unexplained",
                format!("{repository}#{}", issue.number),
                format!("no comment states its cause, starting `Cause:`, since {deadline}, two business days after it opened (ADR 0064)"),
            ));
        }
    }
    Ok(findings)
}

/// Two business days, Monday to Friday in UTC, after `opened`; an issue
/// opened at the weekend counts from the Monday's start.
fn two_business_days_after(opened: Timestamp) -> Result<Timestamp> {
    let mut day = opened.to_zoned(TimeZone::UTC);
    while is_weekend(day.weekday()) {
        day = day.tomorrow()?.start_of_day()?;
    }
    let mut counted = 0;
    while counted < 2 {
        day = day.checked_add(1.day())?;
        if !is_weekend(day.weekday()) {
            counted += 1;
        }
    }
    Ok(day.timestamp())
}

fn is_weekend(day: Weekday) -> bool {
    matches!(day, Weekday::Saturday | Weekday::Sunday)
}

#[derive(Deserialize)]
struct FuzzIssue {
    number: u64,
    title: String,
    pull_request: Option<IgnoredAny>,
}

/// Each open fuzz crash: an issue `ci/fuzz-nightly.sh` filed in the private
/// fuzzing repository, `Crash in <target> (<day>)`, open until fixed or
/// closed by triage (ADR 0067).
fn fuzz_crashes(issues: &Issues<'_>) -> Result<Vec<Finding>> {
    let repository = issues.fuzz_repository;
    let open: Vec<FuzzIssue> = issues.api.get_all(
        &format!("/repos/{repository}/issues?state=open"),
        List::Array,
    )?;
    Ok(open
        .into_iter()
        .filter(|issue| issue.pull_request.is_none() && issue.title.starts_with("Crash in "))
        .map(|issue| {
            Finding::new(
                "fuzz-crash-open",
                format!("{repository}#{}", issue.number),
                format!(
                    "{}: an open fuzz crash holds the next minor (ADR 0067)",
                    issue.title
                ),
            )
        })
        .collect())
}

/// Each format's version and schema in `compat/<format>/head/format.json`.
#[derive(Deserialize, PartialEq)]
struct Format {
    version: serde_json::Value,
    schema: serde_json::Value,
}

/// Each format whose version or schema at `commit` differs from the
/// `released` tag's, an added or removed format included: a patch changes
/// none (ADR 0066).
fn formats(root: &Path, commit: &str, released: &str) -> Result<Vec<Finding>> {
    if !tagged(root, released)? {
        bail!("the minor's release, {released}, is not tagged");
    }
    let before = formats_at(root, released)?;
    let after = formats_at(root, commit)?;
    let names: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    Ok(names
        .into_iter()
        .filter_map(|name| {
            let change = match (before.get(name), after.get(name)) {
                (Some(old), Some(new)) if old == new => return None,
                (Some(old), Some(new)) if old.version != new.version => {
                    format!("its version moved from {} to {}", old.version, new.version)
                }
                (Some(_), Some(_)) => "its schema changed".to_owned(),
                (None, _) => "it is new".to_owned(),
                (_, None) => "it is gone".to_owned(),
            };
            Some(Finding::new(
                "format-changed",
                name.clone(),
                format!("{change} since {released}; a patch changes no format (ADR 0066)"),
            ))
        })
        .collect())
}

fn formats_at(root: &Path, revision: &str) -> Result<BTreeMap<String, Format>> {
    let listed = git::text(
        root,
        &["ls-tree", "-r", "--name-only", revision, "--", "compat"],
    )?;
    let mut formats = BTreeMap::new();
    for path in listed.lines() {
        let Some(name) = path
            .strip_prefix("compat/")
            .and_then(|rest| rest.strip_suffix("/head/format.json"))
            .filter(|name| !name.contains('/'))
        else {
            continue;
        };
        let text = git::text(root, &["show", &format!("{revision}:{path}")])?;
        let format: Format =
            serde_json::from_str(&text).with_context(|| format!("parsing {path} at {revision}"))?;
        formats.insert(name.to_owned(), format);
    }
    Ok(formats)
}
