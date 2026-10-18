//! The store's files: `runs/<year>/<month>/<run>-<attempt>.tar.gz`, by the
//! month the attempt started, each a gzipped tar of `run.json` (the
//! attempt), `test-runs.jsonl` (one [`TestRun`] a line), `jobs.jsonl` (one
//! [`JobRun`] a line) and `junit/<artifact>/…`, the JUnit files as written.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::io::Read as _;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use jiff::Timestamp;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// How one test run ended. A Live check's inconclusive result (its external
/// side was unavailable) is never a pass (ADR 0064).
#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Outcome {
    Passed,
    Failed,
    Skipped,
    Inconclusive,
}

/// One run of one test, in one job of one run attempt.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct TestRun {
    /// The test's ID as its runner names it (the quarantine list's IDs).
    pub(crate) test: String,
    /// `nextest`, `vitest`, `playwright`, `pytest`, or the tool that wrote
    /// the JUnit file of a check outside them.
    pub(crate) runner: String,
    pub(crate) suite: String,
    /// The CI tier: `pull-request`, `test-health`, `diagnostic`, `live`,
    /// `chromium-build` and the others of ADR 0032.
    pub(crate) tier: String,
    /// The workflow job's ID.
    pub(crate) job: String,
    pub(crate) os: String,
    pub(crate) arch: String,
    pub(crate) runtime: Option<String>,
    pub(crate) shard: String,
    pub(crate) commit: String,
    /// The commit's tree hash: the same tree is the same input.
    pub(crate) tree: String,
    /// The workflow's file, `.github/workflows/<name>.yml`.
    pub(crate) workflow: String,
    pub(crate) run: u64,
    pub(crate) attempt: u32,
    /// When the run attempt started.
    pub(crate) started_at: Timestamp,
    pub(crate) outcome: Outcome,
    pub(crate) duration_seconds: f64,
    pub(crate) order_seed: Option<String>,
    pub(crate) entropy_seed: Option<String>,
}

/// One job of a run attempt, for the minutes it took (ADRs 0064 and 0108).
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct JobRun {
    /// The job's name as GitHub shows it.
    pub(crate) job: String,
    /// The labels its `runs-on` asked for.
    pub(crate) labels: Vec<String>,
    pub(crate) runner_os: String,
    pub(crate) self_hosted: bool,
    pub(crate) started_at: Option<Timestamp>,
    pub(crate) completed_at: Option<Timestamp>,
    pub(crate) duration_seconds: u64,
    pub(crate) conclusion: Option<String>,
}

/// The run attempt a file holds.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct RunAttempt {
    pub(crate) repository: String,
    pub(crate) workflow: String,
    pub(crate) run: u64,
    pub(crate) attempt: u32,
    pub(crate) event: String,
    pub(crate) branch: Option<String>,
    pub(crate) commit: String,
    pub(crate) tree: String,
    pub(crate) started_at: Timestamp,
}

/// One file of the store.
pub(crate) struct Attempt {
    pub(crate) run: RunAttempt,
    pub(crate) test_runs: Vec<TestRun>,
    pub(crate) jobs: Vec<JobRun>,
}

/// One file the store keeps as written, at its path under `junit/`.
pub(crate) struct KeptFile {
    pub(crate) path: String,
    pub(crate) contents: Vec<u8>,
}

/// The store-relative path of an attempt's file.
pub(crate) fn path_of(run: &RunAttempt) -> String {
    format!(
        "runs/{}/{:02}/{}-{}.tar.gz",
        run.started_at.to_zoned(jiff::tz::TimeZone::UTC).year(),
        run.started_at.to_zoned(jiff::tz::TimeZone::UTC).month(),
        run.run,
        run.attempt
    )
}

/// Writes the attempt's file into the store at `root`, replacing one an
/// earlier ingest of the same attempt wrote, and returns its path.
pub(crate) fn write(root: &Path, attempt: &Attempt, kept: &[KeptFile]) -> Result<PathBuf> {
    let path = root.join(path_of(&attempt.run));
    let mut archive = tar::Builder::new(flate2::write::GzEncoder::new(
        Vec::new(),
        flate2::Compression::best(),
    ));
    append(&mut archive, "run.json", &serde_json::to_vec(&attempt.run)?)?;
    append(
        &mut archive,
        "test-runs.jsonl",
        &json_lines(&attempt.test_runs)?,
    )?;
    append(&mut archive, "jobs.jsonl", &json_lines(&attempt.jobs)?)?;
    for file in kept {
        append(
            &mut archive,
            &format!("junit/{}", file.path),
            &file.contents,
        )?;
    }
    let bytes = archive.into_inner()?.finish()?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(&path, bytes).with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

fn append(
    archive: &mut tar::Builder<impl std::io::Write>,
    path: &str,
    contents: &[u8],
) -> Result<()> {
    let mut header = tar::Header::new_gnu();
    header.set_size(u64::try_from(contents.len())?);
    header.set_mode(0o644);
    header.set_cksum();
    archive
        .append_data(&mut header, path, contents)
        .with_context(|| format!("adding {path}"))
}

fn json_lines<T: Serialize>(values: &[T]) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    for value in values {
        serde_json::to_writer(&mut bytes, value)?;
        bytes.push(b'\n');
    }
    Ok(bytes)
}

/// Every attempt of the store at `root`, in the order of their paths.
pub(crate) fn read_all(root: &Path) -> Result<Vec<Attempt>> {
    let mut files = Vec::new();
    collect(&root.join("runs"), &mut files)?;
    files.sort();
    files.iter().map(|file| read(file)).collect()
}

fn collect(folder: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    if !folder.is_dir() {
        return Ok(());
    }
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let path = entry?.path();
        if path.is_dir() {
            collect(&path, files)?;
        } else if path.to_string_lossy().ends_with(".tar.gz") {
            files.push(path);
        }
    }
    Ok(())
}

fn read(path: &Path) -> Result<Attempt> {
    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    let mut archive = tar::Archive::new(flate2::read::GzDecoder::new(file));
    let (mut run, mut test_runs, mut jobs) = (None, None, None);
    for entry in archive
        .entries()
        .with_context(|| format!("reading {}", path.display()))?
    {
        let mut entry = entry.with_context(|| format!("reading {}", path.display()))?;
        let name = entry.path()?.to_string_lossy().into_owned();
        let mut text = String::new();
        match name.as_str() {
            "run.json" => {
                entry.read_to_string(&mut text)?;
                run = Some(serde_json::from_str(&text).with_context(|| context(path, &name))?);
            }
            "test-runs.jsonl" => {
                entry.read_to_string(&mut text)?;
                test_runs = Some(lines(&text).with_context(|| context(path, &name))?);
            }
            "jobs.jsonl" => {
                entry.read_to_string(&mut text)?;
                jobs = Some(lines(&text).with_context(|| context(path, &name))?);
            }
            _ => {}
        }
    }
    let (Some(run), Some(test_runs), Some(jobs)) = (run, test_runs, jobs) else {
        bail!(
            "{} lacks run.json, test-runs.jsonl or jobs.jsonl",
            path.display()
        );
    };
    Ok(Attempt {
        run,
        test_runs,
        jobs,
    })
}

fn context(path: &Path, name: &str) -> String {
    format!("parsing {name} of {}", path.display())
}

fn lines<T: DeserializeOwned>(text: &str) -> Result<Vec<T>> {
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).map_err(anyhow::Error::from))
        .collect()
}
