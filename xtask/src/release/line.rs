//! The release line's branches (ADR 0032): a minor's cut, the backports its
//! branch lacks and each release's git-cliff range.

use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{CUT_POINT, Minor, ProductVersion, Stage, Tree, prepare, tagged};
use crate::git;
use crate::github::{Api, Write};
use crate::output;

/// The label of a pull request whose fix a release branch takes too.
const BACKPORT: &str = "backport";

/// The branch a minor is cut from.
const MAIN: &str = "main";

/// The remote a minor is cut from and pushed to.
const REMOTE: &str = "origin";

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

#[derive(Deserialize)]
struct Created {
    html_url: String,
}

/// Cuts `minor`: pushes its branch and cut point at the remote's `main` and
/// the first candidate's version commit over it, then opens that
/// candidate's release pull request against the branch. Nothing is pushed
/// when the branch, the tag or the pull request's branch exists, here or on
/// the remote.
pub(super) fn cut(root: &Path, minor: Minor, repository: &str, api: Api<'_>) -> Result<()> {
    let candidate = ProductVersion {
        major: minor.major,
        minor: minor.minor,
        patch: 0,
        stage: Stage::Candidate(NonZeroU64::MIN),
    };
    let branch = minor.branch();
    let tag = minor.cut_point();
    let head = format!("release-pr/{}", candidate.semver());
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

    let title = format!("chore(release): {}", candidate.semver());
    let version_commit = version_commit(root, &main, &candidate, &title)?;
    git::text(
        root,
        &[
            "push",
            "--atomic",
            "--quiet",
            REMOTE,
            &format!("{main}:{}", references[0]),
            &format!("{main}:{}", references[1]),
            &format!("{version_commit}:{}", references[2]),
        ],
    )?;
    git::text(root, &["branch", &branch, &main])?;
    git::text(root, &["tag", &tag, &main])?;
    output::line(&format!("{branch} and {tag} at {main}"));

    let body = serde_json::json!({
        "title": title,
        "head": head,
        "base": branch,
        "body": format!(
            "The first release candidate of {minor}, cut from `main` at {main} (ADR 0032)."
        ),
    });
    let created: Created = api
        .send(Write::Post, &format!("/repos/{repository}/pulls"), &body)
        .with_context(|| {
            format!(
                "{branch}, {tag} and {head} are pushed, so the minor is cut; open its release pull request by hand: gh pr create --base {branch} --head {head} --title '{title}'"
            )
        })?;
    output::line(&format!("{head}: {}", created.html_url));
    Ok(())
}

/// A commit over `parent` with `version` prepared in its released
/// manifests, signed off by the committer, written without touching the
/// workspace's files or index.
fn version_commit(
    root: &Path,
    parent: &str,
    version: &ProductVersion,
    title: &str,
) -> Result<String> {
    let prepared = prepare(
        Tree::Commit {
            root,
            commit: parent,
        },
        version,
    )?;
    let folder = tempfile::tempdir().context("making a folder for the version commit's index")?;
    let index = folder.path().join("index");
    let indexed = |arguments: &[&str]| -> Result<String> {
        let output = git::command(root)
            .env("GIT_INDEX_FILE", &index)
            .args(arguments)
            .output()
            .with_context(|| format!("running git {}", arguments.join(" ")))?;
        if !output.status.success() {
            bail!(
                "git {} failed: {}",
                arguments.join(" "),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    indexed(&["read-tree", parent])?;
    for (path, text) in prepared {
        let file = folder.path().join("blob");
        std::fs::write(&file, text).with_context(|| format!("writing {path}'s new text"))?;
        let file = file
            .to_str()
            .context("the temporary folder's path is no UTF-8")?;
        let blob = git::line(root, &["hash-object", "-w", file])?;
        let entry = git::line(root, &["ls-tree", parent, "--", &path])?;
        let mode = entry
            .split_whitespace()
            .next()
            .with_context(|| format!("{path} is not in {parent}'s tree"))?;
        indexed(&[
            "update-index",
            "--cacheinfo",
            &format!("{mode},{blob},{path}"),
        ])?;
    }
    let tree = indexed(&["write-tree"])?;
    let ident = git::line(root, &["var", "GIT_COMMITTER_IDENT"])?;
    let signer = ident
        .rsplitn(3, ' ')
        .nth(2)
        .with_context(|| format!("the committer's identity, {ident}, has no time"))?;
    let message = format!("{title}\n\nSigned-off-by: {signer}\n");
    Ok(git::line(
        root,
        &["commit-tree", &tree, "-p", parent, "-m", &message],
    )?)
}
