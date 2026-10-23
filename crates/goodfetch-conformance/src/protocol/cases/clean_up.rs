//! The page suite's Clean-up cases (ADRs 0047 and 0117, ticket #713) on
//! the Proving ground's `clean-up.test` pages: the Filter lists block ads
//! and trackers by default, each failing as `BlockedByClient`; the
//! never-block list keeps a challenge's anti-bot sensors loading over the
//! lists and over `browser.block_resources`; a redirect rule's stand-in
//! keeps a page that calls a blocked tracker working; and
//! `browser.block_resources` blocks its kinds before every navigation and
//! warns once. Each is judged by the Proving ground's own record of the
//! requests it received, by the Errors the engine answers, by the Run
//! report and by the Run trace the engine writes.

use std::path::{Path, PathBuf};

use goodfetch_protocol::page::route::{PAGE_ROUTE, ROUTE_CONTINUE};
use goodfetch_protocol::page::{PAGE_GOTO, PageOpened};
use goodfetch_protocol::run::{RUN_FINISH, RUN_START};
use goodfetch_protocol::settings::SETTING_INVALID;
use serde_json::{Value, json};

use super::navigation::{answered, call, failed_with};
use super::routes::{Act, Routes, glob, goto};
use super::{Checks, Site, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// The host the pages are served on.
pub(super) const HOST: &str = "clean-up.test";

/// The browser's network error for a request a blocker failed.
const BLOCKED_BY_CLIENT: &str = "net::ERR_BLOCKED_BY_CLIENT";

/// The `/ads` page's own subresources, which load.
const OWN: [&str; 3] = ["/css/site.css", "/js/site.js", "/img/photo.gif"];
/// The `/ads` page's ad (EasyList), tracker (EasyPrivacy) and annoyance
/// (Fanboy Annoyance); the tracker and the annoyance are the site's own
/// scripts.
const AD: &str = "/img/top-ad-sidebar.gif";
const TRACKER: &str = "/js/adobe-analytics.js";
const ANNOYANCE: &str = "/js/copy_text.js";
/// A document EasyList blocks wherever it is (`/page/bouncy.php?$document`).
const BLOCKED_DOCUMENT: &str = "/page/bouncy.php?id=1";
/// The `/challenge` page's anti-bot sensors and challenge files, which the
/// never-block list keeps loading.
const CHALLENGE: [&str; 4] = [
    "/akam/13/sensor.js",
    "/perimeterx/px.js",
    "/cdn-cgi/challenge-platform/h/b/orchestrate/chl_page/v1",
    "/cdn-cgi/challenge-platform/h/b/img/logo.gif",
];

/// One Clean-up case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Blocked,
    NeverBlocked,
    StandIn,
    BlockResources,
    RouteSendsOn,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "ads-and-trackers-fail-as-blocked-by-client-and-the-sites-own-content-loads" => {
                Self::Blocked
            }
            "challenge-and-anti-bot-scripts-load-over-the-filter-lists-and-block-resources" => {
                Self::NeverBlocked
            }
            "a-redirect-stand-in-keeps-a-page-that-calls-a-blocked-tracker-working" => {
                Self::StandIn
            }
            "block-resources-blocks-its-kinds-before-every-navigation-and-warns-once" => {
                Self::BlockResources
            }
            "a-request-a-route-sends-on-passes-the-filter-lists" => Self::RouteSendsOn,
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
            Self::Blocked => blocked(connection, target, site, checks),
            Self::NeverBlocked => never_blocked(connection, site, checks),
            Self::StandIn => stand_in(connection, site, checks),
            Self::BlockResources => block_resources(connection, site, checks),
            Self::RouteSendsOn => route_sends_on(connection, site, checks),
        }
    }
}

/// How many requests for `target` on the pages' host the Proving ground
/// received.
fn received(site: &Site, target: &str) -> usize {
    site.proving_ground
        .observations()
        .ok()
        .and_then(|observations| observations["requests"].as_array().cloned())
        .unwrap_or_default()
        .iter()
        .filter(|request| request["host"] == json!(HOST) && request["target"] == json!(target))
        .count()
}

/// How many requests for each of `targets` the Proving ground received.
fn counts<const N: usize>(site: &Site, targets: [&str; N]) -> [usize; N] {
    targets.map(|target| received(site, target))
}

/// Starts the Run `run_id` with `settings` and opens its page.
fn open(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    settings: &Value,
    checks: &mut Checks,
) -> Result<Option<PageOpened>, Gone> {
    start_run(connection, site, run_id, settings, checks)?;
    new_page(connection, run_id, checks)
}

/// `page.goto` on `page` to `path` on the pages' host, and its answer.
fn visit(
    connection: &mut Connection,
    site: &Site,
    page: &PageOpened,
    path: &str,
) -> Result<(String, Value), Gone> {
    let url = site.named_url(HOST, path).unwrap_or_default();
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &json!(page.page_id),
        &json!({ "url": url }),
    )?;
    Ok((url, answer))
}

