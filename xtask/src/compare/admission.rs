//! `xtask compare admission` (#2318): admission's time per URL at scale,
//! Foxhound's Frontier at the Run state's default memory budget against
//! Spider 2.53.9 with its `disk` feature on and off. Each side admits the
//! same synthetic new URLs of one site, then each again, and checks every
//! answer, so each figure is an exact visited set's: a side that admits a
//! duplicate or refuses a new URL fails the run. Each side runs once per
//! count of URLs, as the only child of an `xtask rss probe`, for its peak
//! resident memory, with its temporary folder, where Foxhound's run file
//! and Spider's SQLite file go, inside the report's folder, on the disk
//! rather than in a memory-backed `/tmp`. The report gives each side's
//! nanoseconds per new URL and per duplicate and its peak, Foxhound's
//! budget, what its memory held and its lookups that read the run file
//! (`spilled_lookups`), and the links Spider kept in memory. It carries no
//! bar until the owner sets one, gates nothing and declares no Feature.
//!
//! Foxhound's side times the Frontier's own admission on the writer's
//! connection, not the reads on the run file's readers a crawl past its
//! budget makes before a links batch's job (#2169).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use clap::Args;
use serde::{Deserialize, Serialize};

use crate::environment::Environment;
use crate::output;
use crate::rss::report::mebibytes;

/// The report's files, in its folder.
const REPORT_JSON: &str = "admission.json";
const REPORT_MARKDOWN: &str = "admission.md";

#[derive(Args)]
pub(crate) struct AdmissionArguments {
    /// Foxhound's side: the Frontier's `admission` example.
    #[arg(long)]
    foxhound: PathBuf,
    /// Spider's side, `goodfetch-compare-spider`, built with its `disk`
    /// feature.
    #[arg(long)]
    spider: PathBuf,
    /// Spider's side built without its `disk` feature.
    #[arg(long)]
    spider_without_disk: PathBuf,
    /// The counts of new URLs each side admits, one run of each side per
    /// count.
    #[arg(long = "urls", default_values_t = [1_000_000_u64, 10_000_000])]
    urls: Vec<u64>,
    /// The sides to run; every side when none is given.
    #[arg(long = "side", value_enum)]
    sides: Vec<Side>,
    /// The folder the report is written to.
    #[arg(long, default_value = "target/compare/admission")]
    out: PathBuf,
}

/// One side of the comparison.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, clap::ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub(crate) enum Side {
    Foxhound,
    Spider,
    SpiderWithoutDisk,
}

impl Side {
    const ALL: [Self; 3] = [Self::Foxhound, Self::Spider, Self::SpiderWithoutDisk];

    fn label(self) -> &'static str {
        match self {
            Self::Foxhound => "Foxhound, default budget",
            Self::Spider => "Spider 2.53.9, `disk` on",
            Self::SpiderWithoutDisk => "Spider 2.53.9, `disk` off",
        }
    }
}

/// What a side writes to its `--out` file.
#[derive(Debug, Deserialize)]
struct Figures {
    error: Option<String>,
    urls: Option<u64>,
    added_ns_per_url: Option<f64>,
    duplicate_ns_per_url: Option<f64>,
    added: Option<u64>,
    duplicates: Option<u64>,
    budget_bytes: Option<u64>,
    memory_held_bytes: Option<u64>,
    spilled_lookups: Option<f64>,
    links_in_memory: Option<u64>,
}

/// One side's run at one count of URLs.
#[derive(Debug, PartialEq, Serialize)]
pub(crate) struct Row {
    side: Side,
    urls: u64,
    added_ns_per_url: f64,
    duplicate_ns_per_url: f64,
    peak_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    memory_held_bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    spilled_lookups: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    links_in_memory: Option<u64>,
}

pub(crate) fn run(arguments: &AdmissionArguments, environment: &Environment) -> Result<()> {
    let folder = environment.path(&arguments.out);
    std::fs::create_dir_all(&folder).with_context(|| format!("creating {}", folder.display()))?;
    let sides = if arguments.sides.is_empty() {
        Side::ALL.to_vec()
    } else {
        arguments.sides.clone()
    };
    let mut rows = Vec::new();
    for &urls in &arguments.urls {
        for &side in &sides {
            let executable = environment.path(match side {
                Side::Foxhound => &arguments.foxhound,
                Side::Spider => &arguments.spider,
                Side::SpiderWithoutDisk => &arguments.spider_without_disk,
            });
            let row = run_side(side, &executable, urls, &folder)?;
            output::line(&format!(
                "{} at {urls} URLs: {:.0} ns per new URL, {:.0} ns per duplicate, peak {} bytes",
                side.label(),
                row.added_ns_per_url,
                row.duplicate_ns_per_url,
                row.peak_bytes
            ));
            rows.push(row);
        }
    }
    let json = folder.join(REPORT_JSON);
    let text = serde_json::to_string_pretty(&rows).context("writing the report")?;
    std::fs::write(&json, text).with_context(|| format!("writing {}", json.display()))?;
    let markdown = folder.join(REPORT_MARKDOWN);
    std::fs::write(&markdown, report(&rows))
        .with_context(|| format!("writing {}", markdown.display()))?;
    output::line(&format!(
        "wrote {} and {}",
        json.display(),
        markdown.display()
    ));
    Ok(())
}

