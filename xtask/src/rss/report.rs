//! What `xtask rss check` writes: the summary of each peak beside its
//! ceiling and its Performance target, the JUnit file whose test cases carry
//! the figures as properties, and the figures the drift check reads from the
//! results store (`perf-figures.json`, ADR 0071).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use quick_xml::escape::escape;

use crate::perf::{Figure, Figures, Recorded};

use super::ceilings::{self, Ceilings};
use super::peaks::Peak;
use super::platform::Platform;
use super::scenario::Scenario;

const MIB: u64 = 1024 * 1024;

/// One Product's scenario on one platform: its highest peak and its
/// ceiling, when one is committed.
pub(crate) struct Row {
    pub(crate) product: Product,
    pub(crate) platform: Platform,
    pub(crate) scenario: Scenario,
    pub(crate) peak: u64,
    pub(crate) ceiling: Option<u64>,
}

impl Row {
    pub(crate) fn subject(&self) -> String {
        ceilings::subject(self.product, self.platform, self.scenario)
    }

    fn over(&self) -> bool {
        self.ceiling.is_some_and(|ceiling| self.peak > ceiling)
    }
}

/// A row per Product, platform and scenario, holding the highest of its
/// peaks.
pub(crate) fn rows(peaks: &[Peak], ceilings: Option<&Ceilings>) -> Vec<Row> {
    let mut highest: BTreeMap<ceilings::Key, u64> = BTreeMap::new();
    for peak in peaks {
        let entry = highest
            .entry((peak.product, peak.platform, peak.scenario))
            .or_default();
        *entry = (*entry).max(peak.bytes);
    }
    highest
        .into_iter()
        .map(|(key, peak)| Row {
            product: key.0,
            platform: key.1,
            scenario: key.2,
            peak,
            ceiling: ceilings.and_then(|ceilings| ceilings.get(key)),
        })
        .collect()
}

/// Bytes in MiB with two decimals, rounded up, so a figure is never shown
/// under what was measured.
pub(crate) fn mebibytes(bytes: u64) -> String {
    let hundredths = (u128::from(bytes) * 100).div_ceil(u128::from(MIB));
    format!("{}.{:02} MiB", hundredths / 100, hundredths % 100)
}

pub(crate) fn summary(rows: &[Row], has_ceilings: bool, ceilings_file: &Path) -> String {
    let mut text = String::from("## Resident memory\n\n");
    if !has_ceilings {
        let _ = write!(
            text,
            "No ceilings file at `{}`: the check reports and gates nothing until the owner approves the ceilings (ADR 0071).\n\n",
            ceilings_file.display()
        );
    }
    text.push_str(
        "| Product | Scenario | Platform | Peak | Ceiling | Verdict | Performance target |\n",
    );
    text.push_str("|---|---|---|---:|---:|---|---|\n");
    for row in rows {
        let (ceiling, verdict) = match row.ceiling {
            Some(ceiling) if row.over() => (mebibytes(ceiling), "over"),
            Some(ceiling) => (mebibytes(ceiling), "under"),
            None => ("none".to_owned(), "not gated"),
        };
        let target = row.scenario.target_bytes();
        let against = if row.peak <= target {
            format!("{}: met", mebibytes(target))
        } else {
            format!(
                "{}: missed by {}",
                mebibytes(target),
                mebibytes(row.peak - target)
            )
        };
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {ceiling} | {verdict} | {against} |",
            row.product.as_str(),
            row.scenario.name(),
            row.platform.name(),
            mebibytes(row.peak),
        );
    }
    text.push_str(
        "\nA Performance target informs the owner test and never fails a change (ADR 0134).\n",
    );
    text
}

/// One test case per row, `rss <product> <scenario>`, failed when the peak
/// is over its ceiling; its properties hold the peak, the ceiling and the
/// target in bytes.
pub(crate) fn write_junit(path: &Path, rows: &[Row]) -> Result<()> {
    let failures = rows.iter().filter(|row| row.over()).count();
    let mut xml = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        xml,
        "<testsuites name=\"rss\" tests=\"{}\" failures=\"{failures}\">",
        rows.len()
    );
    let _ = writeln!(
        xml,
        "  <testsuite name=\"rss\" tests=\"{}\" failures=\"{failures}\">",
        rows.len()
    );
    for row in rows {
        let _ = writeln!(
            xml,
            "    <testcase classname=\"rss\" name=\"{} {}\" time=\"0\">",
            row.product.as_str(),
            row.scenario.name()
        );
        xml.push_str("      <properties>\n");
        let _ = writeln!(
            xml,
            "        <property name=\"platform\" value=\"{}\"/>",
            row.platform.name()
        );
        let _ = writeln!(
            xml,
            "        <property name=\"peak-bytes\" value=\"{}\"/>",
            row.peak
        );
        if let Some(ceiling) = row.ceiling {
            let _ = writeln!(
                xml,
                "        <property name=\"ceiling-bytes\" value=\"{ceiling}\"/>"
            );
        }
        let _ = writeln!(
            xml,
            "        <property name=\"target-bytes\" value=\"{}\"/>",
            row.scenario.target_bytes()
        );
        xml.push_str("      </properties>\n");
        if let Some(ceiling) = row.ceiling.filter(|_| row.over()) {
            let message = format!(
                "peak of {} over its ceiling of {}",
                mebibytes(row.peak),
                mebibytes(ceiling)
            );
            let _ = writeln!(
                xml,
                "      <failure message=\"{}\">{}</failure>",
                escape(&message),
                escape(&message)
            );
        }
        xml.push_str("    </testcase>\n");
    }
    xml.push_str("  </testsuite>\n</testsuites>\n");
    write(path, xml.as_bytes())
}

/// Each row as a resident-memory figure with its ceiling, the subject
/// `rss/<product>/<scenario>`.
pub(crate) fn write_figures(path: &Path, rows: &[Row]) -> Result<()> {
    let figures = Figures {
        figures: rows
            .iter()
            .map(|row| Recorded {
                subject: format!("rss/{}/{}", row.product.as_str(), row.scenario.name()),
                figure: Figure::ResidentMemory,
                value: row.peak,
                ceiling: row.ceiling,
            })
            .collect(),
    };
    write(path, &serde_json::to_vec_pretty(&figures)?)
}

fn write(path: &Path, contents: &[u8]) -> Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))
}
