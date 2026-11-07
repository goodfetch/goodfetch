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
//! Another reads the family's slow host, whose robots.txt answers 1.5 s
//! late, so a Session closed right after its page loaded closes while that
//! page's reading is under way.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use goodfetch_protocol::home::RUNS;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::request::PAGE_REQUEST;
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE};
use goodfetch_protocol::page::waits::PAGE_WAIT_FOR_URL;
use goodfetch_protocol::page::{LOCATOR_CLICK, PAGE_GOTO, PageOpened};
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
    EveryDocumentIsRecorded,
    PageRequestIsRecorded,
    NotReadIsNotKept,
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
            "a-document-a-click-or-a-script-navigated-to-is-recorded-as-a-gotos-is" => {
                Self::EveryDocumentIsRecorded
            }
            "a-page-request-is-recorded-as-a-document-is" => Self::PageRequestIsRecorded,
            "a-reading-a-closed-session-cut-short-is-not-read-and-the-next-document-reads-the-file" => {
                Self::NotReadIsNotKept
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
            Self::EveryDocumentIsRecorded => every_document(connection, target, site, checks),
            Self::PageRequestIsRecorded => page_request(connection, target, site, checks),
            Self::NotReadIsNotKept => not_read(connection, target, site, checks),
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

/// On a Run whose `network.max_body_size` is 1 MiB, a `goto` to a page
/// whose site's robots.txt decodes past it loads; the reading stops at the limit and
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
    // 1 MiB, so the read reaches the limit within a few chunks however
    // loaded the machine.
    let settings = json!({ "network": { "max_body_size": 1_048_576 } });
    start_run(connection, site, run_id, &settings, checks)?;
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
/// home outside the Runs' folders, where each keeps its own copy in its run
/// state, holds the robots.txt.
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
    let held = holding_the_file(target);
    checks.check(
        "no file of the Goodfetch home outside the Runs' folders holds the robots.txt",
        held.is_empty(),
    );
    Ok(())
}

/// Pages robots.txt disallows that a click and a script reach.
const CLICKED: &str = "/private/clicked";
const SCRIPTED: &str = "/private/scripted";

/// After a `goto`, a click on a link and a script's `location.assign` each
/// navigate the page to a page robots.txt disallows: Provenance records
/// each of the two documents disallowed, naming the rule, as it does a
/// `goto`'s, however its navigation started.
fn every_document(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-robots-txt-every-document";
    let (Some(allowed), Some(clicked), Some(scripted)) = (
        site.named_url(HOST, ALLOWED),
        site.named_url(HOST, CLICKED),
        site.named_url(HOST, SCRIPTED),
    ) else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let page_id = json!(page.page_id);
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &page_id,
        &json!({ "url": allowed }),
    )?;
    answered(
        &answer,
        &format!("`goto` to {allowed}"),
        &allowed,
        200,
        checks,
    );
    call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({
            "function": "(url) => { const link = document.createElement('a'); link.id = 'away'; link.href = url; link.textContent = 'away'; document.body.prepend(link); }",
            "arg": clicked,
        }),
    )?;
    let id = connection.request(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page.page_id, "selector": "#away" } }),
    )?;
    let click = connection.browser_answer(id)?.message;
    let reached = wait_for(connection, &page_id, CLICKED)?;
    checks.check(
        "a click on a link navigates the page to the page it names",
        click.get("result").is_some() && reached,
    );
    call(
        connection,
        PAGE_EVALUATE.name,
        &page_id,
        &json!({
            "function": "(url) => { setTimeout(() => location.assign(url), 0); }",
            "arg": scripted,
        }),
    )?;
    let reached = wait_for(connection, &page_id, SCRIPTED)?;
    checks.check(
        "a script's `location.assign` navigates the page to the page it names",
        reached,
    );
    let entries = readings(target, run_id, 3);
    for path in [CLICKED, SCRIPTED] {
        checks.check(
            format!("Provenance records {path} not allowed by robots.txt, naming its rule"),
            reading_of(&entries, path).is_some_and(|entry| {
                entry["page_id"] == page_id
                    && entry["file"] == json!("found")
                    && entry["allowed"] == json!(false)
                    && entry["rule"]["text"] == json!(RULE)
            }),
        );
    }
    finish_run(connection, run_id)
}

/// A page robots.txt disallows that a `page.request` fetches.
const REQUESTED: &str = "/private/requested";

/// A `page.request` of a page robots.txt disallows answers its response,
/// and Provenance records it disallowed, naming the rule, as it records a
/// document.
fn page_request(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-robots-txt-page-request";
    let Some(requested) = site.named_url(HOST, REQUESTED) else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let page_id = json!(page.page_id);
    let answer = call(
        connection,
        PAGE_REQUEST.name,
        &page_id,
        &json!({ "url": requested }),
    )?;
    checks.check(
        "the `page.request` answers its response",
        answer["result"]["url"] == json!(requested),
    );
    let entries = readings(target, run_id, 1);
    checks.check(
        "Provenance records the requested page not allowed by robots.txt, naming its rule",
        reading_of(&entries, REQUESTED).is_some_and(|entry| {
            entry["page_id"] == page_id
                && entry["file"] == json!("found")
                && entry["allowed"] == json!(false)
                && entry["rule"]["text"] == json!(RULE)
        }),
    );
    finish_run(connection, run_id)
}

