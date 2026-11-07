//! The page suite's consent wall cases (ADR 0048, ticket #718) on the
//! Proving ground's `consent.test` pages, on Chromium: a notice with no
//! refusal is hidden, while a consent wall, a prompt with no refusal that
//! covers the content or offers a subscription instead, is the Block cause
//! `consent_required`, raised as `blocked` by the call that waited for its
//! navigation or else by the page's next call, the Session kept, nothing
//! answered; with `consent.handling` off the wall test still runs. On a
//! domain user configuration lists in `consent.domains`, the wall is
//! accepted through a trusted click and Provenance records the acceptance
//! and the approval it came from, while a prompt with a refusal is still
//! refused. A run that widens `consent.domains` is refused with
//! `setting_locked`, and `consent.on_wall: ask` is `capability_unavailable`
//! until the Human pause can take a person's answer.
//! A domain whose consent wall a Run left is offered in "Needs you" for
//! `consent.domains` (ticket #859).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use goodfetch_protocol::home::{RUN_TRACE_PROVENANCE, RUNS};
use goodfetch_protocol::needs_you::{CONSENT_WALL, NEEDS_YOU_ALLOW_CONSENT_DOMAIN, NEEDS_YOU_LIST};
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::{BLOCKED, BlockedDetails, PAGE_GOTO, PageOpened, RUN_NEW_PAGE};
use goodfetch_protocol::run::RUN_START;
use goodfetch_protocol::settings::{CAPABILITY_UNAVAILABLE, SETTING_LOCKED};
use serde_json::{Value, json};

use super::super::navigation::call;
use super::super::{Checks, Site, start_run};
use super::{ANSWER_WAIT, HOST, arrives, count, finish, open, refused, trace, visit};
use crate::protocol::connection::{Connection, Gone};

/// How long a case waits for the runner's verdict on a page.
const VERDICT_WAIT: Duration = Duration::from_secs(20);

/// User configuration's file in a per-major folder of the Goodfetch home.
const USER_CONFIGURATION: &str = "configuration.toml";

/// The consent wall pages, each with the name its answers report under.
const WALLS: [(&str, &str); 2] = [("/wall", "wall"), ("/pay-or-accept", "pay-or-accept")];

/// The Provenance entries of the Consent prompt runner on the Run
/// `run_id`'s page `page`, each with its `answer`.
fn consent_entries(home: Option<&str>, run_id: &str, page: &PageOpened) -> Vec<Value> {
    trace(home, run_id, RUN_TRACE_PROVENANCE)
        .into_iter()
        .filter(|line| line["action"] == json!("consent") && line["page_id"] == json!(page.page_id))
        .collect()
}

