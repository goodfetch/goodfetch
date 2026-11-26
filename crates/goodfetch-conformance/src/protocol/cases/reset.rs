//! The reset gate's cases (ADRs 0001, 0002, 0015 and 0067): a Run discarded
//! with `run.discard` leaves nothing of itself behind. Each case plants
//! Secret canaries through the Proving ground (`POST /canaries`) into a
//! Run, as a token and a password it keeps in its state or as the Session
//! cookie a login sets in its browser, discards the Run once it ended, and
//! searches every file of the target's Goodfetch home for them, raw and in
//! Goodfetch's encodings; a later Run under the same Run ID then finds none
//! of its state and is sent no cookie of it, thirty Runs in a row on one
//! engine included, the lab's leak with a reused pool (ADR 0015). A
//! Borrowed browser's address, the one field a Run seals today, is out of
//! the runner's reach: it borrows no browser. The home is searched only
//! where the engine keeps its own state: a Profile directory used in place
//! and a Borrowed browser are outside it (ADR 0056).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use goodfetch_protocol::key_value_store::{KEY_VALUE_STORE_GET, KEY_VALUE_STORE_SET};
use goodfetch_protocol::page::events::PAGE_WAIT_FOR_REQUEST_FINISHED;
use goodfetch_protocol::page::{LOCATOR_CLICK, LOCATOR_FILL, LOGGED_OUT, PAGE_GOTO};
use goodfetch_protocol::run::{RUN_DISCARD, RUN_SET_PROGRESS_MESSAGE, RUN_START};
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, new_page, start_run};
use crate::canaries::held_under;
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;
use crate::proving_ground::{CanaryAccount, PlantedCanary};

/// The Proving ground's task whose login sets a Session cookie canary.
const COOKIE_TASK: &str = "canaries/3";

/// A reset gate case.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    SecretValues,
    SessionCookie,
    ReusedPool,
}

impl Case {
    /// The case `id` names, if it is one of these.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-discarded-run-leaves-none-of-its-secret-values-in-the-home" => Self::SecretValues,
            "a-discarded-runs-session-cookie-reaches-no-later-run" => Self::SessionCookie,
            "thirty-runs-in-a-row-on-one-engine-each-log-in-afresh" => Self::ReusedPool,
            _ => return None,
        })
    }

    /// Runs the case on the open Connection to `target`, against `site`.
    pub(crate) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        let home = target.environment.get("GOODFETCH_HOME").map(PathBuf::from);
        checks.check(
            "the target runs in a Goodfetch home of its own",
            home.is_some(),
        );
        let Some(home) = home else {
            return Ok(());
        };
        match self {
            Self::SecretValues => secret_values(connection, &home, site, checks),
            Self::SessionCookie => session_cookie(connection, &home, site, checks),
            Self::ReusedPool => reused_pool(connection, &home, site, checks),
        }
    }
}

/// The canaries the Proving ground plants of each of `kinds`.
fn planted(site: &Site, kinds: &[&str], checks: &mut Checks) -> Option<Vec<PlantedCanary>> {
    let planted: Result<Vec<PlantedCanary>, _> = kinds
        .iter()
        .map(|kind| site.proving_ground.plant_canary(kind))
        .collect();
    checks.read(
        "the Proving ground plants the case's Secret canaries",
        planted,
    )
}

/// Each file of `home` holding one of `canaries`, in any encoding, named
/// by its path below the home and the encoding, never by the value.
fn holders(home: &Path, canaries: &[(&str, &str)]) -> std::io::Result<Vec<String>> {
    Ok(held_under(home, canaries)?
        .into_iter()
        .map(|held| {
            let file = held.file.strip_prefix(home).unwrap_or(&held.file);
            format!("{} in {} ({})", held.canary, file.display(), held.encoding)
        })
        .collect())
}

/// Discards the Run `run_id`, checking the answer names it.
pub(super) fn discard(
    connection: &mut Connection,
    run_id: &str,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let id = connection.request(RUN_DISCARD.name, &json!({ "run_id": run_id }))?;
    let answer = connection.answer(id)?.message;
    checks.check(
        format!("`{}` answers the Run it discarded", RUN_DISCARD.name),
        answer["result"] == json!({ "run_id": run_id }),
    );
    Ok(())
}

