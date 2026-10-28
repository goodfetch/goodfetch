//! One trial, run in a process of its own (`xtask compare-spider trial`):
//! it starts the tool as its only child, so the OS's peak resident memory
//! of its waited-for children is the tool's alone (the resident-memory
//! check's peak reader), drives it through one crawl of the site, and
//! prints `seconds=<s> peak-bytes=<n>`, from the tool's start to its exit.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::io::Read as _;
use std::path::Path;
use std::process::{ChildStdout, Command, Stdio};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};

use crate::rss::crawl::{self, HOST, Site};
use crate::rss::peak;

use super::trials::Tool;

/// Runs `tool`'s `executable` through one crawl of `site` and returns its
/// seconds from start to exit and its peak resident memory in bytes.
pub(crate) fn run(tool: Tool, executable: &Path, site: &Site<'_>) -> Result<(f64, u64)> {
    let home = tempfile::tempdir().context("creating the tool's data folder")?;
    let mut command = Command::new(executable);
    command.stdout(Stdio::piped()).stderr(Stdio::inherit());
    // Neither tool reads Goodfetch's variables a shell may hold, such as
    // a test's: Foxhound refuses one it does not deploy with.
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GOODFETCH_") {
            command.env_remove(name);
        }
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
    let started = Instant::now();
    let peak = peak::of(&mut command, |child| {
        let stdout = child.stdout.take().context("the tool's stdout is piped")?;
        match tool {
            Tool::Foxhound => {
                let stdin = child.stdin.take().context("the engine's stdin is piped")?;
                crawl::crawl(stdin, stdout, site)
            }
            Tool::Spider => read_spider(stdout),
        }
    })?;
    Ok((started.elapsed().as_secs_f64(), peak))
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
