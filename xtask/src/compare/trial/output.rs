//! What a tool left once it exited, which a trial reads afterwards:
//! Foxhound's Records, for their commit times and their pages, and on the
//! article site each tool's pages, held to the right answers the Proving
//! ground serves (#2136; see `extracted`).

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context as _, Result};

use super::{Mode, Plan};
use crate::compare::extracted::{self, Extracted, Written};
use crate::compare::foxhound::Record;
use crate::compare::matrix::Config;

/// The files Spider's and Scrapy's sides write the extraction scenario's
/// pages to, in the folder they run in.
const SPIDER_PAGES: &str = "pages.jsonl";
const SCRAPY_ITEMS: &str = "items.jsonl";

impl Plan<'_> {
    /// Foxhound's Records, read by an engine of their own on `home` once
    /// the measured processes are gone.
    pub(super) fn records(&self, home: &Path) -> Result<Vec<Record>> {
        let mut driven = self.spawn(home, 1, &Mode::Timed, None)?;
        let engine = driven.engine.as_mut().context("Foxhound's engine")?;
        let read = engine.records(&self.site);
        if let Some(engine) = driven.engine.take() {
            engine.close();
        }
        let _ = driven.child.wait();
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
            Config::Spider { .. } => extracted::read(Written::Lines(SPIDER_PAGES), home)?,
            Config::Scrapy => extracted::read(Written::Lines(SCRAPY_ITEMS), home)?,
            Config::Crawlee | Config::CrawleeRs => extracted::read(Written::Dataset, home)?,
        };
        let answers = extracted::answers(self.site.address, self.site.pages)?;
        Ok(Some(extracted::check(&answers, &pages)))
    }
}

/// Foxhound's Records' commit times, in seconds from `started`, by path:
/// each page's first.
pub(super) fn commit_times(
    records: &[Record],
    started: jiff::Timestamp,
) -> Result<HashMap<String, f64>> {
    let mut times = HashMap::new();
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
        times
            .entry(path)
            .and_modify(|first: &mut f64| *first = first.min(seconds))
            .or_insert(seconds);
    }
    Ok(times)
}
