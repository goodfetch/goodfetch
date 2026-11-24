//! The release line's branches (ADR 0032): a minor's cut, the backports its
//! branch lacks and each release's git-cliff range.

use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;

use anyhow::{Result, bail};
use serde::Deserialize;

use super::{CUT_POINT, Minor, ProductVersion, REMOTE, Stage, tagged};
use crate::git;
use crate::github::Api;
use crate::output;

/// The label of a pull request whose fix a release branch takes too.
const BACKPORT: &str = "backport";

/// The branch a minor is cut from.
const MAIN: &str = "main";

/// The git-cliff range of `version`'s changelog, ending at `to`: `from..to`,
/// or `to` alone for the whole history.
pub(super) fn range(root: &Path, version: &ProductVersion, to: &str) -> Result<String> {
    git::commit(root, to)?;
    let previous = match version.stage {
        Stage::Nightly(_) => bail!(
            "a nightly, {}, has no changelog range: it is published from main",
            version.semver()
        ),
        Stage::Candidate(number) => match NonZeroU64::new(number.get() - 1) {
            Some(previous) => Some(released(
                root,
                &ProductVersion {
                    stage: Stage::Candidate(previous),
                    ..version.clone()
                },
            )?),
            None => previous_cut_point(root, version.minor())?,
        },
        Stage::Stable => match version.patch.checked_sub(1) {
            Some(previous) => Some(released(
                root,
                &ProductVersion {
                    patch: previous,
                    ..version.clone()
                },
            )?),
            None => previous_cut_point(root, version.minor())?,
        },
    };
    Ok(match previous {
        Some(from) => format!("{from}..{to}"),
        None => to.to_owned(),
    })
}

/// `version`'s tag, which must exist.
fn released(root: &Path, version: &ProductVersion) -> Result<String> {
    let tag = version.tag();
    if !tagged(root, &tag)? {
        bail!("the previous release on the branch, {tag}, is not tagged");
    }
    Ok(tag)
}

/// The cut point of the latest minor before `minor`, none before the first
/// cut.
fn previous_cut_point(root: &Path, minor: Minor) -> Result<Option<String>> {
    let tags = git::text(root, &["tag", "--list", &format!("*{CUT_POINT}")])?;
    Ok(tags
        .lines()
        .filter_map(|tag| tag.strip_suffix(CUT_POINT)?.parse::<Minor>().ok())
        .filter(|cut| *cut < minor)
        .max()
        .map(Minor::cut_point))
}

/// GitHub's pull request of a commit, as `/commits/{sha}/pulls` lists it.
#[derive(Deserialize)]
struct Pull {
    number: u64,
    labels: Vec<Label>,
}

#[derive(Deserialize)]
struct Label {
    name: String,
}

/// Each commit on `main` since `minor` was cut whose pull request carries
/// the backport label and that the minor's branch lacks, oldest first, as
/// `<sha> #<pull request> <subject>`. The branch has a commit when it holds
/// the same change (`git cherry`) or a `cherry picked from` line naming it.
pub(super) fn backports(
    root: &Path,
    minor: Minor,
    repository: &str,
    api: Api<'_>,
) -> Result<Vec<String>> {
    let cut_point = minor.cut_point();
    if !tagged(root, &cut_point)? {
        bail!("minor {minor} was never cut: no tag {cut_point}");
    }
    let branch = minor.branch();
    let picked: BTreeSet<String> = git::text(
        root,
        &["log", "--format=%B", &format!("{cut_point}..{branch}")],
    )?
    .lines()
    .filter_map(|line| {
        line.trim()
            .strip_prefix("(cherry picked from commit ")?
            .strip_suffix(')')
    })
    .map(str::to_owned)
    .collect();
    let mut listed = Vec::new();
    for line in git::text(root, &["cherry", &branch, MAIN, &cut_point])?.lines() {
        let Some(commit) = line.strip_prefix("+ ") else {
            continue;
        };
        if picked.contains(commit) {
            continue;
        }
        let pulls: Vec<Pull> = api.get(&format!("/repos/{repository}/commits/{commit}/pulls"))?;
        let labelled = pulls
            .iter()
            .find(|pull| pull.labels.iter().any(|label| label.name == BACKPORT));
        if let Some(pull) = labelled {
            let subject = git::line(root, &["log", "-1", "--format=%s", commit])?;
            listed.push(format!("{commit} #{} {subject}", pull.number));
        }
    }
    Ok(listed)
}

/// Cuts `minor`: pushes its branch and cut point at the remote's `main`,
/// and switches the workspace to the branch, where its first candidate,
/// which it returns, is prepared. Nothing is pushed when the branch, the
/// tag or the candidate's pull request branch exists, here or on the
/// remote.
pub(super) fn cut(root: &Path, minor: Minor) -> Result<ProductVersion> {
    let candidate = ProductVersion {
        major: minor.major,
        minor: minor.minor,
        patch: 0,
        stage: Stage::Candidate(NonZeroU64::MIN),
    };
    let branch = minor.branch();
    let tag = minor.cut_point();
    let head = candidate.pull_request_branch();
    let references = [
        format!("refs/heads/{branch}"),
        format!("refs/tags/{tag}"),
        format!("refs/heads/{head}"),
    ];
    for reference in &references {
        if git::succeeds(root, &["rev-parse", "--verify", "--quiet", reference])? {
            bail!("{reference} exists already: minor {minor} is cut");
        }
    }
    let mut listing = vec!["ls-remote", REMOTE];
    listing.extend(references.iter().map(String::as_str));
    if let Some(found) = git::text(root, &listing)?.lines().next() {
        bail!("{REMOTE} holds {found} already: minor {minor} is cut");
    }
    git::text(
        root,
        &["fetch", "--quiet", REMOTE, &format!("refs/heads/{MAIN}")],
    )?;
    let main = git::commit(root, "FETCH_HEAD")?;

    git::text(
        root,
        &[
            "push",
            "--atomic",
            "--quiet",
            REMOTE,
            &format!("{main}:{}", references[0]),
            &format!("{main}:{}", references[1]),
        ],
    )?;
    git::text(root, &["branch", &branch, &main])?;
    git::text(root, &["tag", &tag, &main])?;
    git::text(root, &["switch", "--quiet", &branch])?;
    output::line(&format!("{branch} and {tag} at {main}"));
    Ok(candidate)
}
