//! The page suite's Consent prompt cases (ADR 0048, ticket #717) on the
//! Proving ground's `consent.test` pages, on Chromium: prompts modelled on
//! OneTrust, Cookiebot, Didomi, Usercentrics (in shadow DOM), Sourcepoint
//! (in a cross-origin frame), Quantcast, TrustArc, Google's consent page and
//! Axeptio are refused through trusted clicks; a prompt no rule knows is
//! refused in twelve languages, in the page and in a cross-origin frame,
//! when it appears late and on a route change; a notice with no refusal is
//! hidden by a sheet the page cannot list; the page's own
//! `HTMLElement.prototype.click` hook never sees the runner's clicks, and no
//! `autoconsent-*` element is ever put in a page; `Runtime` is never
//! enabled, a vendor's own API is called in the main world and an isolated
//! snippet never touches it; and `consent.handling: false` leaves prompts
//! alone. Each is judged by the requests the pages report to the Proving
//! ground and by the Run trace the engine writes, its Provenance log
//! among it. The consent wall cases (tickets #718 and #859) are `walls`'.

use std::time::Duration;

use goodfetch_protocol::home::{RUN_TRACE_CDP_COMMANDS, RUN_TRACE_PROVENANCE};
use goodfetch_protocol::page::{PAGE_FORMATS, PAGE_GOTO, PageOpened};
use goodfetch_protocol::run::RUN_FINISH;
use serde_json::{Value, json};

use super::navigation::call;
use super::{Checks, Site, new_page, poll, run_trace, start_run};
use crate::protocol::connection::{Connection, Gone};

pub(super) mod walls;

/// The host the pages are served on.
pub(super) const HOST: &str = "consent.test";

/// The host of the framed prompts, another site.
pub(super) const FRAME_HOST: &str = "consent-frame.test";

/// How long a case waits for a page to report an answer.
const ANSWER_WAIT: Duration = Duration::from_secs(20);

/// The vendors whose prompts the pages model, each with the answer their
/// refusal reports: Cookiebot's is its own API's `withdraw()`.
const VENDORS: [(&str, &str); 10] = [
    ("onetrust", "reject"),
    ("cookiebot", "withdraw"),
    ("didomi", "reject"),
    ("usercentrics", "reject"),
    ("sourcepoint", "reject"),
    ("quantcast", "reject"),
    ("trustarc", "reject"),
    ("google", "reject"),
    ("axeptio", "reject"),
    ("tiktok", "reject"),
];

/// The languages of the prompt no rule knows.
const LANGUAGES: [&str; 12] = [
    "en", "fr", "de", "nl", "es", "it", "pl", "ru", "ja", "ca", "eu", "gl",
];