/// Waits until Provenance records the answer `answer` on the document at
/// `path` of the Run `run_id`'s page `page`; the entry, if it came.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness polls the Run trace on its own thread, outside any async worker"
)]
pub(super) fn recorded(
    home: Option<&str>,
    run_id: &str,
    page: &PageOpened,
    (path, answer): (&str, &str),
) -> Option<Value> {
    let started = Instant::now();
    loop {
        let found = consent_entries(home, run_id, page)
            .into_iter()
            .find(|line| {
                line["answer"] == json!(answer)
                    && line["url"].as_str().is_some_and(|url| url.ends_with(path))
            });
        if found.is_some() || started.elapsed() > VERDICT_WAIT {
            return found;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// `page.evaluate` of a function that reads the page's title.
fn read_title(connection: &mut Connection, page: &PageOpened) -> Result<Value, Gone> {
    call(
        connection,
        PAGE_EVALUATE.name,
        &json!(page.page_id),
        &json!({ "function": "() => document.title" }),
    )
}

/// The details of `answer` when it raised `blocked`.
fn blocked(answer: &Value) -> Option<BlockedDetails> {
    let data = &answer["error"]["data"];
    (data["code"] == json!(BLOCKED.name))
        .then(|| serde_json::from_value(data.clone()).ok())
        .flatten()
}

/// Whether `answer` raised `blocked` with `consent_required` on the
/// document at a URL ending with `path`, answered 200.
fn walled(answer: &Value, path: &str) -> bool {
    blocked(answer).is_some_and(|details| {
        details.cause.as_str() == "consent_required"
            && details.url.ends_with(path)
            && details.status == Some(200)
    })
}

/// The Error code `answer` raised.
fn code_of(answer: &Value) -> &Value {
    &answer["error"]["data"]["code"]
}

/// Visits the wall at `path` on `page` and checks that the runner's verdict
/// is a wall, raised as `blocked` with `consent_required` by the `goto` or
/// else by the page's next call, once, the call after answering, and that
/// nothing answered the wall.
fn blocked_at(
    connection: &mut Connection,
    home: Option<&str>,
    (run_id, site, page): (&str, &Site, &PageOpened),
    (path, vendor): (&str, &str),
    checks: &mut Checks,
) -> Result<(), Gone> {
    let url = site.named_url(HOST, path).unwrap_or_default();
    let went = call(
        connection,
        PAGE_GOTO.name,
        &json!(page.page_id),
        &json!({ "url": url }),
    )?;
    let entry = recorded(home, run_id, page, (path, "wall"));
    checks.check(
        format!("Provenance records the prompt at {path} as a wall, `consent_required`"),
        entry
            .as_ref()
            .is_some_and(|entry| entry["cause"] == json!("consent_required")),
    );
    let raised = if walled(&went, path) {
        true
    } else {
        let next = read_title(connection, page)?;
        walled(&next, path)
    };
    checks.check(
        format!(
            "the wall at {path} raises `blocked` with `consent_required` from its `goto` or the page's next call"
        ),
        raised,
    );
    let after = read_title(connection, page)?;
    checks.check(
        format!("the call after the wall at {path} answers, its Session kept"),
        after.get("result").is_some(),
    );
    checks.check(
        format!("nothing answered the wall at {path}"),
        count(site, "/consent/answered", &[("vendor", vendor)]) == 0,
    );
    Ok(())
}

/// A notice with no refusal is hidden and its page answers its next call;
/// the wall covering the viewport and the "accept or pay" banner are each
/// `consent_required`, answered by nobody.
pub(super) fn walls(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-consent-walls";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    let hidden: &[(&str, &str)] = &[("vendor", "notice"), ("visible", "false")];
    let before = count(site, "/consent/notice", hidden);
    visit(connection, site, &page, "/notice", checks)?;
    checks.check(
        "the notice with no refusal is hidden",
        arrives(site, "/consent/notice", hidden, before, ANSWER_WAIT),
    );
    let title = read_title(connection, &page)?;
    checks.check(
        "the hidden notice's page answers its next call",
        title.get("result").is_some(),
    );
    for wall in WALLS {
        blocked_at(connection, home, (run_id, site, &page), wall, checks)?;
    }
    finish(connection, run_id)
}

/// The call that waited for a wall's navigation raises it, so no call reads
/// the walled page, with no wait on the Run trace between: the runner's
/// verdict comes before the `goto` answers, for the wall covering the
/// viewport and for one a Filter list's cosmetic rule would hide, which
/// live hiding leaves alone until the wall test judged it.
pub(super) fn walls_at_once(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-consent-walls-at-once";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    for path in ["/wall", "/hidden-wall"] {
        let url = site.named_url(HOST, path).unwrap_or_default();
        let went = call(
            connection,
            PAGE_GOTO.name,
            &json!(page.page_id),
            &json!({ "url": url }),
        )?;
        checks.check(
            format!("the `goto` to the wall at {path} raises `blocked` with `consent_required`"),
            walled(&went, path),
        );
    }
    finish(connection, run_id)
}

/// With `consent.handling` off, the wall test still runs: the wall is
/// `consent_required`, raised as `blocked`.
pub(super) fn walls_off(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-consent-walls-off";
    let settings = json!({ "consent": { "handling": false } });
    let Some(page) = open(connection, site, run_id, &settings, checks)? else {
        return finish(connection, run_id);
    };
    blocked_at(
        connection,
        home,
        (run_id, site, &page),
        ("/wall", "wall"),
        checks,
    )?;
    finish(connection, run_id)
}

/// The per-major folder of the Goodfetch home `home` that holds the Run
/// `run_id`'s folder.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
pub(crate) fn major_of(home: &str, run_id: &str) -> Option<PathBuf> {
    std::fs::read_dir(home)
        .ok()?
        .flatten()
        .map(|major| major.path())
        .find(|major| major.join(RUNS).join(run_id).is_dir())
}

/// Writes `text` as user configuration in `major`, answering what was
/// there before.
#[expect(
    clippy::disallowed_methods,
    reason = "the harness is synchronous and runs no async worker"
)]
pub(crate) fn configure(major: &std::path::Path, text: Option<&str>) -> Option<String> {
    let file = major.join(USER_CONFIGURATION);
    let before = std::fs::read_to_string(&file).ok();
    match text {
        Some(text) => drop(std::fs::write(&file, text)),
        None => drop(std::fs::remove_file(&file)),
    }
    before
}

/// With `consent.domains` listing `consent.test` in user configuration, the
/// wall is accepted through a trusted click, its page answers its next
/// call, and Provenance records the acceptance with the domain and the
/// approval, `consent.domains` in user configuration; OneTrust's prompt,
/// which offers a refusal, is still refused there.
pub(super) fn accepted(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let first = "conformance-consent-home";
    start_run(connection, site, first, &json!({}), checks)?;
    finish(connection, first)?;
    let Some(major) = home.and_then(|home| major_of(home, first)) else {
        checks.check("the engine's per-major folder is found", false);
        return Ok(());
    };
    let before = configure(
        &major,
        Some("version = \"0.1\"\n\n[settings.consent]\ndomains = [\"consent.test\"]\n"),
    );
    let run_id = "conformance-consent-accepted";
    let outcome = accepted_on(connection, home, site, run_id, checks);
    configure(&major, before.as_deref());
    outcome
}

