//! What a tool left once it exited, which a trial reads afterwards:
//! Foxhound's Records, for their commit times, their samples and their
//! pages, and on the article site each tool's pages, held to the right answers the Proving
//! ground serves (#2136; see `extracted`); and what a counted run on that
//! site changes so that every page is left ([`traced`]).

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::process::{Command, Stdio};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::foxhound::crawl::CODE_FREE_DATASET;
use serde_json::Value;

use super::{Mode, Plan};
use crate::compare::extracted::{self, Extracted, Written};
use crate::compare::foxhound::{self, Record};
use crate::compare::matrix::{Cell, Config, OutputCheck};

/// `cell`'s configuration as a run in `mode` on the site of `pages` pages
/// gives it to its tool. On the article site Spider's side reads each page
/// in a task of its own, and strace and Callgrind slow those tasks
/// unevenly: at the row's capacity, 5 of 20 crawls of the 1,000-page site
/// lost pages under strace, on the gate host on 2026-11-14, and a crawl
/// that loses one fails its run. A counted run there gives Spider's page
/// channel the whole site, where no page can be lost: it reads counts, not
/// the memory the channel holds.
pub(super) fn traced(cell: &Cell, config: Config, mode: &Mode, pages: u64) -> Config {
    match (config, mode) {
        (Config::Spider { build, capacity }, Mode::Syscalls(_) | Mode::Callgrind(_))
            if cell.output_check().is_some() =>
        {
            Config::Spider {
                build,
                capacity: capacity.max(usize::try_from(pages).unwrap_or(usize::MAX)),
            }
        }
        (
            config,
            Mode::Timed | Mode::Syscalls(_) | Mode::Callgrind(_) | Mode::Rules | Mode::Auto,
        ) => config,
    }
}

/// The files Spider's, pagesift's, OpenSpider's, fastCRW's and crawlberg's
/// sides and Scrapy's write the extraction scenario's pages to, in the
/// folder they run in.
const SPIDER_PAGES: &str = "pages.jsonl";
const SCRAPY_ITEMS: &str = "items.jsonl";

impl Plan<'_> {
    /// Foxhound's Records, read by an engine of their own on `home` once
    /// the measured processes are gone: the code-free crawl's Dataset, or
    /// the one Foxhound's program pushes to.
    pub(super) fn records(&self, home: &Path) -> Result<Vec<Record>> {
        let (mut child, mut engine, dataset) = match self.config {
            Config::FoxhoundProgram => {
                let executable = self
                    .engine
                    .context("Foxhound's program takes Foxhound's executable, --engine")?;
                let mut command = Command::new(executable);
                command
                    .arg(goodfetch_protocol::cli::ATTACHED_ENGINE)
                    .env(goodfetch_protocol::settings::HOME_VARIABLE, home)
                    .stdin(Stdio::piped())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::inherit());
                for name in self.goodfetch_variables {
                    command.env_remove(name);
                }
                let mut child = command.spawn().context("starting Foxhound's engine")?;
                let engine = super::opened(&mut child)?;
                (child, engine, foxhound::PROGRAM_DATASET)
            }
            _ => {
                let mut driven = self.spawn(home, 1, &Mode::Timed, None)?;
                let engine = driven.engine.take().context("Foxhound's engine")?;
                (driven.child, engine, CODE_FREE_DATASET)
            }
        };
        let read = engine.records(&self.site, dataset);
        engine.close();
        let _ = child.wait();
        read
    }

    /// Whether the trial's pages are held to the output check: on the
    /// article site, while the Proving ground serves its right answers.
    pub(super) fn checks_output(&self) -> bool {
        self.cell.output_check().is_some() && self.site.pages <= extracted::ANSWERED_PAGES
    }

    /// What the output check finds of the pages the tool left in `home`,
    /// Foxhound's being its `records`; none where no check holds them.
    pub(super) fn extracted(
        &self,
        home: &Path,
        records: Option<Vec<Record>>,
    ) -> Result<Option<Extracted>> {
        if !self.checks_output() {
            return Ok(None);
        }
        let answers = extracted::answers(self.site.address, self.site.pages)?;
        if let Some(OutputCheck::Fields) = self.cell.output_check() {
            let pages = match self.config {
                Config::FoxhoundProgram => records
                    .context("Foxhound's program keeps its Records")?
                    .into_iter()
                    .map(|record| {
                        let Value::Object(fields) = record.value else {
                            bail!("a Record of {} holds no fields", record.url);
                        };
                        Ok(extracted::Fielded {
                            url: record.url,
                            fields,
                        })
                    })
                    .collect::<Result<_>>()?,
                config => extracted::read(written(config)?, home)?,
            };
            return Ok(Some(extracted::check_fields(&answers, &pages)));
        }
        let pages = match self.config {
            Config::Foxhound { .. } => records
                .context("Foxhound's extraction crawl keeps its Records")?
                .into_iter()
                .map(|record| extracted::Page {
                    url: record.url,
                    title: record.title,
                    text: record.text,
                })
                .collect(),
            config => extracted::read(written(config)?, home)?,
        };
        Ok(Some(extracted::check(&answers, &pages)))
    }
}

/// Where `config`'s side writes the pages it extracts, in the folder it
/// runs in: Foxhound's are its Records.
fn written(config: Config) -> Result<Written> {
    Ok(match config {
        Config::Spider { .. }
        | Config::Pagesift
        | Config::OpenSpider
        | Config::FastCrw
        | Config::Crawlberg => Written::Lines(SPIDER_PAGES),
        Config::Scrapy => Written::Lines(SCRAPY_ITEMS),
        Config::Crawlee | Config::CrawleeRs => Written::Dataset,
        Config::Foxhound { .. } | Config::FoxhoundProgram => {
            bail!("Foxhound's pages are its Records, not a file")
        }
    })
}

/// What a trial reads of Foxhound's Records.
pub(super) struct Read {
    /// Each page's first Record's commit time, in seconds from the trial's
    /// start, by path.
    pub(super) times: HashMap<String, f64>,
    /// The pages whose Record came from a sample's browser run.
    pub(super) sample_pages: u64,
}

/// Foxhound's Records' commit times, in seconds from `started`, by path,
/// and the pages whose Record came from a sample's browser run.
pub(super) fn read(records: &[Record], started: jiff::Timestamp) -> Result<Read> {
    let mut times = HashMap::new();
    let mut samples = HashSet::new();
    for record in records {
        let committed: jiff::Timestamp = record
            .committed_at
            .parse()
            .with_context(|| format!("a Record's commit time: {}", record.committed_at))?;
        let seconds = committed.duration_since(started).as_secs_f64();
        let path = record
            .url
            .split_once("://")
            .and_then(|(_, rest)| rest.find('/').map(|slash| rest[slash..].to_owned()))
            .unwrap_or_else(|| record.url.clone());
        if record.sample {
            samples.insert(path.clone());
        }
        times
            .entry(path)
            .and_modify(|first: &mut f64| *first = first.min(seconds))
            .or_insert(seconds);
    }
    Ok(Read {
        times,
        sample_pages: samples.len() as u64,
    })
}
