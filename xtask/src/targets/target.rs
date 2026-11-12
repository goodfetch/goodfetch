//! The Performance targets of ADR 0134 and each one's verdict on what was
//! measured: met at or under its target, missed over it with the gap, or
//! not measured yet. Only an amendment of ADR 0134 moves a target.

use goodfetch_protocol::catalog::Product;
use serde::Serialize;

use crate::rss::crawl::CONCURRENCY;
use crate::rss::scenario::Scenario;

use super::measured::Measured;

const MIB: u64 = 1024 * 1024;

/// Startup to `hello` of each Product's Attached engine, median (ADR 0134).
pub(crate) const STARTUP_TO_HELLO_SECONDS: f64 = 0.010;

/// A Performance target.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    StartupToHello,
    /// The resident-memory targets: each one's value is the check's own
    /// target for its scenario, [`Scenario::target_bytes`], and its figure
    /// the median of that scenario's peaks.
    IdleAttachedEngine,
    IdleDaemon,
    /// The bar (spec #2165): the peak of the resident-memory check's crawl
    /// of 100, 1,000 or 10,000 pages, at most Spider's at that size and 16
    /// in flight in the latest `just compare` report.
    CrawlPeak100,
    CrawlPeak1000,
    CrawlPeak10000,
    /// At least Spider's pages per second, in the same comparison run.
    CrawlPagesPerSecond,
}

impl Target {
    /// Each target with the Products it is measured on, in the report's
    /// order.
    pub(crate) const ALL: [(Self, &'static [Product]); 7] = [
        (Self::StartupToHello, &[Product::Collie, Product::Foxhound]),
        (
            Self::IdleAttachedEngine,
            &[Product::Collie, Product::Foxhound],
        ),
        (Self::IdleDaemon, &[Product::Collie, Product::Foxhound]),
        (Self::CrawlPeak100, &[Product::Foxhound]),
        (Self::CrawlPeak1000, &[Product::Foxhound]),
        (Self::CrawlPeak10000, &[Product::Foxhound]),
        (Self::CrawlPagesPerSecond, &[Product::Foxhound]),
    ];

    /// Its name in the report and in the write-ups file.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::StartupToHello => "startup-to-hello",
            Self::IdleAttachedEngine => "idle-attached-engine",
            Self::IdleDaemon => "idle-daemon",
            Self::CrawlPeak100 => "crawl-peak-100",
            Self::CrawlPeak1000 => "crawl-peak-1000",
            Self::CrawlPeak10000 => "crawl-peak-10000",
            Self::CrawlPagesPerSecond => "crawl-pages-per-second",
        }
    }

    /// The resident-memory check's scenario that measures it, if one does.
    fn scenario(self) -> Option<Scenario> {
        match self {
            Self::IdleAttachedEngine => Some(Scenario::IdleAttachedEngine),
            Self::IdleDaemon => Some(Scenario::IdleDaemon),
            Self::CrawlPeak100 => Some(Scenario::Crawl100),
            Self::CrawlPeak1000 => Some(Scenario::Crawl1000),
            Self::CrawlPeak10000 => Some(Scenario::Crawl10000),
            Self::StartupToHello | Self::CrawlPagesPerSecond => None,
        }
    }

    /// Its value, as the report states it before a figure is measured.
    fn stated(self) -> String {
        match (self, self.scenario()) {
            (Self::CrawlPeak100 | Self::CrawlPeak1000 | Self::CrawlPeak10000, _) => {
                "at most Spider's".to_owned()
            }
            (_, Some(scenario)) => mebibytes(scenario.target_bytes()),
            (Self::StartupToHello, None) => milliseconds(STARTUP_TO_HELLO_SECONDS),
            (_, None) => "at least Spider's".to_owned(),
        }
    }

    /// What it measures, as the report's table says it.
    pub(crate) fn title(self) -> &'static str {
        match self {
            Self::StartupToHello => "Startup to `hello`, Attached engine, median",
            Self::IdleAttachedEngine => "Idle RSS of an Attached engine after `hello`",
            Self::IdleDaemon => "Idle RSS of a Daemon with no run",
            Self::CrawlPeak100 => "Peak RSS of the 100-page HTTP crawl, 16 in flight",
            Self::CrawlPeak1000 => "Peak RSS of the 1,000-page HTTP crawl, 16 in flight",
            Self::CrawlPeak10000 => "Peak RSS of the 10,000-page HTTP crawl, 16 in flight",
            Self::CrawlPagesPerSecond => "Pages per second on the 1,000-page HTTP crawl",
        }
    }
}

