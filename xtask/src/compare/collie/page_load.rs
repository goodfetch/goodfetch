//! The Collie scenario's page-load cell (issue 2319): each tool launches
//! its own browser and loads the Proving ground's `page-load.test` page of
//! 150 subresources to its `load` event, on two pages, `/plain` and `/ads`
//! (30 of them on hosts the embedded Filter lists block), in seven rows:
//! Collie at its defaults, then with one default off at a time, and
//! Playwright with and without a route.
//!
//! Collie runs Sessions it launches itself, its default path: only those go
//! through its raw CDP egress relay, so a Session it borrowed over CDP
//! would make row 5 the same as row 1. Each trial runs
//! `xtask/compare-collie/load.ts` in a fresh process against a fresh
//! Proving ground, and takes its figures between the marks the driver asks
//! for around the load: wall time to `load` and the CPU time of the tool's
//! process tree, its browser in it, beside that tree's peak resident memory
//! and the requests the Proving ground received. The same-work check holds
//! every trial to its page's own subresources and `#content` text; a
//! request the Filter lists blocked is no missing work.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;
use serde_json::Value;
use sha2::{Digest as _, Sha256};

use crate::check::Finding;

use super::super::relay::Relay;
use super::super::report::{Figure, Unit, cell_text, float, median, spread};
use super::super::trial::{self, Outcome};
use super::super::trials::{Contender, LoadPage, LoadStopped, LoadTrial, Row};
use super::Tools;

/// The host name the Proving ground serves both pages on.
pub(crate) const HOST: &str = "page-load.test";

/// The issue that gives Collie's engine the counters of the paused requests
/// and of the CDP commands of a launched Session, which this cell reads as
/// missing until then.
pub(crate) const COUNTERS_ISSUE: &str = "#2635";

/// The pages, in the order the report gives them.
pub(crate) const PAGES: [LoadPage; 2] = [LoadPage::Plain, LoadPage::Ads];

impl LoadPage {
    /// Its name in the report and in findings.
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Plain => "plain",
            Self::Ads => "ads",
        }
    }

    /// Its path on [`HOST`].
    pub(crate) fn path(self) -> &'static str {
        match self {
            Self::Plain => "/plain",
            Self::Ads => "/ads",
        }
    }

    /// The subresources on its own host, which every tool loads.
    pub(crate) fn own(self) -> u64 {
        match self {
            Self::Plain => 150,
            Self::Ads => 120,
        }
    }
}

impl Row {
    /// Every row, in the order the report numbers them.
    pub(crate) const ALL: [Self; 7] = [
        Self::Defaults,
        Self::LiveHidingOff,
        Self::ConsentOff,
        Self::FilterListsOff,
        Self::EgressRelayOff,
        Self::Playwright,
        Self::PlaywrightRouted,
    ];

    /// Its number in the report, from 1.
    pub(crate) fn number(self) -> u8 {
        match self {
            Self::Defaults => 1,
            Self::LiveHidingOff => 2,
            Self::ConsentOff => 3,
            Self::FilterListsOff => 4,
            Self::EgressRelayOff => 5,
            Self::Playwright => 6,
            Self::PlaywrightRouted => 7,
        }
    }

