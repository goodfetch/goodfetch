//! A release's entries in `CHANGELOG.md` (spec #130): git-cliff reads the
//! Conventional Commits of the release's range as `cliff.toml` configures,
//! each commit is marked as reaching both Products or Foxhound only by the
//! paths it changes (ADR 0094), and the entries git-cliff writes from them
//! go above the earlier releases', in place of the section they replace
//! (ADR 0032).

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{ProductVersion, Stage, line};
use crate::foxhound_side;
use crate::git;

/// The changelog, at the repository's root.
pub(super) const CHANGELOG: &str = "CHANGELOG.md";

/// The changelog's first line, above every release's entries.
const HEADER: &str = "# Changelog\n";

/// What starts a release's section, before its version.
const SECTION: &str = "## ";

/// git-cliff's configuration, at the repository's root.
const CLIFF_TOML: &str = "cliff.toml";

/// The installation the command names when git-cliff is missing: the CI
/// image's version.
const INSTALL: &str = "cargo install --locked git-cliff@2.14.2";

/// A release of git-cliff's context: the fields the command reads or sets,
/// and every other as git-cliff wrote it.
#[derive(Deserialize, Serialize)]
struct Release {
    commits: Vec<Commit>,
    /// Its date, in seconds since the epoch.
    timestamp: Option<i64>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

/// A commit of git-cliff's context.
#[derive(Deserialize, Serialize)]
struct Commit {
    id: String,
    #[serde(rename = "merge_commit")]
    merge: bool,
    /// What `cliff.toml`'s template reads beside git-cliff's own fields:
    /// `reach`.
    extra: Option<Map<String, Value>>,
    #[serde(flatten)]
    rest: Map<String, Value>,
}

/// Which Products a commit's change reaches (ADR 0094), as `cliff.toml`'s
/// template reads it from the commit's `extra.reach`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Reach {
    Both,
    /// Every path the commit changes is on Foxhound's side.
    Foxhound,
}

impl Reach {
    fn name(self) -> &'static str {
        match self {
            Self::Both => "both",
            Self::Foxhound => "foxhound",
        }
    }
}

/// Writes `version`'s entries, from the commits of its range up to `to`,
/// dated `at`, above the earlier releases' in the changelog of the
/// repository at `root`, which it makes when there is none. They replace
/// the version's own section, written before, and for a stable minor its
/// candidates', since its range holds theirs (ADR 0032).
pub(super) fn write(
    root: &Path,
    version: &ProductVersion,
    to: &str,
    at: jiff::Timestamp,
) -> Result<()> {
    let range = line::range(root, version, to)?;
    let tag = version.tag();
    // Every tag inside the range is this release's: a candidate's tag
    // inside a stable minor's range, say. Without `--ignore-tags`, git-cliff
    // would split the range at it.
    let context = cliff(
        root,
        &["--tag", &tag, "--ignore-tags", ".*", "--context", &range],
        None,
    )?;
    let mut releases: Vec<Release> =
        serde_json::from_slice(&context).context("reading git-cliff's context")?;
    for release in &mut releases {
        release.timestamp = Some(at.as_second());
        // A merge's subject repeats a commit of the branch it merges, which
        // is listed itself.
        release.commits.retain(|commit| !commit.merge);
        for commit in &mut release.commits {
            let reach = reach(root, &commit.id)?;
            commit
                .extra
                .get_or_insert_default()
                .insert("reach".to_owned(), reach.name().into());
        }
    }
    let entries = cliff(
        root,
        &["--from-context", "-"],
        Some(&serde_json::to_vec(&releases)?),
    )?;
    let entries = String::from_utf8(entries).context("git-cliff's entries are no UTF-8 text")?;

    let path = root.join(CHANGELOG);
    let earlier = match std::fs::read_to_string(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => HEADER.to_owned(),
        Err(error) => return Err(error).with_context(|| format!("reading {CHANGELOG}")),
    };
    let Some(earlier) = earlier.strip_prefix(HEADER) else {
        bail!(
            "{CHANGELOG} does not start with its header, `{}`",
            HEADER.trim_end()
        );
    };
    let mut written = format!("{HEADER}\n{}\n", entries.trim());
    let earlier = kept(earlier, version);
    let earlier = earlier.trim_start_matches('\n');
    if !earlier.is_empty() {
        written.push('\n');
        written.push_str(earlier);
    }
    std::fs::write(&path, written).with_context(|| format!("writing {CHANGELOG}"))
}

/// `earlier`'s sections but those `version`'s entries replace.
fn kept(earlier: &str, version: &ProductVersion) -> String {
    let mut kept = String::new();
    let mut keeping = true;
    for text in earlier.split_inclusive('\n') {
        if let Some(heading) = text.strip_prefix(SECTION) {
            keeping = !heading
                .split_whitespace()
                .next()
                .and_then(|written| written.parse::<ProductVersion>().ok())
                .is_some_and(|written| replaces(version, &written));
        }
        if keeping {
            kept.push_str(text);
        }
    }
    kept
}

/// Whether `version`'s section replaces `written`'s: it is the same
/// version's, or `version` is the stable minor `written` is a candidate of.
fn replaces(version: &ProductVersion, written: &ProductVersion) -> bool {
    let candidate = matches!(written.stage, Stage::Candidate(_))
        && version.stage == Stage::Stable
        && (written.major, written.minor, written.patch)
            == (version.major, version.minor, version.patch);
    written == version || candidate
}

/// The reach of commit `id`, from the paths it changes.
fn reach(root: &Path, id: &str) -> Result<Reach> {
    let paths = git::bytes(
        root,
        &[
            "diff-tree",
            "-r",
            "-z",
            "--name-only",
            "--no-commit-id",
            "--root",
            id,
        ],
    )?;
    let paths = String::from_utf8_lossy(&paths);
    Ok(
        if foxhound_side::confines(paths.split('\0').filter(|path| !path.is_empty())) {
            Reach::Foxhound
        } else {
            Reach::Both
        },
    )
}

/// What git-cliff prints when run in `root` with `cliff.toml` and
/// `arguments`, `input` on its standard input.
fn cliff(root: &Path, arguments: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
    let mut child = match Command::new("git-cliff")
        .current_dir(root)
        .arg("--config")
        .arg(root.join(CLIFF_TOML))
        .args(arguments)
        .stdin(if input.is_some() {
            Stdio::piped()
        } else {
            Stdio::null()
        })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
    {
        Ok(child) => child,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            bail!("git-cliff is not installed: `{INSTALL}`")
        }
        Err(error) => return Err(error).context("running git-cliff"),
    };
    if let Some(input) = input {
        // git-cliff reads its whole input before it writes anything.
        child
            .stdin
            .take()
            .context("git-cliff's standard input")?
            .write_all(input)
            .context("writing git-cliff's context")?;
    }
    let output = child.wait_with_output().context("running git-cliff")?;
    if !output.status.success() {
        bail!(
            "git-cliff {} failed: {}",
            arguments.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        );
    }
    Ok(output.stdout)
}
