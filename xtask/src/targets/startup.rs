//! Startup to `hello` (ADR 0134): hyperfine times each Product's Attached
//! engine, started with no shell, from its start until it exits once it
//! has answered the `hello` on its input and the input has ended, after the
//! declared warm-up runs. The engine runs as shipped: its own data folder,
//! its default logging.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use serde::Deserialize;

use crate::output;
use crate::rss::scenario::hello_line;

use super::measured::startup_file;

/// hyperfine's warm-up runs before it times any.
pub(crate) const WARMUP: u32 = 10;
/// hyperfine's timed runs of each Product.
pub(crate) const RUNS: u32 = 100;

/// The entries of the test tool manifest startup runs.
#[derive(Deserialize)]
struct Tools {
    collie: Option<PathBuf>,
    foxhound: Option<PathBuf>,
}

/// Times each Product's Attached engine and writes hyperfine's export of
/// each into `out`.
pub(crate) fn measure(tools: &Path, out: &Path) -> Result<()> {
    let text = std::fs::read(tools).with_context(|| format!("reading {}", tools.display()))?;
    let tools: Tools =
        serde_json::from_slice(&text).with_context(|| format!("parsing {}", tools.display()))?;
    std::fs::create_dir_all(out).with_context(|| format!("creating {}", out.display()))?;
    for (product, executable) in [
        (Product::Collie, tools.collie),
        (Product::Foxhound, tools.foxhound),
    ] {
        let Some(executable) = executable else {
            bail!(
                "the test tool manifest names no executable of {}",
                product.name()
            );
        };
        let hello = out.join(format!("hello-{}.jsonl", product.as_str()));
        std::fs::write(&hello, hello_line(product)?)
            .with_context(|| format!("writing {}", hello.display()))?;
        let home = tempfile::tempdir().context("creating the engine's data folder")?;
        output::line(&format!(
            "timing {}'s startup to hello: {WARMUP} warm-up runs, then {RUNS} runs",
            product.name()
        ));
        let status = Command::new("hyperfine")
            .args(["--shell=none", "--style", "basic", "--warmup"])
            .arg(WARMUP.to_string())
            .arg("--runs")
            .arg(RUNS.to_string())
            .arg("--input")
            .arg(&hello)
            .arg("--export-json")
            .arg(out.join(startup_file(product)))
            .arg("--command-name")
            .arg(format!("{} attached engine to hello", product.as_str()))
            .arg(format!("{} attached-engine", quoted(&executable)))
            .env("GOODFETCH_HOME", home.path())
            .env_remove("GOODFETCH_LOG")
            .stdin(Stdio::null())
            .stderr(Stdio::inherit())
            .status();
        match status {
            Ok(status) if status.success() => {}
            Ok(status) => bail!("hyperfine timing {} exited with {status}", product.name()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                bail!("hyperfine is not installed: install it (`brew install hyperfine`)")
            }
            Err(error) => return Err(error).context("running hyperfine"),
        }
    }
    Ok(())
}

/// `path` as one word of hyperfine's command line, which it splits as a
/// POSIX shell would without running one.
fn quoted(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', r"'\''"))
}
