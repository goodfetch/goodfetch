//! The guard (ADRs 0019 and 0085): a locator action waits until its
//! target is unique and actionable, acts with trusted input, and replays a
//! gesture that landed elsewhere, until its window, `timeout.action`,
//! passes. The conditions are uniqueness, visibility, enabled, stable and
//! receiving the event for a pointer action; visible and enabled for a text
//! action; and editable too for `fill`. A click or a key press that starts
//! a navigation of the page answers once that navigation's document
//! committed, or once it ended without one (ADR 0085), within
//! `timeout.navigation`, the page's turn, and the pointer with it, let go
//! of meanwhile. Each check and the arming of the interception listener are
//! one round trip into the guard's isolated world, and stability is waited
//! for only while an animation or a transition runs. The locator is resolved again at every check, so a
//! re-render that replaces its element changes nothing. A target live
//! hiding hid has its rule lifted for the Session, and is looked at again
//! at once (ADR 0047). At the deadline the last observation decides: still
//! ambiguous raises `target_not_unique` with every match, anything else
//! `deadline_exceeded` naming the step it waited for. The same check and
//! arm serve the Consent prompt runner's clicks on the elements it hands
//! the guard ([`Element`]), in any frame, and the same check, arming
//! nothing, the locator waits (ADR 0080): a state of the locator's one
//! element, or a count of its matches, under `timeout.wait`, outside any
//! gesture's turn.

mod check;
mod target;
#[cfg(test)]
mod tests;

use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::Row;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::lease::LeaseCheck;
use goodfetch_engine::runs::RunSendWindow;
use goodfetch_protocol::connection::{DEADLINE_EXCEEDED, DeadlineExceededDetails, Form};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::waits::{Count, ElementState};
use goodfetch_protocol::page::{TARGET_NOT_UNIQUE, TargetMatch, TargetNotUniqueDetails};
use goodfetch_protocol::settings::SettingLimit;
use serde::Deserialize;

use self::check::{CHECK, Seen, Step, Target};
use crate::cdp::driver::PageDriver;
use crate::cdp::{CdpPage, ExecutionContextId, KeyPress, TargetSession, keys};
use crate::errors::Unanswered;
use crate::hiding::Hidden;
use crate::human_input::{KeyKind, PageHumanInput, Point, Viewport};
use crate::page_owner::PageOwner;
use crate::page_reads::Turn;

/// The most time a human-like approach move or wheel scroll takes of an
/// action's remaining deadline, keeping it well under the guard's 10 s arm
/// expiry while remaining natural.
const HUMAN_MOTION_CAP: Duration = Duration::from_secs(2);

/// The disarm, which says where the gesture landed.
const DISARM: &str = include_str!("guard/disarm.js");

/// The fill of an input whose value a person picks rather than types.
const SET_VALUE: &str = include_str!("guard/set_value.js");

/// A function that does nothing, called in the guard's world once a key
/// press's input was sent.
const SETTLED: &str = "function settled() {}";

/// The longest one check waits in the page before it answers, so that a
/// navigation that replaced the document, or the call's cancellation, is
/// seen within it.
const SLICE: Duration = Duration::from_secs(1);

/// What an action does once its target is ready.
pub(crate) enum Action {
    /// Clicks its centre with the left button.
    Click,
    /// Replaces its text with this text.
    Fill(String),
    /// Sends this key press to it.
    Press(KeyPress),
    /// Presses and holds its centre with the left button for this duration.
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "wired for Challenge solvers in #1090; exercised through the guard's interface in tests"
        )
    )]
    PressAndHold(Duration),
}

impl Action {
    /// A press of `key`, by Playwright's key names.
    ///
    /// # Errors
    ///
    /// `invalid_params` for a key the keyboard has no key for.
    pub(crate) fn press(key: &str) -> Result<Self, CallError> {
        keys::events(key).map(Self::Press).map_err(|unknown| {
            CallError::protocol(ProtocolFailure::InvalidParams, unknown.to_string())
        })
    }

