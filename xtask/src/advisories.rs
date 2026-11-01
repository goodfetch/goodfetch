//! The advisory snapshot the gate's audit reads (ADR 0064): RustSec's
//! database at one commit, and OSV's databases of npm and PyPI packages, each
//! a file with its SHA-256, all pinned in `ci/advisories.toml`. `fetch` keeps
//! them under `target/advisories/` before the outbound-traffic deny, and
//! `ci/audit.sh` checks the lockfiles against them offline, so the gate's
//! verdict moves only with a commit. The daily live audit reads the newest
//! data instead (`just audit-live`).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::path::Path;
use std::process::{Command as Process, ExitCode, Output};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::github;
use crate::pinned::{self, Pin};

/// The pins, from the repository root.
pub(crate) const PINS_FILE: &str = "ci/advisories.toml";

/// Where cargo-deny 0.20 looks for RustSec's database under its `db-path`,
/// which it names after its default URL's hash, and where cargo-audit's
/// `--db` points; from the repository root.
const RUSTSEC_CHECKOUT: &str = "target/advisories/rustsec/advisory-db-3157b0e258782691";

/// Where `fetch` keeps OSV's databases, each named `<ecosystem>.zip`.
const OSV_FOLDER: &str = "target/advisories/osv";

/// The largest database kept; npm's was 219 MB on 2026-10-22.
const MAXIMUM_DOWNLOAD: u64 = 1024 * 1024 * 1024;

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Checks out RustSec's pinned commit into
    /// `target/advisories/rustsec/`, unless it is checked out there
    /// unchanged, and downloads each OSV database `ci/advisories.toml` pins
    /// into `target/advisories/osv/`, unless the kept file already has its
    /// pinned SHA-256. Fails on a commit the repository lacks and on a
    /// download whose SHA-256 differs from its pin. A database pinned as a
    /// release asset is read through GitHub's API with `GITHUB_TOKEN`.
    Fetch {
        /// GitHub's API, or a stand-in's address.
        #[arg(long, default_value = github::API_URL)]
        api_url: String,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PinsFile {
    rustsec: RustSec,
    osv: Vec<Pin>,
}

/// RustSec's advisory database: a commit of its Git repository.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RustSec {
    repository: String,
    /// Lowercase hexadecimal, all 40 digits.
    commit: String,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    let root = &environment.root;
    let Command::Fetch { api_url } = command;
    let mut rustsec = None;
    let osv = pinned::read(root, PINS_FILE, |file: PinsFile| {
        rustsec = Some(file.rustsec);
        file.osv
    })?;
    let rustsec = rustsec.context("reading RustSec's pin")?;
    let commit_digits = rustsec.commit.len() == 40
        && rustsec
            .commit
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'));
    if !commit_digits {
        bail!("{PINS_FILE}: RustSec's commit is not 40 lowercase hexadecimal digits");
    }
    let mut findings = fetch_rustsec(&root.join(RUSTSEC_CHECKOUT), &rustsec)?;
    findings.extend(pinned::fetch(
        &root.join(OSV_FOLDER),
        &osv,
        PINS_FILE,
        MAXIMUM_DOWNLOAD,
        github::Api {
            url: api_url,
            token: environment.token(),
        },
    )?);
    Ok(check::report(&findings))
}

/// Checks out `pin`'s commit, alone, into `checkout`, unless it is checked
/// out there with no change. A commit the repository lacks, or a repository
/// it cannot reach, is a finding, and leaves no checkout.
fn fetch_rustsec(checkout: &Path, pin: &RustSec) -> Result<Vec<Finding>> {
    if checked_out_unchanged(checkout, &pin.commit)? {
        return Ok(Vec::new());
    }
    if checkout.exists() {
        std::fs::remove_dir_all(checkout)
            .with_context(|| format!("removing {}", checkout.display()))?;
    }
    std::fs::create_dir_all(checkout)
        .with_context(|| format!("creating {}", checkout.display()))?;
    succeeded(&git(checkout, &["init", "--quiet"])?, "git init")?;
    let fetched = git(
        checkout,
        &[
            "fetch",
            "--quiet",
            "--depth",
            "1",
            "--end-of-options",
            &pin.repository,
            &pin.commit,
        ],
    )?;
    if !fetched.status.success() {
        std::fs::remove_dir_all(checkout)
            .with_context(|| format!("removing {}", checkout.display()))?;
        return Ok(vec![Finding::new(
            "missing-commit",
            "rustsec",
            format!(
                "{} has no commit {}, or cannot be reached ({}): correct the pin in {PINS_FILE}",
                pin.repository,
                pin.commit,
                String::from_utf8_lossy(&fetched.stderr).trim()
            ),
        )]);
    }
    succeeded(
        &git(checkout, &["checkout", "--quiet", "--detach", "FETCH_HEAD"])?,
        "git checkout",
    )?;
    Ok(Vec::new())
}

/// Whether `checkout` is its own repository with `commit` checked out and no
/// file changed, added or removed. The `.git` check comes first: without
/// it, Git would answer for the repository around `target/`.
fn checked_out_unchanged(checkout: &Path, commit: &str) -> Result<bool> {
    if !checkout.join(".git").is_dir() {
        return Ok(false);
    }
    let head = git(checkout, &["rev-parse", "HEAD"])?;
    if !head.status.success() || String::from_utf8_lossy(&head.stdout).trim() != commit {
        return Ok(false);
    }
    let status = git(
        checkout,
        &["status", "--porcelain", "--untracked-files=all"],
    )?;
    Ok(status.status.success() && status.stdout.is_empty())
}

/// Runs `git <args>` in `folder`, with no user or system configuration, so
/// a host's settings never change what is checked out.
fn git(folder: &Path, args: &[&str]) -> Result<Output> {
    Process::new("git")
        .args([
            "-c",
            "init.defaultBranch=pinned",
            "-c",
            "advice.detachedHead=false",
        ])
        .args(args)
        .current_dir(folder)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .output()
        .context("running git")
}

fn succeeded(output: &Output, what: &str) -> Result<()> {
    if output.status.success() {
        Ok(())
    } else {
        bail!(
            "{what} failed: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )
    }
}
