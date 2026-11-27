//! The page suite's Fingerprint method case (spec #599, ticket #913), on
//! the Proving ground's consistency probe (ADR 0074): a page on
//! `consistency.test` with a cross-origin, out-of-process frame on
//! `consistency-frame.test` and a dedicated, a shared and a service worker,
//! each of which reports what its script reads. A `standard` Session
//! shows no automation in any of them, and a `basic` Session's page, its
//! frame and its workers read one identity (ADR 0035's rows).
//!
//! A shared or service worker's script still reads the browser's own
//! `navigator.userAgent`, and a worker its machine's platform, which no
//! override reaches yet (#4001): the case reads the user agent of the
//! three contexts the page's overrides reach, and every context's
//! requests.

use std::time::Duration;

use serde_json::{Value, json};

use super::launch::{open_at, reports};
use super::{Checks, Site, finish_run, poll, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::proving_ground::ReceivedRequests;

/// A stealth case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    EveryContextShowsTheSessionsIdentity,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "every-frame-and-worker-of-a-page-shows-its-sessions-identity" => {
                Self::EveryContextShowsTheSessionsIdentity
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::EveryContextShowsTheSessionsIdentity => every_context(connection, site, checks),
        }
    }
}

/// The host of the probe page and its workers.
pub(super) const HOST: &str = "consistency.test";

/// The host of the probe page's frame, another site, so its frame runs
/// out of process.
pub(super) const FRAME_HOST: &str = "consistency-frame.test";

/// Each context of the probe page, as its reports name it: the main frame,
/// its frame, and its dedicated, shared and service workers.
const CONTEXTS: [&str; 5] = [
    "/",
    "/frame",
    "/dedicated-worker.js",
    "/shared-worker.js",
    "/service-worker.js",
];

/// The contexts whose script reads the user agent the page's overrides
/// set: the main frame, its frame and its dedicated worker.
const PAGE_CONTEXTS: [&str; 3] = ["/", "/frame", "/dedicated-worker.js"];

/// The surfaces the case reads of each context's reports.
const SURFACES: [&str; 4] = [
    "user_agent",
    "webdriver",
    "time_zone",
    "hardware_concurrency",
];

/// How long, and how often, the case reads the observations while it waits
/// for every context to report: a service worker installs before it reads.
const REPORT_WAIT: Duration = Duration::from_secs(30);
const POLL: Duration = Duration::from_millis(100);

/// What one Run's probe page read and sent.
struct Probed {
    /// Each context's reports, in [`CONTEXTS`]' order, as
    /// `(surface, value as JSON text)`.
    reports: Vec<Vec<(String, String)>>,
    /// The `user-agent` header of each request the Run sent to the probe's
    /// two hosts.
    user_agents: Vec<String>,
}

impl Probed {
    /// The value context `context` read of `surface`, as JSON text.
    fn read(&self, context: &str, surface: &str) -> Option<&str> {
        let index = CONTEXTS.iter().position(|known| *known == context)?;
        self.reports[index]
            .iter()
            .find(|(reported, _)| reported == surface)
            .map(|(_, observed)| observed.as_str())
    }

    /// Whether every context reported every surface the case reads.
    fn complete(&self) -> bool {
        CONTEXTS.iter().all(|context| {
            SURFACES
                .iter()
                .all(|surface| self.read(context, surface).is_some())
        })
    }

    /// The user agent each of [`PAGE_CONTEXTS`]' scripts read.
    fn page_user_agents(&self) -> Vec<Option<String>> {
        PAGE_CONTEXTS
            .iter()
            .map(|context| {
                self.read(context, "user_agent")
                    .and_then(|text| serde_json::from_str::<String>(text).ok())
            })
            .collect()
    }

    /// Whether every context read what the main frame read of `surface`.
    fn agree_on(&self, surface: &str) -> bool {
        let main = self.read(CONTEXTS[0], surface);
        main.is_some()
            && CONTEXTS
                .iter()
                .all(|context| self.read(context, surface) == main)
    }
}