    /// The kind the check takes: which conditions apply.
    fn kind(&self) -> &'static str {
        match self {
            Self::Click | Self::PressAndHold(_) => "pointer",
            Self::Fill(_) => "fill",
            Self::Press(_) => "text",
        }
    }

    /// The row its attempt's commands count on: `C3.commands` of its kind
    /// (ADR 0140), with human-like input counted on its own action kind.
    const fn commands(&self, human_input: bool) -> Row {
        if human_input {
            return Row::ActionHumanInputCommands;
        }
        match self {
            Self::Click | Self::PressAndHold(_) => Row::ActionClickCommands,
            Self::Fill(_) => Row::ActionFillCommands,
            Self::Press(_) => Row::ActionPressCommands,
        }
    }
}

/// How long an action may wait: `timeout.action`'s value and where it was
/// set.
pub(crate) struct Window {
    /// The setting, its value and its layer.
    pub(crate) limit: SettingLimit,
    /// Its duration.
    pub(crate) duration: Duration,
    /// What the call spent of it before its work began, waiting for its
    /// page's `Runtime` (ADR 0019).
    pub(crate) spent: Duration,
}

impl Window {
    /// What is left of it for the call's work.
    fn left(&self) -> Duration {
        self.duration.saturating_sub(self.spent)
    }
}

/// One locator action: what it acts on, and for whom.
pub(crate) struct Request<'a> {
    /// The operation, such as `locator.click`.
    pub(crate) operation: &'static str,
    /// The locator's CSS selector.
    pub(crate) selector: &'a str,
    /// What it does.
    pub(crate) action: Action,
    /// How long it may wait.
    pub(crate) window: Window,
    /// The Client's form, in which its errors show targets.
    pub(crate) form: Form,
    /// What hides elements on the page, which the guard lifts from the
    /// target: its live hiding and the Consent prompt runner's rules.
    pub(crate) hiding: Hidden<'a>,
    /// The page's owner, which grants the action its turn (ADR 0146).
    pub(crate) owner: &'a PageOwner,
    /// The page's send window, which the action's first input marks before
    /// it goes out (ADR 0106).
    pub(crate) sent: SentMark<'a>,
    /// The page's human-like input model when the Session is `stealth` with
    /// `stealth.human_input` on; `None` for direct input.
    pub(crate) human_input: Option<PageHumanInput>,
}

/// ADR 0106's send window of one page's commands: its Run, and the lease
/// naming the attempt when a handler's page, whose bodies the first command
/// that may change the page marks sent.
pub(crate) struct SentMark<'a> {
    /// The send window of the page's Run.
    pub(crate) window: &'a RunSendWindow,
    /// The page's lease, when a handler's page.
    pub(crate) lease: Option<Arc<dyn LeaseCheck>>,
}

impl SentMark<'_> {
    /// Marks sent each body of the window that nothing was sent for yet,
    /// before `command` goes out.
    ///
    /// # Errors
    ///
    /// `run_state_unavailable` when the mark could not be written:
    /// `command` must not go out.
    pub(crate) async fn mark(&self, command: &str) -> Result<(), CallError> {
        self.window.sending(self.lease.as_deref(), command).await
    }
}

/// Where a gesture landed.
#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Landed {
    Landed,
    Intercepted { observed: String },
}

/// Performs `request` on `page`, through the guard, timed by `clock`. The
/// deadline holds however long the page takes to answer: an action whose
/// page answers none of its checks, its main thread busy, or that never
/// gets its turn, ends one check's slice after it (ADR 0080).
///
/// # Errors
///
/// `target_not_unique` or `deadline_exceeded` at the deadline,
/// `invalid_params` for a selector the page does not parse, and the
/// browser's failures as their codes.
///
/// # Cancel safety
///
/// Dropping the future sends no further input, and leaves no key and no
/// button down: a gesture's input is one burst, sent whole or not at all. An
/// interception listener
/// armed for a gesture never sent removes itself at the next check, or
/// after 10 s.
pub(crate) async fn act(
    page: &PageDriver,
    request: Request<'_>,
    clock: &Clock,
) -> Result<(), CallError> {
    let started = clock.now();
    let asked = Asked {
        selector: request.selector,
        form: request.form,
        limit: request.window.limit.clone(),
    };
    // A check answers within its slice of the deadline, so the guard's own
    // loop reports what it saw last; past that, the page is not answering.
    let unanswered = started + request.window.left() + SLICE;
    let spent = request.window.spent;
    tokio::select! {
        biased;
        acted = Box::pin(attempt(page, request, clock)) => acted,
        () = clock.sleep_until(unanswered) => Err(at_deadline(
            asked,
            Seen::Waiting {
                step: Step::Attached,
                observed: "the page answered none of the action's checks in time: its main thread is busy, or another action holds its turn".to_owned(),
                matches: Vec::new(),
            },
            clock.now() - started + spent,
        )),
    }
}

