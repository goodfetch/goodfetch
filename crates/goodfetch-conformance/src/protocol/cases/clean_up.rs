//! The page suite's Clean-up cases (ADRs 0047 and 0117, ticket #713) on
//! the Proving ground's `clean-up.test` pages: the Filter lists block ads
//! and trackers by default, each failing as `BlockedByClient`; the
//! never-block list keeps a challenge's anti-bot sensors and a payment
//! script loading over the lists and over `browser.block_resources`; a redirect rule's stand-in
//! keeps a page that calls a blocked tracker working; and
//! `browser.block_resources` blocks its kinds before every navigation and
//! warns once. Live hiding (ticket #715) hides an ad in a page and in its
//! frame through a sheet the page's own script never sees in
//! `document.styleSheets`, lifts the rule for the Session when a click's
//! target sits in a hidden ad, and follows `live_view.hiding`; and
//! extraction-side removal leaves hidden text out of the Markdown, takes
//! ads inside open and closed shadow roots, and flags a removal over a
//! fifth of the main content. Each is judged by the Proving ground's own
//! record of the requests it received, its pages' own reports among them,
//! by the Errors the engine answers, by the Run report and by the Run
//! trace the engine writes, its Provenance log included. The retries with
//! blocking off (ticket #716) are `blocking_off`'s.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use goodfetch_protocol::page::route::{PAGE_ROUTE, ROUTE_CONTINUE};
use goodfetch_protocol::page::{LOCATOR_CLICK, PAGE_FORMATS, PAGE_GOTO, PageOpened};
use goodfetch_protocol::run::{RUN_FINISH, RUN_START};
use goodfetch_protocol::settings::SETTING_INVALID;
use serde_json::{Value, json};

use super::blocking_off;
use super::navigation::{answered, call, failed_with};
use super::routes::{Act, Routes, glob, goto};
use super::{Checks, Site, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// The host the pages are served on.
pub(super) const HOST: &str = "clean-up.test";

/// The host of `/cross-site`'s frame, another site, which the browser loads
/// in a process of its own.
pub(super) const FRAME_HOST: &str = "clean-up-frame.test";

/// The browser's network error for a request a blocker failed.
const BLOCKED_BY_CLIENT: &str = "net::ERR_BLOCKED_BY_CLIENT";

/// The `/ads` page's own subresources, which load.
const OWN: [&str; 3] = ["/css/site.css", "/js/site.js", "/img/photo.gif"];
/// The `/ads` page's ad (EasyList), tracker (EasyPrivacy) and annoyance
/// (Fanboy Annoyance); the tracker and the annoyance are the site's own
/// scripts.
pub(super) const AD: &str = "/img/top-ad-sidebar.gif";
pub(super) const TRACKER: &str = "/js/adobe-analytics.js";
pub(super) const ANNOYANCE: &str = "/js/copy_text.js";
/// A document EasyList blocks wherever it is (`/page/bouncy.php?$document`).
const BLOCKED_DOCUMENT: &str = "/page/bouncy.php?id=1";
/// The host of the `/challenge` page's payment script, Stripe's own.
pub(super) const PAYMENT_HOST: &str = "m.stripe.com";
/// Stripe's fraud-signals script, which EasyPrivacy blocks on another site
/// and the never-block list keeps loading.
const PAYMENT: &str = "/6";
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
    CrossSiteFrame,
    HiddenTextRemoved,
    RemovalShareMeasures,
    HidingSheetUnseen,
    HiddenTargetLifted,
    LiveHidingModes,
    /// A retry with blocking off (ticket #716).
    BlockingOff(blocking_off::Case),
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "ads-and-trackers-fail-as-blocked-by-client-and-the-sites-own-content-loads" => {
                Self::Blocked
            }
            "challenge-anti-bot-and-payment-scripts-load-over-the-filter-lists-and-block-resources" => {
                Self::NeverBlocked
            }
            "a-redirect-stand-in-keeps-a-page-that-calls-a-blocked-tracker-working" => {
                Self::StandIn
            }
            "block-resources-blocks-its-kinds-before-every-navigation-and-warns-once" => {
                Self::BlockResources
            }
            "a-request-a-route-sends-on-passes-the-filter-lists" => Self::RouteSendsOn,
            "requests-of-another-sites-frame-pass-the-filter-lists-and-block-resources" => {
                Self::CrossSiteFrame
            }
            "hidden-text-is-absent-from-extracted-output-in-shadow-roots-too-and-a-large-removal-is-flagged" => {
                Self::HiddenTextRemoved
            }
            "the-share-a-removal-took-is-an-estimate-by-default-and-exact-when-set-and-provenance-names-each" => {
                Self::RemovalShareMeasures
            }
            "the-hiding-sheet-hides-in-every-frame-and-is-absent-from-document-style-sheets" => {
                Self::HidingSheetUnseen
            }
            "a-click-on-a-target-a-hidden-element-covers-lifts-the-rule-for-the-session-and-lands" => {
                Self::HiddenTargetLifted
            }
            "live-hiding-off-leaves-the-page-as-served-and-always-hides" => Self::LiveHidingModes,
            _ => return blocking_off::Case::from_id(id).map(Self::BlockingOff),
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
            Self::CrossSiteFrame => cross_site_frame(connection, site, checks),
            Self::HiddenTextRemoved => hidden_text_removed(connection, target, site, checks),
            Self::RemovalShareMeasures => removal_share_measures(connection, target, site, checks),
            Self::HidingSheetUnseen => hiding_sheet_unseen(connection, target, site, checks),
            Self::HiddenTargetLifted => hidden_target_lifted(connection, target, site, checks),
            Self::LiveHidingModes => live_hiding_modes(connection, target, site, checks),
            Self::BlockingOff(case) => case.run(connection, target, site, checks),
        }
    }
}

