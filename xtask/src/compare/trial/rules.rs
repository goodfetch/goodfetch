//! A hostile pages' cell's run (#2439): its tool crawls each task of its
//! pass in turn, from the task's start page on its own host, in a fresh
//! data folder, until it ends its crawl or reaches a bound, when the
//! comparison stops it; then the Proving ground's observations give the
//! rules it broke on each task ([`hostile::broken`]). A tool that exits
//! with a failure the comparison did not cause fails the run.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a watcher thread reads the Proving ground's request count, sleeping between reads, while the tool runs (STYLE.md)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "xtask is no engine crate loom models (ADR 0060): the watcher thread reads whether the crawl ended from std's atomic"
)]

use std::path::Path;
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use super::{Mode, Plan, browser_cache, kill, now, observations, request_count};
use crate::compare::hostile::{self, Broken, Task};
use crate::compare::matrix::{Config, Scenario};
use crate::compare::sampler;

/// The most requests a tool may send one task before the comparison stops
/// it: over thirty times the pages a task asks for, so only a crawl caught
/// in a trap without end reaches it.
pub(crate) const TASK_REQUESTS: u64 = 1_000;
/// The longest a tool may crawl one task: a crawl that waits out every
/// `Retry-After` of `limits/1` ends well inside it.
pub(crate) const TASK_SECONDS: u64 = 300;
/// How often the watcher reads the request count.
const WATCH: Duration = Duration::from_millis(100);

/// What a hostile pages' run prints: the rules its tool broke on each task
/// of its pass, in the pass's order.
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct Ruled {
    pub(crate) tasks: Vec<Broken>,
}

/// Why a task's crawl, `seconds` in and `requests` sent, is stopped; none
/// while it goes on.
pub(crate) fn bound(seconds: u64, requests: u64) -> Option<String> {
    if requests >= TASK_REQUESTS {
        return Some(format!(
            "stopped after {requests} requests, the most a task may take"
        ));
    }
    (seconds >= TASK_SECONDS).then(|| {
        format!("stopped after {seconds} s, the longest a task may take, at {requests} requests")
    })
}

/// Reads a tool's output to its end, whatever it printed: a hostile pages'
/// crawl is read from what the Proving ground saw.
fn drained(child: &mut Child) -> Result<()> {
    let mut stdout = child.stdout.take().context("the tool's stdout is piped")?;
    std::io::copy(&mut stdout, &mut std::io::sink()).context("reading the tool's output")?;
    Ok(())
}

impl Plan<'_> {
    /// Runs the cell's tool on each task of its pass and reads the rules it
    /// broke on each.
    pub(crate) fn ruled(&self) -> Result<Ruled> {
        // Foxhound's defaults fetch on the `auto` tier, which may start a
        // browser: its pinned build, laid out as the browser scenario's.
        let cache = match (self.config, self.cell.scenario, self.browser) {
            (Config::Foxhound { .. }, Scenario::Hostile, Some(browser)) => {
                Some(browser_cache(browser)?)
            }
            _ => None,
        };
        let tasks = self.cell.scenario.hostile_tasks();
        let mut stopped = Vec::new();
        for task in tasks {
            stopped.push(self.crawl_task(task, cache.as_ref().map(tempfile::TempDir::path))?);
        }
        let seen = observations(self.site.address)?;
        let tasks = tasks
            .iter()
            .zip(stopped)
            .map(|(task, stopped)| {
                Ok(Broken {
                    stopped,
                    ..hostile::broken(&seen, task)?
                })
            })
            .collect::<Result<_>>()?;
        Ok(Ruled { tasks })
    }

    /// Crawls `task` to its end, or until [`bound`] stops it: why it was
    /// stopped, if it was.
    fn crawl_task(&self, task: &Task, cache: Option<&Path>) -> Result<Option<String>> {
        let home = tempfile::tempdir().context("creating the tool's data folder")?;
        let address = self.site.address;
        let host = format!("{}:{}", task.host, self.site.port());
        let baseline = request_count(address)?;
        let mut driven = self.spawn_at(
            home.path(),
            (1, &Mode::Rules),
            cache,
            &format!("http://{address}/"),
            Some(&host),
        )?;
        let pid = driven.child.id();
        let done = AtomicBool::new(false);
        let started = now();
        let (ended, watched) = std::thread::scope(|scope| {
            let bounds = scope.spawn(|| -> Result<Option<String>> {
                while !done.load(Ordering::Relaxed) {
                    std::thread::sleep(WATCH);
                    let sent = request_count(address).map(|count| count.saturating_sub(baseline));
                    let reason = match &sent {
                        Ok(sent) => bound(started.elapsed().as_secs(), *sent),
                        Err(error) => Some(format!("{error:#}")),
                    };
                    if let Some(reason) = reason {
                        // Every process of the tool's tree, gathered before
                        // any dies and leaves its children to another
                        // parent; one may have exited meanwhile.
                        for pid in sampler::tree(pid) {
                            let _ = kill(pid);
                        }
                        return sent.map(|_| Some(reason));
                    }
                }
                Ok(None)
            });
            let ended = match driven.engine.as_mut() {
                Some(engine) => engine
                    .start(
                        &self.site,
                        self.cell.scenario,
                        &format!("http://{host}/"),
                        self.durability(),
                    )
                    .and_then(|()| engine.ended()),
                None => drained(&mut driven.child),
            };
            done.store(true, Ordering::Relaxed);
            let watched = bounds
                .join()
                .unwrap_or_else(|_| Err(anyhow::anyhow!("the request count's watcher panicked")));
            (ended, watched)
        });
        if ended.is_err() && !matches!(watched, Ok(Some(_))) {
            let _ = driven.child.kill();
        }
        if let Some(engine) = driven.engine.take() {
            engine.close();
        }
        let status = driven.child.wait().context("waiting for the tool")?;
        if let Some(reason) = watched? {
            return Ok(Some(reason));
        }
        ended.with_context(|| format!("{} on {}", self.cell.name, task.name))?;
        if !status.success() {
            bail!("{} exited with {status} on {}", self.cell.name, task.name);
        }
        Ok(None)
    }
}

#[cfg(test)]
mod tests;