    /// What it runs, as the report names it.
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Defaults => "Collie, defaults",
            Self::LiveHidingOff => "Collie, `live_view.hiding: off`",
            Self::ConsentOff => "Collie, row 2 and `consent.handling: off`",
            Self::FilterListsOff => "Collie, row 2 and the Filter lists off",
            Self::EgressRelayOff => {
                "Collie, `security.raw_cdp_egress_proxy: false`, held by interception alone (#2435)"
            }
            Self::Playwright => "Playwright, defaults",
            Self::PlaywrightRouted => "Playwright, a route letting every request through",
        }
    }

    /// The tool it runs.
    pub(crate) fn tool(self) -> Contender {
        match self {
            Self::Defaults
            | Self::LiveHidingOff
            | Self::ConsentOff
            | Self::FilterListsOff
            | Self::EgressRelayOff => Contender::Collie,
            Self::Playwright | Self::PlaywrightRouted => Contender::Playwright,
        }
    }

    /// The settings Collie loads with beyond its defaults, as tables of its
    /// user configuration; none for Playwright's rows.
    pub(crate) fn settings(self) -> String {
        const HIDING_OFF: &str = "\n[settings.live_view]\nhiding = \"off\"\n";
        const CONSENT_OFF: &str = "\n[settings.consent]\nhandling = \"off\"\n";
        const LISTS_OFF: &str =
            "\n[settings.filter_lists]\nads = false\ntrackers = false\nannoyances = false\n";
        const EGRESS_RELAY_OFF: &str = "\n[settings.security]\nraw_cdp_egress_proxy = false\n";
        let tables: &[&str] = match self {
            Self::Defaults | Self::Playwright | Self::PlaywrightRouted => &[],
            Self::LiveHidingOff => &[HIDING_OFF],
            Self::ConsentOff => &[HIDING_OFF, CONSENT_OFF],
            Self::FilterListsOff => &[HIDING_OFF, LISTS_OFF],
            Self::EgressRelayOff => &[EGRESS_RELAY_OFF],
        };
        tables.concat()
    }

    /// Whether Playwright loads through a route that lets every request
    /// through.
    pub(crate) fn routed(self) -> bool {
        self == Self::PlaywrightRouted
    }
}

/// Every (page, row) cell, in the order the report gives them.
pub(crate) fn cells() -> Vec<(LoadPage, Row)> {
    PAGES
        .into_iter()
        .flat_map(|page| Row::ALL.into_iter().map(move |row| (page, row)))
        .collect()
}

/// The hosts of the absolute subresource URLs of `page`, the HTML the
/// Proving ground serves at `/ads`, each once: the ad hosts.
pub(crate) fn ad_hosts(page: &str) -> Vec<String> {
    let mut hosts: Vec<String> = Vec::new();
    for (at, _) in page.match_indices("src=\"http://") {
        let rest = &page[at + "src=\"http://".len()..];
        let Some(host) = rest.split([':', '/', '"']).next() else {
            continue;
        };
        if !host.is_empty() && !hosts.iter().any(|known| known == host) {
            hosts.push(host.to_owned());
        }
    }
    hosts
}

/// Collie's user configuration in `row`: its page's host and `ad_hosts`
/// on loopback, as the other cells map theirs, and the row's settings.
pub(crate) fn configuration(port: u16, ad_hosts: &[String], row: Row) -> Result<String> {
    let urls: Vec<String> = std::iter::once(HOST)
        .chain(ad_hosts.iter().map(String::as_str))
        .map(|host| format!("http://{host}:{port}/"))
        .collect();
    let mut text = super::configuration(&urls)?;
    text.push_str(&row.settings());
    Ok(text)
}