/// Performs `request` on `page` through the guard, as [`act`] does, until
/// it lands, fails or its deadline passes between two checks.
async fn attempt(page: &PageDriver, request: Request<'_>, clock: &Clock) -> Result<(), CallError> {
    // Every command the attempt sends counts on its kind's row: its checks,
    // its input, its disarm and its wait for a navigation it started alike.
    let page = &page.counting(request.action.commands(request.human_input.is_some()));
    let started = clock.now();
    let end = started + request.window.left();
    let mut turn = Turn::granted(
        CdpPage::of(page),
        request.owner.worlds().clone(),
        request.operation,
        request.owner.turn().await?,
    );
    let mut last = Seen::waiting(Step::Attached, "the action waited its turn on the page");
    let mut human_scrolled = false;
    loop {
        // The pointer is the turn's from the check that arms the listener
        // to the disarm, and lent between two checks to the Consent prompt
        // runner's clicks.
        turn.lend_pointer().await;
        let remaining = end.saturating_duration_since(clock.now());
        if remaining.is_zero() {
            let elapsed = clock.now() - started + request.window.spent;
            let asked = Asked {
                selector: request.selector,
                form: request.form,
                limit: request.window.limit,
            };
            return Err(at_deadline(asked, last, elapsed));
        }
        let argument = check::argument(
            Target::Selector(request.selector),
            request.action.kind(),
            remaining.min(SLICE),
            request.human_input.is_some() && !human_scrolled,
        );
        let seen = match turn.check(CHECK, argument).await {
            Ok(value) => Seen::read(&value).map_err(|error| unreadable(&value, &error))?,
            Err(Unanswered::WorldGone) => continue,
            Err(Unanswered::Failed(error)) => return Err(error),
        };
        match seen {
            Seen::Scroll {
                dx,
                dy,
                width,
                height,
            } => {
                human_scrolled = true;
                if let Some(input) = &request.human_input {
                    let viewport = Viewport { width, height };
                    human_scroll(page, &turn, input, (dx, dy), viewport, (clock, end)).await?;
                }
            }
            Seen::Armed {
                x,
                y,
                width,
                height,
            } => {
                let target = Point { x, y };
                let viewport = Viewport { width, height };
                match perform_pointer(page, &mut turn, &request, (target, viewport), (clock, end))
                    .await?
                {
                    PointerOutcome::Landed(watch) => {
                        drop(turn);
                        Box::pin(page.settle_gesture(*watch)).await;
                        return Ok(());
                    }
                    PointerOutcome::Intercepted(observed) => {
                        last = Seen::waiting(Step::ReceivesEvents, observed);
                    }
                }
            }
            Seen::Focused { set_value } => {
                match perform_focused(page, &mut turn, &request, set_value, (clock, end)).await? {
                    FocusedOutcome::Done => return Ok(()),
                    FocusedOutcome::Retry => {}
                    FocusedOutcome::Settle(watch) => {
                        drop(turn);
                        Box::pin(page.settle_gesture(*watch)).await;
                        return Ok(());
                    }
                }
            }
            Seen::Ready { .. } => return Err(mismatched(&seen)),
            Seen::Invalid { message } => return Err(unparsed(request.selector, &message)),
            Seen::Waiting {
                step: Step::Visible,
                ..
            } => {
                match request
                    .hiding
                    .lift(&mut turn, request.selector, request.operation)
                    .await
                {
                    Ok(_) | Err(Unanswered::WorldGone) => last = seen,
                    Err(Unanswered::Failed(error)) => return Err(error),
                }
            }
            waiting @ Seen::Waiting { .. } => last = waiting,
        }
    }
}

/// What a focused keyboard/fill action produced before settling any navigation.
enum FocusedOutcome {
    Done,
    Retry,
    Settle(Box<crate::cdp::driver::GestureWatch>),
}

