//! The handlers of input with no target (ADR 0085): `page.keyboard`,
//! `page.mouse` and `page.touchscreen`. Each checks its call's keys and
//! counts, writes the Provenance entry that marks it unguarded, and sends
//! its input as a Gesture of its page.

use std::time::Duration;

use goodfetch_engine::{CallContext, CallError, Contribution, Handler};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::input::{
    KeyboardInsertTextParams, KeyboardKeyParams, KeyboardPressParams, KeyboardTypeParams,
    MouseButtonParams, MouseClickParams, MouseDblclickParams, MouseMoveParams, MouseWheelParams,
    PAGE_KEYBOARD_DOWN, PAGE_KEYBOARD_INSERT_TEXT, PAGE_KEYBOARD_PRESS, PAGE_KEYBOARD_TYPE,
    PAGE_KEYBOARD_UP, PAGE_MOUSE_CLICK, PAGE_MOUSE_DBLCLICK, PAGE_MOUSE_DOWN, PAGE_MOUSE_MOVE,
    PAGE_MOUSE_UP, PAGE_MOUSE_WHEEL, PAGE_TOUCHSCREEN_TAP, TouchscreenTapParams,
};
use goodfetch_protocol::page::{Acted, PageId};
use serde::de::DeserializeOwned;
use serde_json::json;

use super::{Pages, in_call};
use crate::cdp::Press;
use crate::cdp::keys::{self, UnknownKey};
use crate::guard::SentMark;
use crate::input::{self, Input};
use crate::sessions::PageEvent;
use crate::trace;

/// The Provenance entry's action for input with no target, which no guard
/// checked.
const UNGUARDED_INPUT: &str = "unguarded_input";

/// The most moves `page.mouse.move` takes, as its schema says.
const MAX_STEPS: u32 = 1000;

/// The most clicks `page.mouse.click` takes, as its schema says.
const MAX_CLICKS: u32 = 10;

/// Adds the handlers of `page.keyboard`, `page.mouse` and
/// `page.touchscreen`.
pub(super) fn with_input_handlers(contribution: Contribution, pages: &Pages) -> Contribution {
    let contribution = handled(
        contribution,
        pages,
        &PAGE_KEYBOARD_DOWN,
        |params: KeyboardKeyParams| {
            let key = keys::key(&params.key).map_err(unknown)?;
            Ok((params.page_id, Input::Key(Press::Down, key)))
        },
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_KEYBOARD_UP,
        |params: KeyboardKeyParams| {
            let key = keys::key(&params.key).map_err(unknown)?;
            Ok((params.page_id, Input::Key(Press::Up, key)))
        },
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_KEYBOARD_PRESS,
        |params: KeyboardPressParams| {
            let keys = keys::keys(&params.key).map_err(unknown)?;
            let delay = millis(params.delay);
            Ok((params.page_id, Input::Press { keys, delay }))
        },
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_KEYBOARD_TYPE,
        |params: KeyboardTypeParams| {
            let delay = millis(params.delay);
            let text = params.text;
            Ok((params.page_id, Input::Type { text, delay }))
        },
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_KEYBOARD_INSERT_TEXT,
        |params: KeyboardInsertTextParams| Ok((params.page_id, Input::InsertText(params.text))),
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_MOUSE_MOVE,
        |params: MouseMoveParams| {
            let steps = params.steps.unwrap_or(1);
            if steps > MAX_STEPS {
                return Err(invalid(format!(
                    "`steps` is {steps}: give at most {MAX_STEPS} moves"
                )));
            }
            let to = (params.x, params.y);
            Ok((params.page_id, Input::Move { to, steps }))
        },
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_MOUSE_DOWN,
        |params: MouseButtonParams| button(params, Press::Down),
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_MOUSE_UP,
        |params: MouseButtonParams| button(params, Press::Up),
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_MOUSE_CLICK,
        |params: MouseClickParams| {
            let click_count = params.click_count.unwrap_or(1);
            if !(1..=MAX_CLICKS).contains(&click_count) {
                return Err(invalid(format!(
                    "`click_count` is {click_count}: give from 1 to {MAX_CLICKS} clicks"
                )));
            }
            let click = Input::Click {
                at: (params.x, params.y),
                button: params.button.unwrap_or_default(),
                click_count,
                delay: millis(params.delay),
            };
            Ok((params.page_id, click))
        },
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_MOUSE_DBLCLICK,
        |params: MouseDblclickParams| {
            let click = Input::Click {
                at: (params.x, params.y),
                button: params.button.unwrap_or_default(),
                click_count: 2,
                delay: millis(params.delay),
            };
            Ok((params.page_id, click))
        },
    );
    let contribution = handled(
        contribution,
        pages,
        &PAGE_MOUSE_WHEEL,
        |params: MouseWheelParams| {
            let delta = (params.delta_x, params.delta_y);
            Ok((params.page_id, Input::Wheel(delta)))
        },
    );
    handled(
        contribution,
        pages,
        &PAGE_TOUCHSCREEN_TAP,
        |params: TouchscreenTapParams| Ok((params.page_id, Input::Tap((params.x, params.y)))),
    )
}

