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
use crate::test_tools::shipped_target;

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
    /// The Rust target `just targets` builds for, when not the host's own:
    /// the static musl one on Linux (ADR 0024).
    build_target: Option<String>,
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
        build_target: shipped_target(),
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
    let build = shipped_target().map_or_else(
        || "this host's own Rust target, the one the Products ship for here".to_owned(),
        |target| format!("`{target}`, the static binary the Products ship (ADR 0024)"),
    );
    let mut text = format!(
        "# Performance targets\n\n\
         `local-controlled`, never published (ADR 0076): measured on {platform}, on the builds \
         `just targets` makes with the release profile for {build}. The targets are ADR 0134's; a target never fails a change, \
         and the owner weighs each miss at the owner test.\n\n\
         | Target | Product | Figure | Executable's pages | Target value | Verdict | Gap |\n|---|---|---:|---:|---:|---|---:|\n"
    );
    for row in rows {
        let verdict = match row.verdict {
            Verdict::Met => "met",
            Verdict::Missed => "missed",
            Verdict::NotMeasured => "not measured",
        };
        let _ = writeln!(
            text,
            "| {} | {} | {} | {} | {} | {verdict} | {} |",
            row.title,
            row.product,
            row.figure.as_deref().unwrap_or("–"),
            row.executable_pages.as_deref().unwrap_or("–"),
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
         input ends. Every resident-memory figure, idle and the crawl's peaks, is the median of \
         the resident-memory check's scenario's peaks (`xtask rss measure`), each run once the \
         executable is dropped from the page cache; its executable's \
         pages are the median of the highest samples, on Linux, of the measured executable's own \
         file-backed resident pages (its mappings' `Rss` less `Anonymous` in `/proc/<pid>/smaps`), \
         the part of the figure that is its code and read-only data rather than what it allocated. \
         Each crawl peak's target, the bar of spec #2165, is Spider's median peak at the same size and \
         16 in flight in the sizes cell of the latest `just compare` report, and the crawl's \
         pages per second is Foxhound's median in scenario 1 of that report{}.\n",
        if measured.spider.is_some() {
            ""
        } else {
            ", which does not exist yet"
        }
    );
    text
}
