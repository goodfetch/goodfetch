//! Foxhound's side of a trial: its Attached engine driven over JSON-RPC
//! through one code-free crawl of the scenario's site at a cell's
//! settings, started, or resumed after a kill, and, once the measured
//! process is gone, its Records' commit times and samples read by another
//! engine on the same home, which no figure counts. The connection is the
//! HTTP crawl's client.

use std::process::{ChildStdin, ChildStdout};

use anyhow::{Result, bail};
use goodfetch_protocol::events::CREDIT;
use goodfetch_protocol::foxhound::dataset::RECORD;
use serde_json::{Value, json};

use crate::rss::crawl::{Client, ReadFailure, Site};

use super::matrix::{AtDefaults, Cell, Scenario};

/// The Run every trial crawls in.
pub(crate) const RUN: &str = "crawl";

/// The Dataset Foxhound's program pushes each page's Record to (#2317).
pub(crate) const PROGRAM_DATASET: &str = "pages";

/// A trial's connection to an Attached engine.
pub(crate) struct Engine {
    client: Client,
}

impl Engine {
    /// Opens the connection with `hello`; a failed read of the engine's
    /// output reads as its end.
    pub(crate) fn open(stdin: ChildStdin, stdout: ChildStdout) -> Result<Self> {
        Ok(Self {
            client: Client::open(stdin, stdout, ReadFailure::Ended)?,
        })
    }

    fn call(&mut self, id: &str, method: &str, params: &Value) -> Result<Value> {
        self.client.call(id, method, params)
    }

    /// Starts the code-free crawl of `cell`'s scenario's site from `start`
    /// at `durability`, with its `run_state.commit_interval` at `periodic`,
    /// and `fetch.tier` left at `auto` where `auto`.
    pub(crate) fn start(
        &mut self,
        site: &Site<'_>,
        cell: &Cell,
        start: &str,
        durability: (&str, Option<u64>),
        auto: bool,
    ) -> Result<()> {
        self.call(
            "start",
            "crawl.start",
            &json!({
                "run_id": RUN,
                "seeds": [{ "url": start, "label": "page" }],
                "datasets": [],
                "extraction": extraction(cell.scenario),
                "settings": settings(site, cell, durability, auto),
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
        auto: bool,
    ) -> Result<()> {
        self.call(
            "resume",
            "run.resume",
            &json!({ "run_id": RUN, "settings": settings(site, cell, durability, auto) }),
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

    /// Each Record of the crawl's Dataset `dataset`.
    pub(crate) fn records(&mut self, site: &Site<'_>, dataset: &str) -> Result<Vec<Record>> {
        let id = "records";
        let mut records = Vec::new();
        self.client.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "run.records",
            "params": { "run_id": RUN, "dataset": dataset },
        }))?;
        // Credit for every Record at once: a crawl with duplicates after
        // a kill still commits one Record a page.
        self.client.send(&json!({
            "jsonrpc": "2.0", "method": CREDIT,
            "params": { "id": id, "credits": site.pages * 2 },
        }))?;
        loop {
            let message = self.client.next("run.records")?;
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
                    sample: record["provenance"]["tier"]["chosen"] == "sample",
                    title: text(&record["value"]["title"]),
                    text: text(&record["value"]["text"]),
                    id: text(&record["value"]["id"]),
                    name: text(&record["value"]["name"]),
                    tier: text(&record["provenance"]["tier"]["tier"]),
                    value: record["value"].clone(),
                });
            }
        }
    }

    /// Ends the input, so the engine exits.
    pub(crate) fn close(self) {
        self.client
            .close()
            .expect("invariant: a read the trials read as an end never fails the close");
    }
}

/// What the comparison reads of one Record of the crawl's Dataset, as
/// `run.records` sends it.
pub(crate) struct Record {
    /// Its crawl request's URL.
    pub(crate) url: String,
    /// When it was committed, RFC 3339.
    pub(crate) committed_at: String,
    /// Whether its page was a sample of `auto`'s comparison of tiers, its
    /// document from the sample's browser run (ADR 0093).
    pub(crate) sample: bool,
    /// Its field `title` and its `text` format, which the extraction
    /// scenario's Records hold.
    pub(crate) title: Option<String>,
    pub(crate) text: Option<String>,
    /// Its fields `id` and `name`, which the auto cell's Records hold
    /// (#2411).
    pub(crate) id: Option<String>,
    pub(crate) name: Option<String>,
    /// The tier its page was served on, as its Provenance names it.
    pub(crate) tier: Option<String>,
    /// Its value whole: the handled crawl's twenty fields (#2317).
    pub(crate) value: Value,
}