fn accepted_on(
    connection: &mut Connection,
    home: Option<&str>,
    site: &Site,
    run_id: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    let accept: &[(&str, &str)] = &[
        ("vendor", "wall"),
        ("answer", "accept"),
        ("trusted", "true"),
    ];
    let before = count(site, "/consent/answered", accept);
    visit(connection, site, &page, "/wall", checks)?;
    checks.check(
        "on a domain in `consent.domains`, the wall is accepted through a trusted click",
        arrives(site, "/consent/answered", accept, before, ANSWER_WAIT),
    );
    let entry = recorded(home, run_id, &page, ("/wall", "accepted"));
    checks.check(
        "Provenance records the acceptance, its domain and its approval, `consent.domains` in user configuration",
        entry.as_ref().is_some_and(|entry| {
            entry["domain"] == json!(HOST)
                && entry["approval"] == json!({ "key": "consent.domains", "layer": "user_configuration" })
        }),
    );
    let title = read_title(connection, &page)?;
    checks.check(
        "the accepted wall's page answers its next call",
        title.get("result").is_some(),
    );
    refused(
        connection,
        site,
        &page,
        ("/vendor/onetrust", "onetrust", "reject"),
        checks,
    )?;
    finish(connection, run_id)
}

/// How long "Needs you" may take to list a wall's domain: the Run index's
/// write, then its source's next read at a storage sample.
const OFFER_WAIT: Duration = Duration::from_secs(20);

/// The `consent_wall` item of `domain` in "Needs you" once it names the
/// Run `run_id` as the last to meet the wall, or as it is listed when the
/// wait ends: an earlier case's Run may have met the same wall.
#[expect(
    clippy::disallowed_methods,
    reason = "the synchronous harness polls the engine on its own thread, outside any async worker"
)]
fn listed_wall(
    connection: &mut Connection,
    domain: &str,
    run_id: &str,
) -> Result<Option<Value>, Gone> {
    let started = Instant::now();
    loop {
        let id = connection.request(NEEDS_YOU_LIST.name, &json!({ "kinds": [CONSENT_WALL] }))?;
        let answer = connection.answer(id)?.message;
        let found = answer["result"]["items"].as_array().and_then(|items| {
            items
                .iter()
                .find(|item| item["details"]["domain"] == json!(domain))
                .cloned()
        });
        let named = found
            .as_ref()
            .is_some_and(|item| item["run"] == json!(run_id));
        if named || started.elapsed() > OFFER_WAIT {
            return Ok(found);
        }
        std::thread::sleep(Duration::from_millis(250));
    }
}

/// A consent wall a Run left, raised as `blocked`, has "Needs you" list its
/// domain as a `consent_wall` item offered for `consent.domains`, naming
/// the Run.
pub(super) fn offered(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-consent-offered";
    let Some(page) = open(connection, site, run_id, &json!({}), checks)? else {
        return finish(connection, run_id);
    };
    let url = site.named_url(HOST, "/wall").unwrap_or_default();
    let went = call(
        connection,
        PAGE_GOTO.name,
        &json!(page.page_id),
        &json!({ "url": url }),
    )?;
    if !walled(&went, "/wall") {
        // The runner's verdict came after the navigation: the next call
        // raises it.
        read_title(connection, &page)?;
    }
    let item = listed_wall(connection, HOST, run_id)?;
    checks.check(
        format!(
            "\"Needs you\" lists {HOST}, whose consent wall the Run left, as a `consent_wall` item"
        ),
        item.is_some(),
    );
    checks.check(
        "the item offers the domain for `consent.domains` and names the Run",
        item.as_ref().is_some_and(|item| {
            item["actions"].as_array().is_some_and(|actions| {
                actions.contains(&json!(NEEDS_YOU_ALLOW_CONSENT_DOMAIN.name))
            }) && item["run"] == json!(run_id)
        }),
    );
    finish(connection, run_id)
}

/// A run that widens `consent.domains` is refused with `setting_locked`;
/// `consent.on_wall: ask` opens no page, `capability_unavailable`, until
/// the Human pause takes a person's answer.
pub(super) fn settings(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let id = connection.request(
        RUN_START.name,
        &json!({
            "run_id": "conformance-consent-widened",
            "settings": site.mapped(&json!({ "consent": { "domains": [HOST] } }), false),
        }),
    )?;
    let widened = connection.answer(id)?.message;
    checks.check(
        "a run that widens `consent.domains` is refused with `setting_locked`",
        code_of(&widened) == &json!(SETTING_LOCKED.name),
    );
    let run_id = "conformance-consent-ask";
    start_run(
        connection,
        site,
        run_id,
        &json!({ "consent": { "on_wall": "ask" } }),
        checks,
    )?;
    let id = connection.request(RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
    let asked = connection.browser_answer(id)?.message;
    checks.check(
        "`consent.on_wall: ask` opens no page, `capability_unavailable`, until the Human pause takes a person's answer",
        code_of(&asked) == &json!(CAPABILITY_UNAVAILABLE.name),
    );
    finish(connection, run_id)
}
