//! The page suite's Session cases (ADRs 0021, 0078, 0092 and 0102, ticket
//! #697): a Run's own Sessions, each its own browser context whose cookies
//! and `localStorage` no other Session of the Run shares, as the Proving
//! ground's record of what each request sent shows; `session.close`, after
//! which a call on the Session raises `closed` and a second close changes
//! nothing, also once its Run ended; `session.source`, refused in a
//! Session's own settings and the default of the Sessions beneath a Run
//! that sets it; and new work on a finished Run, which raises `run_ended`.

use goodfetch_protocol::connection::CLOSED;
use goodfetch_protocol::page::session::{RUN_NEW_SESSION, SESSION_CLOSE, SESSION_NEW_PAGE};
use goodfetch_protocol::page::{PAGE_GOTO, PageOpened, RUN_NEW_PAGE, SessionId};
use goodfetch_protocol::run::{RUN_ENDED, RUN_START};
use goodfetch_protocol::settings::{CAPABILITY_UNAVAILABLE, SETTING_NOT_APPLICABLE};
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;

/// One Session case of the page suite.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Isolated,
    Closed,
    Source,
    Ended,
    ClosedWithItsRun,
    /// One of a Session's proxy cases.
    Proxy(super::proxies::Case),
    /// One of a Session's proxy limit cases.
    ProxyLimit(super::proxy_limits::Case),
    /// One of a Session's Page check cases.
    PageCheck(super::page_check::Case),
    /// One of a Session's Block cause cases.
    Blocked(super::blocks::Case),
}

impl Case {
    /// The case a suite's case ID names.
    pub(crate) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "two-sessions-of-a-run-keep-their-own-cookies-and-storage" => Self::Isolated,
            "a-closed-session-raises-closed-and-closing-it-again-changes-nothing" => Self::Closed,
            "session-source-in-a-sessions-own-settings-is-not-applicable-and-the-runs-is-the-default-beneath" => {
                Self::Source
            }
            "new-work-on-a-finished-run-raises-run-ended" => Self::Ended,
            "closing-a-session-whose-run-ended-changes-nothing" => Self::ClosedWithItsRun,
            _ => {
                return super::proxies::Case::from_id(id)
                    .map(Self::Proxy)
                    .or_else(|| super::proxy_limits::Case::from_id(id).map(Self::ProxyLimit))
                    .or_else(|| super::page_check::Case::from_id(id).map(Self::PageCheck))
                    .or_else(|| super::blocks::Case::from_id(id).map(Self::Blocked));
            }
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
            Self::Isolated => isolated(connection, site, checks),
            Self::Closed => closed(connection, site, checks),
            Self::Source => source(connection, site, checks),
            Self::Ended => ended(connection, checks),
            Self::ClosedWithItsRun => closed_with_its_run(connection, site, checks),
            Self::Proxy(case) => case.run(connection, target, site, checks),
            Self::ProxyLimit(case) => case.run(connection, target, site, checks),
            Self::PageCheck(case) => case.run(connection, target, site, checks),
            Self::Blocked(case) => case.run(connection, target, site, checks),
        }
    }
}

/// Calls `method` with `params` and answers the engine's whole answer,
/// waiting as long as a browser's launch may take.
fn call(connection: &mut Connection, method: &str, params: &Value) -> Result<Value, Gone> {
    let id = connection.request(method, params)?;
    Ok(connection.browser_answer(id)?.message)
}

/// The Error code a failure answer names.
fn code_of(answer: &Value) -> &Value {
    &answer["error"]["data"]["code"]
}

/// The Session a `run.new_session` answer names.
fn session_of(answer: &Value) -> Option<SessionId> {
    serde_json::from_value(answer["result"]["session_id"].clone()).ok()
}

/// The page a `run.new_page` or `session.new_page` answer names.
fn page_of(answer: &Value) -> Option<PageOpened> {
    serde_json::from_value(answer["result"].clone()).ok()
}

/// Opens a Session of the Run `run_id` and a page in it.
fn session_and_page(
    connection: &mut Connection,
    run_id: &str,
    checks: &mut Checks,
) -> Result<Option<(SessionId, PageOpened)>, Gone> {
    let opened = call(
        connection,
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id }),
    )?;
    let Some(session) = session_of(&opened) else {
        checks.check(
            format!("`{}` answers the Session it opened", RUN_NEW_SESSION.name),
            false,
        );
        return Ok(None);
    };
    let paged = call(
        connection,
        SESSION_NEW_PAGE.name,
        &json!({ "session_id": session }),
    )?;
    let page = page_of(&paged);
    checks.check(
        format!("`{}` answers a page of the Session", SESSION_NEW_PAGE.name),
        page.as_ref().is_some_and(|page| page.session_id == session),
    );
    Ok(page.map(|page| (session, page)))
}

