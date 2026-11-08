//! The watchlist job (story 61; ADRs 0035 and 0043): webappanalyzer
//! (GPL-3.0) and Scrapfly's Antibot-Detector (NPOSL-3.0) are read for the
//! names of bot-protection, challenge and CAPTCHA vendors only, compared
//! with the vendors Collie's block-detection rules cover (`vendors.toml`
//! beside the rules), and each vendor they list that no rule covers gets one
//! `watchlist` issue. A rule for it is written from the
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
use crate::products;

/// The vendors the block-detection rules cover, from the workspace's folder.
pub(crate) const VENDORS: &str = concat!(
    products::collie_folder!(),
    "/src/block_detection/vendors.toml"
);

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

    /// The watchlist `text` (a source's name or address) names, if any:
    /// its repository's name, compared as keys, so `Antibot Detector` and
    /// `antibot-detector` both name it.
    pub(crate) fn named_in(text: &str) -> Option<Self> {
        let text = key(text);
        Self::ALL.into_iter().find(|watchlist| {
            let repository = watchlist.repository();
            let name = repository
                .rsplit_once('/')
                .map_or(repository, |(_, name)| name);
            text.contains(&key(name))
        })
    }

    /// The vendor names a checkout of the list at `folder` lists; an error
    /// when it lists none, which means its layout changed, never that every
    /// vendor went away.
    fn read(self, folder: &Path) -> Result<Vec<String>> {
        let names = match self {
            Self::WebAppAnalyzer => read_webappanalyzer(folder)?,
            Self::AntibotDetector => read_antibot_detector(folder)?,
        };
        anyhow::ensure!(
            !names.is_empty(),
            "{} at {} lists no bot-protection vendor: its layout may have changed",
            self.name(),
            folder.display()
        );
        Ok(names)
    }
}

/// The technologies of webappanalyzer's "Security" category that
/// [`describes_bot_protection`].
fn read_webappanalyzer(folder: &Path) -> Result<Vec<String>> {
    #[derive(Deserialize)]
    struct Category {
        name: String,
    }
    #[derive(Deserialize)]
    struct Technology {
        #[serde(default)]
        cats: Vec<u64>,
        description: Option<String>,
    }
    let source = folder.join("src");
    let categories: BTreeMap<u64, Category> = read_json(&source.join("categories.json"))?;
    let security: BTreeSet<u64> = categories
        .into_iter()
        .filter(|(_, category)| category.name == "Security")
        .map(|(number, _)| number)
        .collect();
    anyhow::ensure!(
        !security.is_empty(),
        "{} has no \"Security\" category: its layout may have changed",
        source.join("categories.json").display()
    );
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
                .filter(|(name, technology)| {
                    technology.cats.iter().any(|cat| security.contains(cat))
                        && describes_bot_protection(name, technology.description.as_deref())
                })
                .map(|(name, _)| name),
        );
    }
    Ok(names)
}

/// Whether a webappanalyzer Security technology is a bot-protection,
/// challenge or CAPTCHA vendor: its name or description (read, never kept)
/// says "bot" or "bots" as a word (also `anti-bot`, `AntiBot`), "captcha" or
/// "Turing test". The category alone also holds HSTS, bug bounty platforms,
/// fraud scoring and the like. A bot vendor whose entry says none of these
/// (Anubis, for one) is found only if Antibot-Detector lists it.
pub(crate) fn describes_bot_protection(name: &str, description: Option<&str>) -> bool {
    [Some(name), description].into_iter().flatten().any(|text| {
        let text = text.to_ascii_lowercase();
        text.contains("captcha")
            || text.contains("turing test")
            || text
                .split(|character: char| !character.is_ascii_alphanumeric())
                .any(|word| matches!(word, "bot" | "bots" | "antibot" | "antibots"))
    })
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

/// A vendor the coverage file names: its name and the other names a
/// watchlist lists it under.
#[derive(Deserialize)]
struct Known {
    name: String,
    #[serde(default)]
    listed_as: Vec<String>,
}

/// The coverage file, `vendors.toml`: the vendors the rules cover
/// (`block_detection`'s tests check their `rules` exist), and the vendors no
/// rule covers yet that the watchlists list under several names, which
/// each get one issue, keyed by their `name`.
#[derive(Deserialize)]
pub(crate) struct Coverage {
    #[serde(rename = "vendor")]
    covered: Vec<Known>,
    #[serde(default)]
    uncovered: Vec<Known>,
}

impl Coverage {
    /// The coverage file `text`.
    pub(crate) fn parse(text: &str) -> Result<Self> {
        Ok(toml::from_str(text)?)
    }
}

/// The coverage file of the workspace at `root`.
pub(crate) fn read_coverage(root: &Path) -> Result<Coverage> {
    let path = root.join(VENDORS);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    Coverage::parse(&text).with_context(|| format!("parsing {}", path.display()))
}

/// A vendor the watchlists list that no rule covers.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Missing {
    /// The names compared: lowercase letters and digits only.
    pub(crate) key: String,
    /// Each watchlist that lists it, with the name it gives.
    pub(crate) listings: Vec<Listed>,
}

