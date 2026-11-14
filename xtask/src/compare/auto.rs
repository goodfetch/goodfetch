//! The auto cell (#2411): each tool's own choice of HTTP or a browser,
//! page by page, on the Proving ground's rungs site (#2410), whose
//! manifest names each page's right Records. Since the tools render
//! different shares of the pages, the same work is each page of the
//! manifest requested on the site's host, as the Proving ground's
//! observations record it, never a tool's report; then each page's Records,
//! as the tool read them, are held to the manifest's, and a page whose
//! Records differ is wrong. A run gives its rate, the pages it rendered
//! and the pages wrong, with no bar (G-04 Q11).

use std::collections::{BTreeMap, BTreeSet};

use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::hostile;
use super::matrix::Site;

/// A Record of the rungs site: an item's `id` and `name`.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub(crate) struct Record {
    pub(crate) id: String,
    pub(crate) name: String,
}

/// The rungs site's manifest, `/manifest.json`: each page's path and the
/// Records a right reading of it gives.
#[derive(Debug, Deserialize)]
pub(crate) struct Manifest {
    pages: Vec<Listed>,
}

#[derive(Debug, Deserialize)]
struct Listed {
    path: String,
    records: Vec<Record>,
}

/// One page as a tool read it: its URL, whether the tool rendered it in a
/// browser, and the Records it read there. A tool other than Foxhound
/// writes one JSON line of it a page to `pages.jsonl`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct Read {
    pub(crate) url: String,
    pub(crate) rendered: bool,
    pub(crate) records: Vec<Record>,
}

/// One run of an auto cell's tool against the manifest.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Judged {
    /// The crawl's seconds, from its tool's start to its end.
    pub(crate) seconds: f64,
    /// The manifest's pages, and those the Proving ground saw requested on
    /// the site's host.
    pub(crate) pages: u64,
    pub(crate) requested: u64,
    /// The manifest's pages the tool rendered in a browser.
    pub(crate) rendered: u64,
    /// The manifest's pages whose Records the tool read differ from the
    /// manifest's, a page it never read counting as read with none.
    pub(crate) wrong: u64,
}

impl Judged {
    /// Whether the run did the same work: every page of the manifest
    /// requested.
    pub(crate) fn same_work(&self) -> bool {
        self.requested == self.pages
    }

    /// Pages a second.
    pub(crate) fn rate(&self) -> f64 {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a site's pages stay far below 2^52, where a float is exact"
        )]
        let pages = self.pages as f64;
        pages / self.seconds
    }
}

/// A URL's path, without its query or fragment: `/` where it has none.
fn path_of(url: &str) -> &str {
    let after_scheme = url.find("://").map_or(0, |at| at + "://".len());
    let rest = &url[after_scheme..];
    let path = rest.find('/').map_or("", |at| &rest[at..]);
    let path = path.split(['?', '#']).next().unwrap_or_default();
    if path.is_empty() { "/" } else { path }
}

/// What a run whose tool read `reads` in `seconds` did against `manifest`,
/// the Proving ground's `observations` showing the requests it sent; an
/// error when they cannot be read whole.
pub(crate) fn judged(
    manifest: &Manifest,
    observations: &Value,
    reads: &[Read],
    seconds: f64,
) -> Result<Judged> {
    let host = Site::Rungs.host_of(1);
    let requested: BTreeSet<&str> = hostile::every(observations, "requests")?
        .iter()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|seen| seen.split_once(':').map_or(seen, |(name, _)| name) == host)
        })
        .filter_map(|request| request["target"].as_str())
        .map(path_of)
        .collect();
    // Each page's reads together: rendered when any was, its Records all
    // of theirs.
    let mut read: BTreeMap<&str, (bool, BTreeSet<&Record>)> = BTreeMap::new();
    for page in reads {
        let entry = read.entry(path_of(&page.url)).or_default();
        entry.0 |= page.rendered;
        entry.1.extend(&page.records);
    }
    let mut judged = Judged {
        seconds,
        pages: 0,
        requested: 0,
        rendered: 0,
        wrong: 0,
    };
    let none = (false, BTreeSet::new());
    for listed in &manifest.pages {
        let (rendered, records) = read.get(listed.path.as_str()).unwrap_or(&none);
        judged.pages += 1;
        judged.requested += u64::from(requested.contains(listed.path.as_str()));
        judged.rendered += u64::from(*rendered);
        judged.wrong += u64::from(*records != listed.records.iter().collect());
    }
    Ok(judged)
}

#[cfg(test)]
mod tests;
