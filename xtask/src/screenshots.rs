//! Reference screenshots (ADR 0073): `fetch` applies the screenshots of one
//! `update-screenshots` run, downloaded by `just screenshots-fetch <run>`,
//! only when the run rendered them in the pinned Playwright image on the
//! branch's head; `orphans` refuses a reference no visual test read; `check`
//! (Quick) refuses a comparison tolerance set outside the shared visual
//! config and a Reference screenshot over 512 KiB.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and blocking-process bans target async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::check::Finding;
use crate::environment::Environment;
use crate::output;
use crate::tree;

mod tolerance;

/// The pinned Playwright image, read by the workflows and by this module.
const PINNED_IMAGE: &str = "ci/visual/image.toml";

/// What a downloaded run says about itself, written by the workflow.
const RUN_PROVENANCE: &str = "provenance.json";

/// Where a downloaded run keeps its screenshots, by repository path.
const RUN_SCREENSHOTS: &str = "screenshots";

/// Where the visual projects record each Reference screenshot they read, one
/// repository path a line, in files of their own (the shared helper for
/// Playwright, `vitest.config.ts` for Vitest).
const READ_RECORDS: &str = "target/visual/read";

/// The shared visual config of Vitest and Playwright, the one file that may
/// set a comparison's tolerance (ADR 0073).
const SHARED_CONFIG: &str = "test-support/typescript/src/visual-config.ts";

/// ADR 0073's limit for one Reference screenshot: 512 KiB.
const REFERENCE_LIMIT_BYTES: u64 = 512 * 1024;

/// An image's content digest, `sha256:` and 64 lowercase hexadecimal digits.
#[derive(Debug, PartialEq, Eq)]
struct Digest(String);

impl Digest {
    fn parse(text: &str) -> Result<Self> {
        let hex = text
            .strip_prefix("sha256:")
            .context("an image digest starts with `sha256:`")?;
        if hex.len() != 64 || !is_lower_hex(hex) {
            bail!("`{text}` is not a sha256 digest");
        }
        Ok(Self(text.to_owned()))
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A commit's full hexadecimal name: 40 digits for SHA-1, 64 for SHA-256.
#[derive(Debug, PartialEq, Eq)]
struct CommitId(String);

impl CommitId {
    fn parse(text: &str) -> Result<Self> {
        let text = text.trim();
        if !matches!(text.len(), 40 | 64) || !is_lower_hex(text) {
            bail!("`{text}` is not a full commit name");
        }
        Ok(Self(text.to_owned()))
    }
}

impl fmt::Display for CommitId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

fn is_lower_hex(text: &str) -> bool {
    text.bytes()
        .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinnedImage {
    #[expect(
        dead_code,
        reason = "read by the workflows' scripts; parsed here so the file keeps its shape"
    )]
    repository: String,
    #[expect(
        dead_code,
        reason = "read by the workflows' scripts; parsed here so the file keeps its shape"
    )]
    tag: String,
    digest: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Provenance {
    digest: String,
    commit: String,
    /// The workflow run's ID, which the author's commit names.
    run: u64,
}

