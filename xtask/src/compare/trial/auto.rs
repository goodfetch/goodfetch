//! An auto cell's run (#2411): its tool crawls the rungs site from its
//! root, in a fresh data folder, to its end; then each page's Records, as
//! the tool read them, are held to the site's manifest, and the Proving
//! ground's observations show whether it requested every page
//! ([`auto::judged`]). Foxhound's are its Records, each counted rendered
//! when its Provenance names a browser tier; another tool writes each page
//! it read to `pages.jsonl` in its data folder. A tool that exits with a
//! failure fails the run.

use std::path::Path;

use anyhow::{Context as _, Result, bail};

use super::{Mode, Plan, Route, now, observations};
use crate::compare::auto::{self, Judged, Manifest, Read, Record};
use crate::compare::foxhound;
use crate::compare::matrix::{Config, Site};

/// The file a tool other than Foxhound writes each page it read to, one
/// JSON line of [`Read`] a page, in the folder it runs in.
const PAGES: &str = "pages.jsonl";

/// The tiers on which Foxhound renders a page in a browser.
const RENDERED_TIERS: [&str; 2] = ["browser", "light_browser"];

/// The rungs site's manifest, from the Proving ground at `address`.
fn manifest(address: &str, port: &str) -> Result<Manifest> {
    ureq::get(&format!("http://{address}/manifest.json"))
        .header("Host", &format!("{}:{port}", Site::Rungs.host_of(1)))
        .call()
        .context("reading the rungs site's manifest")?
        .body_mut()
        .read_json()
        .context("parsing the rungs site's manifest")
}

/// Foxhound's Records as the pages it read: a Record with neither field
/// holds none of the site's.
impl From<foxhound::Record> for Read {
    fn from(record: foxhound::Record) -> Self {
        let rendered = record
            .tier
            .as_deref()
            .is_some_and(|tier| RENDERED_TIERS.contains(&tier));
        let records = match (record.id, record.name) {
            (None, None) => Vec::new(),
            (id, name) => vec![Record {
                id: id.unwrap_or_default(),
                name: name.unwrap_or_default(),
            }],
        };
        Self {
            url: record.url,
            rendered,
            records,
        }
    }
}

/// The pages a tool other than Foxhound wrote to `home`'s `pages.jsonl`.
fn written(home: &Path) -> Result<Vec<Read>> {
    let file = home.join(PAGES);
    let text =
        std::fs::read_to_string(&file).with_context(|| format!("reading {}", file.display()))?;
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line).with_context(|| format!("parsing {PAGES}'s line {line}"))
        })
        .collect()
}

impl Plan<'_> {
    /// Runs the cell's tool on the rungs site and holds what it read to
    /// the site's manifest.
    pub(crate) fn auto_read(&self) -> Result<Judged> {
        let manifest = manifest(self.site.address, self.site.port())?;
        let home = tempfile::tempdir().context("creating the tool's data folder")?;
        self.configure(home.path())?;
        let cache = self.cache()?;
        let started = now();
        let mut driven = self.spawn_at(
            home.path(),
            (1, &Mode::Auto),
            cache.as_ref().map(tempfile::TempDir::path),
            &self.start_url(),
            Route::Direct,
        )?;
        let ended = match driven.engine.as_mut() {
            Some(engine) => engine
                .start(
                    &self.site,
                    self.cell,
                    &self.start_url(),
                    self.durability(),
                    self.config.samples(),
                )
                .and_then(|()| engine.ended()),
            None => self.read_count(&mut driven.child),
        };
        if ended.is_err() {
            let _ = driven.child.kill();
        }
        if let Some(engine) = driven.engine.take() {
            engine.close();
        }
        let status = driven.child.wait().context("waiting for the tool")?;
        let seconds = started.elapsed().as_secs_f64();
        ended.with_context(|| format!("{} on the rungs site", self.cell.name))?;
        if !status.success() {
            bail!("{} exited with {status} on the rungs site", self.cell.name);
        }
        let reads = if let Config::Foxhound { .. } = self.config {
            self.records(home.path())?
                .into_iter()
                .map(Read::from)
                .collect()
        } else {
            written(home.path())?
        };
        auto::judged(
            &manifest,
            &observations(self.site.address)?,
            &reads,
            seconds,
        )
    }
}

#[cfg(test)]
mod tests;
