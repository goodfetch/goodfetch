//! The page suite's dialog cases (ADR 0085, ticket #719; pitfall B16), on
//! the Proving ground's dialogs page, whose buttons open an `alert`, a
//! `confirm` and a `prompt` and report what each returned, and arm a
//! `beforeunload` handler that asks to stay. With no handler, each dialog
//! is dismissed, as the page's reports show, and a `beforeunload` dialog is
//! accepted, so the navigation that raised it completes. Dialog handlers
//! answer under the routes' rules: the latest first, one that answers
//! nothing falling through, the first answer winning and a second raising
//! `dialog_already_handled`, and one that throws giving the dialog its
//! default and failing the click the dialog blocked with
//! `page_handler_failed`.

use goodfetch_protocol::connection::{CALL_CANCELLED, CANCEL};
use goodfetch_protocol::page::dialog::{
    DIALOG_ACCEPT, DIALOG_ALREADY_HANDLED, DIALOG_DISMISS, DIALOG_HANDLE, PAGE_ON_DIALOG,
};
use goodfetch_protocol::page::route::PAGE_HANDLER_FAILED;
use goodfetch_protocol::page::{LOCATOR_CLICK, PAGE_GOTO};
use serde_json::{Value, json};

use super::exchange::Exchange;
use super::launch::{reports, reports_once};
use super::{Checks, Site, finish_run, new_page, start_run};
use crate::protocol::connection::{Connection, Gone};

/// The dialogs page's path on the access family.
const DIALOGS: &str = "/dialogs";

/// What the dialogs page reports for each of its buttons.
const BUTTONS: [&str; 4] = ["alert", "confirm", "prompt", "guard-leaving"];

/// A case of these lines, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    Defaults,
    Handlers,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "an-alert-a-confirm-and-a-prompt-with-no-handler-are-dismissed-and-a-beforeunload-is-accepted" => {
                Self::Defaults
            }
            "dialog-handlers-answer-later-first-one-that-answers-nothing-falls-through-and-one-that-throws-gives-the-default" => {
                Self::Handlers
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
            Self::Defaults => defaults(connection, site, checks),
            Self::Handlers => handlers(connection, site, checks),
        }
    }
}

/// The dialogs page's reports of its buttons so far, `(button, observed)`
/// in order, every page's together; none when the observations cannot be
/// read.
fn button_reports(site: &Site) -> Option<Vec<(String, String)>> {
    let observations = site.proving_ground.observations().ok()?;
    Some(
        reports(&observations, DIALOGS)
            .into_iter()
            .filter(|(check, _)| BUTTONS.contains(&check.as_str()))
            .collect(),
    )
}

/// The `count` reports of the dialogs page's buttons after the first
/// `before`, once they arrived.
fn reports_after(site: &Site, before: usize, count: usize) -> Option<Vec<(String, String)>> {
    let seen = reports_once(site, DIALOGS, |seen| {
        seen.iter()
            .filter(|(check, _)| BUTTONS.contains(&check.as_str()))
            .count()
            >= before + count
    })?;
    Some(
        seen.into_iter()
            .filter(|(check, _)| BUTTONS.contains(&check.as_str()))
            .skip(before)
            .collect(),
    )
}

/// The dialogs page's reports in the order of the clicks that made them.
/// Each report is a fetch of its own, which the browser may start only once
/// the page runs again, after the next click's dialog closed, so two
/// reports in flight at once reach the Proving ground in either order
/// (fix-queue 89): each click waits for its report before the next action.
struct Reported<'site> {
    site: &'site Site,
    /// The reports of the dialogs page's buttons already seen, every
    /// page's together.
    seen: usize,
    reports: Vec<(String, String)>,
}

impl<'site> Reported<'site> {
    /// Reports from now on.
    fn from_now(site: &'site Site) -> Self {
        Self {
            site,
            seen: button_reports(site).map_or(0, |seen| seen.len()),
            reports: Vec::new(),
        }
    }

    /// Waits for the report of the click just made, and keeps it.
    fn wait(&mut self) {
        if let Some(report) =
            reports_after(self.site, self.seen, 1).and_then(|after| after.into_iter().next())
        {
            self.seen += 1;
            self.reports.push(report);
        }
    }
}

