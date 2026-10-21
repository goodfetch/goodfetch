//! The watchlist job (story 61; ADRs 0035 and 0043): webappanalyzer
//! (GPL-3.0) and Scrapfly's Antibot-Detector (NPOSL-3.0) are read for vendor
//! names only, compared with the vendors Collie's block-detection rules
//! cover (`vendors.toml` beside the rules), and each vendor they list that no
//! rule covers gets one `watchlist` issue. A rule for it is written from the
//! vendor's documentation and Goodfetch's own captures: nothing else of
//! either list enters Goodfetch, and the notice inventory refuses a source
//! that names one ([`Watchlist::named_in`]).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::environment::Environment;
use crate::github;
use crate::issues::{Issues, Label};
use crate::output;

/// The vendors the block-detection rules cover, from the workspace's folder.
pub(crate) const VENDORS: &str = "crates/goodfetch-collie/src/block_detection/vendors.toml";

/// A list of vendors Goodfetch watches and never takes anything but names
/// from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Watchlist {
    WebAppAnalyzer,
    AntibotDetector,
}

impl Watchlist {
    const ALL: [Self; 2] = [Self::WebAppAnalyzer, Self::AntibotDetector];

    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::WebAppAnalyzer => "webappanalyzer",
            Self::AntibotDetector => "Scrapfly's Antibot-Detector",
        }
    }

    /// The license its content is under, which Goodfetch's code cannot take.
    pub(crate) fn license(self) -> &'static str {
        match self {
            Self::WebAppAnalyzer => "GPL-3.0",
            Self::AntibotDetector => "NPOSL-3.0",
        }
    }

    /// Its repository on GitHub, as `owner/name`.
    fn repository(self) -> &'static str {
        match self {
            Self::WebAppAnalyzer => "enthec/webappanalyzer",
            Self::AntibotDetector => "scrapfly/Antibot-Detector",
        }
    }

    /// The watchlist `text` (a source's name or address) names, if any.
    pub(crate) fn named_in(text: &str) -> Option<Self> {
        let text = text.to_ascii_lowercase();
        Self::ALL.into_iter().find(|watchlist| {
            let repository = watchlist.repository().to_ascii_lowercase();
            let name = repository
                .rsplit_once('/')
                .map_or(repository.as_str(), |(_, name)| name);
            text.contains(name)
        })
    }

    /// The vendor names a checkout of the list at `folder` lists.
    fn read(self, folder: &Path) -> Result<Vec<String>> {
        match self {
            Self::WebAppAnalyzer => read_webappanalyzer(folder),
            Self::AntibotDetector => read_antibot_detector(folder),
        }
    }
}

/// The technologies of webappanalyzer's "Security" category, where its
/// bot-protection and captcha vendors are.
fn read_webappanalyzer(folder: &Path) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Category {
        name: String,
    }
    #[derive(Deserialize)]
    struct Technology {
        #[serde(default)]
        cats: Vec<u64>,
    }
    let source = folder.join("src");
    let categories: BTreeMap<u64, Category> = read_json(&source.join("categories.json"))?;
    let security: BTreeSet<u64> = categories
        .into_iter()
        .filter(|(_, category)| category.name == "Security")
        .map(|(number, _)| number)
        .collect();
    let technologies = source.join("technologies");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&technologies)
        .with_context(|| format!("reading {}", technologies.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<_>>()
        .with_context(|| format!("reading {}", technologies.display()))?;
    files.retain(|file| {
        file.extension()
            .is_some_and(|extension| extension == "json")
    });
    files.sort();
    let mut names = Vec::new();
    for file in files {
        let listed: BTreeMap<String, Technology> = read_json(&file)?;
        names.extend(
            listed
                .into_iter()
                .filter(|(_, technology)| technology.cats.iter().any(|cat| security.contains(cat)))
                .map(|(name, _)| name),
        );
    }
    Ok(names)
}

/// The detectors of Antibot-Detector's `antibot` and `captcha` categories,
/// by their id without its `detect-` prefix; its `fingerprint` category
/// lists techniques, not vendors.
fn read_antibot_detector(folder: &Path) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Category {
        detectors: Vec<String>,
    }
    #[derive(Deserialize)]
    struct Index {
        antibot: Category,
        captcha: Category,
    }
    let index: Index = read_json(&folder.join("detectors/index.json"))?;
    Ok(index
        .antibot
        .detectors
        .into_iter()
        .chain(index.captcha.detectors)
        .map(|id| id.strip_prefix("detect-").map_or(id.clone(), str::to_owned))
        .collect())
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// A vendor a watchlist lists, under the name it gives it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Listed {
    pub(crate) watchlist: Watchlist,
    pub(crate) name: String,
}

/// Every vendor the watchlists checked out at these folders list.
pub(crate) fn read_listed(folders: &[(Watchlist, &Path)]) -> Result<Vec<Listed>> {
    let mut listed = Vec::new();
    for &(watchlist, folder) in folders {
        listed.extend(
            watchlist
                .read(folder)
                .with_context(|| format!("reading {}", watchlist.name()))?
                .into_iter()
                .map(|name| Listed { watchlist, name }),
        );
    }
    Ok(listed)
}

