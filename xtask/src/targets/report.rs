//! The report: `report.json` and `report.md`, each target, its figure and
//! its verdict, labelled `local-controlled` and never published (ADR 0076),
//! for the owner test's Reports (ADR 0134).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Serialize;

use crate::rss::platform::Platform;

use super::measured::Measured;
use super::startup::{RUNS, WARMUP};
use super::target::{Row, Verdict};

/// The report, as `report.json` holds it.
#[derive(Serialize)]
struct Report<'a> {
    label: &'static str,
    published: bool,
    adr: &'static str,
    platform: &'static str,
    rows: &'a [Row],
}

/// Writes `report.json` and `report.md` into `folder` and returns the
/// Markdown.
pub(crate) fn write(folder: &Path, rows: &[Row], measured: &Measured) -> Result<String> {
    let platform = Platform::current()?.name();
    let report = Report {
        label: "local-controlled",
        published: false,
        adr: "0134",
        platform,
        rows,
    };
    std::fs::create_dir_all(folder).with_context(|| format!("creating {}", folder.display()))?;
    let json = folder.join("report.json");
    std::fs::write(&json, serde_json::to_vec_pretty(&report)?)
        .with_context(|| format!("writing {}", json.display()))?;
    let markdown = markdown(platform, rows, measured);
    let path = folder.join("report.md");
    std::fs::write(&path, &markdown).with_context(|| format!("writing {}", path.display()))?;
    Ok(markdown)
}

fn markdown(platform: &str, rows: &[Row], measured: &Measured) -> String {
    let mut text = format!(
        "# Performance targets\n\n\
         `local-controlled`, never published (ADR 0076): measured on {platform}, on the builds \
         `just targets` makes with the release profile. The targets are ADR 0134's; a target never fails a change, \
         and the owner weighs each miss at the owner test.\n\n\
         | Target | Product | Figure | Target value | Verdict | Gap |\n|---|---|---:|---:|---|---:|\n"
    );
    for row in rows {
        let verdict = match row.verdict {
            Verdict::Met => "met",
            Verdict::Missed => "missed",
            Verdict::NotMeasured => "not measured",
        };
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {verdict} | {} |",
            row.title,
            row.product,
            row.figure.as_deref().unwrap_or("–"),
            row.value,
            row.gap.as_deref().unwrap_or("–"),
        );
    }
    let misses: Vec<&Row> = rows
        .iter()
        .filter(|row| matches!(row.verdict, Verdict::Missed))
        .collect();
    if !misses.is_empty() {
        text.push_str("\n## Misses\n");
        for row in misses {
            let _ = write!(
                text,
                "\n### {}, {}\n\n{} over its target of {}.\n\n{}\n",
                row.title,
                row.product,
                row.gap.as_deref().unwrap_or_default(),
                row.value,
                row.write_up
                    .as_deref()
                    .unwrap_or("No write-up from the optimization pass yet."),
            );
        }
    }
    text.push_str("\n## Sources\n\n");
    for row in rows {
        let _ = writeln!(text, "- {}, {}: {}.", row.title, row.product, row.source);
    }
    let _ = write!(
        text,
        "\nStartup to `hello` is hyperfine's median after {WARMUP} warm-up runs and {RUNS} runs, \
         each from the engine's start to its exit once `hello` on its input is answered and its \
         input ends. Idle RSS is the resident-memory check's scenario. The crawl figures are \
         Foxhound's medians in the latest `just compare-spider` report{}.\n",
        if measured.spider.is_some() {
            ""
        } else {
            ", which does not exist yet"
        }
    );
    text
}
