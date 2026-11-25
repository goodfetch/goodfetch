//! The Live checks of `ci/live-checks.toml` (ADR 0064; TESTING.md, Live
//! checks): each check's ID, its owning area and, for a check a shell job
//! runs, that job and the command it runs. `xtask results live-run` runs a
//! job's commands and writes one JUnit test case per check, named by its
//! ID, which the ingest keeps under the `live` tier: exit 0 passes, exit 75
//! (the outside side was unavailable) is skipped, which the store keeps as
//! inconclusive, and any other exit fails. The scan reads the areas.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::num::NonZeroU8;
use std::path::Path;
use std::process::Command;
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use quick_xml::escape::escape;
use serde::Deserialize;

use crate::output;

/// The Live checks and their owning areas, numbered as ADR 0037 numbers them.
pub(crate) const LIVE_CHECKS: &str = "ci/live-checks.toml";

/// The exit of a check whose outside side was unavailable: `EX_TEMPFAIL`.
pub(crate) const INCONCLUSIVE: u8 = 75;

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub(crate) struct LiveChecks {
    #[serde(default, rename = "live-check")]
    checks: Vec<LiveCheck>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LiveCheck {
    /// The check's ID, as its results name it.
    id: String,
    area: NonZeroU8,
    /// The job that runs it, `<workflow file>#<job>`, when a shell job does.
    job: Option<String>,
    /// The command, run by `bash -c` from the workspace's root.
    run: Option<String>,
}

impl LiveChecks {
    /// The list in the workspace at `root`; none when the file is absent.
    pub(crate) fn read(root: &Path) -> Result<Self> {
        let path = root.join(LIVE_CHECKS);
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let checks: Self =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        if let Some(check) = checks
            .checks
            .iter()
            .find(|check| check.run.is_some() && check.job.is_none())
        {
            bail!(
                "{}: the Live check `{}` has a command but no job to run it",
                path.display(),
                check.id
            );
        }
        Ok(checks)
    }

    pub(crate) fn area(&self, id: &str) -> Option<NonZeroU8> {
        self.checks
            .iter()
            .find(|check| check.id == id)
            .map(|check| check.area)
    }

    /// Runs the commands of the job `job` from `root` and writes their
    /// results to the JUnit file `junit`. Whether any check failed.
    pub(crate) fn run(&self, root: &Path, job: &str, junit: &Path) -> Result<bool> {
        let mut ran = Vec::new();
        for check in &self.checks {
            let (Some(check_job), Some(command)) = (&check.job, &check.run) else {
                continue;
            };
            if check_job != job {
                continue;
            }
            let started = Instant::now();
            let result = Command::new("bash")
                .args(["-c", command])
                .current_dir(root)
                .output()
                .with_context(|| format!("running the Live check `{}`", check.id))?;
            let ending = match result.status.code() {
                Some(0) => Ending::Passed,
                Some(code) if code == i32::from(INCONCLUSIVE) => Ending::Inconclusive,
                _ => Ending::Failed(result.status.to_string()),
            };
            let run = Run {
                id: &check.id,
                ending,
                seconds: started.elapsed().as_secs_f64(),
                stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
                stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
            };
            output::line(&format!("{}: {}", run.id, run.ending.as_str()));
            for text in [&run.stdout, &run.stderr] {
                if !text.is_empty() {
                    output::line(text.trim_end());
                }
            }
            ran.push(run);
        }
        write_junit(&ran, junit)?;
        Ok(ran
            .iter()
            .any(|run| matches!(run.ending, Ending::Failed(_))))
    }
}

enum Ending {
    Passed,
    Inconclusive,
    /// With the command's exit status.
    Failed(String),
}

impl Ending {
    fn as_str(&self) -> &'static str {
        match self {
            Self::Passed => "passed",
            Self::Inconclusive => "inconclusive",
            Self::Failed(_) => "failed",
        }
    }
}

/// One check's run.
struct Run<'a> {
    id: &'a str,
    ending: Ending,
    seconds: f64,
    stdout: String,
    stderr: String,
}

/// The suite and file name of every job's Live checks.
const SUITE: &str = "live";

fn write_junit(runs: &[Run<'_>], junit: &Path) -> Result<()> {
    let failures = runs
        .iter()
        .filter(|run| matches!(run.ending, Ending::Failed(_)))
        .count();
    let skipped = runs
        .iter()
        .filter(|run| matches!(run.ending, Ending::Inconclusive))
        .count();
    let counts = format!(
        "tests=\"{}\" failures=\"{failures}\" skipped=\"{skipped}\"",
        runs.len()
    );
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(xml, "<testsuites name=\"{SUITE}\" {counts}>");
    let _ = writeln!(xml, "  <testsuite name=\"{SUITE}\" {counts}>");
    for run in runs {
        // No class: the ingest names the test by its ID alone.
        let _ = writeln!(
            xml,
            "    <testcase classname=\"\" name=\"{}\" time=\"{:.3}\">",
            escape(run.id),
            run.seconds
        );
        match &run.ending {
            Ending::Passed => {}
            Ending::Inconclusive => {
                let _ = writeln!(
                    xml,
                    "      <skipped message=\"inconclusive: exit {INCONCLUSIVE}\"/>"
                );
            }
            Ending::Failed(status) => {
                let _ = writeln!(
                    xml,
                    "      <failure message=\"{}\">{}</failure>",
                    escape(status.as_str()),
                    escape(run.stderr.as_str())
                );
            }
        }
        let _ = writeln!(
            xml,
            "      <system-out>{}</system-out>",
            escape(run.stdout.as_str())
        );
        let _ = writeln!(
            xml,
            "      <system-err>{}</system-err>",
            escape(run.stderr.as_str())
        );
        xml.push_str("    </testcase>\n");
    }
    xml.push_str("  </testsuite>\n</testsuites>\n");
    if let Some(folder) = junit.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(junit, xml).with_context(|| format!("writing {}", junit.display()))
}