/// `(check, observed)` pairs, owned.
fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
    expected
        .iter()
        .map(|(check, observed)| ((*check).to_owned(), (*observed).to_owned()))
        .collect()
}

/// The dialogs page of `site`, opened on a new page of the Run `run_id`:
/// the page's ID.
fn on_the_dialogs_page(
    connection: &mut Connection,
    site: &Site,
    run_id: &str,
    checks: &mut Checks,
) -> Result<Option<Value>, Gone> {
    checks.check(
        "the Proving ground serves the access family",
        site.access_origin.is_some() && site.page_url.is_some(),
    );
    let Some(origin) = site.access_origin.clone() else {
        return Ok(None);
    };
    start_run(connection, site, run_id, &json!({}), checks)?;
    let Some(page) = new_page(connection, run_id, checks)? else {
        return Ok(None);
    };
    let page_id = json!(page.page_id);
    let id = connection.request(
        PAGE_GOTO.name,
        &json!({ "page_id": page_id, "url": format!("{origin}{DIALOGS}") }),
    )?;
    let went = connection.browser_answer(id)?.message;
    checks.check(
        "the dialogs page loads",
        went["result"]["response"]["status"] == json!(200),
    );
    Ok(Some(page_id))
}

/// An `alert`, a `confirm` and a `prompt` with no handler are dismissed,
/// and a `beforeunload` dialog is accepted: the navigation completes.
fn defaults(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-dialog-defaults";
    let reported = Reported::from_now(site);
    let Some(page) = on_the_dialogs_page(connection, site, run_id, checks)? else {
        return Ok(());
    };
    let mut handlers = Handlers::new(connection, reported);
    let mut clicked = Vec::new();
    for selector in ["#alert", "#confirm", "#prompt", "#guard-leaving"] {
        clicked.push(handlers.click(&page, selector, &[])?);
    }
    checks.check(
        "each click that opens a dialog answers",
        clicked.iter().all(|answer| answer.get("result").is_some()),
    );
    checks.check(
        "the page reports an alert closed, a confirm and a prompt dismissed, and its guard armed",
        handlers.reported.reports
            == pairs(&[
                ("alert", "closed"),
                ("confirm", "false"),
                ("prompt", "null"),
                ("guard-leaving", "guarded"),
            ]),
    );
    let url = site.page_url.clone().unwrap_or_default();
    let left = handlers.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    checks.check(
        "a navigation away from a page whose `beforeunload` asks to stay completes",
        left["result"]["response"]["status"] == json!(200),
    );
    finish_run(handlers.exchange.connection, run_id)
}

/// What a scripted dialog handler does with a dialog it is asked about.
#[derive(Clone, Copy, Debug)]
enum Does {
    /// Replies without answering: the dialog falls through.
    Nothing,
    /// Accepts it, a prompt with this text, then dismisses it too.
    Accept(&'static str),
    /// Fails, as a handler that throws does.
    Throw,
}

/// The Connection as the dialog cases drive it, through the shared reader,
/// what the handlers were asked and what the page reported.
struct Handlers<'connection, 'site> {
    exchange: Exchange<'connection>,
    reported: Reported<'site>,
    /// The call of each handler asked so far, in order, with the dialog it
    /// was asked about.
    asked: Vec<(Value, Value)>,
    /// The answers to each accepting handler's accept and its dismiss after
    /// it.
    answers: Vec<(Value, Value)>,
}

impl<'connection, 'site> Handlers<'connection, 'site> {
    fn new(connection: &'connection mut Connection, reported: Reported<'site>) -> Self {
        Self {
            exchange: Exchange::new(connection),
            reported,
            asked: Vec::new(),
            answers: Vec::new(),
        }
    }

    /// The answer to the request `id`, keeping every other message.
    fn answer(&mut self, id: i64) -> Result<Value, Gone> {
        self.exchange.answer(id)
    }

    /// Cancels the call `id`.
    fn cancel(&mut self, id: i64) -> Result<(), Gone> {
        self.exchange
            .connection
            .notify(CANCEL, &json!({ "id": id }))
    }

    /// Arms a dialog handler on `page`, answering its call's id.
    fn arm(&mut self, page: &Value) -> Result<i64, Gone> {
        self.exchange
            .connection
            .request(PAGE_ON_DIALOG.name, &json!({ "page_id": page }))
    }

