//! Foxhound's side of a trial: its Attached engine driven over JSON-RPC
//! through one code-free crawl of the site on its `http` tier, at a cell's
//! durability, started, or resumed after a kill, and, once the measured
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
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::foxhound::crawl::CODE_FREE_DATASET;
use serde_json::{Value, json};

use crate::rss::crawl::{HOST, Site};

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
            protocol_version: PROTOCOL_VERSION,
            surface: Surface::Rust,
            form: None,
            token: None,
            needs: Some(Product::Foxhound),
            product_version: ProductVersion::parse(env!("CARGO_PKG_VERSION"))
                .context("xtask's version is a Product version")?,
            log_level: None,
            client_surface: None,
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

    /// Starts the code-free crawl of `site` at `durability`.
    pub(crate) fn start(&mut self, site: &Site<'_>, durability: &str) -> Result<()> {
        self.call(
            "start",
            "crawl.start",
            &json!({
                "run_id": RUN,
                "seeds": [{ "url": site.start_url(), "label": "page" }],
                "datasets": [],
                // Each page's Record holds no format: the crawl fetches
                // and follows links, as Spider's does.
                "extraction": { "formats": {}, "follow": "scope" },
                "settings": settings(site, durability),
            }),
        )?;
        Ok(())
    }

    /// Resumes the killed crawl from its run file.
    pub(crate) fn resume(&mut self, site: &Site<'_>, durability: &str) -> Result<()> {
        self.call(
            "resume",
            "run.resume",
            &json!({ "run_id": RUN, "settings": settings(site, durability) }),
        )?;
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

    /// Each Record of the crawl's Dataset: its URL and when it was
    /// committed, RFC 3339.
    pub(crate) fn records(&mut self, site: &Site<'_>) -> Result<Vec<(String, String)>> {
        let id = "records";
        let mut records = Vec::new();
        self.send(&json!({
            "jsonrpc": "2.0", "id": id, "method": "run.records",
            "params": { "run_id": RUN, "dataset": CODE_FREE_DATASET },
        }))?;
        // Credit for every Record at once: a crawl with duplicates after
        // a kill still commits one Record a page.
        self.send(&json!({
            "jsonrpc": "2.0", "method": "$/credit",
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
            if message["method"] == "$/record" && message["params"]["id"] == id {
                let record = &message["params"]["record"];
                records.push((
                    record["url"].as_str().unwrap_or_default().to_owned(),
                    record["committed_at"]
                        .as_str()
                        .unwrap_or_default()
                        .to_owned(),
                ));
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

/// A cell's Run settings: its own Politeness at the crawl's concurrency, no
/// delay and robots.txt not obeyed, at `durability`.
fn settings(site: &Site<'_>, durability: &str) -> Value {
    let concurrency = site.concurrency;
    json!({
        "network": { "resolve": { HOST: [site.ip()] } },
        "egress": { "private_ranges": [HOST] },
        "fetch": { "tier": "http" },
        // Its own Politeness, so no other run on the machine shares its
        // domain's count, at the crawl's concurrency.
        "politeness": {
            "sharing": "run",
            "domain": { "delay": 0, "concurrency": concurrency },
        },
        "crawl": { "handler": { "concurrency": concurrency } },
        "run_state": { "durability": durability },
        "site_preferences": { "respect_robots_txt": false },
    })
}
