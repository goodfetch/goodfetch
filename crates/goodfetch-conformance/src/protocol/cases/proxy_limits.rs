//! The page suite's proxy limit cases (ADRs 0053, 0086 and 0088; spec #577,
//! ticket #709), part of the Session ownership and proxy limit cases, on
//! the Proving ground's `limited` forward proxy, which relays one request
//! at a time and answers any other `429` with `Proxy-Status`, as a vendor's
//! plan does, and on its quiet page, which gives a browser nothing to load
//! after its document, so each navigation is one relay the proxy records.
//! The proxy stops counting a request once it read the site's answer,
//! before the browser loads it, and counts none for the robots.txt the
//! engine reads of each document's origin or the icon the browser asks
//! for, so what a navigation meets there follows from the Permits alone.
//! Two navigations the engine admits together meet there with the proxy's
//! hold on: it keeps the first until the other's answer came, however late
//! the browser sends either.
//! A proxy's own 429 is a proxy error, never a Block cause, and lowers the
//! proxy's ceiling for the user's other runs, which then take turns instead
//! of failing; `politeness.proxy.concurrency` and `politeness.proxy.rate`
//! hold a program's own navigation; `politeness.sharing: run` keeps the
//! ceiling a Run learned to itself; and a site's 429 lowers nothing and
//! writes no domain limit to the Politeness file. Each
//! case uses a proxy credential of its own, its key, so no case meets the
//! ceiling another taught.

use std::collections::BTreeMap;
use std::time::Duration;

use goodfetch_protocol::page::proxy::PROXY_UNAVAILABLE;
use goodfetch_protocol::page::{BLOCKED, PAGE_GOTO, PageOpened};
use serde_json::{Value, json};

use super::blocks::politeness_names;
use super::proxies::server;
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;
use crate::proving_ground::ProvingGround;

/// How long concurrent navigations may take to answer together, a wait
/// for a Permit included.
const TOGETHER: Duration = Duration::from_secs(120);

/// One proxy limit case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Learned,
    OwnLimits,
    RunsOwn,
    SitesRefusal,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "two-runs-through-a-limited-proxy-take-turns-under-the-ceiling-the-first-learned" => {
                Self::Learned
            }
            "the-proxy-concurrency-and-rate-hold-a-programs-own-navigation" => Self::OwnLimits,
            "a-run-that-shares-no-politeness-keeps-the-ceiling-it-learned-to-itself" => {
                Self::RunsOwn
            }
            "a-sites-429-through-a-proxy-lowers-no-ceiling-and-writes-no-domain-limit" => {
                Self::SitesRefusal
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
        let Some(limited) = server(site, "limited", checks) else {
            return Ok(());
        };
        match self {
            Self::Learned => learned(connection, site, &limited, checks),
            Self::OwnLimits => own_limits(connection, site, &limited, checks),
            Self::RunsOwn => runs_own(connection, site, &limited, checks),
            Self::SitesRefusal => sites_refusal(connection, target, site, &limited, checks),
        }
    }
}

/// A Run's settings: the proxy `server` used as `username`, and `more`.
fn through(server: &str, username: &str, more: &Value) -> Value {
    let mut settings = more.clone();
    settings["proxy"] = json!({ "server": server, "username": username });
    settings
}

/// Starts the Run `run_id` with `settings` and opens `pages` of its pages.
fn run_with_pages(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    settings: &Value,
    pages: usize,
    checks: &mut Checks,
) -> Result<Vec<PageOpened>, Gone> {
    start_run(connection, site, run_id, settings, checks)?;
    let mut opened = Vec::new();
    for _ in 0..pages {
        if let Some(page) = new_page(connection, run_id, checks)? {
            opened.push(page);
        }
    }
    Ok(opened)
}

/// The quiet page's URL for `mark`, with `status` when one is asked.
fn quiet(site: &Site, mark: &str, status: Option<u16>) -> String {
    let query = status.map_or_else(String::new, |status| format!("&status={status}"));
    site.named_url("scope.test", &format!("/quiet?mark={mark}{query}"))
        .unwrap_or_default()
}

/// Sends a `page.goto` for each page and URL at once, then answers each
/// one's answer, in the order given.
fn together(
    connection: &mut Connection,
    gotos: &[(&PageOpened, String)],
) -> Result<Vec<Value>, Gone> {
    answers_of(connection, gotos, || {})
}