/// Checks that no file of `home` holds any of `canaries`.
fn none_left(home: &Path, canaries: &[(&str, &str)], checks: &mut Checks) {
    let left = holders(home, canaries);
    checks.check(
        format!(
            "after the discard no file of the Goodfetch home holds a planted canary, in any encoding (found: {})",
            left.as_ref().map_or_else(
                |error| format!("the home could not be read: {error}"),
                |left| left.join(", ")
            )
        ),
        left.is_ok_and(|left| left.is_empty()),
    );
}

/// A token kept in the Run's Key-value store and a password as its status
/// line, found in the home while the Run keeps them, are found nowhere in
/// it once it is discarded; a start under its Run ID then begins a new Run
/// whose Key-value store holds nothing.
fn secret_values(
    connection: &mut Connection,
    home: &Path,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-reset-values";
    let Some(planted) = planted(site, &["endpoint_token", "typed_password"], checks) else {
        return Ok(());
    };
    let (token, password) = (&planted[0].value, &planted[1].value);
    let canaries = [("token", token.as_str()), ("password", password.as_str())];
    start_run(connection, site, run_id, &json!({}), checks)?;
    let set = connection.request(
        KEY_VALUE_STORE_SET.name,
        &json!({ "run_id": run_id, "key": "token", "value": token }),
    )?;
    let set = connection.answer(set)?.message;
    let noted = connection.request(
        RUN_SET_PROGRESS_MESSAGE.name,
        &json!({ "run_id": run_id, "message": password }),
    )?;
    let noted = connection.answer(noted)?.message;
    checks.check(
        "the Run keeps the token in its Key-value store and the password as its status line",
        set.get("result").is_some() && noted.get("result").is_some(),
    );
    let kept: BTreeSet<String> = held_under(home, &canaries)
        .unwrap_or_default()
        .into_iter()
        .map(|held| held.canary)
        .collect();
    checks.check(
        "while the Run keeps them, the search finds both in the Goodfetch home",
        kept.len() == canaries.len(),
    );
    finish_run(connection, run_id)?;
    discard(connection, run_id, checks)?;
    none_left(home, &canaries, checks);
    let started = connection.request(RUN_START.name, &json!({ "run_id": run_id }))?;
    let started = connection.answer(started)?.message;
    checks.check(
        "a start under the discarded Run's ID begins a new Run",
        started["result"]["started_as"] == json!("new"),
    );
    let got = connection.request(
        KEY_VALUE_STORE_GET.name,
        &json!({ "run_id": run_id, "key": "token" }),
    )?;
    let got = connection.answer(got)?.message;
    checks.check(
        "the new Run's Key-value store holds none of the discarded Run's values",
        got["result"] == json!({ "value": null }),
    );
    finish_run(connection, run_id)?;
    discard(connection, run_id, checks)
}

/// The Session cookie a login set in a Run's browser, sent back to its host
/// while the Run lives, is found nowhere in the home once the Run is
/// discarded, and a later Run under the same Run ID is not logged in.
fn session_cookie(
    connection: &mut Connection,
    home: &Path,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-reset-cookie";
    let login = site.start_url(COOKIE_TASK).map(str::to_owned);
    let Some(planted) = planted(site, &["session_cookie"], checks) else {
        return Ok(());
    };
    let (Some(login), Some(account)) = (login, planted[0].account.clone()) else {
        checks.check(
            format!("the Proving ground serves the task `{COOKIE_TASK}` and its account"),
            false,
        );
        return Ok(());
    };
    let account_url = login.replace("/login", "/account");
    let canaries = [("session cookie", planted[0].value.as_str())];
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(());
    };
    log_in(connection, &json!(page.page_id), &login, &account, checks)?;
    finish_run(connection, run_id)?;
    discard(connection, run_id, checks)?;
    none_left(home, &canaries, checks);
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(later) = new_page(connection, run_id, checks)? {
        let went = connection.call(
            PAGE_GOTO.name,
            &json!({ "page_id": later.page_id, "url": account_url }),
        )?;
        checks.check(
            "a later Run under the same Run ID sends no cookie of it: the account page sends it to log in",
            sent_to_log_in(&went),
        );
    }
    finish_run(connection, run_id)?;
    discard(connection, run_id, checks)
}

/// How many Runs in a row the reused pool case logs in and discards: the
/// lab's pool, reused without reset, sent one Run's cookie to the next 30
/// times out of 30 (ADRs 0001 and 0015).
const POOL_ROUNDS: usize = 30;

