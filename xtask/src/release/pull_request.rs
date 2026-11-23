//! A release's pull request (ADR 0032): the version commit, prepared on the
//! branch the release is built from, and the pull request that merges it
//! there.

use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{ProductVersion, REMOTE, Stage, Tree, changelog, now, prepare};
use crate::git;
use crate::github::{Api, Write};
use crate::output;

/// Where a release's pull request is prepared and opened.
pub(super) struct Releaser<'a> {
    pub(super) root: &'a Path,
    /// The Cargo that runs the workspace's own `xtask codegen`.
    pub(super) cargo: &'a OsStr,
    pub(super) repository: &'a str,
    pub(super) api: Api<'a>,
}

#[derive(Deserialize)]
struct Created {
    html_url: String,
}

impl Releaser<'_> {
    /// Opens `version`'s release pull request against the branch the
    /// workspace is on, which must be the minor's `release/x.y` as the
    /// remote holds it: on a new `release-pr/<version>` branch, one commit,
    /// signed off by the committer, that sets the version in the released
    /// manifests, holds what `xtask codegen` writes from them, both SDKs'
    /// `PRODUCT_VERSION` among it, and adds the version's changelog
    /// entries. Nothing is written when the workspace has changes or the
    /// pull request's branch exists, here or on the remote. A step that
    /// fails before the branch is pushed leaves the workspace as it was;
    /// once the branch is pushed, the workspace is left on it.
    pub(super) fn open(&self, version: &ProductVersion) -> Result<()> {
        let root = self.root;
        if let Stage::Nightly(_) = version.stage {
            bail!(
                "a nightly, {}, is published from main and has no release pull request",
                version.semver()
            );
        }
        let base = version.minor().branch();
        let on = git::line(root, &["branch", "--show-current"])?;
        if on != base {
            bail!(
                "{} is released from {base}, but the workspace is on {}",
                version.semver(),
                if on.is_empty() { "no branch" } else { &on }
            );
        }
        require_clean(root)?;
        let head = version.pull_request_branch();
        let reference = format!("refs/heads/{head}");
        if git::succeeds(root, &["rev-parse", "--verify", "--quiet", &reference])? {
            bail!("{head} exists already");
        }
        if let Some(found) = git::text(root, &["ls-remote", REMOTE, &reference])?
            .lines()
            .next()
        {
            bail!("{REMOTE} holds {found} already");
        }
        let parent = git::commit(root, "HEAD")?;
        git::text(
            root,
            &["fetch", "--quiet", REMOTE, &format!("refs/heads/{base}")],
        )?;
        let remote = git::commit(root, "FETCH_HEAD")?;
        if remote != parent {
            bail!(
                "{base} is at {parent} here but at {remote} on {REMOTE}; release from the remote's"
            );
        }

        let title = format!("chore(release): {}", version.semver());
        let had_changelog = root.join(changelog::CHANGELOG).exists();
        git::text(root, &["switch", "--quiet", "--create", &head])?;
        if let Err(error) = self.commit_and_push(version, &title, &reference) {
            let rolled_back = roll_back(root, &base, &head, &parent, had_changelog);
            let error = match rolled_back {
                Ok(()) => error.context(format!(
                    "nothing is pushed, and the workspace is back on {base}; once fixed, run: xtask release pull-request --version {}",
                    version.semver()
                )),
                Err(failed) => error.context(format!(
                    "nothing is pushed, but the workspace stays on {head}: {failed:#}"
                )),
            };
            return Err(error);
        }

        let body = serde_json::json!({
            "title": title,
            "head": head,
            "base": base,
            "body": format!(
                "The release of {}, prepared on `{base}` at {parent} (ADR 0032).",
                version.semver()
            ),
        });
        let created: Created = self
            .api
            .send(Write::Post, &format!("/repos/{}/pulls", self.repository), &body)
            .with_context(|| {
                format!(
                    "{head} is pushed; open its pull request by hand: gh pr create --base {base} --head {head} --title '{title}'"
                )
            })?;
        output::line(&format!("{head}: {}", created.html_url));
        Ok(())
    }

    /// Writes the release's files on the pull request's branch, commits
    /// them as `title` and pushes the commit to `reference`.
    fn commit_and_push(
        &self,
        version: &ProductVersion,
        title: &str,
        reference: &str,
    ) -> Result<()> {
        let root = self.root;
        for (path, text) in prepare(Tree::Workspace(root), version)? {
            std::fs::write(root.join(&path), text).with_context(|| format!("writing {path}"))?;
        }
        self.codegen()?;
        changelog::write(root, version, "HEAD", now())?;
        git::text(root, &["add", "--update"])?;
        git::text(root, &["add", "--", changelog::CHANGELOG])?;
        git::text(
            root,
            &["commit", "--quiet", "--signoff", "--message", title],
        )?;
        git::text(
            root,
            &["push", "--quiet", REMOTE, &format!("HEAD:{reference}")],
        )?;
        Ok(())
    }

    /// Runs the workspace's `xtask codegen`, which reads the Product version
    /// from the manifests `prepare` wrote.
    fn codegen(&self) -> Result<()> {
        let output = crate::child_cargo::command(self.cargo)
            .args(["run", "--quiet", "--bin", "xtask", "--", "codegen"])
            .current_dir(self.root)
            .output()
            .context("running xtask codegen")?;
        if !output.status.success() {
            bail!(
                "xtask codegen failed after the preparation: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        Ok(())
    }
}

/// Puts the workspace back as it was before `head` was made at `parent`:
/// its tracked files, the changelog when there was none, and `base`
/// checked out.
fn roll_back(root: &Path, base: &str, head: &str, parent: &str, had_changelog: bool) -> Result<()> {
    git::text(root, &["reset", "--quiet", "--hard", parent])?;
    let changelog = root.join(changelog::CHANGELOG);
    if !had_changelog && changelog.exists() {
        std::fs::remove_file(&changelog)
            .with_context(|| format!("removing {}", changelog::CHANGELOG))?;
    }
    git::text(root, &["switch", "--quiet", base])?;
    git::text(root, &["branch", "--quiet", "--delete", "--force", head])?;
    Ok(())
}

/// Refuses a workspace whose tracked files have changes, which a release's
/// commit would take.
pub(super) fn require_clean(root: &Path) -> Result<()> {
    let changes = git::text(root, &["status", "--porcelain", "--untracked-files=no"])?;
    if !changes.trim().is_empty() {
        bail!("the workspace has changes; commit them or set them aside first:\n{changes}");
    }
    Ok(())
}