/// Runs a ready `Fill` or `Press` action on a focused field.
async fn perform_focused(
    page: &PageDriver,
    turn: &mut Turn,
    request: &Request<'_>,
    set_value: bool,
    timing: (&Clock, tokio::time::Instant),
) -> Result<FocusedOutcome, CallError> {
    match &request.action {
        Action::Fill(value) if set_value => {
            request.sent.mark(request.operation).await?;
            if set_picked(turn, value).await? {
                return Ok(FocusedOutcome::Done);
            }
            Ok(FocusedOutcome::Retry)
        }
        Action::Fill(value) => {
            request.sent.mark(request.operation).await?;
            match &request.human_input {
                Some(input) if !value.is_empty() => {
                    human_fill(page, turn, input, value, timing).await?;
                }
                _ => fill(page, turn, value).await?,
            }
            Ok(FocusedOutcome::Done)
        }
        Action::Press(events) => {
            request.sent.mark(request.operation).await?;
            let mut watch = page.watch_gesture();
            Box::pin(watch.during(async {
                CdpPage::of(page)
                    .press_keys(events, turn.gesture_call())
                    .await?;
                settled(turn).await
            }))
            .await?;
            Ok(FocusedOutcome::Settle(Box::new(watch)))
        }
        Action::Click | Action::PressAndHold(_) => Err(mismatched(&"focused")),
    }
}

/// What a pointer action produced before settling any navigation it started.
enum PointerOutcome {
    Landed(Box<crate::cdp::driver::GestureWatch>),
    Intercepted(String),
}

/// Runs a ready pointer action (`Click` or `PressAndHold`) inside a
/// navigation watch. Returns `Landed(watch)` so the caller drops `turn`
/// before settling the navigation (ADR 0085), or `Intercepted(observed)` so
/// the guard retries.
async fn perform_pointer(
    page: &PageDriver,
    turn: &mut Turn,
    request: &Request<'_>,
    (target, viewport): (Point, Viewport),
    timing: (&Clock, tokio::time::Instant),
) -> Result<PointerOutcome, CallError> {
    match &request.action {
        Action::Click => {
            request.sent.mark(request.operation).await?;
            let mut watch = page.watch_gesture();
            let landed = if let Some(input) = &request.human_input {
                let clicked = human_click(page, turn, input, (target, viewport), timing);
                Box::pin(watch.during(clicked)).await?
            } else {
                let clicked = click(page, turn, (target.x, target.y));
                Box::pin(watch.during(clicked)).await?
            };
            Ok(match landed {
                Landed::Landed => PointerOutcome::Landed(Box::new(watch)),
                Landed::Intercepted { observed } => PointerOutcome::Intercepted(observed),
            })
        }
        Action::PressAndHold(hold) => {
            request.sent.mark(request.operation).await?;
            let mut watch = page.watch_gesture();
            let action = press_and_hold(
                page,
                turn,
                request.human_input.as_ref(),
                (target, viewport, *hold),
                timing,
            );
            let landed = Box::pin(watch.during(action)).await?;
            Ok(match landed {
                Landed::Landed => PointerOutcome::Landed(Box::new(watch)),
                Landed::Intercepted { observed } => PointerOutcome::Intercepted(observed),
            })
        }
        Action::Fill(_) | Action::Press(_) => Err(mismatched(&"armed")),
    }
}

/// An element another engine script handed the guard, by its handle in the
/// registry of guard targets of the world whose context is `context` on
/// `session`: the Consent prompt runner's click targets, in any frame.
pub(crate) struct Element<'a> {
    /// The session of the frame's target.
    pub(crate) session: &'a TargetSession,
    /// The context of the guard's world in the frame's document.
    pub(crate) context: ExecutionContextId,
    /// Its handle.
    pub(crate) handle: u64,
}

/// What one check of an [`Element`] saw.
pub(crate) enum Readiness {
    /// It is unique, visible, enabled, stable and receives the event at
    /// (`x`, `y`) of its frame's viewport, the listener armed there.
    Ready {
        /// The point's distance from the frame viewport's left.
        x: f64,
        /// Its distance from the frame viewport's top.
        y: f64,
    },
    /// It is not yet.
    Waiting,
    /// Its document or the element went away, or the check could not run.
    Gone,
}