/// Sends a `page.goto` for each page and URL at once through the `limited`
/// proxy held, so the first to reach it stays there until another's answer
/// came, then releases it, and answers each one's answer, in the order
/// given: navigations the engine admits together meet at the proxy however
/// late the browser sends one, while one the engine holds back answers
/// nothing until the hold is released.
fn meeting(
    connection: &mut Connection,
    site: &Site,
    gotos: &[(&PageOpened, String)],
    checks: &mut Checks,
) -> Result<Vec<Value>, Gone> {
    let held = site.proving_ground.set_limited_proxy_hold(true).is_ok();
    checks.check(
        "the Proving ground holds the limited proxy's first request",
        held,
    );
    let release = || {
        let _ = site.proving_ground.set_limited_proxy_hold(false);
    };
    let answers = answers_of(connection, gotos, release);
    let released = site.proving_ground.set_limited_proxy_hold(false).is_ok();
    checks.check(
        "the Proving ground releases the limited proxy's hold",
        released,
    );
    answers
}

/// Sends the `gotos` at once, calls `after_first` once the first answer
/// came, and answers each one's answer, in the order given.
fn answers_of(
    connection: &mut Connection,
    gotos: &[(&PageOpened, String)],
    after_first: impl FnOnce(),
) -> Result<Vec<Value>, Gone> {
    let mut after_first = Some(after_first);
    let mut waiting = BTreeMap::new();
    for (at, (page, url)) in gotos.iter().enumerate() {
        let id = connection.request(
            PAGE_GOTO.name,
            &json!({ "page_id": page.page_id, "url": url }),
        )?;
        waiting.insert(id, at);
    }
    let mut answers = vec![Value::Null; gotos.len()];
    while !waiting.is_empty() {
        let message = connection.next_within(TOGETHER)?;
        if message.get("method").is_some() {
            continue;
        }
        if let Some(at) = message["id"].as_i64().and_then(|id| waiting.remove(&id)) {
            answers[at] = message;
            if let Some(after_first) = after_first.take() {
                after_first();
            }
        }
    }
    Ok(answers)
}

/// How many `answers` loaded their page, and how many raised `code`.
fn tally(answers: &[Value], code: &str) -> (usize, usize) {
    let loaded = answers
        .iter()
        .filter(|answer| answer["result"]["response"]["status"] == json!(200))
        .count();
    let raised = answers
        .iter()
        .filter(|answer| answer["error"]["data"]["code"] == json!(code))
        .count();
    (loaded, raised)
}

/// The relays the Proving ground's proxies carried out to the quiet page
/// with a mark starting `prefix`, in arrival order: each one's outcome and
/// the microsecond it arrived at.
fn relays(site: &Site, prefix: &str) -> Vec<(String, u64)> {
    site.proving_ground
        .observations()
        .ok()
        .and_then(|observations| observations["relays"].as_array().cloned())
        .unwrap_or_default()
        .into_iter()
        .filter(|relay| {
            relay["target"]
                .as_str()
                .and_then(|target| target.split_once("/quiet?mark="))
                .is_some_and(|(_, mark)| mark.starts_with(prefix))
        })
        .filter_map(|relay| {
            Some((
                relay["outcome"].as_str()?.to_owned(),
                relay["at_microseconds"].as_u64()?,
            ))
        })
        .collect()
}

/// The Proving ground's microsecond now or a little before: the stamp of
/// the latest connection it accepted, after a fresh client opened one.
fn now(site: &Site) -> Option<u64> {
    let fresh = ProvingGround::local(site.proving_ground.address());
    let observations = fresh.observations().ok()?;
    observations["connections"]
        .as_array()?
        .iter()
        .filter_map(|connection| connection["at_microseconds"].as_u64())
        .max()
}

/// How many of the relays to marks starting `prefix` the proxy refused
/// with its own `429`.
fn refused(site: &Site, prefix: &str) -> usize {
    relays(site, prefix)
        .iter()
        .filter(|(outcome, _)| outcome == "too_many_requests")
        .count()
}

