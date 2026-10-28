//! The page suite's robots.txt reading cases (ADRs 0086 and 0098, spec
//! #580, ticket #789): a Collie program's own navigation reads the site's
//! robots.txt and records in Provenance whether each page was allowed,
//! naming the group and the rule that disallowed it, and never gates,
//! counts or delays the navigation. Each Run reads an origin's robots.txt
//! once, into a copy of its own: the Goodfetch home holds no copy of the
//! file outside the Run's folder, where Foxhound's shared copy would be.
//!
//! The site is the `access-policy` family's first task's host, whose robots.txt
//! disallows `/private/` to every crawler in its `*` group and asks for a
//! Crawl-delay of 2 s (`proving-ground/src/raw_site/policy.rs`). One case
//! reads the `body-limits` family's named host instead, whose robots.txt is
//! a gzip body within the ratio that decodes past the 256 MiB a browser's
//! pipe frames in one message (`proving-ground/src/raw_site/body_limits.rs`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use goodfetch_protocol::page::PAGE_GOTO;
use serde_json::{Value, json};

use super::clean_up::provenance;
use super::navigation::{answered, call};
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// The host of the `access-policy` family's first task, whose site states
/// the robots.txt, reached over plain HTTP/1.1 as the browser trusts no
/// Proving ground authority until the engine's TLS relay does (ADR 0042).
const HOST: &str = "1.access-policy.test";
/// A page its robots.txt disallows, and one it allows.
const DISALLOWED: &str = "/private/catalog/2";
const ALLOWED: &str = "/catalog/1";
/// The rule that disallows it, in the `*` group.
const RULE: &str = "Disallow: /private/";
/// The Crawl-delay the robots.txt asks for.
const CRAWL_DELAY: Duration = Duration::from_secs(2);
/// A line of the robots.txt that no Provenance entry quotes: a file of the
/// Goodfetch home holding it holds a copy of the file.
const MARKER: &str = "Disallow: /example/page/disallowed.gif";
/// The Provenance action of a robots.txt reading.
const ACTION: &str = "robots_txt";
/// How long a reading may take to land in Provenance after its page
/// loaded: it runs beside the navigation, never before it.
const RECORDED_WITHIN: Duration = Duration::from_secs(20);

