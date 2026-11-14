//! Measuring: each scenario on each Product's executable it runs on, each
//! run in a probe, `xtask rss probe` started anew, whose only child is the
//! executable, so the OS's figure is that executable's alone. The crawls'
//! Proving ground is served from here, outside every probe, and each
//! executable is dropped from the page cache before each run, so the run
//! pages it in as a first start does (spec #2165).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::ffi::OsString;
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;

use crate::output;
use crate::test_tools::{Tool, ToolManifest};

use super::crawl::{self, Site};
use super::page_cache;
use super::peak::{self, Resident};
use super::peaks::{self, Peak};
use super::platform::Platform;
use super::scenario::Scenario;

/// Runs each scenario `runs` times on each executable it runs on and
/// appends each peak to `out`.
pub(crate) fn measure(scenarios: &[Scenario], tools: &Path, out: &Path, runs: u32) -> Result<()> {
    let tools = ToolManifest::read(tools)?;
    let platform = Platform::current()?;
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    if !page_cache::DROPS {
        output::line(page_cache::NOT_DROPPED);
    }
    // Every executable a scenario runs on is measured, or none is.
    let mut measured = Vec::new();
    for &scenario in scenarios {
        if !scenario.runs_here() {
            output::line(&format!(
                "{} runs on Linux alone, whose allocator `governor.huge_pages` changes: not run on {}",
                scenario.name(),
                platform.name()
            ));
            continue;
        }
        for &product in scenario.products() {
            let Some(executable) = tools.get(Tool::of_product(product)) else {
                bail!(
                    "the test tool manifest names no executable of {}, which {} runs on",
                    product.name(),
                    scenario.name()
                );
            };
            measured.push((scenario, product, executable));
        }
    }
    let crawling = measured
        .iter()
        .find(|(scenario, ..)| scenario.crawl().is_some());
    let serving = if let Some((scenario, ..)) = crawling {
        let proving_ground = tools.get(Tool::ProvingGround).with_context(|| {
            format!(
                "the test tool manifest names no Proving ground, which {} crawls",
                scenario.name()
            )
        })?;
        Some(crawl::serve(proving_ground)?)
    } else {
        None
    };
    let address = serving.as_ref().map(|serving| serving.address.as_str());
    for run in 1..=runs {
        for &(scenario, product, executable) in &measured {
            let address = address.filter(|_| scenario.crawl().is_some());
            page_cache::drop_cached(executable)?;
            let resident = run_probe(&probe, scenario, product, executable, address)?;
            let bytes = resident.peak_bytes;
            output::line(&format!(
                "{} {} {}, run {run} of {runs}: {bytes} bytes{}",
                product.as_str(),
                scenario.name(),
                platform.name(),
                resident
                    .executable_bytes
                    .map(|bytes| format!(", {bytes} of them the executable's own pages"))
                    .unwrap_or_default()
            ));
            peaks::append(
                out,
                &Peak {
                    product,
                    platform,
                    scenario,
                    bytes,
                    executable_bytes: resident.executable_bytes,
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
) -> Result<Resident> {
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
    let figure = |name: &str| {
        stdout
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix('='))
            .map(|figure| {
                figure
                    .parse::<u64>()
                    .with_context(|| format!("parsing the probe's {name}"))
            })
            .transpose()
    };
    Ok(Resident {
        peak_bytes: figure("peak-bytes")?.context("xtask rss probe printed no peak")?,
        executable_bytes: figure("executable-bytes")?,
    })
}

/// Runs `command` as this process's only child, driven by the scenario if
/// one is given, the crawl against the Proving ground at `address`, and
/// returns its peak resident memory in bytes, with its executable's own
/// pages. A scenario's child runs
/// without `goodfetch_variables`, this process's Goodfetch variables.
pub(crate) fn probe(
    scenario: Option<(Scenario, Product)>,
    address: Option<&str>,
    command: &[OsString],
    goodfetch_variables: &[OsString],
) -> Result<Resident> {
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
    scenario.configure(home.path())?;
    super::scenario::as_shipped(&mut child, home.path(), goodfetch_variables);
    child.stdin(Stdio::piped()).stdout(Stdio::piped());
    peak::of(&mut child, |child| {
        let site = address.zip(scenario.crawl()).map(|(address, crawl)| Site {
            address,
            pages: crawl.pages,
            concurrency: crawl::CONCURRENCY,
        });
        scenario.drive(needs, site.as_ref(), child)
    })
}