/// A target's verdict.
#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Verdict {
    Met,
    Missed,
    NotMeasured,
}

/// One target on one Product, as the report shows it.
#[derive(Serialize)]
pub(crate) struct Row {
    pub(crate) target: &'static str,
    #[serde(skip)]
    pub(crate) title: &'static str,
    pub(crate) product: &'static str,
    /// The target's value.
    pub(crate) value: String,
    /// The measured figure, absent when not measured.
    pub(crate) figure: Option<String>,
    /// For a resident-memory target, how much of its figure is the
    /// executable's own file-backed pages: the median of the peaks' samples,
    /// absent where they were not sampled (outside Linux).
    pub(crate) executable_pages: Option<String>,
    pub(crate) verdict: Verdict,
    /// For a miss, how far the figure is from its target.
    pub(crate) gap: Option<String>,
    /// For a miss, the optimization pass's write-up of it, if there is one.
    pub(crate) write_up: Option<String>,
    /// Where the figure comes from, or why it was not measured.
    pub(crate) source: String,
}

/// A figure measured against its target.
enum Figure {
    Seconds {
        figure: f64,
        target: f64,
    },
    Bytes {
        figure: u64,
        target: u64,
    },
    /// Foxhound's peak against Spider's at the same size.
    PeakAgainstSpider {
        figure: u64,
        spider: u64,
    },
    /// Foxhound's pages per second against Spider's.
    PagesPerSecond {
        figure: f64,
        spider: f64,
    },
}

/// `bytes` in MiB, one decimal.
#[expect(
    clippy::cast_precision_loss,
    reason = "a resident set stays far below 2^52 bytes, where a float is exact"
)]
fn mebibytes(bytes: u64) -> String {
    format!("{:.1} MiB", bytes as f64 / MIB as f64)
}

fn milliseconds(seconds: f64) -> String {
    format!("{:.1} ms", seconds * 1000.0)
}

fn pages_per_second(rate: f64) -> String {
    format!("{rate:.1} pages per second")
}

impl Figure {
    fn value(&self) -> String {
        match *self {
            Self::Seconds { target, .. } => milliseconds(target),
            Self::Bytes { target, .. } => mebibytes(target),
            Self::PeakAgainstSpider { spider, .. } => {
                format!("at most Spider's {}", mebibytes(spider))
            }
            Self::PagesPerSecond { spider, .. } => {
                format!("at least Spider's {}", pages_per_second(spider))
            }
        }
    }

    fn figure(&self) -> String {
        match *self {
            Self::Seconds { figure, .. } => milliseconds(figure),
            Self::Bytes { figure, .. } | Self::PeakAgainstSpider { figure, .. } => {
                mebibytes(figure)
            }
            Self::PagesPerSecond { figure, .. } => pages_per_second(figure),
        }
    }

    /// The gap of a miss; none when the target is met.
    fn gap(&self) -> Option<String> {
        match *self {
            Self::Seconds { figure, target } => {
                (figure > target).then(|| milliseconds(figure - target))
            }
            Self::Bytes { figure, target }
            | Self::PeakAgainstSpider {
                figure,
                spider: target,
            } => (figure > target).then(|| mebibytes(figure - target)),
            Self::PagesPerSecond { figure, spider } => {
                (figure < spider).then(|| pages_per_second(spider - figure))
            }
        }
    }
}

/// The middle value, the mean of the middle two rounded down for an even
/// count; none for no value.
fn median(mut values: Vec<u64>) -> Option<u64> {
    values.sort_unstable();
    let middle = values.len() / 2;
    match values.len() {
        0 => None,
        length if length % 2 == 0 => Some(u64::midpoint(values[middle - 1], values[middle])),
        _ => Some(values[middle]),
    }
}