/// A vendor the rules cover: its name and the other names a watchlist lists
/// it under (`block_detection`'s tests check its `rules` exist).
#[derive(Deserialize)]
pub(crate) struct Covered {
    name: String,
    #[serde(default)]
    listed_as: Vec<String>,
}

/// The vendors the rules of the workspace at `root` cover.
pub(crate) fn read_covered(root: &Path) -> Result<Vec<Covered>> {
    #[derive(Deserialize)]
    struct File {
        vendor: Vec<Covered>,
    }
    let path = root.join(VENDORS);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let file: File =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(file.vendor)
}

/// A vendor the watchlists list that no rule covers.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Missing {
    /// The names compared: lowercase letters and digits only.
    pub(crate) key: String,
    /// Each watchlist that lists it, with the name it gives.
    pub(crate) listings: Vec<Listed>,
}

/// Each vendor of `listed` that `covered` lacks, by key, its listings in
/// `listed`'s order. A name matches a covered vendor's name or one of its
/// `listed_as` names, compared as keys.
pub(crate) fn missing(listed: &[Listed], covered: &[Covered]) -> Vec<Missing> {
    let covered: BTreeSet<String> = covered
        .iter()
        .flat_map(|vendor| std::iter::once(&vendor.name).chain(&vendor.listed_as))
        .map(|name| key(name))
        .collect();
    let mut missing: BTreeMap<String, Vec<Listed>> = BTreeMap::new();
    for listing in listed {
        let key = key(&listing.name);
        if !key.is_empty() && !covered.contains(&key) {
            missing.entry(key).or_default().push(listing.clone());
        }
    }
    missing
        .into_iter()
        .map(|(key, listings)| Missing { key, listings })
        .collect()
}

/// A vendor's name as compared: its lowercase ASCII letters and digits, so
/// `DataDome` and `datadome` are one vendor.
fn key(name: &str) -> String {
    name.chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|character| character.to_ascii_lowercase())
        .collect()
}

/// `xtask watchlist`'s arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// A checkout of webappanalyzer (github.com/enthec/webappanalyzer).
    #[arg(long)]
    webappanalyzer: PathBuf,
    /// A checkout of Scrapfly's Antibot-Detector
    /// (github.com/scrapfly/Antibot-Detector).
    #[arg(long)]
    antibot_detector: PathBuf,
    /// The most issues one run opens; the others wait for the next run.
    #[arg(long, default_value_t = 10)]
    limit: usize,
    /// The repository whose issues the job opens, and GitHub's API.
    #[command(flatten)]
    github: github::Arguments,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let webappanalyzer = environment.path(&arguments.webappanalyzer);
    let antibot_detector = environment.path(&arguments.antibot_detector);
    let listed = read_listed(&[
        (Watchlist::WebAppAnalyzer, &webappanalyzer),
        (Watchlist::AntibotDetector, &antibot_detector),
    ])?;
    let covered = read_covered(&environment.root)?;
    let api = arguments.github.api(environment.token());
    let mut issues = Issues::new(&api, &arguments.github.repository);
    let mut opened = 0;
    let mut waiting = 0;
    for vendor in missing(&listed, &covered) {
        if opened == arguments.limit {
            waiting += 1;
            continue;
        }
        if issues.open_once(
            Label::Watchlist,
            &vendor.key,
            &title(&vendor),
            &body(&vendor),
        )? {
            opened += 1;
        }
    }
    for action in issues.actions() {
        output::line(action);
    }
    if waiting > 0 {
        output::line(&format!(
            "{waiting} more vendors may lack a rule; the next run opens up to {} of them",
            arguments.limit
        ));
    }
    Ok(ExitCode::SUCCESS)
}

fn title(vendor: &Missing) -> String {
    let name = vendor
        .listings
        .first()
        .map_or(vendor.key.as_str(), |listed| listed.name.as_str());
    format!("Block detection: no rule covers {name}, which a watchlist lists")
}

fn body(vendor: &Missing) -> String {
    let mut listings = String::new();
    for listed in &vendor.listings {
        // Writing to a String cannot fail.
        let _ = writeln!(
            listings,
            "- {} ({}) lists `{}`.",
            listed.watchlist.name(),
            listed.watchlist.license(),
            listed.name
        );
    }
    format!(
        "A watchlist lists a vendor no block-detection rule covers:\n\n{listings}\n\
         If the vendor refuses automated clients, write a rule from the vendor's own \
         documentation and Goodfetch's captures, never from either watchlist (story 61, \
         ADR 0035), and add the vendor to `{VENDORS}` with the rules that cover it. If it \
         refuses none, close this issue: the watchlist job never opens it again.\n"
    )
}

#[cfg(test)]
mod tests;
