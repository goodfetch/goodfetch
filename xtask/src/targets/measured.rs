//! What `report` reads: hyperfine's export of each Product's startup, the
//! peaks `xtask rss measure` wrote, the latest `just compare-spider`
//! report and the optimization pass's write-ups. A missing file is a
//! figure not measured yet, never an error.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::rss::peaks::{self, Peak};
use crate::rss::scenario::Scenario;

/// The peaks file in the measured folder.
pub(crate) const PEAKS: &str = "peaks.jsonl";

/// hyperfine's export of one Product's startup, in the measured folder.
pub(crate) fn startup_file(product: Product) -> String {
    format!("startup-{}.json", product.as_str())
}

/// One timed command of a hyperfine export, in either of its formats.
#[derive(Deserialize)]
#[serde(untagged)]
enum Timed {
    /// hyperfine 2's (its schema 2): a summary per metric.
    Summarized { summary: Summary },
    /// hyperfine 1's: the wall-clock figures at the top, in seconds.
    Flat { median: f64, times: Vec<f64> },
}

#[derive(Deserialize)]
struct Summary {
    time_wall_clock: Metric,
}

/// One metric's summary over the runs, in seconds for a time.
#[derive(Deserialize)]
struct Metric {
    median: f64,
    count: usize,
}

impl Timed {
    fn startup(self) -> Startup {
        match self {
            Self::Summarized { summary } => Startup {
                median_seconds: summary.time_wall_clock.median,
                runs: summary.time_wall_clock.count,
            },
            Self::Flat { median, times } => Startup {
                median_seconds: median,
                runs: times.len(),
            },
        }
    }
}

#[derive(Deserialize)]
struct HyperfineExport {
    results: Vec<Timed>,
}

/// A Product's startup to `hello`: hyperfine's median over its runs.
pub(crate) struct Startup {
    pub(crate) median_seconds: f64,
    pub(crate) runs: usize,
}

/// One tool's median pages per second in a `just compare-spider` report;
/// the crawl's peak is the resident-memory check's `crawl` scenario's.
#[derive(Deserialize)]
pub(crate) struct CrawlFigures {
    pub(crate) pages_per_second: f64,
}

#[derive(Deserialize)]
pub(crate) struct SpiderTools {
    pub(crate) foxhound: CrawlFigures,
    pub(crate) spider: CrawlFigures,
}

/// The parts of a `just compare-spider` report the targets read.
#[derive(Deserialize)]
pub(crate) struct SpiderReport {
    pub(crate) commit: String,
    pub(crate) tools: SpiderTools,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct WriteUp {
    #[serde(rename = "write-up")]
    write_up: String,
}

/// Everything measured, each part absent when not measured yet.
pub(crate) struct Measured {
    pub(crate) startup: BTreeMap<Product, Startup>,
    pub(crate) peaks: Vec<Peak>,
    pub(crate) spider: Option<SpiderReport>,
    /// The write-ups, by target name and Product mark.
    write_ups: BTreeMap<String, BTreeMap<String, WriteUp>>,
}

impl Measured {
    /// The peaks of `scenario` on `product`, in bytes.
    pub(crate) fn peaks(&self, scenario: Scenario, product: Product) -> Vec<u64> {
        self.peaks
            .iter()
            .filter(|peak| peak.product == product && peak.scenario == scenario)
            .map(|peak| peak.bytes)
            .collect()
    }

    /// The optimization pass's write-up of `target` on `product`.
    pub(crate) fn write_up(&self, target: &str, product: Product) -> Option<&str> {
        self.write_ups
            .get(target)?
            .get(product.as_str())
            .map(|write_up| write_up.write_up.as_str())
    }
}

/// Reads what was measured; a file that does not exist is left out.
pub(crate) fn read(folder: &Path, spider: &Path, gaps: &Path) -> Result<Measured> {
    let mut startup = BTreeMap::new();
    for product in [Product::Collie, Product::Foxhound] {
        let path = folder.join(startup_file(product));
        if let Some(export) = json::<HyperfineExport>(&path)? {
            let timed = export
                .results
                .into_iter()
                .next()
                .with_context(|| format!("{} holds no timed command", path.display()))?;
            startup.insert(product, timed.startup());
        }
    }
    let peaks_path = folder.join(PEAKS);
    let peaks = if peaks_path.exists() {
        peaks::read(&peaks_path)?
    } else {
        Vec::new()
    };
    let write_ups = match text(gaps)? {
        Some(text) => {
            toml::from_str(&text).with_context(|| format!("parsing {}", gaps.display()))?
        }
        None => BTreeMap::new(),
    };
    Ok(Measured {
        startup,
        peaks,
        spider: json(spider)?,
        write_ups,
    })
}

fn text(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

fn json<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    text(path)?
        .map(|text| {
            serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
        })
        .transpose()
}
