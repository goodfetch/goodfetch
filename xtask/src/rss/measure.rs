//! Measuring: each scenario on each Product's executable it runs on, each
//! run in a probe, `xtask rss probe` started anew, whose only child is the
//! executable, so the OS's figure is that executable's alone. The crawl's
//! Proving ground is served from here, outside every probe.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use serde::Deserialize;

use crate::output;

use super::crawl::{self, Site};
use super::peak;
use super::peaks::{self, Peak};
use super::platform::Platform;
use super::scenario::Scenario;

/// The entries of the test tool manifest the scenarios run.
#[derive(Deserialize)]
struct Tools {
    collie: Option<PathBuf>,
    foxhound: Option<PathBuf>,
    proving_ground: Option<PathBuf>,
}

impl Tools {
    fn executable(&self, product: Product) -> Option<&Path> {
        match product {
            Product::Collie => self.collie.as_deref(),
            Product::Foxhound => self.foxhound.as_deref(),
        }
    }
}

/// Runs each scenario `runs` times on each executable it runs on and
/// appends each peak to `out`.
pub(crate) fn measure(scenarios: &[Scenario], tools: &Path, out: &Path, runs: u32) -> Result<()> {
    let text = std::fs::read(tools).with_context(|| format!("reading {}", tools.display()))?;
    let tools: Tools =
        serde_json::from_slice(&text).with_context(|| format!("parsing {}", tools.display()))?;
    let platform = Platform::current()?;
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    // Every executable a scenario runs on is measured, or none is.
    let mut measured = Vec::new();
    for &scenario in scenarios {
        for &product in scenario.products() {
            let Some(executable) = tools.executable(product) else {
                bail!(
                    "the test tool manifest names no executable of {}, which {} runs on",
                    product.name(),
                    scenario.name()
                );
            };
            measured.push((scenario, product, executable));
        }
    }
    let serving = if scenarios.iter().any(|scenario| scenario.crawls()) {
        let proving_ground = tools.proving_ground.as_deref().with_context(|| {
            format!(
                "the test tool manifest names no Proving ground, which {} crawls",
                Scenario::Crawl.name()
            )
        })?;
        Some(crawl::serve(proving_ground)?)
    } else {
        None
    };
    let address = serving.as_ref().map(|serving| serving.address.as_str());
    for run in 1..=runs {
        for &(scenario, product, executable) in &measured {
            let address = address.filter(|_| scenario.crawls());
            let bytes = run_probe(&probe, scenario, product, executable, address)?;
            output::line(&format!(
                "{} {} {}, run {run} of {runs}: {bytes} bytes",
                product.as_str(),
                scenario.name(),
                platform.name()
            ));
            peaks::append(
                out,
                &Peak {
                    product,
                    platform,
                    scenario,
                    bytes,
                },
            )?;
        }
    }
    Ok(())
}

fn run_probe(
    probe: &Path,
    scenario: Scenario,
    product: Product,
    executable: &Path,
    address: Option<&str>,
) -> Result<u64> {
    let output = Command::new(probe)
        .args([
            "rss",
            "probe",
            "--scenario",
            scenario.name(),
            "--needs",
            product.as_str(),
        ])
        .args(address.iter().flat_map(|address| ["--address", address]))
        .arg("--")
        .arg(executable)
        .args(scenario.arguments())
        .stderr(Stdio::inherit())
        .output()
        .context("running xtask rss probe")?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    if !output.status.success() {
        bail!(
            "xtask rss probe of {} on {} failed",
            scenario.name(),
            product.name()
        );
    }
    stdout
        .lines()
        .find_map(|line| line.strip_prefix("peak-bytes="))
        .context("xtask rss probe printed no peak")?
        .parse()
        .context("parsing the probe's peak")
}

/// Runs `command` as this process's only child, driven by the scenario if
/// one is given, the crawl against the Proving ground at `address`, and
/// returns its peak resident memory in bytes.
pub(crate) fn probe(
    scenario: Option<(Scenario, Product)>,
    address: Option<&str>,
    command: &[OsString],
) -> Result<u64> {
    let (program, arguments) = command.split_first().context("the probe needs a command")?;
    let mut child = Command::new(program);
    child.args(arguments).stderr(Stdio::null());
    let Some((scenario, needs)) = scenario else {
        child.stdin(Stdio::null()).stdout(Stdio::null());
        return peak::of(&mut child, |_| Ok(()));
    };
    // A scenario measures the executable as shipped, with its own data
    // folder and its default logging.
    let home = tempfile::tempdir().context("creating the engine's data folder")?;
    super::scenario::as_shipped(&mut child, home.path());
    child.stdin(Stdio::piped()).stdout(Stdio::piped());
    peak::of(&mut child, |child| {
        let stdin = child.stdin.take().context("the engine's stdin is piped")?;
        let stdout = child
            .stdout
            .take()
            .context("the engine's stdout is piped")?;
        let site = address.map(|address| Site {
            address,
            pages: crawl::PAGES,
            concurrency: crawl::CONCURRENCY,
        });
        scenario.drive(needs, site.as_ref(), stdin, stdout)
    })
}