/// A target's figure on `product` with where it comes from, or why it was
/// not measured.
fn figure(target: Target, product: Product, measured: &Measured) -> (Option<Figure>, String) {
    let spider_source =
        |commit: &str| format!("scenario 1 of `just compare`'s report, commit `{commit}`");
    match target {
        Target::StartupToHello => match measured.startup.get(&product) {
            Some(startup) => (
                Some(Figure::Seconds {
                    figure: startup.median_seconds,
                    target: STARTUP_TO_HELLO_SECONDS,
                }),
                format!("hyperfine, median of {} runs", startup.runs),
            ),
            None => (None, "no hyperfine export; run `just targets`".to_owned()),
        },
        Target::IdleAttachedEngine
        | Target::IdleDaemon
        | Target::CrawlPeak100
        | Target::CrawlPeak1000
        | Target::CrawlPeak10000 => {
            let Some(scenario) = target.scenario() else {
                return (None, "no scenario measures it".to_owned());
            };
            let peaks = measured.peaks(scenario, product);
            let runs = peaks.len();
            let Some(bytes) = median(peaks) else {
                return (
                    None,
                    format!("no {} peak; run `just targets`", scenario.name()),
                );
            };
            let source = format!(
                "`xtask rss measure {}`, median of {runs} runs",
                scenario.name()
            );
            let Some(crawl) = scenario.crawl() else {
                return (
                    Some(Figure::Bytes {
                        figure: bytes,
                        target: scenario.target_bytes(),
                    }),
                    source,
                );
            };
            match measured.spider_peak(crawl.pages) {
                Some((spider, commit)) => (
                    Some(Figure::PeakAgainstSpider {
                        figure: bytes,
                        spider,
                    }),
                    format!(
                        "{source}; Spider's from the sizes cell of `just compare`'s report at {} pages and {CONCURRENCY} in flight, commit `{commit}`",
                        crawl.pages
                    ),
                ),
                None => (
                    None,
                    format!(
                        "{source} gave {}, but the latest `just compare` report has no Spider peak at {} pages and {CONCURRENCY} in flight",
                        mebibytes(bytes),
                        crawl.pages
                    ),
                ),
            }
        }
        Target::CrawlPagesPerSecond => match &measured.spider {
            Some(report) => (
                Some(Figure::PagesPerSecond {
                    figure: report.tools.foxhound.pages_per_second,
                    spider: report.tools.spider.pages_per_second,
                }),
                spider_source(&report.commit),
            ),
            None => (None, "no `just compare` report yet".to_owned()),
        },
    }
}

/// The median of the executable's own pages of `target`'s peaks on
/// `product`, for a resident-memory target whose peaks sampled them.
fn executable_pages(target: Target, product: Product, measured: &Measured) -> Option<String> {
    let scenario = target.scenario()?;
    median(measured.executable_bytes(scenario, product)).map(mebibytes)
}

/// Every target's row on each Product it is measured on.
pub(crate) fn rows(measured: &Measured) -> Vec<Row> {
    let mut rows = Vec::new();
    for (target, products) in Target::ALL {
        for &product in products {
            let (figure, source) = figure(target, product, measured);
            let gap = figure.as_ref().and_then(Figure::gap);
            let verdict = match (&figure, &gap) {
                (None, _) => Verdict::NotMeasured,
                (Some(_), None) => Verdict::Met,
                (Some(_), Some(_)) => Verdict::Missed,
            };
            let write_up = gap
                .as_ref()
                .and_then(|_| measured.write_up(target.name(), product))
                .map(str::to_owned);
            rows.push(Row {
                target: target.name(),
                title: target.title(),
                product: product.as_str(),
                value: figure
                    .as_ref()
                    .map_or_else(|| target.stated(), Figure::value),
                figure: figure.as_ref().map(Figure::figure),
                executable_pages: executable_pages(target, product, measured),
                verdict,
                gap,
                write_up,
                source,
            });
        }
    }
    rows
}