/// One Consent prompt case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Vendors,
    Languages,
    LateAndRouteChange,
    Notice,
    Overrides,
    Worlds,
    Off,
    Walls,
    WallsAtOnce,
    WallsOff,
    Accepted,
    WallSettings,
    Offered,
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "prompts-modelled-on-each-vendor-are-refused-through-trusted-clicks" => Self::Vendors,
            "a-prompt-no-rule-knows-is-refused-in-twelve-languages-in-the-page-and-in-a-cross-origin-frame" => {
                Self::Languages
            }
            "a-prompt-that-appears-late-or-on-a-route-change-is-refused" => {
                Self::LateAndRouteChange
            }
            "a-prompt-with-no-refusal-that-leaves-the-content-readable-is-hidden" => Self::Notice,
            "the-pages-own-click-hook-never-sees-the-runners-clicks-and-no-autoconsent-element-is-left" => {
                Self::Overrides
            }
            "runtime-is-never-enabled-and-a-snippets-world-is-as-the-snippet-table-says" => {
                Self::Worlds
            }
            "consent-handling-off-leaves-prompts-alone" => Self::Off,
            "a-notice-is-hidden-and-a-wall-raises-blocked-with-consent-required-and-is-never-answered" => {
                Self::Walls
            }
            "with-consent-handling-off-the-wall-test-still-raises-blocked" => Self::WallsOff,
            "a-walls-navigation-raises-it-before-any-call-reads-the-page-even-one-a-filter-list-would-hide" => {
                Self::WallsAtOnce
            }
            "on-a-domain-in-consent-domains-the-wall-is-accepted-and-provenance-records-the-acceptance" => {
                Self::Accepted
            }
            "a-run-widening-consent-domains-is-setting-locked-and-on-wall-ask-is-unavailable-until-the-human-pause" => {
                Self::WallSettings
            }
            "a-domain-whose-consent-wall-a-run-left-is-offered-in-needs-you-for-consent-domains" => {
                Self::Offered
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to the engine whose Goodfetch
    /// home is `home`, against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        home: Option<&str>,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::Vendors => vendors(connection, site, checks),
            Self::Languages => languages(connection, site, checks),
            Self::LateAndRouteChange => late_and_route_change(connection, site, checks),
            Self::Notice => notice(connection, home, site, checks),
            Self::Overrides => overrides(connection, site, checks),
            Self::Worlds => worlds(connection, home, site, checks),
            Self::Off => off(connection, home, site, checks),
            Self::Walls => walls::walls(connection, home, site, checks),
            Self::WallsAtOnce => walls::walls_at_once(connection, site, checks),
            Self::WallsOff => walls::walls_off(connection, home, site, checks),
            Self::Accepted => walls::accepted(connection, home, site, checks),
            Self::WallSettings => walls::settings(connection, site, checks),
            Self::Offered => walls::offered(connection, site, checks),
        }
    }
}

/// The query parameters of each report the pages sent to `path`.
fn reports(site: &Site, path: &str) -> Vec<Vec<(String, String)>> {
    site.received_requests()
        .of_the_case()
        .filter(|request| [HOST, FRAME_HOST].contains(&request.host.as_str()))
        .filter_map(|request| {
            let (target, query) = request.target.split_once('?')?;
            (target == path).then(|| {
                query
                    .split('&')
                    .filter_map(|pair| pair.split_once('='))
                    .map(|(name, value)| (name.to_owned(), value.to_owned()))
                    .collect()
            })
        })
        .collect()
}

/// How many reports to `path` carry every one of `wanted`.
fn count(site: &Site, path: &str, wanted: &[(&str, &str)]) -> usize {
    reports(site, path)
        .iter()
        .filter(|report| {
            wanted
                .iter()
                .all(|(name, value)| report.iter().any(|(n, v)| n == name && v == value))
        })
        .count()
}

/// Waits until more than `before` reports to `path` carry `wanted`, or
/// `wait` passes; whether they did.
fn arrives(
    site: &Site,
    path: &str,
    wanted: &[(&str, &str)],
    before: usize,
    wait: Duration,
) -> bool {
    let counted = poll::until(
        wait,
        Duration::from_millis(200),
        || count(site, path, wanted),
        |counted| *counted > before,
    );
    counted > before
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

/// `page.goto` on `page` to `path` on the pages' host; whether it answered.
fn visit(
    connection: &mut Connection,
    site: &Site,
    page: &PageOpened,
    path: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let url = site.named_url(HOST, path);
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &json!(page.page_id),
        &json!({ "url": url }),
    )?;
    checks.check(
        format!("the `goto` to {path} answers its response"),
        answer["result"]["response"]["status"] == json!(200),
    );
    Ok(())
}

/// Lets the Run `run_id` finish.
fn finish(connection: &mut Connection, run_id: &str) -> Result<(), Gone> {
    let id = connection.request(RUN_FINISH.name, &json!({ "run_id": run_id }))?;
    connection.browser_answer(id).map(drop)
}