/// Checks `element` for a click, waiting in the page for at most `slice`;
/// a ready element has the guard's listener armed. The caller holds the
/// page's lent pointer from this check to [`landed_on`].
pub(crate) async fn check_element(
    page: &PageDriver,
    element: &Element<'_>,
    slice: Duration,
) -> Readiness {
    let argument = check::argument(Target::Handle(element.handle), "pointer", slice, false);
    page.work().count(Row::ActionScriptSends);
    let Ok(value) = page
        .counting(Row::ActionCheckCommands)
        .call_in(element.session, element.context, CHECK, argument)
        .await
    else {
        return Readiness::Gone;
    };
    match Seen::read(&value) {
        Ok(Seen::Armed { x, y, .. }) => Readiness::Ready { x, y },
        Ok(Seen::Waiting { .. }) => Readiness::Waiting,
        _ => Readiness::Gone,
    }
}

/// Whether the click just sent at a ready [`Element`] landed on it: the
/// guard's listener disarmed; a document gone with the click means it
/// landed, since nothing was left to cancel it.
pub(crate) async fn landed_on(page: &PageDriver, element: &Element<'_>) -> bool {
    match page
        .call_in(
            element.session,
            element.context,
            DISARM,
            serde_json::Value::Null,
        )
        .await
    {
        Ok(value) => matches!(Landed::deserialize(&value), Ok(Landed::Landed)),
        Err(Unanswered::WorldGone) => true,
        Err(Unanswered::Failed(_)) => false,
    }
}

/// Waits for one round trip into the page's renderer once a key press's
/// input was sent, as the click's disarm does: the browser acknowledges
/// input before the events its handling sent, such as the main frame's
/// request to navigate, so those arrive before this answer (Playwright's
/// input epilogue). A document gone with the input sent them already.
async fn settled(turn: &mut Turn) -> Result<(), CallError> {
    match turn.call(SETTLED, serde_json::Value::Null).await {
        Ok(_) | Err(Unanswered::WorldGone) => Ok(()),
        Err(Unanswered::Failed(error)) => Err(error),
    }
}

/// Clicks at the point `(x, y)`, then reads where the click landed.
async fn click(
    page: &PageDriver,
    turn: &mut Turn,
    (x, y): (f64, f64),
) -> Result<Landed, CallError> {
    CdpPage::of(page)
        .pointer_click(x, y, turn.gesture_call())
        .await?;
    landed(turn).await
}

/// Sleeps `delay` bounded by `end` on `clock`.
async fn pace(clock: &Clock, end: tokio::time::Instant, delay: Duration) {
    let wait = delay.min(end.saturating_duration_since(clock.now()));
    if !wait.is_zero() {
        clock.sleep(wait).await;
    }
}

/// Moves the pointer along a human-like curve onto `target`, presses and
/// releases as one atomic burst, then reads where the click landed.
async fn human_click(
    page: &PageDriver,
    turn: &mut Turn,
    input: &PageHumanInput,
    (target, viewport): (Point, Viewport),
    (clock, end): (&Clock, tokio::time::Instant),
) -> Result<Landed, CallError> {
    let budget = end
        .saturating_duration_since(clock.now())
        .min(HUMAN_MOTION_CAP);
    let moves = input.path_to(target, viewport, budget).await;
    let cdp = CdpPage::of(page);
    for step in moves {
        pace(clock, end, step.after).await;
        cdp.pointer_move_to(step.to.x, step.to.y, turn.gesture_call())
            .await?;
    }
    cdp.pointer_press_release(target.x, target.y, turn.gesture_call())
        .await?;
    landed(turn).await
}

/// Scrolls the viewport by `delta` using human-like wheel notches.
async fn human_scroll(
    page: &PageDriver,
    turn: &Turn,
    input: &PageHumanInput,
    delta: (f64, f64),
    viewport: Viewport,
    (clock, end): (&Clock, tokio::time::Instant),
) -> Result<(), CallError> {
    let budget = end
        .saturating_duration_since(clock.now())
        .min(HUMAN_MOTION_CAP);
    let notches = input.wheel(delta, budget).await;
    let cdp = CdpPage::of(page);
    let anchor = (viewport.width / 2.0, viewport.height / 2.0);
    for notch in notches {
        pace(clock, end, notch.after).await;
        cdp.pointer_wheel(anchor, notch.delta, turn.gesture_call())
            .await?;
    }
    Ok(())
}