/// Lets the Run `run_id` finish and answers its Run report.
fn finish(connection: &mut Connection, run_id: &str) -> Result<Value, Gone> {
    let id = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    Ok(connection.browser_answer(id)?.message)
}

/// The lines of the Run `run_id`'s request log in the Run trace, under the
/// Goodfetch home `target` names: `<home>/<major>/runs/<run_id>/trace/`.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn run_trace(target: &EngineTarget, run_id: &str) -> Vec<Value> {
    let Some(home) = target.environment.get("GOODFETCH_HOME") else {
        return Vec::new();
    };
    std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .map(|major| {
            major
                .path()
                .join("runs")
                .join(run_id)
                .join("trace/requests.jsonl")
        })
        .filter(|file| file.is_file())
        .flat_map(|file: PathBuf| lines(&file))
        .collect()
}

/// The JSON lines of `file`.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
fn lines(file: &Path) -> Vec<Value> {
    std::fs::read_to_string(file)
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect()
}

/// By default the `/ads` page's own style sheet, script and photo load and
/// its ad, tracker and annoyance do not, each recorded in the Run trace as
/// failed with `net::ERR_BLOCKED_BY_CLIENT`; a document a Filter list
/// blocks fails its navigation with it; with `filter_lists.ads`,
/// `.trackers` and `.annoyances` off, all three load; with
/// `filter_lists.first_party_scripts` on, the site's own tracker and
/// annoyance scripts load and its ad image stays blocked.
fn blocked(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-blocked";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let (own, blocked) = (counts(site, OWN), counts(site, [AD, TRACKER, ANNOYANCE]));
    let (url, answer) = visit(connection, site, &page, "/ads")?;
    answered(&answer, "the `goto` to the ads page", &url, 200, checks);
    checks.check(
        "the page's own style sheet, script and photo reached the Proving ground",
        counts(site, OWN)
            .iter()
            .zip(own)
            .all(|(after, before)| *after == before + 1),
    );
    checks.check(
        "neither its ad, its tracker nor its annoyance reached the Proving ground",
        counts(site, [AD, TRACKER, ANNOYANCE]) == blocked,
    );
    let (_, answer) = visit(connection, site, &page, BLOCKED_DOCUMENT)?;
    failed_with(
        &answer,
        "a `goto` to a document a Filter list blocks",
        BLOCKED_BY_CLIENT,
        checks,
    );
    checks.check(
        "the blocked document never reached the Proving ground",
        received(site, BLOCKED_DOCUMENT) == 0,
    );
    finish(connection, run_id)?;
    let trace = run_trace(target, run_id);
    for path in [AD, TRACKER, ANNOYANCE] {
        checks.check(
            format!("the Run trace records {path} failed as `{BLOCKED_BY_CLIENT}`"),
            trace.iter().any(|line| {
                line["event"] == json!("page.request_failed")
                    && line["data"]["url"]
                        .as_str()
                        .is_some_and(|url| url.ends_with(path))
                    && line["data"]["failure"]["error_text"] == json!(BLOCKED_BY_CLIENT)
                    && line["data"]["failure"]["error"]["code"] == json!("navigation_failed")
            }),
        );
    }

    let run_id = "conformance-clean-up-lists-off";
    let off = json!({ "filter_lists": { "ads": false, "trackers": false, "annoyances": false } });
    let Some(page) = open(connection, site, run_id, &off, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = counts(site, [AD, TRACKER, ANNOYANCE]);
    let (url, answer) = visit(connection, site, &page, "/ads")?;
    answered(&answer, "the `goto` with the lists off", &url, 200, checks);
    checks.check(
        "with `filter_lists.ads`, `.trackers` and `.annoyances` off, all three load",
        counts(site, [AD, TRACKER, ANNOYANCE]) == before.map(|count| count + 1),
    );
    finish(connection, run_id)?;

    let run_id = "conformance-clean-up-first-party-scripts";
    let first_party = json!({ "filter_lists": { "first_party_scripts": true } });
    let Some(page) = open(connection, site, run_id, &first_party, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = counts(site, [AD, TRACKER, ANNOYANCE]);
    let (url, answer) = visit(connection, site, &page, "/ads")?;
    answered(
        &answer,
        "the `goto` with the site's own scripts allowed",
        &url,
        200,
        checks,
    );
    checks.check(
        "with `filter_lists.first_party_scripts` on, the site's own tracker and annoyance load and its ad does not",
        counts(site, [AD, TRACKER, ANNOYANCE]) == [before[0], before[1] + 1, before[2] + 1],
    );
    finish(connection, run_id).map(drop)
}

/// The `/challenge` page's anti-bot sensors, which EasyPrivacy blocks, and
/// its challenge script and image load under `browser.block_resources:
/// ["image"]`, while its tracker stays blocked.
fn never_blocked(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-never-blocked";
    let settings = json!({ "browser": { "block_resources": ["image"] } });
    let Some(page) = open(connection, site, run_id, &settings, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let (before, tracker) = (counts(site, CHALLENGE), received(site, TRACKER));
    let (url, answer) = visit(connection, site, &page, "/challenge")?;
    answered(
        &answer,
        "the `goto` to the challenge page",
        &url,
        200,
        checks,
    );
    let after = counts(site, CHALLENGE);
    for ((path, after), before) in CHALLENGE.iter().zip(after).zip(before) {
        checks.check(
            format!(
                "{path} reached the Proving ground over the lists and `browser.block_resources`"
            ),
            after == before + 1,
        );
    }
    checks.check(
        "the challenge page's tracker stays blocked",
        received(site, TRACKER) == tracker,
    );
    finish(connection, run_id).map(drop)
}

/// The `/stand-in` page calls Chartbeat's `pSUPERFLY`, which EasyPrivacy's
/// redirect rule answers with its stand-in, so its script goes on: it
/// shows `/stand-in/worked.gif`, never `/stand-in/broken.gif`.
fn stand_in(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-stand-in";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = counts(site, ["/stand-in/worked.gif", "/stand-in/broken.gif"]);
    let (url, answer) = visit(connection, site, &page, "/stand-in")?;
    answered(
        &answer,
        "the `goto` to the stand-in page",
        &url,
        200,
        checks,
    );
    checks.check(
        "the page's call into the blocked tracker worked through its stand-in",
        counts(site, ["/stand-in/worked.gif", "/stand-in/broken.gif"])
            == [before[0] + 1, before[1]],
    );
    finish(connection, run_id).map(drop)
}

/// `browser.block_resources: ["image"]` blocks the `/ads` page's images on
/// its first navigation and every later one, its style sheet and script
/// loading; the Run warns once that the site can see it; a kind outside the
/// closed list is `setting_invalid`.
fn block_resources(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-block-resources";
    let settings = json!({ "browser": { "block_resources": ["image"] } });
    let Some(page) = open(connection, site, run_id, &settings, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let images = counts(site, ["/img/photo.gif", AD]);
    let script = received(site, "/js/site.js");
    for (attempt, path) in [(1, "/ads"), (2, "/ads?again")] {
        let (url, answer) = visit(connection, site, &page, path)?;
        answered(
            &answer,
            &format!("navigation {attempt} under `browser.block_resources`"),
            &url,
            200,
            checks,
        );
    }
    checks.check(
        "no image of either navigation reached the Proving ground",
        counts(site, ["/img/photo.gif", AD]) == images,
    );
    checks.check(
        "the page's script still loaded",
        received(site, "/js/site.js") > script,
    );
    let report = finish(connection, run_id)?;
    let warned = report["result"]["warnings"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|warning| warning["code"] == json!("block_resources_visible"))
        .count();
    checks.check(
        "the Run report warns once that the site can see the missing loads",
        warned == 1,
    );

    let refused = connection.request(
        RUN_START.name,
        &json!({
            "run_id": "conformance-clean-up-block-video",
            "settings": { "browser": { "block_resources": ["video"] } },
        }),
    )?;
    let answer = connection.answer(refused)?.message;
    checks.check(
        format!(
            "a kind outside the closed list is `{}`",
            SETTING_INVALID.name
        ),
        answer["error"]["data"]["code"] == json!(SETTING_INVALID.name),
    );
    Ok(())
}

/// A route that sends the `/ads` page's own script on as its tracker's URL
/// does not get past the Filter lists: neither reaches the Proving ground.
fn route_sends_on(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-route";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let (Some(ads), Some(tracker)) = (
        site.named_url(HOST, "/ads?route=tracker"),
        site.named_url(HOST, TRACKER),
    ) else {
        return finish(connection, run_id).map(drop);
    };
    let before = counts(site, ["/js/site.js", TRACKER]);
    let mut routes = Routes::new(move |_| {
        vec![
            Act::Call(ROUTE_CONTINUE.name, json!({ "url": tracker })),
            Act::End,
        ]
    });
    Routes::route(
        connection,
        PAGE_ROUTE.name,
        &json!({ "page_id": page.page_id, "url": glob("**/js/site.js") }),
    )?;
    let answer = goto(&mut routes, connection, &page, &ads)?;
    answered(&answer, "the `goto` with the route", &ads, 200, checks);
    checks.check("the route saw the page's script", routes.handled.len() == 1);
    checks.check(
        "what the route sent on was blocked: neither URL reached the Proving ground",
        counts(site, ["/js/site.js", TRACKER]) == before,
    );
    finish(connection, run_id).map(drop)
}
