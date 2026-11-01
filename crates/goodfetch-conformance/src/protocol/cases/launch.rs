//! The launch cases (spec #577, ticket #696), judged from the Proving
//! ground's own observations: a headless page shows the site the identity
//! its browser shows with a window, with no `HeadlessChrome` anywhere, and a
//! page behind another page of its Session keeps its 100 ms timer and its
//! focus (ADR 0085's background-tab question).

use std::time::Duration;

use goodfetch_protocol::page::PAGE_GOTO;
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::proving_ground::ReceivedRequests;

/// What the identity page reports, beside that it ran.
const IDENTITY_CHECKS: [&str; 11] = [
    "user_agent",
    "brands",
    "full_version_list",
    "platform",
    "platform_version",
    "architecture",
    "bitness",
    "mobile",
    "model",
    "wow64",
    "form_factors",
];

/// The host the identity page answers on in a secure context over plain
/// HTTP, where every client hint exists.
pub(super) const IDENTITY_HOST: &str = "access.localhost";

/// How often, and how many times, the harness reads the observations while
/// it waits for a page's script to report: 20 s in all.
const POLL: Duration = Duration::from_millis(100);
const POLLS: u32 = 200;

/// The fewest times a second the timer of a page behind another must fire:
/// twice the one wake-up a second a throttled page gets, so a loaded host,
/// which slows an unthrottled timer, is never taken for a throttled one.
const UNTHROTTLED_RATE: f64 = 2.0;

/// The reports of the page at `path`, as `(check, observed)`, once `done`
/// holds of them or the polls run out; `None` when the observations cannot
/// be read.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness polls the Proving ground on its own thread, never an async worker"
)]
pub(super) fn reports_once(
    site: &Site,
    path: &str,
    done: impl Fn(&[(String, String)]) -> bool,
) -> Option<Vec<(String, String)>> {
    let mut seen = Vec::new();
    for _ in 0..POLLS {
        seen = reports(&site.proving_ground.observations().ok()?, path);
        if done(&seen) {
            break;
        }
        std::thread::sleep(POLL);
    }
    Some(seen)
}

pub(super) fn reports(observations: &Value, path: &str) -> Vec<(String, String)> {
    observations["reports"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|report| report["page"] == path)
        .filter_map(|report| {
            Some((
                report["check"].as_str()?.to_owned(),
                report["observed"].as_str()?.to_owned(),
            ))
        })
        .collect()
}

/// The values of the header `name` on every request to `host`.
fn header_values(observations: &Value, host: &str, name: &str) -> Vec<String> {
    ReceivedRequests::of(observations)
        .every()
        .filter(|request| {
            request["host"]
                .as_str()
                .is_some_and(|seen| seen.split(':').next() == Some(host))
        })
        .flat_map(|request| request["headers"].as_array().cloned().unwrap_or_default())
        .filter(|header| {
            header[0]
                .as_str()
                .is_some_and(|seen| seen.eq_ignore_ascii_case(name))
        })
        .filter_map(|header| header[1].as_str().map(str::to_owned))
        .collect()
}

/// Opens a page of the Run `run_id` and takes it to `url`; whether the
/// document answered 200.
fn open_at(
    connection: &mut Connection,
    run_id: &str,
    url: &str,
    checks: &mut Checks,
) -> Result<bool, Gone> {
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(false);
    };
    let id = connection.request(
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )?;
    let answer = connection.browser_answer(id)?.message;
    Ok(answer["result"]["response"]["status"] == json!(200))
}

