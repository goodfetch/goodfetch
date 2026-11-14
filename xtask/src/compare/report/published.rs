//! What a tool publishes of its own benchmark, written on its measured
//! cells beside what is measured here, never compared with it: the CPU per
//! page crawlee-rs publishes (#2214), a figure from another machine and
//! another site, and the buffer pagesift's benchmark reads Spider's pages
//! with (#2215), which is not the one the spider row here reads them with.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::Figure;
use crate::compare::matrix::{CELLS, Config, Row, Scenario, Status, Tool};
use crate::compare::trials::PAGESIFT_VERSION;

/// crawlee-rs's published CPU per page, with what it was measured on.
pub(crate) const CRAWLEE_RS_CPU: &str = "1.32 ms published by crawlee-rs for its `HtmlCrawler` (its README's \
     Benchmark: 5,000 pages of 58 KB product listings from its own fixture \
     server, each parsed for its title and 20 products, on 4 vCPUs with 50 \
     requests at once and its storage in memory)";

/// On a crawlee-rs cell, the line that puts [`CRAWLEE_RS_CPU`] beside the
/// median measured here: seconds per 1,000 pages are milliseconds per page.
pub(super) fn cpu(text: &mut String, tool: &str, metrics: &BTreeMap<&'static str, Figure>) {
    if tool != Tool::CrawleeRs.name() {
        return;
    }
    let measured = match metrics.get("cpu_seconds_per_1000_pages") {
        Some(Figure::Measured { median, .. }) => {
            format!("{median:.2} ms measured here, the median of its trials")
        }
        Some(Figure::NotMeasured { reason }) => format!("not measured here: {reason}"),
        None => "not measured here".to_owned(),
    };
    let _ = writeln!(text, "  - CPU per page: {measured}; {CRAWLEE_RS_CPU}.");
}

/// The capacity pagesift's own crawl benchmark subscribes to each site's
/// pages with, where it runs Spider (`site.subscribe(256)`).
const PAGESIFT_SPIDER_CAPACITY: usize = 256;

/// On a pagesift cell of a scenario whose spider row is measured, the line
/// that puts the buffer pagesift's own benchmark reads Spider's pages with
/// beside the capacity that row reads them at.
pub(super) fn spider_buffer(text: &mut String, tool: &str, scenario: Scenario) {
    if tool != Tool::Pagesift.name() {
        return;
    }
    let capacity = CELLS.iter().find_map(|cell| match cell.status {
        Status::Measured(Config::Spider { capacity, .. })
            if cell.scenario == scenario && cell.row == Row::Main =>
        {
            Some(capacity)
        }
        Status::Measured(_) | Status::NotSupported(_) | Status::Pending(_) => None,
    });
    let Some(capacity) = capacity else {
        return;
    };
    let _ = writeln!(
        text,
        "  - Spider beside it: pagesift's own crawl benchmark reads Spider's pages with \
         `subscribe({PAGESIFT_SPIDER_CAPACITY})` on each site \
         (`benchmarks/crawl/src/bin/spider_crawl.rs` at its release {PAGESIFT_VERSION}); the \
         spider row here reads them with `subscribe({capacity})`, the smallest capacity that \
         lost no page here (#2213). Its published figures against Spider are that benchmark's, \
         never compared with these."
    );
}
