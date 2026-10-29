//! The 1,000-page HTTP crawl of the local Proving ground (ADRs 0071 and
//! 0134): the Proving ground serving the site, started outside the process
//! measured, and Foxhound's Attached engine driven through one code-free
//! crawl of it on its `http` tier. The resident-memory check's `crawl`
//! scenario and `just compare-spider`'s Foxhound trials run this same
//! crawl.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use serde::Deserialize;
use serde_json::{Value, json};

/// The host the Proving ground serves the site on.
pub(crate) const HOST: &str = "scale.test";

/// The pages of the site crawled.
pub(crate) const PAGES: u64 = 1000;

/// Requests in flight at once: M1's default of
/// `crawl.handler.concurrency`, which the crawl's target is tied to (ADR
/// 0134).
pub(crate) const CONCURRENCY: u32 = 16;

/// The Proving ground's seed.
const SEED: u64 = 1574;

/// Where one crawl goes: the Proving ground's address and the site's size.
pub(crate) struct Site<'a> {
    /// `<ip>:<port>`.
    pub(crate) address: &'a str,
    pub(crate) pages: u64,
    pub(crate) concurrency: u32,
}

impl Site<'_> {
    pub(crate) fn port(&self) -> &str {
        self.address
            .rsplit_once(':')
            .map_or(self.address, |(_, port)| port)
    }

    fn ip(&self) -> &str {
        self.address
            .rsplit_once(':')
            .map_or(self.address, |(ip, _)| ip)
    }

    /// The site's first page on its host.
    fn start_url(&self) -> String {
        format!("http://{HOST}:{}/sites/{}/1", self.port(), self.pages)
    }
}

/// A serving Proving ground, stopped when dropped.
pub(crate) struct Serving {
    child: Child,
    /// `<ip>:<port>`.
    pub(crate) address: String,
}

impl Drop for Serving {
    fn drop(&mut self) {
        // A crawl's Proving ground holds nothing worth a clean stop.
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[derive(Deserialize)]
struct ServeLine {
    address: String,
}

/// Starts `proving_ground` serving on a loopback port of its own.
pub(crate) fn serve(proving_ground: &Path) -> Result<Serving> {
    let mut child = Command::new(proving_ground)
        .args(["serve", "--seed", &SEED.to_string(), "--copy", "local"])
        .args(["--address", "127.0.0.1:0", "--network-profile", "none"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .context("starting the Proving ground")?;
    let mut line = String::new();
    let stdout = child.stdout.take().context("its stdout is piped")?;
    BufReader::new(stdout)
        .read_line(&mut line)
        .context("reading the Proving ground's first line")?;
    // Stopped on drop from here, even when its line is not one.
    let mut serving = Serving {
        child,
        address: String::new(),
    };
    let first: ServeLine = serde_json::from_str(&line)
        .with_context(|| format!("the Proving ground's first line: {}", line.trim_end()))?;
    serving.address = first.address;
    Ok(serving)
}

/// A JSON-RPC connection to an Attached engine.
struct Engine {
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Engine {
    fn send(&mut self, message: &Value) -> Result<()> {
        let mut line = serde_json::to_vec(message)?;
        line.push(b'\n');
        self.stdin
            .write_all(&line)
            .and_then(|()| self.stdin.flush())
            .context("writing to the engine")
    }

    /// Sends the call `id` and reads until its reply, which it answers.
    fn call(&mut self, id: &str, method: &str, params: &Value) -> Result<Value> {
        self.send(&json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }))?;
        loop {
            let mut line = String::new();
            if self
                .stdout
                .read_line(&mut line)
                .context("reading the engine")?
                == 0
            {
                bail!("the engine ended before it answered {method}");
            }
            let message: Value = serde_json::from_str(&line)
                .with_context(|| format!("the engine wrote no JSON: {}", line.trim_end()))?;
            if message["method"] == "crawl_request.handle" {
                bail!("a code-free crawl sent its program {message}");
            }
            if message["id"] == id {
                if message.get("error").is_some() {
                    bail!("{method} failed: {message}");
                }
                return Ok(message["result"].clone());
            }
        }
    }
}

/// Opens with `hello`, starts a code-free crawl of the site on the `http`
/// tier, waits for its Run to finish complete, and ends the input.
pub(crate) fn crawl(stdin: ChildStdin, stdout: ChildStdout, site: &Site<'_>) -> Result<()> {
    let mut engine = Engine {
        stdin,
        stdout: BufReader::new(stdout),
    };
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
    let concurrency = site.concurrency;
    engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "crawl",
            "seeds": [{ "url": site.start_url(), "label": "page" }],
            "datasets": [],
            // Each page's Record holds no format: the crawl fetches and
            // follows links, as Spider's does.
            "extraction": { "formats": {}, "follow": "scope" },
            "settings": {
                "network": { "resolve": { HOST: [site.ip()] } },
                "egress": { "private_ranges": [HOST] },
                "fetch": { "tier": "http" },
                // Its own Politeness, so no other run on the machine shares
                // its domain's count, at the crawl's concurrency.
                "politeness": {
                    "sharing": "run",
                    "domain": { "delay": 0, "concurrency": concurrency },
                },
                "crawl": { "handler": { "concurrency": concurrency } },
            },
        }),
    )?;
    let report = engine.call("finish", "run.finish", &json!({ "run_id": "crawl" }))?;
    if report["lifecycle"] != "finished" || report["completeness"] != "complete" {
        bail!("Foxhound's crawl did not finish complete: {report}");
    }
    let Engine { stdin, mut stdout } = engine;
    drop(stdin);
    // Its output stays open until it exits, so ending never fails a write.
    std::io::copy(&mut stdout, &mut std::io::sink()).context("reading the engine's output")?;
    Ok(())
}