/// Adds the handler of `operation`, whose call's params `input` reads as
/// the page and the input it sends there.
fn handled<P: DeserializeOwned + Send + 'static>(
    contribution: Contribution,
    pages: &Pages,
    operation: &'static goodfetch_protocol::catalog::Operation,
    input: fn(P) -> Result<(PageId, Input), CallError>,
) -> Contribution {
    let pages = pages.clone();
    contribution.with_handler(Handler::new(
        operation,
        move |params: P, context: CallContext| {
            let pages = pages.clone();
            in_call((operation.name, context.form()), async move {
                let (page_id, input) = input(params)?;
                pages.input(operation.name, &page_id, input).await
            })
        },
    ))
}

impl Pages {
    /// Sends `input`, the call `operation`'s, to the page `page_id`, as a
    /// Gesture of the page, once Provenance marks it unguarded.
    #[tracing::instrument(name = "page", level = "debug", skip_all, fields(page_id = %page_id.0, operation, run_id = tracing::field::Empty))]
    async fn input(
        &self,
        operation: &'static str,
        page_id: &PageId,
        input: Input,
    ) -> Result<Acted, CallError> {
        let (page, _) = self
            .open(page_id, |page| {
                Ok(Some(page.driver.action_deadline().clone()))
            })
            .await?;
        tracing::Span::current().record("run_id", page.run.identity.id().as_str());
        trace::provenance(
            &page.run.trace,
            json!({
                "action": UNGUARDED_INPUT,
                "operation": operation,
                "page_id": page_id,
                "session_id": page.session,
            }),
        )
        .await;
        let sent = SentMark {
            window: &page.run.send_window,
            lease: page.lease(),
        };
        let input = input::send(
            (&page.driver, &page.owner, sent),
            (operation, input),
            &page.clock,
        );
        let sent = page.life.call(PageEvent::Gesture, input).await;
        // A dialog handler that failed on a dialog the input opened fails
        // the input, which the dialog blocked.
        if let Some(failure) = page.owner.dialogs().take_failure() {
            return Err(failure);
        }
        sent?;
        Ok(Acted {})
    }
}

/// The press or release of `params`' button, the nth of a run of clicks.
fn button(params: MouseButtonParams, press: Press) -> Result<(PageId, Input), CallError> {
    let click_count = params.click_count.unwrap_or(1);
    if click_count == 0 {
        return Err(invalid("`click_count` is 0: give at least 1".to_owned()));
    }
    let button = Input::Button {
        press,
        button: params.button.unwrap_or_default(),
        click_count,
    };
    Ok((params.page_id, button))
}

/// A delay of `millis` milliseconds; none is no wait.
fn millis(millis: Option<u64>) -> Duration {
    Duration::from_millis(millis.unwrap_or(0))
}

/// `invalid_params` for a key the keyboard has no key for.
fn unknown(unknown: UnknownKey) -> CallError {
    invalid(unknown.to_string())
}

/// `invalid_params`, saying `why`.
fn invalid(why: String) -> CallError {
    CallError::protocol(ProtocolFailure::InvalidParams, why)
}