/// A headless page (`browser.headless` by default) shows the site one
/// `Chrome/` identity: its requests and its script read the same user
/// agent, the high-entropy client hints reach the site, and nothing names
/// `HeadlessChrome` (ADRs 0021 and 0092).
pub(super) fn headless_identity(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-headless-identity";
    let url = format!("http://{IDENTITY_HOST}:{}/identity", site.port);
    start_run(connection, site, run_id, &json!({}), checks)?;
    let loaded = open_at(connection, run_id, &url, checks)?;
    checks.check("the identity page loads", loaded);
    let reported = reports_once(site, "/identity", |seen| {
        IDENTITY_CHECKS
            .iter()
            .all(|check| seen.iter().any(|(reported, _)| reported == check))
    });
    finish_run(connection, run_id)?;
    let observations = site.proving_ground.observations().ok();
    checks.check(
        "the Proving ground's observations can be read",
        reported.is_some() && observations.is_some(),
    );
    let (Some(reported), Some(observations)) = (reported, observations) else {
        return Ok(());
    };
    let read = |check: &str| {
        reported
            .iter()
            .find(|(reported, _)| reported == check)
            .map(|(_, observed)| observed.clone())
    };
    checks.check(
        "the page's script reads its identity",
        IDENTITY_CHECKS.iter().all(|check| read(check).is_some()),
    );
    let user_agents = header_values(&observations, IDENTITY_HOST, "user-agent");
    let page_user_agent = read("user_agent").unwrap_or_default();
    checks.check(
        "every request and the page's script show one user agent",
        !user_agents.is_empty() && user_agents.iter().all(|agent| *agent == page_user_agent),
    );
    checks.check(
        "it names `Chrome/`, not `HeadlessChrome/`",
        page_user_agent.contains(" Chrome/"),
    );
    let mut every_value = user_agents;
    for hint in [
        "sec-ch-ua",
        "sec-ch-ua-full-version-list",
        "sec-ch-ua-platform-version",
    ] {
        let values = header_values(&observations, IDENTITY_HOST, hint);
        checks.check(
            format!("the site receives `{hint}`"),
            values.iter().any(|value| !value.is_empty()),
        );
        every_value.extend(values);
    }
    every_value.extend(reported.into_iter().map(|(_, observed)| observed));
    checks.check(
        "no request header and no value the page reads names `HeadlessChrome`",
        every_value
            .iter()
            .all(|value| !value.contains("HeadlessChrome")),
    );
    Ok(())
}

/// How many times a second the timer page's timer fired between the
/// `from`th `ticks` report and the last, and how many reports that spans.
fn rate_from(reported: &[(String, String)], from: usize) -> (f64, usize) {
    let ticks: Vec<(f64, f64)> = reported
        .iter()
        .filter(|(check, _)| check == "ticks")
        .skip(from)
        .filter_map(|(_, observed)| {
            let (count, elapsed) = observed.split_once(' ')?;
            Some((count.parse().ok()?, elapsed.parse().ok()?))
        })
        .collect();
    match (ticks.first(), ticks.last()) {
        (Some(first), Some(last)) if last.1 > first.1 => (
            (last.0 - first.0) * 1000.0 / (last.1 - first.1),
            ticks.len(),
        ),
        _ => (0.0, ticks.len()),
    }
}

/// A page behind another page of its Session keeps firing its 100 ms timer
/// and reports that it has focus (ADR 0085's question).
pub(super) fn background_timer(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-background-timer";
    checks.check(
        "the Proving ground serves the access family",
        site.access_origin.is_some(),
    );
    let (Some(origin), Some(front_url)) = (site.access_origin.clone(), site.page_url.clone())
    else {
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let behind = open_at(connection, run_id, &format!("{origin}/timer"), checks)?;
    let front = open_at(connection, run_id, &front_url, checks)?;
    checks.check(
        "the timer page and a page in front of it load",
        behind && front,
    );
    let ticks =
        |seen: &[(String, String)]| seen.iter().filter(|(check, _)| check == "ticks").count();
    let before = reports_once(site, "/timer", |_| true).map_or(0, |seen| ticks(&seen));
    let reported = reports_once(site, "/timer", |seen| ticks(seen) >= before + 4);
    finish_run(connection, run_id)?;
    let Some(reported) = reported else {
        checks.check("the Proving ground's observations can be read", false);
        return Ok(());
    };
    let (rate, spanned) = rate_from(&reported, before);
    checks.check("the page behind reports its timer for 3 s", spanned >= 4);
    checks.check(
        format!("its timer fires at least {UNTHROTTLED_RATE} times a second"),
        rate >= UNTHROTTLED_RATE,
    );
    let focus: Vec<&str> = reported
        .iter()
        .filter(|(check, _)| check == "has_focus")
        .skip(before)
        .map(|(_, observed)| observed.as_str())
        .collect();
    checks.check(
        "it reports that it has focus",
        !focus.is_empty() && focus.iter().all(|observed| *observed == "true"),
    );
    Ok(())
}