/// A `standard` Session shows no automation in any context of a page,
/// frames and workers included, and a `basic` Session's page, its
/// out-of-process frame and its dedicated worker read the user agent every
/// request of the Session sends, while each of its five contexts reads the
/// page's time zone and core count (ADR 0035's rows; ticket #913).
fn every_context(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    for (label, run_id, stealth) in [
        ("`standard`", "conformance-stealth-standard", None),
        ("`basic`", "conformance-stealth-basic", Some("basic")),
    ] {
        let Some(probed) = probe(connection, site, run_id, stealth, checks)? else {
            continue;
        };
        checks.check(
            format!("every context of a {label} Session's page reports what it reads"),
            probed.complete(),
        );
        checks.check(
            format!("no context of a {label} Session reads `navigator.webdriver` true"),
            CONTEXTS
                .iter()
                .all(|context| probed.read(context, "webdriver") != Some("true")),
        );
        let agents = probed.page_user_agents();
        checks.check(
            format!(
                "no request of a {label} Session and no user agent its page, frame or dedicated worker reads names `HeadlessChrome`"
            ),
            agents.iter().all(Option::is_some)
                && agents
                    .iter()
                    .flatten()
                    .chain(&probed.user_agents)
                    .all(|agent| !agent.contains("HeadlessChrome")),
        );
        let main = agents.first().cloned().flatten();
        checks.check(
            format!(
                "a {label} Session's page, frame and dedicated worker read the user agent each of its requests sends"
            ),
            main.is_some()
                && !probed.user_agents.is_empty()
                && agents.iter().all(|agent| *agent == main)
                && probed
                    .user_agents
                    .iter()
                    .all(|agent| Some(agent) == main.as_ref()),
        );
        if stealth.is_some() {
            checks.check(
                format!(
                    "every context of a {label} Session's page reads its time zone and core count"
                ),
                probed.agree_on("time_zone") && probed.agree_on("hardware_concurrency"),
            );
        }
    }
    Ok(())
}

/// Starts the Run `run_id`, `stealth` with that Fingerprint method or
/// `standard` without one, opens the probe page, waits for each context's
/// reports and finishes the Run: what the probe read and sent, none when
/// the page did not load or the observations could not be read. The Run
/// takes the Proving ground's leaf for the probe's two hosts on first use
/// (ADR 0042): a service worker and the client hints need a secure context.
fn probe(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    stealth: Option<&str>,
    checks: &mut Checks,
) -> Result<Option<Probed>, Gone> {
    let mut settings = json!({
        "tls": { "accept_invalid_certs": { HOST: null, FRAME_HOST: null } },
    });
    if let Some(method) = stealth {
        settings["browser"] = json!({ "access_mode": "stealth", "headless": true });
        settings["stealth"] = json!({ "fingerprint_method": method, "human_input": false });
    }
    let Some(before) = site.observations() else {
        return Ok(None);
    };
    let (reported, sent) = (counts(&before), last_request(&before));
    start_run(connection, site, run_id, &settings, checks)?;
    let url = site.named_secure_url(HOST, "/");
    let loaded = open_at(connection, run_id, &url, checks)?;
    checks.check(
        format!("the probe page loads in the Run `{run_id}`"),
        loaded,
    );
    let observed = poll::try_until(
        REPORT_WAIT,
        POLL,
        || {
            site.proving_ground
                .observations()
                .map(|observations| since(&observations, &reported, sent))
        },
        Probed::complete,
    );
    finish_run(connection, run_id)?;
    let Some(probed) = checks.needs("the Proving ground's observations can be read", observed)
    else {
        return Ok(None);
    };
    Ok(loaded.then_some(probed))
}

/// How many reports each context had posted in `observations`.
fn counts(observations: &Value) -> Vec<usize> {
    CONTEXTS
        .iter()
        .map(|context| reports(observations, context).len())
        .collect()
}

/// The sequence number of the last request in `observations`, if any.
fn last_request(observations: &Value) -> Option<u64> {
    ReceivedRequests::read(observations)
        .ok()?
        .every()
        .map(|request| request.sequence)
        .max()
}

/// What the probe read and sent in `observations` past the `reported`
/// reports of each context and the request numbered `sent`.
fn since(observations: &Value, reported: &[usize], sent: Option<u64>) -> Probed {
    let reports = CONTEXTS
        .iter()
        .zip(reported)
        .map(|(context, before)| {
            reports(observations, context)
                .into_iter()
                .skip(*before)
                .collect()
        })
        .collect();
    let user_agents = ReceivedRequests::read(observations)
        .map(|requests| {
            requests
                .every()
                .filter(|request| sent.is_none_or(|sent| request.sequence > sent))
                .filter(|request| request.host == HOST || request.host == FRAME_HOST)
                .flat_map(|request| &request.headers)
                .filter(|(name, _)| name.eq_ignore_ascii_case("user-agent"))
                .map(|(_, value)| value.clone())
                .collect()
        })
        .unwrap_or_default();
    Probed {
        reports,
        user_agents,
    }
}