/// Visits `path` and checks that the prompt reporting as `vendor` is
/// refused with `answer` through a trusted click, or its own API, and never
/// accepted.
fn refused(
    connection: &mut Connection,
    site: &Site,
    page: &PageOpened,
    (path, vendor, answer): (&str, &str, &str),
    checks: &mut Checks,
) -> Result<(), Gone> {
    let wanted: &[(&str, &str)] = if answer == "reject" {
        &[("vendor", vendor), ("answer", answer), ("trusted", "true")]
    } else {
        &[("vendor", vendor), ("answer", answer)]
    };
    let before = count(site, "/consent/answered", wanted);
    visit(connection, site, page, path, checks)?;
    checks.check(
        format!("{vendor}'s prompt at {path} is refused ({answer})"),
        arrives(site, "/consent/answered", wanted, before, ANSWER_WAIT),
    );
    checks.check(
        format!("{vendor}'s prompt is never accepted"),
        count(
            site,
            "/consent/answered",
            &[("vendor", vendor), ("answer", "accept")],
        ) == 0,
    );
    Ok(())
}

/// Each vendor's prompt is refused: OneTrust's, Didomi's, Quantcast's,
/// TrustArc's, Google's and Axeptio's by a trusted click on their refusal,
/// Usercentrics' and TikTok's in a shadow root, Sourcepoint's in a
/// cross-origin frame, and Cookiebot's through its own API.
fn vendors(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-consent-vendors";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    for (vendor, answer) in VENDORS {
        refused(
            connection,
            site,
            &page,
            (&format!("/vendor/{vendor}"), vendor, answer),
            checks,
        )?;
    }
    finish(connection, run_id)
}

/// The prompt no rule knows is refused in each of twelve languages, in the
/// page and from a cross-origin frame.
fn languages(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-consent-languages";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    for language in LANGUAGES {
        refused(
            connection,
            site,
            &page,
            (
                &format!("/heuristic/{language}"),
                &format!("heuristic-{language}"),
                "reject",
            ),
            checks,
        )?;
        refused(
            connection,
            site,
            &page,
            (
                &format!("/framed/{language}"),
                &format!("framed-{language}"),
                "reject",
            ),
            checks,
        )?;
    }
    finish(connection, run_id)
}

/// A prompt shown 3 s after the page loaded, and one shown on a single-page
/// app's next route, are refused.
fn late_and_route_change(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-consent-late";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    refused(connection, site, &page, ("/late", "late", "reject"), checks)?;
    refused(connection, site, &page, ("/spa", "spa", "reject"), checks)?;
    finish(connection, run_id)
}

/// A notice with only an acknowledgement is no longer shown, though the
/// page's `document.styleSheets` lists no sheet, and nothing answers it;
/// the runner's rule and live hiding's share the document's one hiding
/// sheet, made by a single `CSS.createStyleSheet`.
fn notice(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-consent-notice";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    let hidden: &[(&str, &str)] = &[("vendor", "notice"), ("visible", "false"), ("sheets", "0")];
    let before = count(site, "/consent/notice", hidden);
    visit(connection, site, &page, "/notice", checks)?;
    checks.check(
        "the notice is hidden by a sheet the page's `document.styleSheets` does not list",
        arrives(site, "/consent/notice", hidden, before, ANSWER_WAIT),
    );
    checks.check(
        "nothing answered the notice",
        count(site, "/consent/answered", &[("vendor", "notice")]) == 0,
    );
    let formats = call(
        connection,
        PAGE_FORMATS.name,
        &json!(page.page_id),
        &json!({ "formats": { "markdown": true } }),
    )?;
    let markdown = formats["result"]["markdown"].as_str().unwrap_or_default();
    checks.check(
        "the hidden notice's text is absent from the page's Markdown, and its own text is there",
        !markdown.contains("This website uses cookies") && markdown.contains("The article's text"),
    );
    finish(connection, run_id)?;
    let made = checks
        .run_trace(run_trace::lines(home, run_id, RUN_TRACE_CDP_COMMANDS))
        .iter()
        .filter(|line| line["method"] == json!("CSS.createStyleSheet"))
        .count();
    checks.check(
        format!(
            "the notice's document has one hiding sheet, live hiding's and the runner's ({made} made)"
        ),
        made == 1,
    );
    Ok(())
}