/// The lab's leak as a regression fixture (ADR 0015): thirty Runs in a
/// row under one Run ID, on one Connection and so one engine's browsers,
/// each sent to log in by the account page before its own login sets a new
/// Session cookie, then discarded; none of the thirty cookies is then found
/// in the home.
fn reused_pool(
    connection: &mut Connection,
    home: &Path,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-reset-pool";
    let Some(login) = site.start_url(COOKIE_TASK).map(str::to_owned) else {
        checks.check(
            format!("the Proving ground serves the task `{COOKIE_TASK}`"),
            false,
        );
        return Ok(());
    };
    let account_url = login.replace("/login", "/account");
    let mut cookies = Vec::with_capacity(POOL_ROUNDS);
    let mut failed = Vec::new();
    for round in 1..=POOL_ROUNDS {
        let mut round_checks = Checks::default();
        let cookie = pool_round(
            connection,
            site,
            run_id,
            [&login, &account_url],
            &mut round_checks,
        )?;
        cookies.extend(cookie);
        failed.extend(
            round_checks
                .checks
                .into_iter()
                .filter(|check| !check.passed)
                .map(|check| format!("round {round}: {}", check.name)),
        );
    }
    checks.check(
        format!(
            "each of {POOL_ROUNDS} Runs in a row under one Run ID is sent to log in before its own login, which the account page then takes (failed: {})",
            failed.join("; ")
        ),
        failed.is_empty() && cookies.len() == POOL_ROUNDS,
    );
    let canaries: Vec<(&str, &str)> = cookies
        .iter()
        .map(|cookie| ("session cookie", cookie.as_str()))
        .collect();
    none_left(home, &canaries, checks);
    Ok(())
}

/// One round of the reused pool case: a Run under `run_id` whose account
/// page, at the second of `urls`, sends it to log in, which logs in at the
/// first with a newly planted Session cookie canary, ends and is
/// discarded. Its canary's value, once planted.
fn pool_round(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    [login, account_url]: [&str; 2],
    checks: &mut Checks,
) -> Result<Option<String>, Gone> {
    let Some(planted) = planted(site, &["session_cookie"], checks) else {
        return Ok(None);
    };
    let Some(account) = planted[0].account.clone() else {
        checks.check(
            "the Proving ground gives the Session cookie canary's account",
            false,
        );
        return Ok(None);
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some(page) = new_page(connection, run_id, checks)? {
        let page = json!(page.page_id);
        let went = connection.call(
            PAGE_GOTO.name,
            &json!({ "page_id": page, "url": account_url }),
        )?;
        checks.check(
            "no earlier Run's cookie reaches it: the account page sends it to log in",
            sent_to_log_in(&went),
        );
        log_in(connection, &page, login, &account, checks)?;
    }
    finish_run(connection, run_id)?;
    discard(connection, run_id, checks)?;
    Ok(Some(planted[0].value.clone()))
}

/// Logs in at `login` on the page `page` with `account`, checking that the
/// login sets the Session cookie the account page takes.
fn log_in(
    connection: &mut Connection,
    page: &Value,
    login: &str,
    account: &CanaryAccount,
    checks: &mut Checks,
) -> Result<(), Gone> {
    connection.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": login }))?;
    for (selector, value) in [
        ("input[name=username]", account.username.as_str()),
        ("input[name=password]", account.password.as_str()),
    ] {
        connection.call(
            LOCATOR_FILL.name,
            &json!({ "locator": { "page_id": page, "selector": selector }, "value": value }),
        )?;
    }
    let waiting = connection.request(
        PAGE_WAIT_FOR_REQUEST_FINISHED.name,
        &json!({ "page_id": page, "url": { "kind": "glob", "pattern": "**/account" } }),
    )?;
    let clicked = connection.request(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page, "selector": "button[type=submit]" } }),
    )?;
    let (_, finished) = connection.both(clicked, waiting)?;
    checks.check(
        "the login sets the Session cookie, which the account page takes",
        finished["result"]["response"]["status"] == json!(200),
    );
    Ok(())
}

/// Whether `went`, the answer to a visit of the account page, is the lost
/// login the built-in detector judges the login page it sends to (ADR
/// 0006).
fn sent_to_log_in(went: &Value) -> bool {
    went["error"]["data"]["code"] == json!(LOGGED_OUT.name)
        && went["error"]["data"]["url"]
            .as_str()
            .is_some_and(|url| url.ends_with("/login"))
}
