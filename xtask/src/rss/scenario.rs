//! The resident-memory scenarios (ADRs 0071 and 0134), which join as their
//! areas' tickets land: an idle Daemon once the Daemon listens, and the
//! 1,000-page crawl on Foxhound's executable once it crawls. Each names the
//! Products whose executables it runs on, its Performance target and how it
//! drives the executable.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: a scenario idles its child on a blocking sleep (STYLE.md)"
)]

use std::io::{BufRead as _, BufReader, Write as _};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command};
use std::time::Duration;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::{PROTOCOL_VERSION, Product};
use goodfetch_protocol::connection::{HELLO, HelloParams, ProductVersion, Surface};
use goodfetch_protocol::jsonrpc::{Message, Request, RequestId, Version};
use serde::{Deserialize, Serialize};

const MIB: u64 = 1024 * 1024;

/// How long an Attached engine stays idle after its `hello`, so the peak
/// covers what it allocates once settled, not only its start.
const IDLE: Duration = Duration::from_secs(1);

#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Scenario {
    /// An Attached engine idle after `hello`.
    IdleAttachedEngine,
}

impl Scenario {
    pub(crate) const ALL: [Self; 1] = [Self::IdleAttachedEngine];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::IdleAttachedEngine => "idle-attached-engine",
        }
    }

    /// The Products whose executables it runs on: each, for a figure of the
    /// executable itself (ADR 0094).
    pub(crate) fn products(self) -> &'static [Product] {
        match self {
            Self::IdleAttachedEngine => &[Product::Collie, Product::Foxhound],
        }
    }

    /// Its Performance target in bytes (ADR 0134), which never gates.
    pub(crate) fn target_bytes(self) -> u64 {
        match self {
            Self::IdleAttachedEngine => 8 * MIB,
        }
    }

    /// The arguments the executable takes for this scenario.
    pub(crate) fn arguments(self) -> &'static [&'static str] {
        match self {
            Self::IdleAttachedEngine => &["attached-engine"],
        }
    }

    /// Drives the started executable through the scenario, then ends its
    /// input, so it exits.
    pub(crate) fn drive(
        self,
        needs: Product,
        stdin: ChildStdin,
        stdout: ChildStdout,
    ) -> Result<()> {
        match self {
            Self::IdleAttachedEngine => idle_attached_engine(needs, stdin, stdout),
        }
    }
}

/// Runs the engine `command` starts as shipped: with `home` as its data
/// folder and no other Goodfetch variable of this process's environment,
/// since an engine stops on one it does not know and logs at its default
/// level without `GOODFETCH_LOG`.
pub(crate) fn as_shipped(command: &mut Command, home: &Path) {
    for (name, _) in std::env::vars_os() {
        if name.to_string_lossy().starts_with("GOODFETCH_") {
            command.env_remove(name);
        }
    }
    command.env("GOODFETCH_HOME", home);
}

/// The `hello` line a Client needing `needs` opens with, as the engine
/// reads it from its input.
pub(crate) fn hello_line(needs: Product) -> Result<Vec<u8>> {
    let hello = Message::Request(Request {
        jsonrpc: Version,
        id: RequestId::Number(0),
        method: HELLO.to_owned(),
        params: Some(serde_json::to_value(HelloParams {
            protocol_version: PROTOCOL_VERSION,
            surface: Surface::Rust,
            form: None,
            token: None,
            needs: Some(needs),
            product_version: ProductVersion::parse(env!("CARGO_PKG_VERSION"))
                .context("xtask's version is a Product version")?,
            log_level: None,
        })?),
    });
    let mut line = serde_json::to_vec(&hello)?;
    line.push(b'\n');
    Ok(line)
}

/// Opens with `hello`, checks the engine accepted it, idles, and ends the
/// input.
fn idle_attached_engine(needs: Product, mut stdin: ChildStdin, stdout: ChildStdout) -> Result<()> {
    stdin
        .write_all(&hello_line(needs)?)
        .context("sending hello")?;
    stdin.flush().context("sending hello")?;
    let mut stdout = BufReader::new(stdout);
    let mut reply = String::new();
    stdout
        .read_line(&mut reply)
        .context("reading the reply to hello")?;
    match serde_json::from_str::<Message>(&reply) {
        Ok(Message::Response(_)) => {}
        _ => bail!("the engine did not accept hello: {}", reply.trim_end()),
    }
    std::thread::sleep(IDLE);
    drop(stdin);
    // Its output stays open until it exits, so ending never fails a write.
    std::io::copy(&mut stdout, &mut std::io::sink()).context("reading the engine's output")?;
    Ok(())
}