/// Presses and holds `target` for `hold`, using human-like approach and
/// micro-tremors when `input` is present, then reads where it landed.
async fn press_and_hold(
    page: &PageDriver,
    turn: &mut Turn,
    input: Option<&PageHumanInput>,
    (target, viewport, hold): (Point, Viewport, Duration),
    (clock, end): (&Clock, tokio::time::Instant),
) -> Result<Landed, CallError> {
    let cdp = CdpPage::of(page);
    let remaining = end.saturating_duration_since(clock.now());
    if let Some(input) = input {
        let plan = input
            .press_and_hold(target, hold, viewport, remaining)
            .await;
        for step in plan.approach {
            pace(clock, end, step.after).await;
            cdp.pointer_move_to(step.to.x, step.to.y, turn.gesture_call())
                .await?;
        }
        cdp.pointer_down(target.x, target.y, turn.gesture_call())
            .await?;
        for tremor in plan.hold_moves {
            pace(clock, end, tremor.after).await;
            cdp.pointer_drag_to(tremor.to.x, tremor.to.y, turn.gesture_call())
                .await?;
        }
        pace(clock, end, plan.release_after).await;
        cdp.pointer_up(target.x, target.y, turn.gesture_call())
            .await?;
    } else {
        cdp.pointer_move_to(target.x, target.y, turn.gesture_call())
            .await?;
        cdp.pointer_down(target.x, target.y, turn.gesture_call())
            .await?;
        pace(clock, end, hold).await;
        cdp.pointer_up(target.x, target.y, turn.gesture_call())
            .await?;
    }
    landed(turn).await
}

/// Types `value` into the focused field key by key with human-like pacing
/// and corrected neighbouring-key slips.
async fn human_fill(
    page: &PageDriver,
    turn: &mut Turn,
    input: &PageHumanInput,
    value: &str,
    (clock, end): (&Clock, tokio::time::Instant),
) -> Result<(), CallError> {
    let budget = end.saturating_duration_since(clock.now());
    let strokes = input.typing(value, budget).await;
    let cdp = CdpPage::of(page);
    for stroke in strokes {
        pace(clock, end, stroke.after).await;
        match stroke.kind {
            KeyKind::Backspace => {
                let key = keys::events("Backspace").map_err(|unknown| {
                    CallError::protocol(ProtocolFailure::InternalError, unknown.to_string())
                })?;
                cdp.press_keys(&key, turn.gesture_call()).await?;
            }
            KeyKind::Character('\n') => {
                let key = keys::events("Enter").map_err(|unknown| {
                    CallError::protocol(ProtocolFailure::InternalError, unknown.to_string())
                })?;
                cdp.press_keys(&key, turn.gesture_call()).await?;
            }
            KeyKind::Character('\t') => {
                let key = keys::events("Tab").map_err(|unknown| {
                    CallError::protocol(ProtocolFailure::InternalError, unknown.to_string())
                })?;
                cdp.press_keys(&key, turn.gesture_call()).await?;
            }
            KeyKind::Character(ch) if ch.is_ascii() && !ch.is_ascii_control() => {
                let text = ch.to_string();
                let key = keys::events(&text).map_err(|unknown| {
                    CallError::protocol(ProtocolFailure::InternalError, unknown.to_string())
                })?;
                cdp.press_keys(&key, turn.gesture_call()).await?;
            }
            KeyKind::Character(ch) => {
                turn.insert_text(&ch.to_string()).await?;
            }
        }
    }
    Ok(())
}

/// Where the gesture just sent landed; a document gone with it means it
/// landed, since nothing was left to cancel it.
async fn landed(turn: &mut Turn) -> Result<Landed, CallError> {
    match turn.call(DISARM, serde_json::Value::Null).await {
        Ok(value) => Landed::deserialize(&value).map_err(|error| unreadable(&value, &error)),
        Err(Unanswered::WorldGone) => Ok(Landed::Landed),
        Err(Unanswered::Failed(error)) => Err(error),
    }
}

/// Replaces the selected text of the focused field with `value`, as typed
/// text arrives; an empty value deletes it.
async fn fill(page: &PageDriver, turn: &mut Turn, value: &str) -> Result<(), CallError> {
    if !value.is_empty() {
        return turn.insert_text(value).await;
    }
    let delete = keys::events("Delete").map_err(|unknown| {
        CallError::protocol(ProtocolFailure::InternalError, unknown.to_string())
    })?;
    CdpPage::of(page)
        .press_keys(&delete, turn.gesture_call())
        .await
}

