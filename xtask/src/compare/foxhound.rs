//! Foxhound's side of a trial: its Attached engine driven over JSON-RPC
//! through one code-free crawl of the scenario's site at a cell's
//! settings, started, or resumed after a kill, and, once the measured
//! process is gone, its Records' commit times read by another engine on the
//! same home, which no figure counts.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: the engine's output is read on a thread of its own with blocking calls (STYLE.md)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::process::{ChildStdin, ChildStdout};
use std::sync::mpsc::{Receiver, channel};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::events::CREDIT;
use goodfetch_protocol::foxhound::crawl::CODE_FREE_DATASET;
use goodfetch_protocol::foxhound::dataset::RECORD;
use serde_json::{Value, json};

use crate::rss::crawl::Site;

use super::matrix::{Cell, Scenario};

/// The Run every trial crawls in.
const RUN: &str = "crawl";

/// A JSON-RPC connection to an Attached engine, its output read by a
/// thread of its own so the engine never waits on a full pipe while the
/// trial waits on the Proving ground.
pub(crate) struct Engine {
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Engine {
    /// Opens the connection with `hello`.
    pub(crate) fn open(stdin: ChildStdin, stdout: ChildStdout) -> Result<Self> {
        let (sender, lines) = channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                let Ok(line) = line else { break };
                if sender.send(line).is_err() {
                    break;
                }
            }
        });
        let mut engine = Self { stdin, lines };
        let hello = serde_json::to_value(HelloParams {
            needs: Some(Product::Foxhound),
            ..HelloParams::new(
                Surface::Rust,
                ProductVersion::parse(env!("CARGO_PKG_VERSION"))
                    .context("xtask's version is a Product version")?,
            )
        })?;
        engine.call("hello", HELLO, &hello)?;
        Ok(engine)
    }

    fn send(&mut self, message: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .and_then(|()| self.stdin.flush())
            .context("writing to the engine")
    }

    /// The engine's next message.
    fn next(&self, waiting_for: &str) -> Result<Value> {
        let line = self
            .lines
            .recv()
            .with_context(|| format!("the engine ended before it answered {waiting_for}"))?;
        serde_json::from_str(&line)
            .with_context(|| format!("the engine wrote no JSON: {}", line.trim_end()))
    }

    /// Sends the call `id` and reads until its reply, handing each other
    /// message to `other`.
    fn call_with(
        &mut self,
        id: &str,
        method: &str,
        params: &Value,
        mut other: impl FnMut(&Value),
    ) -> Result<Value> {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let message = self.next(method)?;
            if message["method"] == "crawl_request.handle" {
                bail!("a code-free crawl sent its program {message}");
            }
            if message["id"] == id {
                if message.get("error").is_some() {
                    bail!("{method} failed: {message}");
                }
                return Ok(message["result"].clone());
            }
            other(&message);
        }
    }

    fn call(&mut self, id: &str, method: &str, params: &Value) -> Result<Value> {
        self.call_with(id, method, params, |_| {})
    }

    /// Starts the code-free crawl of `cell`'s scenario's site from `start`
    /// at `durability`, with its `run_state.commit_interval` at `periodic`.
    pub(crate) fn start(
        &mut self,
        site: &Site<'_>,
        cell: &Cell,
        start: &str,
        durability: (&str, Option<u64>),
    ) -> Result<()> {
        self.call(
            "start",
            "crawl.start",
            &json!({
                "run_id": RUN,
                "seeds": [{ "url": start, "label": "page" }],
                "datasets": [],
                "extraction": extraction(cell.scenario),
                "settings": settings(site, cell, durability),
            }),
        )?;
        Ok(())
    }

    /// Resumes the killed crawl from its run file.
    pub(crate) fn resume(
        &mut self,
        site: &Site<'_>,
        cell: &Cell,
        durability: (&str, Option<u64>),
    ) -> Result<()> {
        self.call(
            "resume",
            "run.resume",
            &json!({ "run_id": RUN, "settings": settings(site, cell, durability) }),
        )?;
        Ok(())
    }

    /// Waits for the Run to end, however it ended: a hostile pages' crawl
    /// (#2439) is read from what the Proving ground saw, not from its end.
    pub(crate) fn ended(&mut self) -> Result<()> {
        self.call("finish", "run.finish", &json!({ "run_id": RUN }))?;
        Ok(())
    }

    /// Waits for the Run to finish complete, having fetched each page.
    pub(crate) fn finish(&mut self) -> Result<()> {
        let report = self.call("finish", "run.finish", &json!({ "run_id": RUN }))?;
        if report["lifecycle"] != "finished" || report["completeness"] != "complete" {
            bail!("Foxhound's crawl did not finish complete: {report}");
        }
        Ok(())
    }

    /// Each Record of the crawl's Dataset.
    pub(crate) fn records(&mut self, site: &Site<'_>) -> Result<Vec<Record>> {
        let id = "records";
        let mut records = Vec::new();
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "run.records",
            "params": { "run_id": RUN, "dataset": CODE_FREE_DATASET },
        }))?;
        // Credit for every Record at once: a crawl with duplicates after
        // a kill still commits one Record a page.
        self.send(&json!({
            "jsonrpc": "2.0", "method": CREDIT,
            "params": { "id": id, "credits": site.pages * 2 },
        }))?;
        loop {
            let message = self.next("run.records")?;
            if message["id"] == id {
                if message.get("error").is_some() {
                    bail!("run.records failed: {message}");
                }
                return Ok(records);
            }
            if message["method"] == RECORD && message["params"]["id"] == id {
                let record = &message["params"]["record"];
                let text = |value: &Value| value.as_str().map(str::to_owned);
                records.push(Record {
                    url: text(&record["url"]).unwrap_or_default(),
                    committed_at: text(&record["committed_at"]).unwrap_or_default(),
                    title: text(&record["value"]["title"]),
                    text: text(&record["value"]["text"]),
                });
            }
        }
    }

    /// Ends the input, so the engine exits.
    pub(crate) fn close(self) {
        drop(self.stdin);
        // Its output stays open until it exits, so ending never fails a
        // write.
        while self.lines.recv().is_ok() {}
    }
}