/// Loads the storage page with `mark` on `page`, and answers its status.
fn store(
    connection: &mut Connection,
    site: &Site,
    page: &PageOpened,
    mark: &str,
) -> Result<Value, Gone> {
    let url = site
        .named_url("scope.test", &format!("/storage?mark={mark}"))
        .unwrap_or_default();
    let answer = call(
        connection,
        PAGE_GOTO.name,
        &json!({ "page_id": page.page_id, "url": url }),
    )?;
    Ok(answer["result"]["response"]["status"].clone())
}

/// What the Proving ground received for each request to `scope.test`
/// whose target starts with `prefix`, in arrival order: its target and
/// the cookies it sent.
fn storage_requests(site: &Site, prefix: &str) -> Vec<(Value, Value)> {
    site.proving_ground
        .received_requests()
        .unwrap_or_default()
        .of_the_case()
        .filter(|request| {
            request["host"] == json!("scope.test")
                && request["target"]
                    .as_str()
                    .is_some_and(|target| target.starts_with(prefix))
        })
        .map(|request| (request["target"].clone(), request["cookies"].clone()))
        .collect()
}

/// Two Sessions of one Run keep their own cookies and `localStorage`: each
/// page's second load sends only its own Session's cookie and finds only
/// its own mark.
fn isolated(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-sessions-isolated";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let first = session_and_page(connection, run_id, checks)?;
    let second = session_and_page(connection, run_id, checks)?;
    if let (Some((first, first_page)), Some((second, second_page))) = (first, second) {
        checks.check("the two Sessions are two", first != second);
        let statuses = [
            store(connection, site, &first_page, "first")?,
            store(connection, site, &second_page, "second")?,
            store(connection, site, &first_page, "first")?,
            store(connection, site, &second_page, "second")?,
        ];
        checks.check(
            "each load of the storage page answers 200",
            statuses.iter().all(|status| status == &json!(200)),
        );
        checks.check(
            "each Session's second load sends only its own cookie, and the first loads none",
            storage_requests(site, "/storage?")
                == vec![
                    (json!("/storage?mark=first"), json!([])),
                    (json!("/storage?mark=second"), json!([])),
                    (json!("/storage?mark=first"), json!([["mark", "first"]])),
                    (json!("/storage?mark=second"), json!([["mark", "second"]])),
                ],
        );
        checks.check(
            "each Session's second load finds only its own mark in `localStorage`",
            storage_requests(site, "/storage/found")
                .into_iter()
                .map(|(target, _)| target)
                .collect::<Vec<_>>()
                == vec![
                    json!("/storage/found?mark=none"),
                    json!("/storage/found?mark=none"),
                    json!("/storage/found?mark=first"),
                    json!("/storage/found?mark=second"),
                ],
        );
    }
    finish_run(connection, run_id)
}