/// `invalid_params` for `selector`, which the page does not parse as CSS,
/// saying so in its `message`.
fn unparsed(selector: &str, message: &str) -> CallError {
    CallError::protocol(
        ProtocolFailure::InvalidParams,
        format!("`{selector}` is no CSS selector the page parses: {message}"),
    )
}

/// What the guard's fill of a picked value answered.
#[derive(Deserialize)]
struct ValueSet {
    set: bool,
    gone: bool,
}

/// Sets `value` on the input the check made ready, as a person picks it,
/// firing `input` and `change`; answers whether it was set, or `false`
/// when the input went with its document or a re-render, so the guard
/// checks again.
///
/// # Errors
///
/// `invalid_params` for a value the input does not take, which keeps its
/// own.
async fn set_picked(turn: &mut Turn, value: &str) -> Result<bool, CallError> {
    let answer = match turn.call(SET_VALUE, serde_json::json!(value)).await {
        Ok(answer) => answer,
        Err(Unanswered::WorldGone) => return Ok(false),
        Err(Unanswered::Failed(error)) => return Err(error),
    };
    match ValueSet::deserialize(&answer).map_err(|error| unreadable(&answer, &error))? {
        ValueSet { set: true, .. } => Ok(true),
        ValueSet { gone: true, .. } => Ok(false),
        ValueSet { .. } => Err(CallError::protocol(
            ProtocolFailure::InvalidParams,
            format!("`{value}` is not a value the input takes, so it keeps its own"),
        )),
    }
}

/// The guard's script answered `seen`, which is not an answer to the
/// check it was asked: a pointer action's answer for a text action, or an
/// action's for a locator wait.
fn mismatched(seen: &impl std::fmt::Debug) -> CallError {
    CallError::protocol(
        ProtocolFailure::InternalError,
        format!("the guard's script answered {seen:?}, which is not the check it was asked"),
    )
}

/// The guard's script answered `value`, which does not read as what it
/// returns.
fn unreadable(value: &serde_json::Value, error: &serde_json::Error) -> CallError {
    CallError::protocol(
        ProtocolFailure::InternalError,
        format!("the guard's script answered {value}, which does not read: {error}"),
    )
}

/// What a locator wait reached.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Waited {
    /// Its element's state.
    State,
    /// A count its matches' number is within: that number.
    Count(u64),
}

/// What a locator wait waits for.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Awaited {
    /// Its one element, or none, in this state.
    State(ElementState),
    /// Its matches, as many as this count allows.
    Count(Count),
}

/// One locator wait: what it waits for, and for whom.
pub(crate) struct Wait<'a> {
    /// The operation, such as `locator.wait_for`.
    pub(crate) operation: &'static str,
    /// The locator's CSS selector.
    pub(crate) selector: &'a str,
    /// What it waits for.
    pub(crate) awaited: Awaited,
    /// How long it may wait: its call's own `timeout`, else
    /// `timeout.wait`; none for a call whose `timeout` is `null`.
    pub(crate) window: Option<Window>,
    /// The Client's form, in which its errors show targets.
    pub(crate) form: Form,
    /// The owner of its Page, which grants its reads.
    pub(crate) owner: &'a PageOwner,
}