    /// Calls `method` with `params` while each handler `script` names does
    /// what it says with the dialogs it is asked about, any other doing
    /// nothing; answers the call's answer, keeping every other message,
    /// such as a handler's subscription ending meanwhile.
    fn call_answering(
        &mut self,
        method: &str,
        params: &Value,
        script: &[(i64, Does)],
    ) -> Result<Value, Gone> {
        let call = self.exchange.request(method, params)?;
        let mut others = Vec::new();
        let answer = loop {
            let message = match self.exchange.next() {
                Ok(message) => message,
                Err(gone) => {
                    self.exchange.keep_all(others);
                    return Err(gone);
                }
            };
            if message["method"] == json!(DIALOG_HANDLE.name) {
                self.handle(&message, script)?;
            } else if message.get("method").is_none() && message["id"] == json!(call) {
                break message;
            } else {
                others.push(message);
            }
        };
        self.exchange.keep_all(others);
        Ok(answer)
    }

    fn call(&mut self, method: &str, params: &Value) -> Result<Value, Gone> {
        self.call_answering(method, params, &[])
    }

    /// Clicks `selector` on `page`, one of the dialogs page's buttons, as
    /// [`Self::call_answering`] calls, then waits for the page's report of
    /// it.
    fn click(
        &mut self,
        page: &Value,
        selector: &str,
        script: &[(i64, Does)],
    ) -> Result<Value, Gone> {
        let answer = self.call_answering(
            LOCATOR_CLICK.name,
            &json!({ "locator": { "page_id": page, "selector": selector } }),
            script,
        )?;
        self.reported.wait();
        Ok(answer)
    }

    /// Does what `script` says with the dialog `asked` names.
    fn handle(&mut self, asked: &Value, script: &[(i64, Does)]) -> Result<(), Gone> {
        let params = &asked["params"];
        self.asked
            .push((params["id"].clone(), params["dialog"].clone()));
        let does = script
            .iter()
            .find(|(call, _)| params["id"] == json!(call))
            .map_or(Does::Nothing, |(_, does)| *does);
        let dialog_id = params["dialog_id"].clone();
        match does {
            Does::Nothing => self.exchange.connection.reply(&asked["id"], &json!({})),
            Does::Accept(text) => {
                let accepting = self.exchange.connection.request(
                    DIALOG_ACCEPT.name,
                    &json!({ "dialog_id": dialog_id, "prompt_text": text }),
                )?;
                let accepted = self.answer(accepting)?;
                let dismissing = self
                    .exchange
                    .connection
                    .request(DIALOG_DISMISS.name, &json!({ "dialog_id": dialog_id }))?;
                let dismissed = self.answer(dismissing)?;
                self.answers.push((accepted, dismissed));
                self.exchange.connection.reply(&asked["id"], &json!({}))
            }
            Does::Throw => {
                self.exchange
                    .connection
                    .fail(&asked["id"], "handler_failed", "the handler threw")
            }
        }
    }

