//! One trial, run in a process of its own (`xtask compare-spider trial`):
//! it starts the tool as its only child, so the OS's peak resident memory
//! of its waited-for children is the tool's alone (the resident-memory
//! check's peak reader), drives it through one crawl of the site, and
//! prints `seconds=<s> peak-bytes=<n>`, from the tool's start to its exit.
//! A trial of the Callgrind cell runs the tool under Callgrind on ADR 0138's
//! scenario 1; the instructions it counted are in the file it was given.
use std::ffi::OsString;
use std::io::Read as _;
use std::path::Path;
use std::process::{ChildStdout, Command, Stdio};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};

use crate::rss::crawl::{self, HOST, Settings, Site};
use crate::rss::peak;

use super::trials::Tool;

/// Runs `tool`'s `executable` through one crawl of `site` and returns its
/// seconds from start to exit and its peak resident memory in bytes.
/// `goodfetch_variables` are this process's Goodfetch variables, which the
/// tool runs without. With `callgrind`, the tool runs under Callgrind, which
/// writes its output there.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a trial starts the tool as a blocking child process (STYLE.md)"
)]
pub(crate) fn run(
    tool: Tool,
    executable: &Path,
    site: &Site<'_>,
    goodfetch_variables: &[OsString],
    callgrind: Option<&Path>,
) -> Result<(f64, u64)> {
    let home = tempfile::tempdir().context("creating the tool's data folder")?;
    let mut command = match callgrind {
        Some(output) => {
            let mut valgrind = Command::new(VALGRIND);
            valgrind
                .args(["--tool=callgrind", "--quiet"])
                .arg(format!("--callgrind-out-file={}", output.display()))
                .arg(executable);
            valgrind
        }
        None => Command::new(executable),
    };
    let settings = match callgrind {
        Some(_) => Settings::Raw,
        None => Settings::CodeFree,
    };
    command.stdout(Stdio::piped()).stderr(Stdio::inherit());
    // Neither tool reads Goodfetch's variables a shell may hold, such as
    // a test's: Foxhound refuses one it does not deploy with.
    for name in goodfetch_variables {
        command.env_remove(name);
    }
    match tool {
        Tool::Foxhound => {
            // The executable as shipped, with its own data folder and its
            // default logging, which it writes to stderr.
            command
                .arg("attached-engine")
                .env("GOODFETCH_HOME", home.path())
                .stdin(Stdio::piped());
        }
        Tool::Spider => {
            command
                .args([
                    "--url",
                    &format!("http://{}/sites/{}/1", site.address, site.pages),
                ])
                .args(["--host", &format!("{HOST}:{}", site.port())])
                .args(["--concurrency", &site.concurrency.to_string()])
                .stdin(Stdio::null());
        }
    }
    let started = started();
    let peak = peak::of(&mut command, |child| {
        let stdout = child.stdout.take().context("the tool's stdout is piped")?;
        match tool {
            Tool::Foxhound => {
                let stdin = child.stdin.take().context("the engine's stdin is piped")?;
                crawl::crawl(stdin, stdout, site, settings)
            }
            Tool::Spider => read_spider(stdout),
        }
    })?;
    Ok((started.elapsed().as_secs_f64(), peak))
}

/// Valgrind's program name.
const VALGRIND: &str = "valgrind";

/// Why this host cannot run the Callgrind cell, when it cannot.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it asks Valgrind for its version with a blocking child process (STYLE.md)"
)]
pub(crate) fn callgrind_missing() -> Option<String> {
    let installed = Command::new(VALGRIND)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    (!installed).then(|| "Valgrind is not installed on this host".to_owned())
}

/// The instructions a Callgrind output file totals: its `totals:` line, of
/// the one event it counted.
pub(crate) fn instructions_of(output: &str) -> Result<u64> {
    output
        .lines()
        .find_map(|line| line.strip_prefix("totals:"))
        .context("Callgrind's output holds no `totals:` line")?
        .trim()
        .parse()
        .context("parsing Callgrind's total")
}

/// The moment a trial's timing starts from.
#[expect(
    clippy::disallowed_methods,
    reason = "xtask has no Clock handle (ADR 0059): a trial's seconds are measured on the OS's monotonic clock"
)]
fn started() -> Instant {
    Instant::now()
}

/// Spider's one line, which only says it ended.
fn read_spider(mut stdout: ChildStdout) -> Result<()> {
    let mut text = String::new();
    stdout
        .read_to_string(&mut text)
        .context("reading Spider's output")?;
    if !text.starts_with("pages=") {
        bail!("Spider printed no count: {}", text.trim_end());
    }
    Ok(())
}

#[cfg(test)]
mod tests;