/// A Record of the crawl's Dataset, as `run.records` sends it.
pub(crate) struct Record {
    /// Its crawl request's URL.
    pub(crate) url: String,
    /// When it was committed, RFC 3339.
    pub(crate) committed_at: String,
    /// Its field `title` and its `text` format, which the extraction
    /// scenario's Records hold.
    pub(crate) title: Option<String>,
    pub(crate) text: Option<String>,
}

/// What each page's Record holds: in the extraction scenario, the text of
/// its main content and its title; otherwise no format, the crawl fetching
/// and following links as the other tools' do.
fn extraction(scenario: Scenario) -> Value {
    match scenario {
        Scenario::Extraction => json!({
            "formats": { "text": true },
            "follow": "scope",
            "fields": { "title": { "locator": { "kind": "css", "selector": "title" } } },
        }),
        Scenario::Raw
        | Scenario::Durable
        | Scenario::Polite
        | Scenario::Browser
        | Scenario::Realistic
        | Scenario::Long
        | Scenario::Raw64
        | Scenario::Raw256
        | Scenario::Hostile
        | Scenario::Hostile16 => json!({ "formats": {}, "follow": "scope" }),
    }
}

/// Whether Foxhound's crawl in `scenario` keeps Records: in the durable
/// scenario, whose resume is judged by them, and the extraction scenario,
/// which writes each page's; scenario 1's maximum-throughput combination
/// and the headless browser scenario, which asks for its settings, keep
/// none, as the other tools keep nothing there, nor do #2316's scenarios,
/// which run scenario 1's settings.
pub(crate) fn keeps_records(scenario: Scenario) -> bool {
    matches!(
        scenario.settings(),
        Scenario::Durable | Scenario::Extraction
    )
}

/// A cell's Run settings, its site's hosts resolved to the Proving ground:
/// in the hostile pages' pass at defaults (#2439), nothing else, every
/// other setting the catalog's default, `run_state.durability` too; in the
/// polite scenario, Foxhound's defaults with ADR 0086's delay and
/// concurrency per domain, the `http` tier and a scope of every host;
/// otherwise its own Politeness at the crawl's concurrency, no delay,
/// robots.txt not obeyed and the maximum-throughput combination's switches
/// off, at `durability`, on the `browser` tier in the headless browser
/// scenario and the `http` tier otherwise; on the extraction scenario's row
/// that says so (#2136), the ads, trackers and annoyances Filter lists off.
fn settings(
    site: &Site<'_>,
    cell: &Cell,
    (durability, commit_interval): (&str, Option<u64>),
) -> Value {
    let scenario = cell.scenario;
    let hosts = scenario.site().hosts();
    let resolve: serde_json::Map<String, Value> = hosts
        .iter()
        .map(|host| (host.clone(), json!([site.ip()])))
        .collect();
    let mut settings = json!({
        "network": { "resolve": resolve },
        "egress": { "private_ranges": hosts },
    });
    let concurrency = site.concurrency;
    let added = match scenario {
        Scenario::Hostile => return settings,
        // ADR 0086's 1 s and 1 in flight per domain, which the catalog
        // holds no default for until the fetch tiers take them (ADR 0086's
        // #684 note).
        Scenario::Polite => json!({
            "fetch": { "tier": "http" },
            "politeness": { "domain": { "delay": 1000, "concurrency": 1 } },
            "crawl": { "scope": { "strategy": "all" } },
        }),
        // The maximum-throughput combination (ADR 0138,
        // `docs/throughput.md`): every per-page feature the crawl does not
        // need switched off, its Records kept only where the scenario
        // reads them.
        Scenario::Raw
        | Scenario::Durable
        | Scenario::Extraction
        | Scenario::Browser
        | Scenario::Realistic
        | Scenario::Long
        | Scenario::Raw64
        | Scenario::Raw256
        | Scenario::Hostile16 => json!({
            "fetch": {
                "tier": if scenario == Scenario::Browser { "browser" } else { "http" },
                "block_classification": false,
            },
            "frontier": { "learning": false },
            "provenance": { "log": false },
            "dataset": { "records": keeps_records(scenario) },
            // Its own Politeness, so no other run on the machine shares its
            // domain's count, at the crawl's concurrency.
            "politeness": {
                "sharing": "run",
                "domain": { "delay": 0, "concurrency": concurrency },
            },
            "crawl": { "handler": { "concurrency": concurrency } },
            "site_preferences": { "respect_robots_txt": false },
        }),
    };
    if let (Value::Object(settings), Value::Object(added)) = (&mut settings, added) {
        settings.extend(added);
    }
    if cell.filter_lists_off() {
        settings["filter_lists"] = json!({ "ads": false, "trackers": false, "annoyances": false });
    }
    settings["run_state"] = json!({ "durability": durability });
    if let Some(interval) = commit_interval {
        settings["run_state"]["commit_interval"] = json!(interval);
    }
    settings
}