fn pinned_digest(root: &Path) -> Result<Digest> {
    let path = root.join(PINNED_IMAGE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let image: PinnedImage =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Digest::parse(&image.digest).with_context(|| format!("reading {}", path.display()))
}

/// The commit checked out in the workspace at `root`.
fn head(root: &Path) -> Result<CommitId> {
    CommitId::parse(&String::from_utf8_lossy(&crate::git::bytes(
        root,
        &["rev-parse", "HEAD"],
    )?))
}

/// Applies every screenshot of the downloaded run at `run` to the workspace,
/// or nothing when the run was not made in the pinned image on the head.
pub(crate) fn fetch(root: &Path, run: &Path) -> Result<Vec<Finding>> {
    let pinned = pinned_digest(root)?;
    let head = head(root)?;
    let path = run.join(RUN_PROVENANCE);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let provenance: Provenance =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let digest =
        Digest::parse(&provenance.digest).with_context(|| format!("reading {}", path.display()))?;
    let commit = CommitId::parse(&provenance.commit)
        .with_context(|| format!("reading {}", path.display()))?;

    let mut findings = Vec::new();
    if digest != pinned {
        findings.push(Finding::new(
            "image-not-pinned",
            digest.to_string(),
            format!(
                "the run rendered its screenshots in another image than {PINNED_IMAGE}'s {pinned}"
            ),
        ));
    }
    if commit != head {
        findings.push(Finding::new(
            "commit-not-head",
            commit.to_string(),
            format!("the run rendered another commit than the branch's head, {head}; dispatch the workflow again on the pushed head"),
        ));
    }

    // A run that changed no screenshot uploads no folder of them.
    let screenshots = run.join(RUN_SCREENSHOTS);
    let mut files = Vec::new();
    if screenshots.is_dir() {
        collect(&screenshots, &mut files)?;
    }
    let mut relatives = Vec::new();
    for file in &files {
        let relative = file.strip_prefix(&screenshots)?;
        if !is_reference_screenshot(relative) {
            findings.push(Finding::new(
                "not-a-reference-screenshot",
                slash_path(relative),
                "a run may write only PNG files in `__screenshots__/` and `-snapshots/` folders",
            ));
        }
        relatives.push(relative);
    }
    if !findings.is_empty() {
        return Ok(findings);
    }

    for (file, relative) in files.iter().zip(relatives) {
        let target = root.join(relative);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        std::fs::copy(file, &target).with_context(|| format!("writing {}", target.display()))?;
        output::line(&format!("applied {}", slash_path(relative)));
    }
    if files.is_empty() {
        output::line("The run changed no Reference screenshot.");
        return Ok(findings);
    }
    output::line(&format!(
        "Commit them with your own sign-off (`git commit -s`), naming update-screenshots run {} in the message.",
        provenance.run
    ));
    Ok(findings)
}

/// Every Reference screenshot of the workspace that no visual test read in
/// the run recorded under `target/visual/read/` (ADR 0073's orphan check).
pub(crate) fn orphans(root: &Path) -> Result<Vec<Finding>> {
    let records = root.join(READ_RECORDS);
    let mut read = BTreeSet::new();
    for entry in std::fs::read_dir(&records).with_context(|| {
        format!(
            "reading {}; run every visual project first",
            records.display()
        )
    })? {
        let path = entry?.path();
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        read.extend(
            text.lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .map(str::to_owned),
        );
    }
    let mut references = Vec::new();
    walk(root, root, &mut references)?;
    Ok(references
        .into_iter()
        .filter(|reference| !read.contains(reference))
        .map(|reference| {
            Finding::new(
                "orphaned-reference-screenshot",
                reference,
                "no visual test read this Reference screenshot; remove it, or take the screenshot through the shared helper",
            )
        })
        .collect())
}

/// The xtask tests' fixtures, this check's own cases among them, which set
/// tolerances on purpose.
const OWN_CASES: &str = "xtask/tests/it/fixtures";

/// Every comparison tolerance set outside the shared visual config, in the
/// files the suites run (fixtures and frozen `rnd/` aside), and every
/// Reference screenshot over 512 KiB.
pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    // A fixture module a visual test imports can set a tolerance too, so
    // fixtures folders are read, this check's own cases aside.
    for file in tree::files_with_fixtures(root, "", &[OWN_CASES])? {
        let code = Path::new(&file)
            .extension()
            .and_then(|extension| extension.to_str())
            .is_some_and(|extension| tolerance::EXTENSIONS.contains(&extension));
        if !code || file == SHARED_CONFIG {
            continue;
        }
        let path = root.join(&file);
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let visual_test = Path::new(&file)
            .file_name()
            .is_some_and(|name| name.to_string_lossy().contains(".visual."));
        for (line, setting) in tolerance::settings(&text, visual_test) {
            findings.push(Finding::new(
                "tolerance-outside-shared-config",
                format!("{file}:{line}"),
                format!("`{setting}` is set outside {SHARED_CONFIG}; a comparison takes zero mismatched pixels at the default threshold, and the shared config alone sets that"),
            ));
        }
    }
    let mut references = Vec::new();
    walk(root, root, &mut references)?;
    for reference in references {
        let path = root.join(&reference);
        let bytes = std::fs::metadata(&path)
            .with_context(|| format!("reading {}", path.display()))?
            .len();
        if bytes > REFERENCE_LIMIT_BYTES {
            findings.push(Finding::new(
                "oversized-reference-screenshot",
                reference,
                format!("{bytes} bytes is over the 512 KiB limit; clip the screenshot to the region the test is about"),
            ));
        }
    }
    Ok(findings)
}

/// Collects every Reference screenshot under `folder`, as repository paths.
fn walk(root: &Path, folder: &Path, references: &mut Vec<String>) -> Result<()> {
    let mut entries = std::fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<PathBuf>, _>>()?;
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or_default();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let relative = path.strip_prefix(root)?;
        if metadata.is_dir() {
            if tree::hidden(name) || tree::never_entered(&slash_path(relative)) {
                continue;
            }
            walk(root, &path, references)?;
        } else if is_reference_screenshot(relative) {
            references.push(slash_path(relative));
        }
    }
    Ok(())
}

/// Whether the repository path is a Reference screenshot: a PNG file inside
/// Vitest's `__screenshots__/` folder or a Playwright `<test file>-snapshots/`
/// folder (ADR 0073).
pub(crate) fn is_reference_screenshot(relative: &Path) -> bool {
    let in_reference_folder = relative.parent().is_some_and(|parent| {
        parent.components().any(|component| match component {
            Component::Normal(name) => name
                .to_str()
                .is_some_and(|name| name == "__screenshots__" || name.ends_with("-snapshots")),
            _ => false,
        })
    });
    let plain = relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)));
    plain
        && in_reference_folder
        && relative
            .extension()
            .is_some_and(|extension| extension == "png")
}

fn slash_path(relative: &Path) -> String {
    relative.to_string_lossy().replace('\\', "/")
}

fn collect(folder: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else {
            files.push(path);
        }
    }
    Ok(())
}

/// `xtask screenshots`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Applies the screenshots of a downloaded `update-screenshots` run, or
    /// fails when it was not made in the pinned image on the head.
    Fetch {
        /// The folder the run's artifact was downloaded into.
        run: PathBuf,
    },
    /// Fails on a Reference screenshot that no visual test read, after a
    /// run of every visual project.
    Orphans,
    /// Fails on a comparison tolerance set outside the shared visual config
    /// and on a Reference screenshot over 512 KiB (Quick suite).
    Check,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = environment.root.as_path();
    Ok(crate::check::report(&match command {
        Command::Fetch { run } => fetch(root, &environment.path(&run))?,
        Command::Orphans => orphans(root)?,
        Command::Check => check(root)?,
    }))
}