/// TikTok's rule clicks its refusal with `element.click()`, which the
/// runner's override in its own world sends to the guard: the click is
/// trusted and the page's own `HTMLElement.prototype.click` hook never sees
/// it, nor any other runner click; and no page was ever given an
/// `autoconsent-*` element.
fn overrides(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-consent-overrides";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    refused(
        connection,
        site,
        &page,
        ("/vendor/tiktok", "tiktok", "reject"),
        checks,
    )?;
    checks.check(
        "no page's own click hook saw a click of the runner",
        reports(site, "/consent/hooked").is_empty(),
    );
    checks.check(
        "no page was given an `autoconsent-*` element",
        reports(site, "/consent/leftover").is_empty(),
    );
    finish(connection, run_id)
}

/// Cookiebot's refusal calls its own `Cookiebot.withdraw()`, a page
/// global, in the main world through `DOM.resolveNode`; Moove's snippet
/// unticks its performance cookies from the isolated world, where the
/// page's own `checked` setter never runs; `Runtime` is never enabled;
/// and the Run trace records both refusals with their rules.
fn worlds(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-consent-worlds";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    let closed = count(site, "/consent/closed", &[("vendor", "cookiebot")]);
    refused(
        connection,
        site,
        &page,
        ("/vendor/cookiebot", "cookiebot", "withdraw"),
        checks,
    )?;
    checks.check(
        "Cookiebot's own `hide()` closed its dialog, from the main world",
        arrives(
            site,
            "/consent/closed",
            &[("vendor", "cookiebot")],
            closed,
            ANSWER_WAIT,
        ),
    );
    let saved: &[(&str, &str)] = &[
        ("vendor", "moove"),
        ("answer", "save"),
        ("trusted", "true"),
        ("performance", "false"),
        ("hooks", "0"),
    ];
    let before = count(site, "/consent/answered", saved);
    visit(connection, site, &page, "/vendor/moove", checks)?;
    checks.check(
        "Moove's snippet unticked its performance cookies from the isolated world, the page's own setter never running",
        arrives(site, "/consent/answered", saved, before, ANSWER_WAIT),
    );
    finish(connection, run_id)?;
    let commands = checks.run_trace(run_trace::lines(home, run_id, RUN_TRACE_CDP_COMMANDS));
    let sent = |method: &str| commands.iter().any(|line| line["method"] == json!(method));
    checks.check("`Runtime.enable` was never sent", !sent("Runtime.enable"));
    checks.check(
        "the main-world snippet went through `DOM.resolveNode`",
        sent("DOM.resolveNode"),
    );
    let outcomes = checks.run_trace(run_trace::lines(home, run_id, RUN_TRACE_PROVENANCE));
    for rule in ["Cybotcookiebot", "Moove"] {
        checks.check(
            format!("Provenance records {rule}'s prompt refused"),
            outcomes.iter().any(|line| {
                line["action"] == json!("consent")
                    && line["rule"] == json!(rule)
                    && line["answer"] == json!("refused")
            }),
        );
    }
    Ok(())
}

/// With `consent.handling: false`, the runner only looks: Provenance
/// records OneTrust's prompt left, a state the engine names, and no answer
/// came, read once the Run finished, which ends its pages, never after a
/// fixed wait. The wall test, which still runs, is `walls`'.
fn off(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-consent-off";
    let settings = json!({ "consent": { "handling": false } });
    let Some(page) = open(connection, site, run_id, &settings, checks)? else {
        return finish(connection, run_id);
    };
    let before = count(site, "/consent/answered", &[("vendor", "onetrust")]);
    visit(connection, site, &page, "/vendor/onetrust", checks)?;
    let left = checks.run_trace(walls::recorded(
        home,
        run_id,
        &page,
        ("/vendor/onetrust", "left"),
    ));
    finish(connection, run_id)?;
    checks.check(
        "with `consent.handling` off, Provenance records OneTrust's prompt left",
        left.is_some(),
    );
    checks.check(
        "with `consent.handling` off, nothing answered the prompt",
        count(site, "/consent/answered", &[("vendor", "onetrust")]) == before,
    );
    Ok(())
}
