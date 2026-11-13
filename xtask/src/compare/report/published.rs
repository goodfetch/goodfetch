//! The CPU per page crawlee-rs publishes for its own benchmark, written
//! beside the median measured here on each of its measured cells (#2214): a
//! figure from another machine and another site, read beside ours, never
//! compared with it.

use std::collections::BTreeMap;
use std::fmt::Write as _;

use super::Figure;
use crate::compare::matrix::Tool;

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