/// Waits on `page` until the matches of `wait`'s locator are as it waits
/// for, checking them in the guard's world once a frame, outside any
/// gesture's turn, timed by `clock`; answers what it reached, for a count
/// how many matched.
///
/// # Errors
///
/// `target_not_unique` or `deadline_exceeded` at the deadline,
/// `invalid_params` for a selector the page does not parse, and the
/// browser's failures as their codes.
///
/// # Cancel safety
///
/// Dropping the future leaves nothing behind in the page.
pub(crate) async fn wait(
    page: &PageDriver,
    wait: Wait<'_>,
    clock: &Clock,
) -> Result<Waited, CallError> {
    let started = clock.now();
    let end = wait.window.as_ref().map(|window| started + window.left());
    let (step, bounds) = match wait.awaited {
        Awaited::State(state) => (Step::waiting_for(state), (None, None)),
        Awaited::Count(count) => (Step::Count, (count.min, count.max)),
    };
    let mut last = Seen::Waiting {
        step,
        observed: "the page was not checked yet".to_owned(),
        matches: Vec::new(),
    };
    // One ask of the Page's owner grants every check of the wait.
    let checking = wait
        .owner
        .reads(CdpPage::of(page))
        .await?
        .counting(Row::ActionCheckCommands);
    loop {
        let remaining = end.map_or(SLICE, |end| end.saturating_duration_since(clock.now()));
        if remaining.is_zero()
            && let Some(window) = wait.window
        {
            let elapsed = clock.now() - started + window.spent;
            let asked = Asked {
                selector: wait.selector,
                form: wait.form,
                limit: window.limit,
            };
            return Err(at_deadline(asked, last, elapsed));
        }
        let argument =
            check::wait_argument(wait.selector, step.as_str(), bounds, remaining.min(SLICE));
        // Each of a wait's checks makes the guard's world again, a command of
        // its own (X11).
        page.work().count(Row::ActionScriptSends);
        let Some(value) = checking.in_page(CHECK, argument, wait.operation).await? else {
            // The document went, or is not yet: checked again in the next.
            clock.sleep(remaining.min(RECHECK)).await;
            continue;
        };
        match Seen::read(&value).map_err(|error| unreadable(&value, &error))? {
            Seen::Ready { count } => {
                return match (wait.awaited, count) {
                    (Awaited::State(_), _) => Ok(Waited::State),
                    (Awaited::Count(_), Some(count)) => Ok(Waited::Count(count)),
                    (Awaited::Count(_), None) => Err(CallError::protocol(
                        ProtocolFailure::InternalError,
                        format!(
                            "the guard's script answered {value} to a count wait, with no count"
                        ),
                    )),
                };
            }
            Seen::Armed { .. } | Seen::Scroll { .. } | Seen::Focused { .. } => {
                return Err(mismatched(&value));
            }
            Seen::Invalid { message } => return Err(unparsed(wait.selector, &message)),
            waiting @ Seen::Waiting { .. } => last = waiting,
        }
    }
}

/// How long a locator wait leaves a document that is not there yet before
/// it checks again.
const RECHECK: Duration = Duration::from_millis(20);

/// What a call at its deadline asked: its locator's selector, the
/// Client's form and the setting whose time passed. The call's handler
/// heads the error with its operation.
struct Asked<'a> {
    selector: &'a str,
    form: Form,
    limit: SettingLimit,
}

/// The error of an action or a locator wait whose deadline passed with
/// `last` its last observation.
fn at_deadline(asked: Asked<'_>, last: Seen, elapsed: Duration) -> CallError {
    let elapsed = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    let target = target::render(asked.form, asked.selector, None);
    let limit = asked.limit;
    let (step, observed, matches) = match last {
        Seen::Waiting {
            step,
            observed,
            matches,
        } => (step, observed, matches),
        Seen::Armed { .. }
        | Seen::Scroll { .. }
        | Seen::Focused { .. }
        | Seen::Ready { .. }
        | Seen::Invalid { .. } => (
            Step::ReceivesEvents,
            "the target was ready".to_owned(),
            Vec::new(),
        ),
    };
    if step == Step::Unique {
        let matches: Vec<TargetMatch> = matches
            .into_iter()
            .enumerate()
            .map(|(index, element)| TargetMatch {
                element,
                locator: target::render(asked.form, asked.selector, Some(index)),
            })
            .collect();
        let listed = matches
            .iter()
            .map(|found| format!("{} ({})", found.element, found.locator))
            .collect::<Vec<_>>()
            .join(", ");
        return CallError::new(
            &TARGET_NOT_UNIQUE,
            format!(
                "{target} still matched {} elements when `{}` ({} ms, {}) passed: {listed}. Make the locator match one element, or pick one with `nth()`.",
                matches.len(),
                limit.key,
                limit.value,
                limit.layer.as_str(),
            ),
            &TargetNotUniqueDetails {
                target,
                matches,
                limit,
                elapsed,
            },
        );
    }
    CallError::new(
        &DEADLINE_EXCEEDED,
        format!(
            "it waited for {target} to be {} until `{}` ({} ms, {}) passed: {observed}.",
            match step {
                Step::ReceivesEvents => "receiving the event",
                Step::Count => "matched as often as its count allows",
                other => other.as_str(),
            },
            limit.key,
            limit.value,
            limit.layer.as_str(),
        ),
        &DeadlineExceededDetails {
            limit,
            elapsed,
            waited_for: step.waited_for(),
            ended_by: None,
            observed: None,
        },
    )
}