/// Chrome's `--host-resolver-rules` for Playwright: the page's host and
/// `ad_hosts` on loopback.
pub(crate) fn resolver_rules(ad_hosts: &[String]) -> String {
    std::iter::once(HOST)
        .chain(ad_hosts.iter().map(String::as_str))
        .map(|host| format!("MAP {host} 127.0.0.1"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// What the Proving ground's `observations` show of a load: the page's own
/// subresources answered, each once, and every request the tool sent, to
/// the page's host or an ad host.
pub(crate) fn counted(observations: &Value) -> (u64, u64) {
    let mut own = std::collections::BTreeSet::new();
    let mut sent = 0;
    let requests = observations["requests"]
        .as_array()
        .map_or(&[][..], Vec::as_slice);
    for request in requests {
        let host = request["host"].as_str().unwrap_or_default();
        let host = host.split(':').next().unwrap_or_default();
        let target = request["target"].as_str().unwrap_or_default();
        let to_page = host == HOST;
        if !to_page && !target.starts_with("/ad/") {
            continue;
        }
        sent += 1;
        if to_page && target.starts_with("/assets/") && request["status"].as_u64() == Some(200) {
            own.insert(target.to_owned());
        }
    }
    (own.len() as u64, sent)
}

/// What `load.ts` printed: the document's status and `#content`'s text.
#[derive(serde::Deserialize)]
struct Printed {
    status: Option<u16>,
    read: Option<String>,
}

/// A trial's name in a finding and in the run's lines.
pub(crate) fn subject(page: LoadPage, row: Row, number: u32) -> String {
    format!(
        "collie page_load {} row {} trial {number}",
        page.name(),
        row.number()
    )
}

/// The ad hosts of the page with ads a Proving ground serves: read once
/// a run, from a Proving ground of its own, so no trial's observations hold
/// the read.
pub(crate) fn served_ad_hosts(proving_ground: &Path) -> Result<Vec<String>> {
    let serving = crate::rss::crawl::serve(proving_ground, "none")?;
    let port = serving
        .address
        .rsplit_once(':')
        .map_or("", |(_, port)| port);
    // The Host the page is served under, which ureq sends as it is set.
    let answer = ureq::get(&format!(
        "http://{}{}",
        serving.address,
        LoadPage::Ads.path()
    ))
    .header("Host", format!("{HOST}:{port}"))
    .call()
    .context("asking the Proving ground for the page with ads")?
    .body_mut()
    .read_to_string()
    .context("reading the page with ads")?;
    let hosts = ad_hosts(&answer);
    if hosts.is_empty() {
        bail!("the Proving ground's page with ads names no ad host");
    }
    Ok(hosts)
}

/// One trial of `row` on `page`: a fresh Proving ground, the tool launching
/// its own browser with `ad_hosts` and the page's host on loopback, its
/// figures between the marks around the load; or why the comparison
/// stopped it.
pub(crate) fn trial(
    tools: &Tools<'_>,
    ad_hosts: &[String],
    page: LoadPage,
    row: Row,
    number: u32,
    load: Option<f64>,
) -> Result<Outcome<LoadTrial>> {
    let serving = crate::rss::crawl::serve(tools.proving_ground, "none")?;
    let port: u16 = serving
        .address
        .rsplit_once(':')
        .and_then(|(_, port)| port.parse().ok())
        .with_context(|| {
            format!(
                "the Proving ground's address has no port: {}",
                serving.address
            )
        })?;
    let folder = tempfile::tempdir().context("creating the trial's folder")?;
    let relay = Relay::marks_only()?;
    let tool = row.tool();
    let mut command = super::tools::node(tools, &tools.driver.with_file_name("load.ts"));
    command
        .args(["--tool", tool.name(), "--url"])
        .arg(format!("http://{HOST}:{port}{}", page.path()))
        .arg("--marks")
        .arg(format!("http://{}", relay.address()));
    // Collie's Browser cache, kept until the trial ends.
    let mut cache = None;
    match tool {
        Contender::Collie => {
            super::collie_home(
                &mut command,
                folder.path(),
                tools,
                &configuration(port, ad_hosts, row)?,
            )?;
            let browsers = trial::browser_cache(tools.browser)?;
            command.env(
                goodfetch_protocol::settings::BROWSERS_PATH_VARIABLE,
                browsers.path(),
            );
            cache = Some(browsers);
        }
        Contender::Playwright => {
            command
                .arg("--executable")
                .arg(tools.browser.join(trial::CHROME))
                .arg("--resolve")
                .arg(resolver_rules(ad_hosts));
            if row.routed() {
                command.arg("--routed");
            }
        }
        Contender::CollieProgram
        | Contender::PuppeteerLocator
        | Contender::PuppeteerPage
        | Contender::Stagehand
        | Contender::AgentBrowser
        | Contender::Chromiumoxide => {
            bail!("the page-load cell has no row of {}", tool.name())
        }
    }
    let subject = subject(page, row, number);
    let driven = match super::drive(command, folder.path(), &relay, &subject)? {
        Outcome::Done(driven) => driven,
        Outcome::Stopped(reason) => return Ok(Outcome::Stopped(reason)),
    };
    drop(cache);
    let printed: Printed = serde_json::from_str(&driven.printed)
        .with_context(|| format!("parsing the tool's output: {}", driven.printed))?;
    let (own, sent) = counted(&trial::observations(&serving.address)?);
    let (start, end) = (driven.start, driven.end);
    Ok(Outcome::Done(LoadTrial {
        page,
        row,
        number,
        load,
        status: printed.status,
        read: printed
            .read
            .map(|read| format!("{:x}", Sha256::digest(read.as_bytes()))),
        own,
        sent,
        cpu_seconds: (end.cpu_seconds - start.cpu_seconds).max(0.0),
        wall_seconds: end.at.duration_since(start.at).as_secs_f64(),
        peak_bytes: driven.peak_bytes,
        retried: None,
    }))
}

/// Every page-load trial, `trials` of each row on each page, in one random
/// order drawn from `state` by `next`.
pub(crate) fn order(
    trials: u32,
    state: &mut u64,
    next: fn(&mut u64) -> u64,
) -> Vec<(LoadPage, Row, u32)> {
    let order: Vec<_> = cells()
        .into_iter()
        .flat_map(|(page, row)| (1..=trials).map(move |number| (page, row, number)))
        .collect();
    super::shuffled(order, state, next)
}

/// The same-work check of the page-load cell: every (page, row) ran its
/// `trials` unless one was stopped twice; every trial got its page's
/// status 200, every own subresource and the `#content` most trials of its
/// page read, and sent the Proving ground something.
pub(crate) fn check(trials: u32, loads: &[LoadTrial], stopped: &[LoadStopped]) -> Vec<Finding> {
    let mut findings = Vec::new();
    for (page, row) in cells() {
        let count = loads
            .iter()
            .filter(|trial| trial.page == page && trial.row == row)
            .count();
        let was_stopped = stopped.iter().any(|run| run.page == page && run.row == row);
        if !was_stopped && u32::try_from(count).ok() != Some(trials) {
            findings.push(Finding::new(
                "trial_count",
                format!("collie page_load {} row {}", page.name(), row.number()),
                format!("{count} trials, where {trials} were declared"),
            ));
        }
    }
    let mut ordered: Vec<&LoadTrial> = loads.iter().collect();
    ordered.sort_by_key(|trial| (trial.page, trial.row, trial.number));
    for page in PAGES {
        let mut votes: BTreeMap<Option<&str>, usize> = BTreeMap::new();
        for trial in ordered.iter().filter(|trial| trial.page == page) {
            *votes.entry(trial.read.as_deref()).or_default() += 1;
        }
        let most = votes.values().copied().max().unwrap_or(0);
        let leaders: Vec<Option<&str>> = votes
            .iter()
            .filter(|(_, count)| **count == most)
            .map(|(read, _)| *read)
            .collect();
        for trial in ordered.iter().filter(|trial| trial.page == page) {
            let subject = subject(trial.page, trial.row, trial.number);
            let mut differs = Vec::new();
            if trial.status != Some(200) {
                differs.push(format!(
                    "the page answered {}",
                    trial
                        .status
                        .map_or_else(|| "no status".to_owned(), |status| status.to_string())
                ));
            }
            if trial.own != page.own() {
                differs.push(format!(
                    "{} of its {} own subresources were answered",
                    trial.own,
                    page.own()
                ));
            }
            if leaders.len() > 1 || !leaders.contains(&trial.read.as_deref()) {
                differs.push(format!(
                    "it read {}, where most trials of the page read otherwise",
                    trial.read.as_deref().unwrap_or("nothing")
                ));
            }
            if !differs.is_empty() {
                findings.push(Finding::new("different_work", subject, differs.join("; ")));
                continue;
            }
            if trial.sent == 0 {
                findings.push(Finding::new(
                    "nothing_counted",
                    subject,
                    "the Proving ground received no request for the page, so the trial measured nothing",
                ));
            }
        }
    }
    findings
}

/// One row's figures on one page.
#[derive(Serialize)]
pub(crate) struct RowFigures {
    pub(crate) page: LoadPage,
    pub(crate) row: Row,
    pub(crate) wall_seconds: Figure,
    pub(crate) cpu_seconds: Figure,
    pub(crate) peak_bytes: Figure,
    pub(crate) sent: Figure,
}

/// What one default costs on the plain page: the median wall time to
/// `load` of the row with it less the row without it.
#[derive(Serialize)]
pub(crate) struct Cost {
    pub(crate) default: &'static str,
    pub(crate) with: Row,
    pub(crate) without: Row,
    /// None while either row is not measured.
    pub(crate) seconds: Option<f64>,
}

/// The page-load cell's part of the report.
#[derive(Serialize)]
pub(crate) struct LoadSection {
    pub(crate) rows: Vec<RowFigures>,
    /// Each default's cost on the plain page.
    pub(crate) costs: Vec<Cost>,
    /// Row 7 less row 6 on the plain page: Playwright's route's cost, an
    /// indication while issue 1895 is open.
    pub(crate) route: Option<f64>,
    /// Whether row 1 is no slower than row 6 on each page; none where
    /// either is not measured.
    pub(crate) target: BTreeMap<LoadPage, Option<bool>>,
    /// Whether the target holds: on both pages.
    pub(crate) target_held: Option<bool>,
}

/// The defaults whose cost the report gives: each one's row with it, and
/// its row without it.
const COSTS: [(&str, Row, Row); 4] = [
    ("live_view.hiding", Row::Defaults, Row::LiveHidingOff),
    ("consent.handling", Row::LiveHidingOff, Row::ConsentOff),
    ("filter_lists", Row::LiveHidingOff, Row::FilterListsOff),
    (
        "security.raw_cdp_egress_proxy",
        Row::Defaults,
        Row::EgressRelayOff,
    ),
];

/// The page-load cell's section of the report.
pub(crate) fn section(loads: &[LoadTrial], stopped: &[LoadStopped]) -> LoadSection {
    let mut rows = Vec::new();
    let mut walls: BTreeMap<(LoadPage, Row), f64> = BTreeMap::new();
    for (page, row) in cells() {
        if let Some(stopped) = stopped
            .iter()
            .find(|run| run.page == page && run.row == row)
        {
            let figure = || Figure::NotMeasured {
                reason: format!(
                    "trial {} was stopped twice: {}",
                    stopped.number, stopped.reason
                ),
            };
            rows.push(RowFigures {
                page,
                row,
                wall_seconds: figure(),
                cpu_seconds: figure(),
                peak_bytes: figure(),
                sent: figure(),
            });
            continue;
        }
        let ran: Vec<&LoadTrial> = loads
            .iter()
            .filter(|trial| trial.page == page && trial.row == row)
            .collect();
        let values = |value: &dyn Fn(&LoadTrial) -> f64| -> Vec<f64> {
            ran.iter().map(|trial| value(trial)).collect()
        };
        let wall = values(&|trial| trial.wall_seconds);
        if !wall.is_empty() {
            walls.insert((page, row), median(&wall));
        }
        rows.push(RowFigures {
            page,
            row,
            wall_seconds: spread(&wall, Unit::Seconds),
            cpu_seconds: spread(&values(&|trial| trial.cpu_seconds), Unit::Seconds),
            peak_bytes: spread(&values(&|trial| float(trial.peak_bytes)), Unit::Bytes),
            sent: spread(&values(&|trial| float(trial.sent)), Unit::Count),
        });
    }
    let less = |page: LoadPage, with: Row, without: Row| -> Option<f64> {
        Some(walls.get(&(page, with))? - walls.get(&(page, without))?)
    };
    let costs = COSTS
        .into_iter()
        .map(|(default, with, without)| Cost {
            default,
            with,
            without,
            seconds: less(LoadPage::Plain, with, without),
        })
        .collect();
    let target: BTreeMap<LoadPage, Option<bool>> = PAGES
        .into_iter()
        .map(|page| {
            (
                page,
                less(page, Row::Defaults, Row::Playwright).map(|over| over <= 0.0),
            )
        })
        .collect();
    let target_held = target
        .values()
        .try_fold(true, |held, page| page.map(|page| held && page));
    LoadSection {
        rows,
        costs,
        route: less(LoadPage::Plain, Row::PlaywrightRouted, Row::Playwright),
        target,
        target_held,
    }
}

/// A difference of seconds as the report writes it, in milliseconds.
fn milliseconds(seconds: Option<f64>) -> String {
    seconds.map_or_else(
        || "not measured".to_owned(),
        |seconds| format!("{:+.1} ms", seconds * 1000.0),
    )
}

/// The page-load cell's section in Markdown, appended to `text`.
pub(crate) fn markdown(text: &mut String, section: &LoadSection) {
    let _ = writeln!(
        text,
        "\n### The page-load cell\n\nEach tool launches its own Chrome for Testing {} and loads \
         `page-load.test`, 150 subresources, to its `load` event: `/plain`, every subresource on \
         its own host, and `/ads`, 30 of them on six ad hosts the embedded Filter lists block. \
         Each figure is the median (lowest–highest) of the row's trials on the page; wall time \
         and CPU are between the marks around the load, CPU and peak memory the tool's process \
         tree, its browser in it; requests sent are those the Proving ground received. Collie's \
         paused requests and CDP commands are missing until its engine counts them \
         ({COUNTERS_ISSUE}); Playwright has neither.\n",
        crate::test_facts::CHROME_FOR_TESTING_BUILD
    );
    text.push_str(
        "| Row | Page | Wall to load | CPU | Peak memory | Requests sent | Paused requests | CDP commands |\n\
         |---|---|---:|---:|---:|---:|---:|---:|\n",
    );
    for figures in &section.rows {
        let counters = match figures.row.tool() {
            Contender::Collie => format!("missing ({COUNTERS_ISSUE})"),
            Contender::CollieProgram
            | Contender::Playwright
            | Contender::PuppeteerLocator
            | Contender::PuppeteerPage
            | Contender::Stagehand
            | Contender::AgentBrowser
            | Contender::Chromiumoxide => "n/a".to_owned(),
        };
        let _ = writeln!(
            text,
            "| {} {} | {} | {} | {} | {} | {} | {counters} | {counters} |",
            figures.row.number(),
            figures.row.label(),
            figures.page.name(),
            cell_text(&figures.wall_seconds),
            cell_text(&figures.cpu_seconds),
            cell_text(&figures.peak_bytes),
            cell_text(&figures.sent),
        );
    }
    text.push_str(
        "\nWhat each default costs on the plain page, in median wall time to `load`:\n\n",
    );
    for cost in &section.costs {
        let _ = writeln!(
            text,
            "- `{}`: {} (row {} less row {}).",
            cost.default,
            milliseconds(cost.seconds),
            cost.with.number(),
            cost.without.number()
        );
    }
    let _ = writeln!(
        text,
        "\nPlaywright's route, row 7 less row 6 on the plain page: {}, an indication while \
         #1895 is open.\n\nRow 5 is held by interception alone: the engine resolves and judges \
         each paused request's host (#2435), so the row pays one lookup and one judgment per \
         paused request.\n",
        milliseconds(section.route)
    );
    let pages: Vec<String> = section
        .target
        .iter()
        .map(|(page, held)| {
            format!(
                "{} {}",
                page.name(),
                match held {
                    Some(true) => "held",
                    Some(false) => "not held",
                    None => "not measured",
                }
            )
        })
        .collect();
    let _ = writeln!(
        text,
        "Target, row 1 no slower than row 6 on both pages: {} ({}). It fails nothing.",
        match section.target_held {
            Some(true) => "held",
            Some(false) => "not held",
            None => "not measured",
        },
        pages.join(", ")
    );
}

#[cfg(test)]
pub(super) mod tests;