/// A closed Session's page and the Session itself raise `closed`, and
/// closing it again answers as the first close did; a closed default
/// Session makes `run.new_page` raise `closed`.
fn closed(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-sessions-closed";
    start_run(connection, site, run_id, &json!({}), checks)?;
    if let Some((session, page)) = session_and_page(connection, run_id, checks)? {
        let first = call(
            connection,
            SESSION_CLOSE.name,
            &json!({ "session_id": session }),
        )?;
        let second = call(
            connection,
            SESSION_CLOSE.name,
            &json!({ "session_id": session }),
        )?;
        checks.check(
            "closing the Session, then closing it again, both answer",
            first["result"] == json!({}) && second["result"] == json!({}),
        );
        let url = site.named_url("scope.test", "/about").unwrap_or_default();
        let navigated = call(
            connection,
            PAGE_GOTO.name,
            &json!({ "page_id": page.page_id, "url": url }),
        )?;
        let opened = call(
            connection,
            SESSION_NEW_PAGE.name,
            &json!({ "session_id": session }),
        )?;
        checks.check(
            format!(
                "a page of the closed Session and the Session itself raise `{}`",
                CLOSED.name
            ),
            code_of(&navigated) == &json!(CLOSED.name) && code_of(&opened) == &json!(CLOSED.name),
        );
    }
    let default = call(connection, RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
    if let Some(default) = page_of(&default) {
        let _ = call(
            connection,
            SESSION_CLOSE.name,
            &json!({ "session_id": default.session_id }),
        )?;
        let again = call(connection, RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
        checks.check(
            format!(
                "`{}` raises `{}` once the default Session is closed",
                RUN_NEW_PAGE.name, CLOSED.name
            ),
            code_of(&again) == &json!(CLOSED.name),
        );
    }
    finish_run(connection, run_id)
}

/// `session.source` in an opening Session's own settings is refused with
/// `setting_not_applicable`; a Run's `session.source` is the default the
/// Run's Sessions open from, the default Session's included, which this
/// engine meets with `capability_unavailable` for a kind it does not read;
/// and the call's own `source: null` opens a fresh Session.
fn source(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-sessions-source";
    let saved = json!({ "kind": "saved_identity", "name": "portal" });
    start_run(
        connection,
        site,
        run_id,
        &json!({ "session": { "source": saved } }),
        checks,
    )?;
    let in_settings = call(
        connection,
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "settings": { "session": { "source": saved } } }),
    )?;
    checks.check(
        format!(
            "`session.source` in the Session's own settings raises `{}` naming it",
            SETTING_NOT_APPLICABLE.name
        ),
        code_of(&in_settings) == &json!(SETTING_NOT_APPLICABLE.name)
            && in_settings["error"]["data"]["key"] == json!("session.source"),
    );
    let inherited = call(
        connection,
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id }),
    )?;
    let default = call(connection, RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
    checks.check(
        format!(
            "a Session and the default Session open from the Run's `session.source`, `{}` here naming its kind",
            CAPABILITY_UNAVAILABLE.name
        ),
        [&inherited, &default].iter().all(|answer| {
            code_of(answer) == &json!(CAPABILITY_UNAVAILABLE.name)
                && answer["error"]["data"]["kind"] == json!("saved_identity")
                && answer["error"]["data"]["layer"] == json!("run")
        }),
    );
    let fresh = call(
        connection,
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id, "source": null }),
    )?;
    checks.check(
        "`source: null` opens a fresh Session beneath the Run's `session.source`",
        session_of(&fresh).is_some(),
    );
    finish_run(connection, run_id)
}

/// A finished Run takes no new Session and no new page: both raise
/// `run_ended`.
fn ended(connection: &mut Connection, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-sessions-ended";
    let started = call(connection, RUN_START.name, &json!({ "run_id": run_id }))?;
    checks.check("the Run starts", started.get("result").is_some());
    finish_run(connection, run_id)?;
    let session = call(
        connection,
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id }),
    )?;
    let page = call(connection, RUN_NEW_PAGE.name, &json!({ "run_id": run_id }))?;
    checks.check(
        format!(
            "`{}` and `{}` on the finished Run raise `{}`",
            RUN_NEW_SESSION.name, RUN_NEW_PAGE.name, RUN_ENDED.name
        ),
        code_of(&session) == &json!(RUN_ENDED.name) && code_of(&page) == &json!(RUN_ENDED.name),
    );
    Ok(())
}

/// A Session whose Run finished ended with it: closing it then, twice,
/// changes nothing, as `close()` is idempotent (ADR 0078).
fn closed_with_its_run(
    connection: &mut Connection,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-sessions-closed-with-its-run";
    start_run(connection, site, run_id, &json!({}), checks)?;
    let opened = call(
        connection,
        RUN_NEW_SESSION.name,
        &json!({ "run_id": run_id }),
    )?;
    let Some(session) = session_of(&opened) else {
        checks.check(format!("`{}` opens a Session", RUN_NEW_SESSION.name), false);
        return finish_run(connection, run_id);
    };
    finish_run(connection, run_id)?;
    let closed = call(
        connection,
        SESSION_CLOSE.name,
        &json!({ "session_id": session }),
    )?;
    let again = call(
        connection,
        SESSION_CLOSE.name,
        &json!({ "session_id": session }),
    )?;
    checks.check(
        format!(
            "`{}` of a Session whose Run finished answers, and again",
            SESSION_CLOSE.name
        ),
        closed.get("result").is_some() && again.get("result").is_some(),
    );
    Ok(())
}
