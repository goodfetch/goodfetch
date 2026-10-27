//! One trial, run in a process of its own (`xtask compare-spider trial`):
//! it starts the tool as its only child, so the OS's peak resident memory
//! of its waited-for children is the tool's alone (the resident-memory
//! check's peak reader), drives it through one crawl of the site, and
//! prints `seconds=<s> peak-bytes=<n>`, from the tool's start to its exit.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::time::Instant;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use serde_json::{Value, json};

use crate::rss::peak;

use super::trials::Tool;

/// The host the Proving ground serves the site on.
pub(crate) const HOST: &str = "scale.test";

/// Where one trial crawls: the Proving ground's address and the site's
/// size.
pub(crate) struct Site<'a> {
    /// `<ip>:<port>`.
    pub(crate) address: &'a str,
    pub(crate) pages: u64,
    pub(crate) concurrency: u32,
}

impl Site<'_> {
    fn port(&self) -> &str {
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
                crawl_with_foxhound(stdin, stdout, site)
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
fn crawl_with_foxhound(stdin: ChildStdin, stdout: ChildStdout, site: &Site<'_>) -> Result<()> {
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
    })?;
    engine.call("hello", HELLO, &hello)?;
    let concurrency = site.concurrency;
    engine.call(
        "start",
        "crawl.start",
        &json!({
            "run_id": "compare-spider",
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
                // its domain's count, at the comparison's concurrency.
                "politeness": {
                    "sharing": "run",
                    "domain": { "delay": 0, "concurrency": concurrency },
                },
                "crawl": { "handler": { "concurrency": concurrency } },
            },
        }),
    )?;
    let report = engine.call(
        "finish",
        "run.finish",
        &json!({ "run_id": "compare-spider" }),
    )?;
    if report["lifecycle"] != "finished" || report["completeness"] != "complete" {
        bail!("Foxhound's crawl did not finish complete: {report}");
    }
    let Engine { stdin, mut stdout } = engine;
    drop(stdin);
    // Its output stays open until it exits, so ending never fails a write.
    std::io::copy(&mut stdout, &mut std::io::sink()).context("reading the engine's output")?;
    Ok(())
}