    /// The calls of the handlers asked since the last time, in order, with
    /// their dialogs.
    fn take_asked(&mut self) -> Vec<(Value, Value)> {
        std::mem::take(&mut self.asked)
    }
}

/// The calls of `asked`.
fn calls(asked: &[(Value, Value)]) -> Vec<Value> {
    asked.iter().map(|(call, _)| call.clone()).collect()
}

/// Dialog handlers are asked the latest first: one that answers nothing
/// falls through to the one before, the first answer wins and a second
/// raises `dialog_already_handled`, a `beforeunload` dialog no handler
/// answers is accepted, and one that throws gives the dialog its default
/// and fails the click the dialog blocked with `page_handler_failed`.
fn handlers(connection: &mut Connection, site: &Site, checks: &mut Checks) -> Result<(), Gone> {
    let run_id = "conformance-dialog-handlers";
    let reported = Reported::from_now(site);
    let Some(page) = on_the_dialogs_page(connection, site, run_id, checks)? else {
        return Ok(());
    };
    let mut handlers = Handlers::new(connection, reported);
    let earlier = handlers.arm(&page)?;
    let later = handlers.arm(&page)?;

    // The later handler only looks; the earlier one accepts.
    let confirmed = handlers.click(&page, "#confirm", &[(earlier, Does::Accept("unused"))])?;
    let about_confirm = handlers.take_asked();
    // The later one accepts the prompt: the earlier one is not asked.
    let prompted = handlers.click(&page, "#prompt", &[(later, Does::Accept("Ada"))])?;
    let about_prompt = handlers.take_asked();
    // A `beforeunload` dialog neither answers is accepted.
    let guarded = handlers.click(&page, "#guard-leaving", &[])?;
    let url = site.page_url.clone().unwrap_or_default();
    let left = handlers.call(PAGE_GOTO.name, &json!({ "page_id": page, "url": url }))?;
    let about_leaving = handlers.take_asked();
    let back = handlers.call(
        PAGE_GOTO.name,
        &json!({
            "page_id": page,
            "url": format!("{}{DIALOGS}", site.access_origin.clone().unwrap_or_default()),
        }),
    )?;
    // With the later one cancelled, the earlier one throws.
    handlers.cancel(later)?;
    let removed = handlers.answer(later)?;
    let alerted = handlers.click(&page, "#alert", &[(earlier, Does::Throw)])?;
    let about_alert = handlers.take_asked();
    let ended = handlers.answer(earlier)?;
    // With none left, the confirm gets its default.
    let unhandled = handlers.click(&page, "#confirm", &[])?;
    let at_last = handlers.take_asked();
    let answers = std::mem::take(&mut handlers.answers);

    checks.check(
        "the later handler is asked first, and one that answers nothing falls through to the earlier",
        calls(&about_confirm) == vec![json!(later), json!(earlier)],
    );
    checks.check(
        "a handler sees the dialog's type, message and default value",
        about_confirm.first().map(|(_, dialog)| dialog)
            == Some(&json!({ "type": "confirm", "message": "Confirm this?", "default_value": "" }))
            && about_prompt.first().map(|(_, dialog)| dialog)
                == Some(&json!({ "type": "prompt", "message": "Your name?", "default_value": "a default" })),
    );
    checks.check(
        "a handler that answers is the last one asked",
        calls(&about_prompt) == vec![json!(later)],
    );
    checks.check(
        "an answer answers, and a second one raises `dialog_already_handled`",
        answers.len() == 2
            && answers.iter().all(|(accepted, dismissed)| {
                accepted.get("result").is_some()
                    && dismissed["error"]["data"]["code"] == json!(DIALOG_ALREADY_HANDLED.name)
            }),
    );
    checks.check(
        "a `beforeunload` dialog reaches both handlers, and neither answering it accepts it",
        about_leaving
            .iter()
            .map(|(call, dialog)| (call.clone(), dialog["type"].clone()))
            .collect::<Vec<_>>()
            == vec![
                (json!(later), json!("beforeunload")),
                (json!(earlier), json!("beforeunload")),
            ]
            && left["result"]["response"]["status"] == json!(200),
    );
    checks.check(
        "cancelling a handler ends its call with `call_cancelled`, and it is asked no more",
        removed["error"]["data"]["code"] == json!(CALL_CANCELLED.name)
            && calls(&about_alert) == vec![json!(earlier)],
    );
    checks.check(
        "every other click and navigation answers",
        [&confirmed, &prompted, &guarded, &back, &unhandled]
            .iter()
            .all(|answer| answer.get("result").is_some()),
    );
    checks.check(
        "a handler that throws fails the click its dialog blocked with `page_handler_failed`",
        alerted["error"]["data"]["code"] == json!(PAGE_HANDLER_FAILED.name)
            && alerted["error"]["data"]["handler"] == json!("dialog"),
    );
    checks.check(
        "its subscription ends with `page_handler_failed`, and it is asked no more",
        ended["error"]["data"]["code"] == json!(PAGE_HANDLER_FAILED.name) && at_last.is_empty(),
    );
    checks.check(
        "the page saw the earlier handler's accept, the later one's prompt text and each default",
        handlers.reported.reports
            == pairs(&[
                ("confirm", "true"),
                ("prompt", "\"Ada\""),
                ("guard-leaving", "guarded"),
                ("alert", "closed"),
                ("confirm", "false"),
            ]),
    );
    finish_run(handlers.exchange.connection, run_id)
}