/// Two pages of one Run through the `limited` proxy at once: one loads,
/// the other meets the proxy's own 429, raised as `proxy_unavailable`,
/// retryable, never `blocked`, on that call or the page's next. The
/// ceiling it taught then holds the first Run and a second one: their two
/// navigations at once both load, the second waiting for the first, and
/// the proxy refuses no more.
fn learned(
    connection: &mut Connection,
    site: &Site,
    limited: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let settings = through(limited, "learned", &json!({}));
    let first = run_with_pages(
        connection,
        site,
        "conformance-proxy-learned-first",
        &settings,
        2,
        checks,
    )?;
    let second = run_with_pages(
        connection,
        site,
        "conformance-proxy-learned-second",
        &settings,
        1,
        checks,
    )?;
    if let ([one, two], [other]) = (first.as_slice(), second.as_slice()) {
        let met = meeting(
            connection,
            site,
            &[
                (one, quiet(site, "learned-1", None)),
                (two, quiet(site, "learned-2", None)),
            ],
            checks,
        )?;
        checks.check(
            "of two navigations at once through the limited proxy, one loads and the other raises the proxy's own `proxy_unavailable`",
            tally(&met, PROXY_UNAVAILABLE.name) == (1, 1),
        );
        checks.check(
            "the proxy's own 429 says a retry may succeed and is never a Block cause",
            met.iter()
                .filter(|answer| answer["error"]["data"]["code"] == json!(PROXY_UNAVAILABLE.name))
                .all(|answer| answer["error"]["data"]["retryable"] == json!(true))
                && tally(&met, BLOCKED.name).1 == 0,
        );
        let taking_turns = together(
            connection,
            &[
                (two, quiet(site, "learned-3", None)),
                (other, quiet(site, "learned-4", None)),
            ],
        )?;
        checks.check(
            "under the ceiling the first Run learned, its navigation and another Run's at once both load, the page the proxy refused raising no Block cause",
            tally(&taking_turns, PROXY_UNAVAILABLE.name) == (2, 0),
        );
        checks.check(
            "the proxy refused only the navigation that taught the ceiling",
            refused(site, "learned-") == 1,
        );
    }
    finish_run(connection, "conformance-proxy-learned-first")?;
    finish_run(connection, "conformance-proxy-learned-second")
}

/// `politeness.proxy.concurrency` 1 holds a program's two navigations at
/// once through the `limited` proxy to one at a time, so both load; and
/// `politeness.proxy.rate` 2 starts a program's navigations through the
/// `open` proxy at least half a second apart.
fn own_limits(
    connection: &mut Connection,
    site: &Site,
    limited: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let concurrency = through(
        limited,
        "own-concurrency",
        &json!({ "politeness": { "proxy": { "concurrency": 1 } } }),
    );
    let pages = run_with_pages(
        connection,
        site,
        "conformance-proxy-concurrency",
        &concurrency,
        2,
        checks,
    )?;
    if let [one, two] = pages.as_slice() {
        let answers = together(
            connection,
            &[
                (one, quiet(site, "concurrency-1", None)),
                (two, quiet(site, "concurrency-2", None)),
            ],
        )?;
        checks.check(
            "with `politeness.proxy.concurrency` 1, two navigations at once through the limited proxy both load",
            tally(&answers, PROXY_UNAVAILABLE.name) == (2, 0)
                && refused(site, "concurrency-") == 0,
        );
    }
    finish_run(connection, "conformance-proxy-concurrency")?;
    let Some(open) = server(site, "open", checks) else {
        return Ok(());
    };
    let rate = through(
        &open,
        "own-rate",
        &json!({ "politeness": { "proxy": { "rate": 2 } } }),
    );
    let pages = run_with_pages(connection, site, "conformance-proxy-rate", &rate, 3, checks)?;
    if let [one, two, three] = pages.as_slice() {
        let sent = now(site);
        let answers = together(
            connection,
            &[
                (one, quiet(site, "rate-1", None)),
                (two, quiet(site, "rate-2", None)),
                (three, quiet(site, "rate-3", None)),
            ],
        )?;
        let mut starts: Vec<u64> = relays(site, "rate-")
            .into_iter()
            .map(|(_, at)| at)
            .collect();
        starts.sort_unstable();
        checks.check(
            "with `politeness.proxy.rate` 2, three navigations at once through the open proxy all load",
            tally(&answers, PROXY_UNAVAILABLE.name) == (3, 0) && starts.len() == 3,
        );
        // The k-th to arrive started no sooner than k - 1 intervals after
        // the first start, itself no sooner than they were sent; however
        // late the browser or the proxy is, a request reaches the proxy
        // only after its start, so the bound holds under any load.
        checks.check(
            "with `politeness.proxy.rate` 2, the proxy receives the second no sooner than half a second, and the third a second, after they were sent",
            sent.is_some_and(|sent| {
                starts
                    .iter()
                    .zip(0_u64..)
                    .all(|(at, earlier)| at.saturating_sub(sent) >= earlier * 500_000)
            }),
        );
    }
    finish_run(connection, "conformance-proxy-rate")
}