/// One robots.txt reading case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    DisallowedPageLoadsAndIsRecorded,
    TwoTabsAreNeitherDelayedNorRefused,
    FoxhoundsCopyIsUntouched,
    PastTheSizeIsUnreachable,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-programs-goto-to-a-disallowed-page-loads-and-provenance-names-the-disallowing-group" => {
                Self::DisallowedPageLoadsAndIsRecorded
            }
            "two-tabs-on-one-site-are-neither-delayed-nor-refused-under-crawl-delay-and-disallow" => {
                Self::TwoTabsAreNeitherDelayedNorRefused
            }
            "a-run-reads-robots-txt-once-per-origin-into-its-own-copy-and-leaves-foxhounds-untouched" => {
                Self::FoxhoundsCopyIsUntouched
            }
            "a-robots-txt-past-max-body-size-is-recorded-unreachable-and-the-page-loads" => {
                Self::PastTheSizeIsUnreachable
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target` against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::DisallowedPageLoadsAndIsRecorded => disallowed(connection, target, site, checks),
            Self::TwoTabsAreNeitherDelayedNorRefused => two_tabs(connection, site, checks),
            Self::FoxhoundsCopyIsUntouched => own_copy(connection, target, site, checks),
            Self::PastTheSizeIsUnreachable => past_the_size(connection, target, site, checks),
        }
    }
}

/// A `goto` to the page robots.txt disallows loads with its `200`, and to
/// the page it allows; Provenance records the first disallowed, naming the
/// `*` group and its rule, and the second allowed.
fn disallowed(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-robots-txt-disallowed";
    let (Some(disallowed), Some(allowed)) = (
        site.named_url(HOST, DISALLOWED),
        site.named_url(HOST, ALLOWED),
    ) else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let page_id = json!(page.page_id);
    for url in [&disallowed, &allowed] {
        let answer = call(connection, PAGE_GOTO.name, &page_id, &json!({ "url": url }))?;
        answered(&answer, &format!("`goto` to {url}"), url, 200, checks);
    }
    let entries = readings(target, run_id, 2);
    let of = |path: &str| {
        entries
            .iter()
            .find(|entry| entry["url"].as_str().is_some_and(|url| url.ends_with(path)))
    };
    let refused = of(DISALLOWED);
    checks.check(
        "Provenance records the disallowed page as not allowed by robots.txt",
        refused.is_some_and(|entry| entry["allowed"] == json!(false)),
    );
    checks.check(
        "it names the disallowing group, `*`, and its rule",
        refused.is_some_and(|entry| {
            entry["group"]["user_agents"] == json!(["*"]) && entry["rule"]["text"] == json!(RULE)
        }),
    );
    checks.check(
        "it names the page and the robots.txt it read",
        refused.is_some_and(|entry| {
            entry["page_id"] == page_id
                && entry["robots_txt"]
                    .as_str()
                    .is_some_and(|location| location.ends_with("/robots.txt"))
        }),
    );
    checks.check(
        "Provenance records the allowed page as allowed, with no group",
        of(ALLOWED)
            .is_some_and(|entry| entry["allowed"] == json!(true) && entry["group"].is_null()),
    );
    finish_run(connection, run_id)
}

/// The `body-limits` family's named host, whose robots.txt is past
/// `network.max_body_size` once decoded, and past the browser pipe's frame.
pub(super) const PAST_THE_SIZE_HOST: &str = "robots.body-limits.test";

/// A `goto` to a page whose site's robots.txt decodes past
/// `network.max_body_size` loads; the reading stops at the limit and
/// Provenance records the file unreachable, naming the setting, and the
/// page disallowed, as RFC 9309 reads an unreachable file. The browser's
/// connection goes on: a second `goto` loads too.
fn past_the_size(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-robots-txt-past-the-size";
    let Some(url) = site.named_url(PAST_THE_SIZE_HOST, "/page") else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let page_id = json!(page.page_id);
    let answer = call(connection, PAGE_GOTO.name, &page_id, &json!({ "url": url }))?;
    answered(&answer, &format!("`goto` to {url}"), &url, 200, checks);
    let entries = readings(target, run_id, 1);
    checks.check(
        "Provenance records the robots.txt unreachable, past `network.max_body_size`, and the page disallowed",
        entries.first().is_some_and(|entry| {
            entry["file"] == json!("unreachable")
                && entry["allowed"] == json!(false)
                && entry["reason"]
                    .as_str()
                    .is_some_and(|reason| reason.contains("`network.max_body_size`"))
        }),
    );
    let again = call(connection, PAGE_GOTO.name, &page_id, &json!({ "url": url }))?;
    answered(
        &again,
        &format!("a second `goto` to {url}"),
        &url,
        200,
        checks,
    );
    finish_run(connection, run_id)
}

/// Two pages of one Run each `goto` a page of the site, one robots.txt
/// disallows, one after the other: both load, and the site receives the
/// second document less than its Crawl-delay after the first.
fn two_tabs(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-robots-txt-two-tabs";
    let (Some(first_url), Some(second_url)) = (
        site.named_url(HOST, ALLOWED),
        site.named_url(HOST, DISALLOWED),
    ) else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let first = new_page(connection, run_id, checks)?;
    let second = new_page(connection, run_id, checks)?;
    let (Some(first), Some(second)) = (first, second) else {
        return finish_run(connection, run_id);
    };
    let before = arrivals(site);
    for (page, url) in [(&first, &first_url), (&second, &second_url)] {
        let answer = call(
            connection,
            PAGE_GOTO.name,
            &json!(page.page_id),
            &json!({ "url": url }),
        )?;
        answered(
            &answer,
            &format!("`goto` to {url} in its own tab"),
            url,
            200,
            checks,
        );
    }
    let documents: Vec<u64> = arrivals(site)
        .into_iter()
        .skip(before.len())
        .filter(|(target, _)| target == ALLOWED || target == DISALLOWED)
        .map(|(_, at)| at)
        .collect();
    let crawl_delay = u64::try_from(CRAWL_DELAY.as_micros()).unwrap_or(u64::MAX);
    checks.check(
        "the site receives each tab's document once",
        documents.len() == 2,
    );
    checks.check(
        "the second tab's document arrives less than the Crawl-delay after the first",
        matches!(documents.as_slice(), [first, second] if second.saturating_sub(*first) < crawl_delay),
    );
    finish_run(connection, run_id)
}

/// Two Runs each read the site's robots.txt once, however many pages
/// they load from it, and once each Run ended no file of the Goodfetch
/// home outside their folders holds the robots.txt.
fn own_copy(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (Some(disallowed), Some(allowed)) = (
        site.named_url(HOST, DISALLOWED),
        site.named_url(HOST, ALLOWED),
    ) else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    let run_ids = [
        "conformance-robots-txt-own-copy-1",
        "conformance-robots-txt-own-copy-2",
    ];
    for (number, run_id) in run_ids.iter().enumerate() {
        let before = robots_txt_requests(site);
        start_run(connection, site, run_id, &json!({}), checks)?;
        let Some(page) = new_page(connection, run_id, checks)? else {
            return finish_run(connection, run_id);
        };
        let page_id = json!(page.page_id);
        for url in [&allowed, &disallowed, &allowed] {
            let answer = call(connection, PAGE_GOTO.name, &page_id, &json!({ "url": url }))?;
            answered(&answer, &format!("`goto` to {url}"), url, 200, checks);
        }
        let read = readings(target, run_id, 3);
        checks.check(
            format!(
                "Run {}: Provenance records each of its three pages",
                number + 1
            ),
            read.len() == 3,
        );
        checks.check(
            format!(
                "Run {}: the site receives one robots.txt request",
                number + 1
            ),
            robots_txt_requests(site) - before == 1,
        );
        finish_run(connection, run_id)?;
    }
    let held = holding_the_file(target, &run_ids);
    checks.check(
        "no file of the Goodfetch home outside the Runs' folders holds the robots.txt",
        held.is_empty(),
    );
    Ok(())
}

/// The Run `run_id`'s robots.txt readings in Provenance, once there are
/// `count` of them or the wait ended.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness polls the Run trace on its own thread, outside any async worker"
)]
fn readings(target: &EngineTarget, run_id: &str, count: usize) -> Vec<Value> {
    let started = Instant::now();
    loop {
        let read: Vec<Value> = provenance(target, run_id)
            .into_iter()
            .filter(|entry| entry["action"] == json!(ACTION))
            .collect();
        if read.len() >= count || started.elapsed() > RECORDED_WITHIN {
            return read;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// Each request the Proving ground received on the site's host, its
/// target and arrival time in microseconds, in arrival order.
fn arrivals(site: &Site) -> Vec<(String, u64)> {
    site.proving_ground
        .received_requests()
        .unwrap_or_default()
        .every()
        .filter(|request| request["host"] == json!(HOST))
        .filter_map(|request| {
            Some((
                request["target"].as_str()?.to_owned(),
                request["timing"]["arrived_microseconds"].as_u64()?,
            ))
        })
        .collect()
}

/// How many requests for the site's robots.txt the Proving ground received.
fn robots_txt_requests(site: &Site) -> usize {
    arrivals(site)
        .iter()
        .filter(|(target, _)| target == "/robots.txt")
        .count()
}

/// The files of the Goodfetch home `target` names, outside the folders of
/// the Runs `run_ids`, that hold the robots.txt's [`MARKER`] line.
fn holding_the_file(target: &EngineTarget, run_ids: &[&str]) -> Vec<PathBuf> {
    let Some(home) = target.environment.get("GOODFETCH_HOME") else {
        return Vec::new();
    };
    let mut files = Vec::new();
    walk(Path::new(home), run_ids, &mut files);
    files
}

/// Adds to `found` each regular file under `folder`, no Run folder of
/// `run_ids` and no symbolic link followed, that holds [`MARKER`].
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn walk(folder: &Path, run_ids: &[&str], found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let is_run_folder = folder.file_name().is_some_and(|name| name == "runs")
            && run_ids.iter().any(|run_id| entry.file_name() == **run_id);
        if kind.is_dir() && !is_run_folder {
            walk(&path, run_ids, found);
        } else if kind.is_file()
            && std::fs::read(&path).is_ok_and(|bytes| {
                bytes
                    .windows(MARKER.len())
                    .any(|window| window == MARKER.as_bytes())
            })
        {
            found.push(path);
        }
    }
}