/// Runs `side`'s `executable` at `urls` new URLs under `xtask rss probe`,
/// its temporary folder a fresh one inside `folder`.
fn run_side(side: Side, executable: &Path, urls: u64, folder: &Path) -> Result<Row> {
    let temporary = tempfile::tempdir_in(folder).context("creating the side's temporary folder")?;
    let out = temporary.path().join("figures.json");
    let probe = std::env::current_exe().context("finding xtask's own executable")?;
    let mut command = Command::new(probe);
    command.args(["rss", "probe", "--"]).arg(executable);
    if side != Side::Foxhound {
        command.args(["--scenario", "admission"]);
    }
    command
        .args(["--urls", &urls.to_string()])
        .arg("--out")
        .arg(&out)
        .env("TMPDIR", temporary.path())
        .stderr(Stdio::inherit());
    let printed = command
        .output()
        .with_context(|| format!("running {}", executable.display()))?;
    let written = std::fs::read(&out).ok();
    if !printed.status.success() {
        let error = written
            .and_then(|text| serde_json::from_slice::<Figures>(&text).ok())
            .and_then(|figures| figures.error)
            .unwrap_or_else(|| format!("it exited with {}", printed.status));
        bail!("{} at {urls} URLs failed: {error}", side.label());
    }
    let written = written.with_context(|| format!("reading {}", out.display()))?;
    let peak_bytes = peak(&String::from_utf8_lossy(&printed.stdout))?;
    row(side, urls, &written, peak_bytes)
}

/// The peak resident bytes `xtask rss probe` printed.
fn peak(printed: &str) -> Result<u64> {
    printed
        .lines()
        .find_map(|line| line.strip_prefix("peak-bytes="))
        .context("xtask rss probe printed no peak")?
        .trim()
        .parse()
        .context("parsing xtask rss probe's peak")
}

/// `side`'s row from the figures it wrote, held to the run it was asked
/// for: every new URL added and every one refused again.
fn row(side: Side, urls: u64, written: &[u8], peak_bytes: u64) -> Result<Row> {
    let figures: Figures = serde_json::from_slice(written).context("parsing the side's figures")?;
    if let Some(error) = figures.error {
        bail!("{} at {urls} URLs failed: {error}", side.label());
    }
    if figures.urls != Some(urls) || figures.added != Some(urls) || figures.duplicates != Some(urls)
    {
        let count =
            |count: Option<u64>| count.map_or_else(|| "no".to_owned(), |count| count.to_string());
        bail!(
            "{} at {urls} URLs reported {} URLs, {} added and {} duplicates",
            side.label(),
            count(figures.urls),
            count(figures.added),
            count(figures.duplicates)
        );
    }
    Ok(Row {
        side,
        urls,
        added_ns_per_url: figures
            .added_ns_per_url
            .context("the side's figures give no time per new URL")?,
        duplicate_ns_per_url: figures
            .duplicate_ns_per_url
            .context("the side's figures give no time per duplicate")?,
        peak_bytes,
        budget_bytes: figures.budget_bytes,
        memory_held_bytes: figures.memory_held_bytes,
        spilled_lookups: figures.spilled_lookups,
        links_in_memory: figures.links_in_memory,
    })
}

/// The report's Markdown: one line per side and count of URLs.
fn report(rows: &[Row]) -> String {
    let mut text = String::from(
        "# Admission at scale (#2318)\n\n\
         Each side admits the same new URLs of one site, then each again; every answer is \
         checked, so the visited set is exact. One run per cell, on this machine, \
         with no bar until the owner sets one. Rerun with `just compare-admission`.\n\n\
         | Side | URLs | ns per new URL | ns per duplicate | Peak resident | Budget | \
         Memory held | `spilled_lookups` | Links in memory |\n\
         |---|---:|---:|---:|---:|---:|---:|---:|---:|\n",
    );
    for row in rows {
        let _ = writeln!(
            text,
            "| {} | {} | {:.0} | {:.0} | {} | {} | {} | {} | {} |",
            row.side.label(),
            row.urls,
            row.added_ns_per_url,
            row.duplicate_ns_per_url,
            mebibytes(row.peak_bytes),
            row.budget_bytes.map_or_else(|| "-".to_owned(), mebibytes),
            row.memory_held_bytes
                .map_or_else(|| "-".to_owned(), mebibytes),
            row.spilled_lookups
                .map_or_else(|| "-".to_owned(), |count| format!("{count:.0}")),
            row.links_in_memory
                .map_or_else(|| "-".to_owned(), |count| count.to_string()),
        );
    }
    text
}

#[cfg(test)]
mod tests;