/// The `access-policy` family's host whose robots.txt answers 1.5 s late.
pub(super) const SLOW_ROBOTS_HOST: &str = "slow-robots.access-policy.test";

/// A Session closed as soon as its page's `goto` answered cuts that page's
/// reading short before the site's robots.txt answered: Provenance records
/// it `not_read`, with its reason and no verdict, and the Run keeps
/// nothing of it, so a later Session's page of the same origin fetches the
/// file and is recorded allowed by it.
fn not_read(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-robots-txt-not-read";
    let Some(allowed) = site.named_url(SLOW_ROBOTS_HOST, ALLOWED) else {
        checks.check("the Proving ground serves the case's pages", false);
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(first) = page_in_new_session(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &json!(first.page_id),
        &json!({ "url": allowed }),
    )?;
    answered(
        &answer,
        &format!("`goto` to {allowed}"),
        &allowed,
        200,
        checks,
    );
    let id = connection.request(
        SESSION_CLOSE.name,
        &json!({ "session_id": first.session_id }),
    )?;
    let closed = connection.browser_answer(id)?.message;
    checks.check("the first Session closes", closed.get("result").is_some());
    let Some(second) = page_in_new_session(connection, run_id, checks)? else {
        return finish_run(connection, run_id);
    };
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &json!(second.page_id),
        &json!({ "url": allowed }),
    )?;
    answered(
        &answer,
        &format!("a later Session's `goto` to {allowed}"),
        &allowed,
        200,
        checks,
    );
    let entries = readings(target, run_id, 2);
    let of = |page: &PageOpened| {
        entries
            .iter()
            .find(|entry| entry["page_id"] == json!(page.page_id))
    };
    checks.check(
        "Provenance records the reading the close cut short `not_read`, with its reason and no verdict",
        of(&first).is_some_and(|entry| {
            entry["file"] == json!("not_read")
                && entry["reason"].is_string()
                && entry.get("allowed").is_none()
        }),
    );
    checks.check(
        "the later Session's page is recorded allowed by the site's robots.txt",
        of(&second).is_some_and(|entry| {
            entry["file"] == json!("found") && entry["allowed"] == json!(true)
        }),
    );
    finish_run(connection, run_id)
}

/// Opens a page in a new Session of the Run `run_id`.
fn page_in_new_session(
    connection: &mut Connection,
    run_id: &str,
    checks: &mut Checks,
) -> Result<Option<PageOpened>, Gone> {
    let id = connection.request(RUN_NEW_SESSION.name, &json!({ "run_id": run_id }))?;
    let opened = connection.browser_answer(id)?.message;
    let id = connection.request(
        SESSION_NEW_PAGE.name,
        &json!({ "session_id": opened["result"]["session_id"] }),
    )?;
    let paged = connection.browser_answer(id)?.message;
    let page = serde_json::from_value::<PageOpened>(paged["result"].clone()).ok();
    checks.check("a new Session opens a page", page.is_some());
    Ok(page)
}

/// Whether the page `page_id` reached a URL ending with `path`, waiting
/// for it under the Run's `timeout.wait`.
fn wait_for(connection: &mut Connection, page_id: &Value, path: &str) -> Result<bool, Gone> {
    let waited = call(
        connection,
        PAGE_WAIT_FOR_URL.name,
        page_id,
        &json!({ "url": { "kind": "glob", "pattern": format!("**{path}") } }),
    )?;
    Ok(waited.get("result").is_some())
}

/// The reading among `entries` of the URL ending with `path`.
fn reading_of<'a>(entries: &'a [Value], path: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find(|entry| entry["url"].as_str().is_some_and(|url| url.ends_with(path)))
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

/// The files of the Goodfetch home `target` names, outside the Runs'
/// folders, that hold the robots.txt's [`MARKER`] line.
fn holding_the_file(target: &EngineTarget) -> Vec<PathBuf> {
    let Some(home) = target.environment.get("GOODFETCH_HOME") else {
        return Vec::new();
    };
    let mut files = Vec::new();
    walk(Path::new(home), &mut files);
    files
}

/// Adds to `found` each regular file under `folder`, no Run's folder and
/// no symbolic link followed, that holds [`MARKER`].
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn walk(folder: &Path, found: &mut Vec<PathBuf>) {
    let Ok(entries) = std::fs::read_dir(folder) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(kind) = entry.file_type() else {
            continue;
        };
        let is_run_folder = folder.file_name().is_some_and(|name| name == RUNS);
        if kind.is_dir() && !is_run_folder {
            walk(&path, found);
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