/// What each page's Record holds: in the extraction scenario, the text of
/// its main content and its title; in the auto cell, the rungs site's
/// Record, its `id` and `name` (#2411); otherwise no format, the crawl
/// fetching and following links as the other tools' do.
fn extraction(scenario: Scenario) -> Value {
    match scenario {
        Scenario::Extraction => json!({
            "formats": { "text": true },
            "follow": "scope",
            "fields": { "title": { "locator": { "kind": "css", "selector": "title" } } },
        }),
        Scenario::Auto => json!({
            "formats": {},
            "follow": "scope",
            "fields": {
                "id": { "locator": { "kind": "css", "selector": "p.id" } },
                "name": { "locator": { "kind": "css", "selector": "h1.name" } },
            },
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
        | Scenario::Hostile16
        | Scenario::CrawlLike
        | Scenario::Handled => json!({ "formats": {}, "follow": "scope" }),
    }
}

/// Whether Foxhound's crawl in `scenario` keeps Records: in the durable
/// scenario, whose resume is judged by them, the extraction scenario,
/// which writes each page's, and the auto cell, which holds each page's to
/// the rungs site's manifest; scenario 1's maximum-throughput combination
/// and the headless browser scenario, which asks for its settings, keep
/// none, as the other tools keep nothing there, nor do #2316's scenarios,
/// which run scenario 1's settings.
pub(crate) fn keeps_records(scenario: Scenario) -> bool {
    matches!(
        scenario.settings(),
        Scenario::Durable | Scenario::Extraction | Scenario::Auto | Scenario::Handled
    )
}

/// A cell's Run settings, its site's hosts resolved to the Proving ground:
/// in the hostile pages' pass at defaults (#2439), nothing else, every
/// other setting the catalog's default, `run_state.durability` too; in the
/// polite scenario, Foxhound's defaults with ADR 0086's delay and
/// concurrency per domain, the `http` tier (left at `auto` where `auto`)
/// and a scope of every host; in the auto cell (#2411), `fetch.tier` at
/// `auto`, its default, with `fetch.http_from: learned`, its Records kept,
/// and its own Politeness at the crawl's concurrency, no delay and
/// robots.txt not obeyed, every other setting the catalog's default; in
/// the cell at the catalog's defaults (#2175), nothing else but the
/// scenario's pace, no delay and the crawl's concurrency per domain, and on
/// its second row `fetch.http_from: browser`; otherwise the default shared
/// Politeness at the crawl's concurrency, no delay, robots.txt not obeyed
/// and the maximum-throughput combination's switches off, at `durability`,
/// on the `browser` tier in the headless browser
/// scenario and the `http` tier otherwise; on the extraction scenario's row
/// that says so (#2136), the ads, trackers and annoyances Filter lists off;
/// on a default-cost cell (#2438), scenario 1's first cell's with that one
/// default on.
pub(crate) fn settings(
    site: &Site<'_>,
    cell: &Cell,
    (durability, commit_interval): (&str, Option<u64>),
    auto: bool,
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
    let pace = json!({ "domain": { "delay": 0, "concurrency": concurrency } });
    // The catalog's defaults (#2175): the crawl's site and the scenario's
    // pace alone, and on the second row the other way of
    // `fetch.http_from`.
    if let Some(row) = cell.at_defaults() {
        settings["politeness"] = pace;
        if row == AtDefaults::Browser {
            settings["fetch"] = json!({ "http_from": "browser" });
        }
        return settings;
    }
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
        Scenario::Auto => json!({
            "fetch": { "http_from": "learned" },
            "dataset": { "records": true },
            "politeness": {
                "sharing": "run",
                "domain": { "delay": 0, "concurrency": concurrency },
            },
            "crawl": { "handler": { "concurrency": concurrency } },
            "site_preferences": { "respect_robots_txt": false },
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
        | Scenario::Hostile16
        | Scenario::CrawlLike
        | Scenario::Handled => json!({
            "fetch": {
                "tier": if scenario == Scenario::Browser { "browser" } else { "http" },
                "block_classification": false,
            },
            "frontier": { "learning": false },
            "provenance": { "log": false },
            "dataset": { "records": keeps_records(scenario) },
            // The default shared Politeness, with Delegations, at the
            // crawl's concurrency (#2175).
            "politeness": pace,
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
    if let Some(default) = cell.default_on() {
        settings[default.section][default.key] = json!(true);
    }
    settings["run_state"] = json!({ "durability": durability });
    if let Some(interval) = commit_interval {
        settings["run_state"]["commit_interval"] = json!(interval);
    }
    if auto && let Some(fetch) = settings["fetch"].as_object_mut() {
        fetch.remove("tier");
    }
    settings
}

#[cfg(test)]
mod tests;
