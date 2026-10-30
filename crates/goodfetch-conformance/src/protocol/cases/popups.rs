//! The page suite's popup cases (ADR 0085, ticket #719; pitfall B15), on
//! the Proving ground's popup page, whose button opens a popup with
//! `window.open`: a listener armed for `popup` before the click catches the
//! popup, a page of the opener's Session that the Session's `page` event
//! announces too, while one armed after it opened does not, and
//! `session.pages` still lists it; the popup is driven as any page, and
//! `page.close` closes it, emitting `close`, a second close changing
//! nothing.

use std::time::Duration;

use goodfetch_protocol::connection::CANCEL;
use goodfetch_protocol::events::{CREDIT, EVENTS_SUBSCRIBE};
use goodfetch_protocol::page::events::PAGE_ON;
use goodfetch_protocol::page::session_pages::{PAGE_CLOSE, SESSION_ON, SESSION_PAGES};
use goodfetch_protocol::page::{LOCATOR_CLICK, PAGE_GOTO};
use serde_json::{Value, json};

use super::exchange::Exchange;
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};

/// How long a listener armed late is watched for a popup it must not get.
const QUIET: Duration = Duration::from_secs(2);

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    WaiterArmedBefore,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "a-popup-listener-armed-before-the-click-catches-it-one-armed-after-does-not-and-session-pages-lists-it" => {
                Self::WaiterArmedBefore
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::WaiterArmedBefore => armed_before(connection, site, checks),
        }
    }
}

/// A listener armed for `popup` before the click that opens one catches
/// it; one armed after it opened does not, and `session.pages` lists it.
fn armed_before(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-popups";
    checks.check(
        "the Proving ground serves the access family",
        site.access_origin.is_some(),
    );
    let (Some(origin), Some(catalog)) = (site.access_origin.clone(), site.page_url.clone()) else {
        return Ok(());
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(opener) = new_page(connection, run_id, checks)? else {
        return Ok(());
    };
    let (page, session) = (json!(opener.page_id), json!(opener.session_id));
    let subscription = connection.request(EVENTS_SUBSCRIBE.name, &json!({}))?;
    connection.notify(CREDIT, &json!({ "id": subscription, "credits": 10_000 }))?;
    let mut exchange = Exchange::new(connection).with_events(subscription);
    let went = exchange.call(
        PAGE_GOTO.name,
        &json!({ "page_id": page, "url": format!("{origin}/popup") }),
    )?;
    checks.check(
        "the popup page loads",
        went["result"]["response"]["status"] == json!(200),
    );
    let listening = exchange.listen(PAGE_ON.name, &json!({ "page_id": page, "event": "popup" }))?;
    let session_listening = exchange.connection.request(
        SESSION_ON.name,
        &json!({ "session_id": session, "event": "page" }),
    )?;
    let clicked = exchange.call(
        LOCATOR_CLICK.name,
        &json!({ "locator": { "page_id": page, "selector": "#open" } }),
    )?;
    let popup = exchange.until("page.popup", |data| data["page_id"] == page)?;
    let popup_page = popup["popup"]["page_id"].clone();
    let announced = exchange.until("session.page", |data| data["page_id"] == popup_page)?;
    checks.check(
        "the click answers, and the listener armed before it gets the popup",
        clicked.get("result").is_some() && popup_page.is_string(),
    );
    checks.check(
        "the popup is a page of its opener's Session, which the Session's `page` event announces",
        popup["popup"]["session_id"] == session && announced["session_id"] == session,
    );

    // A listener armed once the popup opened gets none.
    exchange
        .connection
        .notify(CANCEL, &json!({ "id": listening }))?;
    exchange.answer(listening)?;
    let popups_before = exchange.count("page.popup");
    let late = exchange.listen(PAGE_ON.name, &json!({ "page_id": page, "event": "popup" }))?;
    exchange.idle(QUIET)?;
    checks.check(
        "a listener armed after the popup opened gets no popup",
        exchange.count("page.popup") == popups_before,
    );
    let listed = exchange.call(SESSION_PAGES.name, &json!({ "session_id": session }))?;
    checks.check(
        "`session.pages` lists the opener and the popup, in the order they opened",
        listed["result"]["pages"] == json!([page, popup_page]),
    );

    let close_listening = drive_and_close(
        &mut exchange,
        [&page, &popup_page, &session],
        &origin,
        &catalog,
        checks,
    )?;
    let _ = (late, session_listening, close_listening);
    exchange
        .connection
        .notify(CANCEL, &json!({ "id": subscription }))?;
    finish_run(exchange.connection, run_id)
}

/// Drives the popup of `page` in `session` as any page, then closes it
/// twice and calls it once more; answers the id of its `close` listener.
fn drive_and_close(
    exchange: &mut Exchange<'_>,
    [page, popup_page, session]: [&Value; 3],
    origin: &str,
    catalog: &str,
    checks: &mut Checks,
) -> Result<i64, Gone> {
    let popup_went = exchange.call(
        PAGE_GOTO.name,
        &json!({ "page_id": popup_page, "url": catalog }),
    )?;
    checks.check(
        "the popup navigates as any page",
        popup_went["result"]["response"]["status"] == json!(200),
    );
    let close_listening = exchange.listen(
        PAGE_ON.name,
        &json!({ "page_id": popup_page, "event": "close" }),
    )?;
    let closed = exchange.call(PAGE_CLOSE.name, &json!({ "page_id": popup_page }))?;
    exchange.until("page.close", |data| data["page_id"] == *popup_page)?;
    let closed_again = exchange.call(PAGE_CLOSE.name, &json!({ "page_id": popup_page }))?;
    let after = exchange.call(SESSION_PAGES.name, &json!({ "session_id": session }))?;
    let later = exchange.call(
        PAGE_GOTO.name,
        &json!({ "page_id": popup_page, "url": format!("{origin}/popup") }),
    )?;
    checks.check(
        "`page.close` closes the popup, which emits `close`, and closing it again changes nothing",
        closed.get("result").is_some() && closed_again.get("result").is_some(),
    );
    checks.check(
        "the closed popup leaves `session.pages`, and a later call on it raises `closed`",
        after["result"]["pages"] == json!([page])
            && later["error"]["data"]["code"] == json!("closed"),
    );
    Ok(close_listening)
}
