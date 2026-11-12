//! What `report` reads: hyperfine's export of each Product's startup, the
//! peaks `xtask rss measure` wrote, the latest `just compare`
//! report and the optimization pass's write-ups. A missing file is a
//! figure not measured yet, never an error.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use serde::Deserialize;
use serde::de::DeserializeOwned;

use crate::rss::crawl::CONCURRENCY;
use crate::rss::peaks::{self, Peak, parse_name};
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

/// One tool's median pages per second in scenario 1 of a `just compare`
/// report; the crawl's peaks are the resident-memory check's crawl
/// scenarios'.
#[derive(Deserialize)]
pub(crate) struct CrawlFigures {
    pub(crate) pages_per_second: f64,
}

#[derive(Deserialize)]
pub(crate) struct SpiderTools {
    pub(crate) foxhound: CrawlFigures,
    pub(crate) spider: CrawlFigures,
}

/// One tool's figures at one point of the sizes cell: its peak, when
/// measured.
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum SizeFigures {
    Measured { peak_bytes: u64 },
    NotMeasured {},
}

/// One size and concurrency of the sizes cell, each tool's figures by its
/// name.
#[derive(Deserialize)]
pub(crate) struct SizePoint {
    pub(crate) site_pages: u64,
    pub(crate) concurrency: u32,
    pub(crate) tools: BTreeMap<String, SizeFigures>,
}

/// The sizes cell of a `just compare` report.
#[derive(Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub(crate) enum Sizes {
    NotRun {},
    Measured { points: Vec<SizePoint> },
}

/// The parts of a `just compare` report the targets read.
#[derive(Deserialize)]
pub(crate) struct SpiderReport {
    pub(crate) commit: String,
    pub(crate) tools: SpiderTools,
    pub(crate) sizes: Sizes,
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
    /// The write-ups, by target name and Product.
    write_ups: BTreeMap<(String, Product), String>,
}

/// The write-ups file's shape: `[<target>.<product>]` tables, each holding
/// one `write-up`.
type WriteUps = BTreeMap<String, BTreeMap<String, WriteUp>>;

/// The write-ups in `text`, each key checked as the ceilings file's are:
/// the outer one among `targets`, the names of the targets, the inner one
/// a Product's mark. A name the file does not take is refused, never a
/// write-up dropped unseen.
fn write_ups(text: &str, targets: &[&str]) -> Result<BTreeMap<(String, Product), String>> {
    let file: WriteUps = toml::from_str(text)?;
    let mut write_ups = BTreeMap::new();
    for (target, products) in file {
        if !targets.contains(&target.as_str()) {
            bail!(
                "`{target}` is no name this file takes: it takes {}",
                targets.join(", ")
            );
        }
        for (product, write_up) in products {
            let product: Product = parse_name(&product)?;
            write_ups.insert((target.clone(), product), write_up.write_up);
        }
    }
    Ok(write_ups)
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

    /// The executable's own file-backed pages of each peak of `scenario` on
    /// `product` that has them, in bytes.
    pub(crate) fn executable_bytes(&self, scenario: Scenario, product: Product) -> Vec<u64> {
        self.peaks
            .iter()
            .filter(|peak| peak.product == product && peak.scenario == scenario)
            .filter_map(|peak| peak.executable_bytes)
            .collect()
    }

    /// Spider's median peak at `pages` pages and [`CONCURRENCY`] in flight
    /// in the latest `just compare` report, with its commit, if measured.
    pub(crate) fn spider_peak(&self, pages: u64) -> Option<(u64, &str)> {
        let report = self.spider.as_ref()?;
        let Sizes::Measured { points } = &report.sizes else {
            return None;
        };
        let point = points
            .iter()
            .find(|point| point.site_pages == pages && point.concurrency == CONCURRENCY)?;
        match point.tools.get("spider")? {
            SizeFigures::Measured { peak_bytes } => Some((*peak_bytes, report.commit.as_str())),
            SizeFigures::NotMeasured {} => None,
        }
    }

    /// The optimization pass's write-up of `target` on `product`.
    pub(crate) fn write_up(&self, target: &str, product: Product) -> Option<&str> {
        self.write_ups
            .get(&(target.to_owned(), product))
            .map(String::as_str)
    }
}

/// Reads what was measured; a file that does not exist is left out. The
/// write-ups file at `gaps` takes the target names `targets`.
pub(crate) fn read(
    folder: &Path,
    spider: &Path,
    gaps: &Path,
    targets: &[&str],
) -> Result<Measured> {
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
            write_ups(&text, targets).with_context(|| format!("parsing {}", gaps.display()))?
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
