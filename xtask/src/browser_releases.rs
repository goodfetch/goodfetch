//! The release table the version check grades browser builds against (ADR
//! 0035; spec #599): Chrome's majors from Chromium Dash's milestone
//! schedule and Firefox's from Mozilla's release calendar, each with the day
//! it reached Stable, scheduled majors included, so the date alone says
//! which major is Stable today. `xtask browser-releases` writes it to
//! `crates/goodfetch-page/src/version_check/browser-releases.toml`, which
//! every release embeds; the engine never fetches it.
//!
//! Neither source gives the day a vendor stops patching a major, so the
//! table it writes names none.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use jiff::civil::Date;
use serde::Deserialize;

use crate::environment::Environment;

/// The table, from the repository root.
const TABLE_FILE: &str = "crates/goodfetch-page/src/version_check/browser-releases.toml";

/// The oldest release kept.
const SINCE: Date = jiff::civil::date(2018, 1, 1);

/// Chromium Dash's path for the milestones from 100 before the current one
/// to 12 after it, back to Chrome 58 in 2017.
const CHROME_SCHEDULE: &str = "/fetch_milestone_schedule?offset=-100&n=112";

/// Mozilla's calendar: the releases shipped, and those scheduled.
const FIREFOX_RELEASES: &str = "/api/firefox/releases/";
const FIREFOX_SCHEDULED: &str = "/api/firefox/releases/future/";

/// The largest answer read; Chromium Dash's was 250 kB on 2026-11-17.
const MAXIMUM_ANSWER: u64 = 16 * 1024 * 1024;

/// `xtask browser-releases`' arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Chromium Dash, or a stand-in's address.
    #[arg(long, default_value = "https://chromiumdash.appspot.com")]
    chromium_dash: String,
    /// Mozilla's release calendar, or a stand-in's address.
    #[arg(long, default_value = "https://whattrainisitnow.com")]
    firefox_calendar: String,
}

/// Chromium Dash's schedule, with only what the table keeps.
#[derive(Deserialize)]
struct Schedule {
    mstones: Vec<Milestone>,
}

#[derive(Deserialize)]
struct Milestone {
    mstone: u32,
    /// `YYYY-MM-DDT00:00:00`, or none for a milestone not yet scheduled.
    stable_date: Option<String>,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let schedule: Schedule =
        serde_json::from_slice(&fetch(&arguments.chromium_dash, CHROME_SCHEDULE)?)
            .context("reading Chromium Dash's milestone schedule")?;
    let chrome = chrome_releases(schedule)?;
    let mut firefox = BTreeMap::new();
    for path in [FIREFOX_RELEASES, FIREFOX_SCHEDULED] {
        let calendar: BTreeMap<String, String> =
            serde_json::from_slice(&fetch(&arguments.firefox_calendar, path)?)
                .with_context(|| format!("reading Mozilla's calendar at {path}"))?;
        firefox_releases(calendar, &mut firefox)?;
    }
    let table = table(&chrome, &firefox)?;
    let path = environment.root.join(TABLE_FILE);
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(&path, table).with_context(|| format!("writing {}", path.display()))?;
    Ok(ExitCode::SUCCESS)
}

fn fetch(host: &str, path: &str) -> Result<Vec<u8>> {
    let url = format!("{}{path}", host.trim_end_matches('/'));
    ureq::get(&url)
        .call()
        .with_context(|| format!("downloading {url}"))?
        .body_mut()
        .with_config()
        .limit(MAXIMUM_ANSWER)
        .read_to_vec()
        .with_context(|| format!("reading {url}"))
}

/// Each scheduled milestone from [`SINCE`] on, by major.
fn chrome_releases(schedule: Schedule) -> Result<BTreeMap<u32, Date>> {
    let mut releases = BTreeMap::new();
    for milestone in schedule.mstones {
        let Some(stable) = milestone.stable_date else {
            continue;
        };
        let day = stable.get(..10).unwrap_or(&stable);
        let stable: Date = day.parse().with_context(|| {
            format!(
                "reading Chrome {}'s Stable date, {stable}",
                milestone.mstone
            )
        })?;
        if stable >= SINCE {
            releases.insert(milestone.mstone, stable);
        }
    }
    Ok(releases)
}

/// Adds each major release of `calendar`, keyed `<major>.0`, from [`SINCE`]
/// on; dot releases and unnumbered ones are not majors.
fn firefox_releases(
    calendar: BTreeMap<String, String>,
    releases: &mut BTreeMap<u32, Date>,
) -> Result<()> {
    for (version, day) in calendar {
        let Some(major) = version
            .strip_suffix(".0")
            .filter(|major| !major.is_empty() && major.bytes().all(|byte| byte.is_ascii_digit()))
        else {
            continue;
        };
        let major: u32 = major
            .parse()
            .with_context(|| format!("reading Firefox {version}'s major"))?;
        let stable: Date = day
            .parse()
            .with_context(|| format!("reading Firefox {version}'s release date, {day}"))?;
        if stable >= SINCE {
            releases.insert(major, stable);
        }
    }
    Ok(())
}

/// The table's text, refused unless each browser's dates rise with its
/// majors, as the version check reads them.
fn table(chrome: &BTreeMap<u32, Date>, firefox: &BTreeMap<u32, Date>) -> Result<String> {
    let mut text = String::from(
        "# The release table the version check grades browser builds against\n\
         # (ADR 0035): each major of Chrome (Chromium Dash) and Firefox\n\
         # (Mozilla's release calendar) with the day it reached Stable,\n\
         # scheduled majors included. Written by `xtask browser-releases`;\n\
         # never edited by hand.\n",
    );
    for (name, releases) in [("chrome", chrome), ("firefox", firefox)] {
        let dates: Vec<Date> = releases.values().copied().collect();
        if releases.len() < 2 || !dates.windows(2).all(|pair| pair[0] < pair[1]) {
            bail!("{name}'s releases are not at least two, with dates rising with their majors");
        }
        for (major, stable) in releases {
            let _ = write!(
                text,
                "\n[[{name}]]\nmajor = {major}\nstable = \"{stable}\"\n"
            );
        }
    }
    Ok(text)
}