/// Each vendor of `listed` that `coverage` lacks, by key, its listings in
/// `listed`'s order. A name matches a covered vendor's name or one of its
/// `listed_as` names, compared as keys; an uncovered vendor's `listed_as`
/// names fold into its own name's key.
pub(crate) fn missing(listed: &[Listed], coverage: &Coverage) -> Vec<Missing> {
    let names = |vendor: &'_ Known| {
        let own = key(&vendor.name);
        std::iter::once(&vendor.name)
            .chain(vendor.listed_as.iter())
            .map(move |name| (key(name), own.clone()))
            .collect::<Vec<_>>()
    };
    let covered: BTreeSet<String> = coverage
        .covered
        .iter()
        .flat_map(names)
        .map(|(name, _)| name)
        .collect();
    let folded: BTreeMap<String, String> = coverage.uncovered.iter().flat_map(names).collect();
    let mut missing: BTreeMap<String, Vec<Listed>> = BTreeMap::new();
    for listing in listed {
        let key = key(&listing.name);
        if !key.is_empty() && !covered.contains(&key) {
            let vendor = folded.get(&key).cloned().unwrap_or(key);
            missing.entry(vendor).or_default().push(listing.clone());
        }
    }
    missing
        .into_iter()
        .map(|(key, listings)| Missing { key, listings })
        .collect()
}

/// The listings of `listed` whose name has no letter or digit, which no key
/// compares.
pub(crate) fn unkeyed(listed: &[Listed]) -> Vec<&Listed> {
    listed
        .iter()
        .filter(|listing| key(&listing.name).is_empty())
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
    let coverage = read_coverage(&environment.root)?;
    let api = arguments.github.api(environment.token());
    let mut issues = Issues::new(&api, &arguments.github.repository);
    let mut opened = 0;
    let mut waiting = 0;
    for vendor in missing(&listed, &coverage) {
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
    for listing in unkeyed(&listed) {
        output::line(&format!(
            "{} lists a name with no letter or digit, which the job cannot compare: {:?}",
            listing.watchlist.name(),
            listing.name
        ));
    }
    if waiting > 0 {
        output::line(&format!(
            "{waiting} more vendors may lack a rule; the next run opens up to {} of them",
            arguments.limit
        ));
    }
    Ok(ExitCode::SUCCESS)
}

/// The most characters of a listed name an issue shows.
const SHOWN_NAME: usize = 64;

/// A listed name as an issue shows it: only letters, digits, spaces and
/// `.-_+&'`, so nothing from an unpinned checkout becomes live Markdown, a
/// mention or a link, and at most [`SHOWN_NAME`] characters.
fn shown(name: &str) -> String {
    let kept: Vec<char> = name
        .chars()
        .filter(|character| {
            character.is_alphanumeric()
                || matches!(character, ' ' | '.' | '-' | '_' | '+' | '&' | '\'')
        })
        .collect();
    let mut text: String = kept.iter().take(SHOWN_NAME).collect();
    if kept.len() > SHOWN_NAME {
        text.push('…');
    }
    text
}

fn title(vendor: &Missing) -> String {
    let name = vendor
        .listings
        .first()
        .map_or_else(|| vendor.key.clone(), |listed| shown(&listed.name));
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
            shown(&listed.name)
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
