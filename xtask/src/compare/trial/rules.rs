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

use super::{Mode, Plan, Route, browser_cache, kill, now, observations, request_count};
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

/// The address of the open forward proxy among the network fixtures of the
/// Proving ground at `address`.
fn open_proxy(address: &str) -> Result<String> {
    let fixtures: serde_json::Value = ureq::get(&format!("http://{address}/network-fixtures"))
        .call()
        .context("reading the Proving ground's network fixtures")?
        .body_mut()
        .read_json()
        .context("parsing the Proving ground's network fixtures")?;
    fixtures["proxies"]["open"]
        .as_str()
        .map(str::to_owned)
        .context("the Proving ground's network fixtures name no open proxy")
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
        // fastCRW reads robots.txt on a client of its own, which sends none
        // of the headers its pages carry, and resolves no host name to the
        // Proving ground: it reaches each task's host by name through the
        // Proving ground's open forward proxy.
        let proxy = match self.config {
            Config::FastCrw => Some(open_proxy(self.site.address)?),
            _ => None,
        };
        let tasks = self.cell.scenario.hostile_tasks();
        let mut stopped = Vec::new();
        for task in tasks {
            stopped.push(self.crawl_task(
                task,
                cache.as_ref().map(tempfile::TempDir::path),
                proxy.as_deref(),
            )?);
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
    /// stopped, if it was. A tool given `proxy` crawls the task's host by
    /// name through it; another, from the Proving ground's address with
    /// the host as its `Host`.
    fn crawl_task(
        &self,
        task: &Task,
        cache: Option<&Path>,
        proxy: Option<&str>,
    ) -> Result<Option<String>> {
        let home = tempfile::tempdir().context("creating the tool's data folder")?;
        let address = self.site.address;
        let host = format!("{}:{}", task.host, self.site.port());
        let (url, route) = match proxy {
            Some(proxy) => (format!("http://{host}/"), Route::Proxy(proxy)),
            None => (format!("http://{address}/"), Route::Host(&host)),
        };
        let baseline = request_count(address)?;
        let mut driven = self.spawn_at(home.path(), (1, &Mode::Rules), cache, &url, route)?;
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
                        self.cell,
                        &format!("http://{host}/"),
                        self.durability(),
                        self.config.samples(),
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