/// A Run with `politeness.sharing: run` learns a ceiling from the proxy's
/// own 429 and holds its own navigations to it, while a Run that shares
/// Politeness never meets it: its two navigations at once through the same
/// proxy and credential meet the proxy's 429 again.
fn runs_own(
    connection: &mut Connection,
    site: &Site,
    limited: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let alone = through(
        limited,
        "alone",
        &json!({ "politeness": { "sharing": "run" } }),
    );
    let shared = through(limited, "alone", &json!({}));
    let own = run_with_pages(
        connection,
        site,
        "conformance-proxy-alone",
        &alone,
        2,
        checks,
    )?;
    let sharing = run_with_pages(
        connection,
        site,
        "conformance-proxy-alone-sharing",
        &shared,
        2,
        checks,
    )?;
    if let ([one, two], [three, four]) = (own.as_slice(), sharing.as_slice()) {
        let taught = meeting(
            connection,
            site,
            &[
                (one, quiet(site, "alone-1", None)),
                (two, quiet(site, "alone-2", None)),
            ],
            checks,
        )?;
        let held = together(
            connection,
            &[
                (one, quiet(site, "alone-3", None)),
                (two, quiet(site, "alone-4", None)),
            ],
        )?;
        let unshared = meeting(
            connection,
            site,
            &[
                (three, quiet(site, "alone-5", None)),
                (four, quiet(site, "alone-6", None)),
            ],
            checks,
        )?;
        checks.check(
            "a Run that shares no Politeness meets the proxy's own 429 once, then holds its own navigations to the ceiling it learned",
            tally(&taught, PROXY_UNAVAILABLE.name) == (1, 1)
                && tally(&held, PROXY_UNAVAILABLE.name) == (2, 0),
        );
        checks.check(
            "a Run that shares Politeness never meets the ceiling a Run of its own learned",
            tally(&unshared, PROXY_UNAVAILABLE.name) == (1, 1),
        );
    }
    finish_run(connection, "conformance-proxy-alone")?;
    finish_run(connection, "conformance-proxy-alone-sharing")
}

/// A site's 429 through the `limited` proxy is the site's, never the
/// proxy's error, and lowers no ceiling: two navigations at once through
/// the proxy afterwards still meet the proxy's own 429. Nor does it write
/// a domain limit: the Politeness file names no `scope.test` key.
fn sites_refusal(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    limited: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let settings = through(limited, "site", &json!({}));
    let pages = run_with_pages(
        connection,
        site,
        "conformance-proxy-site",
        &settings,
        2,
        checks,
    )?;
    if let [one, two] = pages.as_slice() {
        let sites = together(connection, &[(one, quiet(site, "site-1", Some(429)))])?;
        checks.check(
            "a site's 429 through the proxy is never the proxy's own error",
            tally(&sites, PROXY_UNAVAILABLE.name).1 == 0,
        );
        let after = meeting(
            connection,
            site,
            &[
                (one, quiet(site, "site-2", None)),
                (two, quiet(site, "site-3", None)),
            ],
            checks,
        )?;
        checks.check(
            "after a site's 429, two navigations at once still meet the proxy's own 429: no ceiling was lowered",
            tally(&after, PROXY_UNAVAILABLE.name) == (1, 1),
        );
    }
    finish_run(connection, "conformance-proxy-site")?;
    checks.check(
        "a site's 429 through the proxy writes no domain limit: the Politeness file names no `scope.test` key",
        !politeness_names(target, "scope.test"),
    );
    Ok(())
}