/// How many requests for `target` on the pages' host the Proving ground
/// received.
pub(super) fn received(site: &Site, target: &str) -> usize {
    received_on(site, HOST, target)
}

/// How many requests for `target` of `host` the Proving ground received.
fn received_on(site: &Site, host: &str, target: &str) -> usize {
    site.proving_ground
        .received_requests()
        .unwrap_or_default()
        .of_the_case()
        .filter(|request| request["host"] == json!(host) && request["target"] == json!(target))
        .count()
}

/// How many requests for each of `targets` the Proving ground received.
pub(super) fn counts<const N: usize>(site: &Site, targets: [&str; N]) -> [usize; N] {
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
pub(super) fn visit(
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
pub(super) fn finish(connection: &mut Connection, run_id: &str) -> Result<Value, Gone> {
    let id = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    Ok(connection.browser_answer(id)?.message)
}

/// The lines of the Run `run_id`'s request log in the Run trace, under the
/// Goodfetch home `target` names: `<home>/<major>/runs/<run_id>/trace/`.
pub(super) fn run_trace(target: &EngineTarget, run_id: &str) -> Vec<Value> {
    trace_lines(target, run_id, "trace/requests.jsonl")
}

/// The lines of the Run `run_id`'s Provenance log in the Run trace.
pub(super) fn provenance(target: &EngineTarget, run_id: &str) -> Vec<Value> {
    trace_lines(target, run_id, "trace/provenance.jsonl")
}

/// The lines of the Run trace's file `file` of the Run `run_id`.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
pub(super) fn trace_lines(target: &EngineTarget, run_id: &str, file: &str) -> Vec<Value> {
    let Some(home) = target.environment.get("GOODFETCH_HOME") else {
        return Vec::new();
    };
    std::fs::read_dir(home)
        .into_iter()
        .flatten()
        .flatten()
        .map(|major| major.path().join("runs").join(run_id).join(file))
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

/// The `/challenge` page's anti-bot sensors and Stripe's fraud-signals
/// script, which EasyPrivacy blocks, and its challenge script and image
/// load under `browser.block_resources: ["image"]`, while its tracker stays
/// blocked.
fn never_blocked(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-never-blocked";
    // Stripe's host is reached over HTTPS alone, and the Proving ground's
    // leaf for it chains to an authority no browser trusts: the Run takes
    // it on first use, which a run may list (ADR 0042).
    let settings = json!({
        "browser": { "block_resources": ["image"] },
        "tls": { "accept_invalid_certs": { PAYMENT_HOST: null } },
    });
    let Some(page) = open(connection, site, run_id, &settings, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let (before, tracker) = (counts(site, CHALLENGE), received(site, TRACKER));
    let payment = received_on(site, PAYMENT_HOST, PAYMENT);
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
        format!(
            "the payment script {PAYMENT} of {PAYMENT_HOST} reached the Proving ground over the lists and `browser.block_resources`"
        ),
        received_on(site, PAYMENT_HOST, PAYMENT) == payment + 1,
    );
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

/// How many requests for `target` on `/cross-site`'s frame's host the
/// Proving ground received.
fn received_in_frame(site: &Site, target: &str) -> usize {
    site.proving_ground
        .received_requests()
        .unwrap_or_default()
        .of_the_case()
        .filter(|request| {
            request["host"] == json!(FRAME_HOST) && request["target"] == json!(target)
        })
        .count()
}

/// The requests of another site's frame, which the browser loads in a
/// process of its own, pass the interception as the page's do: its own
/// script and photo load and its ad and tracker do not; under
/// `browser.block_resources: ["image"]` its photo does not load either.
fn cross_site_frame(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let frame = |site: &Site| {
        ["/framed", "/js/site.js", "/img/photo.gif", AD, TRACKER]
            .map(|target| received_in_frame(site, target))
    };
    let run_id = "conformance-clean-up-cross-site-frame";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = frame(site);
    let (url, answer) = visit(connection, site, &page, "/cross-site")?;
    answered(&answer, "the `goto` to the framing page", &url, 200, checks);
    let after = frame(site);
    checks.check(
        "the frame's document, own script and photo reached the Proving ground",
        after[..3]
            .iter()
            .zip(&before[..3])
            .all(|(after, before)| *after == before + 1),
    );
    checks.check(
        "neither the frame's ad nor its tracker reached the Proving ground",
        after[3..] == before[3..],
    );
    finish(connection, run_id)?;

    let run_id = "conformance-clean-up-cross-site-frame-images";
    let settings = json!({ "browser": { "block_resources": ["image"] } });
    let Some(page) = open(connection, site, run_id, &settings, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = frame(site);
    let (url, answer) = visit(connection, site, &page, "/cross-site")?;
    answered(
        &answer,
        "the `goto` to the framing page under `browser.block_resources`",
        &url,
        200,
        checks,
    );
    let after = frame(site);
    checks.check(
        "under `browser.block_resources: [\"image\"]` the frame's script loaded and its photo did not",
        after[1] == before[1] + 1 && after[2] == before[2],
    );
    finish(connection, run_id).map(drop)
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

/// How long a case waits for a page's own report to reach the Proving
/// ground: the page waits up to 2 s for its ad to be hidden.
const REPORT_WAIT: Duration = Duration::from_secs(10);

/// The `/hiding` page's report, and its frame's.
const SEEN: &str = "/hiding/seen?";
const FRAME_SEEN: &str = "/hiding/frame-seen?";

/// What a live-hiding page reports when its ad is hidden and the site sees
/// only its own style sheets: one for `/hiding`, none for its frame.
const HIDDEN_UNSEEN: &str = "/hiding/seen?sheets=1&ad=none";
const FRAME_HIDDEN_UNSEEN: &str = "/hiding/frame-seen?sheets=0&ad=none";

/// What `/hiding` reports when its ad shows.
const SHOWN: &str = "/hiding/seen?sheets=1&ad=block";

/// Text of the `/hiding` article's own, and of its ad.
pub(super) const ARTICLE_TEXT: &str = "The article's text begins here";
pub(super) const AD_TEXT: &str = "An ad's text";

/// The targets of the requests the Proving ground received on the pages'
/// host that start with `prefix`, in the order received.
fn targets(site: &Site, prefix: &str) -> Vec<String> {
    site.proving_ground
        .received_requests()
        .unwrap_or_default()
        .of_the_case()
        .filter(|request| request["host"] == json!(HOST))
        .filter_map(|request| request["target"].as_str().map(str::to_owned))
        .filter(|target| target.starts_with(prefix))
        .collect()
}

/// The report starting with `prefix` the Proving ground receives after the
/// `before` it had, waited for up to [`REPORT_WAIT`].
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness polls the Proving ground on its own thread, outside any async worker"
)]
fn next_report(site: &Site, prefix: &str, before: usize) -> Option<String> {
    let started = Instant::now();
    loop {
        if let Some(report) = targets(site, prefix).get(before) {
            return Some(report.clone());
        }
        if started.elapsed() > REPORT_WAIT {
            return None;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

/// The Markdown `page.formats` answers for `page`.
fn markdown(connection: &mut Connection, page: &PageOpened) -> Result<String, Gone> {
    let answer = call(
        connection,
        PAGE_FORMATS.name,
        &json!(page.page_id),
        &json!({ "formats": { "markdown": true } }),
    )?;
    Ok(answer["result"]["markdown"]
        .as_str()
        .unwrap_or_default()
        .to_owned())
}

/// The Provenance lines of `action` whose page's URL ends with `path`.
fn provenance_of<'a>(lines: &'a [Value], action: &str, path: &str) -> Vec<&'a Value> {
    lines
        .iter()
        .filter(|line| line["action"] == json!(action))
        .filter(|line| line["url"].as_str().is_some_and(|url| url.ends_with(path)))
        .collect()
}

/// The `formats` Provenance lines of the page whose URL ends with `path`
/// whose call's rules removed something.
fn removals_of<'a>(lines: &'a [Value], path: &str) -> Vec<&'a Value> {
    provenance_of(lines, "formats", path)
        .into_iter()
        .filter(|line| {
            line["removed"]
                .as_array()
                .is_some_and(|removed| !removed.is_empty())
        })
        .collect()
}

/// The Markdown of `/hiding` keeps the article and leaves its ad's text
/// out; extraction-side removal takes the ads of `/shadow`'s light DOM and
/// of its open and closed shadow roots, and Provenance records each; the
/// removal of `/hiding`'s ad, a small share of its main content, is not
/// flagged, and `/removal`'s, most of it, is.
fn hidden_text_removed(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-removal";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let mut read = Vec::new();
    for path in ["/hiding", "/shadow", "/removal"] {
        let (url, answer) = visit(connection, site, &page, path)?;
        answered(&answer, &format!("the `goto` to {path}"), &url, 200, checks);
        read.push(markdown(connection, &page)?);
    }
    checks.check(
        "the Markdown of `/hiding` keeps the article's text and leaves its hidden ad's out",
        read[0].contains(ARTICLE_TEXT) && !read[0].contains(AD_TEXT),
    );
    checks.check(
        "the Markdown of `/shadow` keeps the article's text and leaves its light ad out",
        read[1].contains("The article's own text") && !read[1].contains("A light ad"),
    );
    finish(connection, run_id)?;
    let lines = provenance(target, run_id);
    let hiding = removals_of(&lines, "/hiding");
    checks.check(
        "Provenance records `/hiding`'s ad removed by `.ad-slot`, unflagged",
        hiding.first().is_some_and(|line| {
            line["removed"][0]["rule"] == json!(".ad-slot") && line["flagged"] == json!(false)
        }),
    );
    let shadow = removals_of(&lines, "/shadow");
    let in_shadow_roots: Vec<Value> = shadow
        .first()
        .and_then(|line| line["removed"].as_array())
        .into_iter()
        .flatten()
        .map(|removed| removed["in_shadow_root"].clone())
        .collect();
    checks.check(
        "Provenance records `/shadow`'s three ads removed: its light DOM's, its open root's and its closed root's",
        in_shadow_roots == [json!(false), json!(true), json!(true)],
    );
    let flagged = removals_of(&lines, "/removal");
    checks.check(
        "Provenance flags `/removal`'s removal, over a fifth of its main content",
        flagged.first().is_some_and(|line| {
            line["flagged"] == json!(true)
                && line["share"].as_f64().is_some_and(|share| share > 0.2)
        }),
    );
    Ok(())
}

/// Under each value of `extraction.removal_share`, Provenance records the
/// share of `/hiding`'s and `/removal`'s main content their removal took
/// and how it was measured: by default the estimate, the text removed over
/// that text plus the main content left, which is never below the exact
/// share `exact` measures; both leave `/hiding`'s removal unflagged and flag
/// `/removal`'s.
fn removal_share_measures(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let mut shares = Vec::new();
    for (measure, settings) in [
        ("estimate", json!({})),
        (
            "exact",
            json!({ "extraction": { "removal_share": "exact" } }),
        ),
    ] {
        let run_id = format!("conformance-clean-up-share-{measure}");
        let Some(page) = open(connection, site, &run_id, &settings, checks)? else {
            return finish(connection, &run_id).map(drop);
        };
        for path in ["/hiding", "/removal"] {
            let (url, answer) = visit(connection, site, &page, path)?;
            answered(&answer, &format!("the `goto` to {path}"), &url, 200, checks);
            markdown(connection, &page)?;
        }
        finish(connection, &run_id)?;
        let lines = provenance(target, &run_id);
        let [hiding, removal] = ["/hiding", "/removal"].map(|path| {
            removals_of(&lines, path).first().map(|line| {
                (
                    line["share"].as_f64().unwrap_or(-1.0),
                    line["share_measure"].clone(),
                    line["flagged"].clone(),
                )
            })
        });
        checks.check(
            format!("under `{measure}`, Provenance records each share as measured by `{measure}`"),
            [&hiding, &removal].iter().all(|line| {
                line.as_ref()
                    .is_some_and(|(_, named, _)| *named == json!(measure))
            }),
        );
        checks.check(
            format!("under `{measure}`, `/hiding`'s removal is unflagged and `/removal`'s flagged, over a fifth"),
            hiding.as_ref().is_some_and(|(_, _, flagged)| *flagged == json!(false))
                && removal.as_ref().is_some_and(|(share, _, flagged)| {
                    *flagged == json!(true) && *share > 0.2
                }),
        );
        shares.push([
            hiding.map(|(share, ..)| share),
            removal.map(|(share, ..)| share),
        ]);
    }
    checks.check(
        "each page's estimated share is at least its exact share",
        shares.len() == 2
            && shares[0]
                .iter()
                .zip(&shares[1])
                .all(|(estimate, exact)| matches!((estimate, exact), (Some(estimate), Some(exact)) if estimate >= exact)),
    );
    Ok(())
}

/// On `/hiding`, live hiding hides the ad of the page and of its frame, and
/// the page's own script sees in `document.styleSheets` only its own style
/// sheet, and its frame's none.
fn hiding_sheet_unseen(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-hiding";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = (targets(site, SEEN).len(), targets(site, FRAME_SEEN).len());
    let (url, answer) = visit(connection, site, &page, "/hiding")?;
    answered(&answer, "the `goto` to `/hiding`", &url, 200, checks);
    let seen = next_report(site, SEEN, before.0);
    checks.check(
        format!("the page's ad is hidden and its script sees only its own style sheet ({seen:?})"),
        seen.as_deref() == Some(HIDDEN_UNSEEN),
    );
    let frame = next_report(site, FRAME_SEEN, before.1);
    checks.check(
        format!("the frame's ad is hidden and its script sees no style sheet ({frame:?})"),
        frame.as_deref() == Some(FRAME_HIDDEN_UNSEEN),
    );
    finish(connection, run_id)?;
    let lines = provenance(target, run_id);
    checks.check(
        "Provenance records live hiding as `auto` on the page",
        lines
            .iter()
            .any(|line| line["action"] == json!("live_hiding") && line["mode"] == json!("auto")),
    );
    Ok(())
}

/// A click on `/hiding`'s button, inside the ad live hiding hid, lifts the
/// ad's rule and lands; Provenance records the rule; and a second page of
/// the Session shows the ad, the rule lifted for the Session.
fn hidden_target_lifted(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-clean-up-lifted";
    let settings = json!({ "timeout": { "action": 10_000 } });
    let Some(page) = open(connection, site, run_id, &settings, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = targets(site, SEEN).len();
    let (url, answer) = visit(connection, site, &page, "/hiding")?;
    answered(&answer, "the `goto` to `/hiding`", &url, 200, checks);
    let seen = next_report(site, SEEN, before);
    checks.check(
        format!("the ad is hidden before the click ({seen:?})"),
        seen.as_deref() == Some(HIDDEN_UNSEEN),
    );
    let clicks = received(site, "/hiding/clicked");
    let id = connection.request(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page.page_id, "selector": "#inside" } }),
    )?;
    let clicked = connection.browser_answer(id)?.message;
    checks.check(
        format!(
            "`{}` on the button inside the hidden ad is answered ({clicked})",
            LOCATOR_CLICK.name
        ),
        clicked.get("result").is_some(),
    );
    checks.check(
        "the click reached the button",
        next_report(site, "/hiding/clicked", clicks).is_some(),
    );
    let Some(second) = new_page(connection, run_id, checks)? else {
        return finish(connection, run_id).map(drop);
    };
    let before = targets(site, SEEN).len();
    let (url, answer) = visit(connection, site, &second, "/hiding")?;
    answered(&answer, "the second page's `goto`", &url, 200, checks);
    let seen = next_report(site, SEEN, before);
    checks.check(
        format!("the Session's second page shows the ad, its rule lifted ({seen:?})"),
        seen.as_deref() == Some(SHOWN),
    );
    finish(connection, run_id)?;
    let lines = provenance(target, run_id);
    checks.check(
        "Provenance records the rule lifted for the click",
        lines.iter().any(|line| {
            line["action"] == json!("live_hiding_lifted")
                && line["rule"] == json!(".ad-slot")
                && line["page_id"] == json!(page.page_id)
        }),
    );
    Ok(())
}

/// Under `live_view.hiding: off` the ad shows on the live page, and its
/// text is still removed from extracted content, and Provenance records
/// the mode; under `always` it is hidden.
fn live_hiding_modes(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    for (mode, expected) in [("off", SHOWN), ("always", HIDDEN_UNSEEN)] {
        let run_id = format!("conformance-clean-up-hiding-{mode}");
        let settings = json!({ "live_view": { "hiding": mode } });
        let Some(page) = open(connection, site, &run_id, &settings, checks)? else {
            return finish(connection, &run_id).map(drop);
        };
        let before = targets(site, SEEN).len();
        let (url, answer) = visit(connection, site, &page, "/hiding")?;
        answered(
            &answer,
            &format!("the `goto` under `{mode}`"),
            &url,
            200,
            checks,
        );
        let seen = next_report(site, SEEN, before);
        checks.check(
            format!("under `live_view.hiding: {mode}` the page reports {expected} ({seen:?})"),
            seen.as_deref() == Some(expected),
        );
        let read = markdown(connection, &page)?;
        checks.check(
            format!("under `{mode}` the Markdown still leaves the ad's text out"),
            read.contains(ARTICLE_TEXT) && !read.contains(AD_TEXT),
        );
        finish(connection, &run_id)?;
        let lines = provenance(target, &run_id);
        checks.check(
            format!("Provenance records live hiding as `{mode}`"),
            lines
                .iter()
                .any(|line| line["action"] == json!("live_hiding") && line["mode"] == json!(mode)),
        );
    }
    Ok(())
}
