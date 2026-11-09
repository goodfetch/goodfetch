//! The guard (ADRs 0019 and 0085): a locator action waits until its
//! target is unique and actionable, acts with trusted input, and replays a
//! gesture that landed elsewhere, until its window, `timeout.action`,
//! passes. The conditions are uniqueness, visibility, enabled, stable and
//! receiving the event for a pointer action; visible and enabled for a text
//! action; and editable too for `fill`. A click or a key press that starts
//! a navigation of the page answers once that navigation's document
//! committed, or once it ended without one (ADR 0085), within
//! `timeout.navigation`, the page's turn and pointer let go of meanwhile. Each check and the arming of the
//! interception listener are one round trip into the guard's isolated
//! world, and stability is waited for only while an animation or a
//! transition runs. The locator is resolved again at every check, so a
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
mod keys;
mod target;
#[cfg(test)]
mod tests;

use std::time::Duration;

use goodfetch_cdp::SessionId as TargetSession;
use goodfetch_cdp::generated::input::DispatchKeyEventParams;
use goodfetch_cdp::generated::runtime::ExecutionContextId;
use goodfetch_counted_work::Row;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_protocol::connection::{DEADLINE_EXCEEDED, DeadlineExceededDetails, Surface};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::waits::{Count, ElementState};
use goodfetch_protocol::page::{TARGET_NOT_UNIQUE, TargetMatch, TargetNotUniqueDetails};
use goodfetch_protocol::settings::SettingLimit;
use serde::Deserialize;

use self::check::{CHECK, Seen, Step, Target};
use crate::driver::{PageDriver, Turn, Unanswered};
use crate::hiding::Hidden;

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
    /// Sends these key events to it.
    Press(Vec<DispatchKeyEventParams>),
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
            Self::Click => "pointer",
            Self::Fill(_) => "fill",
            Self::Press(_) => "text",
        }
    }

    /// The row its attempt's commands count on: `C3.commands` of its kind
    /// (ADR 0140).
    const fn commands(&self) -> Row {
        match self {
            Self::Click => Row::ActionClickCommands,
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
    /// The Client's surface, in whose language its errors show targets.
    pub(crate) surface: Surface,
    /// What hides elements on the page, which the guard lifts from the
    /// target: its live hiding and the Consent prompt runner's rules.
    pub(crate) hiding: Hidden<'a>,
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
/// Dropping the future sends no further input; an interception listener
/// armed for a gesture never sent removes itself at the next check, or
/// after 10 s.
pub(crate) async fn act(
    page: &PageDriver,
    request: Request<'_>,
    clock: &Clock,
) -> Result<(), CallError> {
    let started = clock.now();
    let asked = Asked {
        operation: request.operation,
        selector: request.selector,
        surface: request.surface,
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
    let page = &page.counting(request.action.commands());
    let started = clock.now();
    let end = started + request.window.left();
    let mut turn = page.turn(request.operation).await;
    let mut last = Seen::waiting(Step::Attached, "the action waited its turn on the page");
    loop {
        let remaining = end.saturating_duration_since(clock.now());
        if remaining.is_zero() {
            let elapsed = clock.now() - started + request.window.spent;
            let asked = Asked {
                operation: request.operation,
                selector: request.selector,
                surface: request.surface,
                limit: request.window.limit,
            };
            return Err(at_deadline(asked, last, elapsed));
        }
        let argument = check::argument(
            Target::Selector(request.selector),
            request.action.kind(),
            remaining.min(SLICE),
        );
        // Held from the check that arms the listener to the disarm, and let
        // go of when the target is not ready, so the Consent prompt
        // runner's clicks come between two checks.
        let gesture = page.gesture().await;
        let seen = match turn.check(CHECK, argument).await {
            Ok(value) => Seen::read(&value).map_err(|error| unreadable(&value, &error))?,
            Err(Unanswered::WorldGone) => continue,
            Err(Unanswered::Failed(error)) => return Err(error),
        };
        match seen {
            Seen::Armed { x, y } => match &request.action {
                Action::Click => {
                    let mut watch = page.watch_gesture();
                    // The disarm is the round trip into the renderer after
                    // the input that a key press makes with `settled`.
                    // Boxed, as the wait below: the action's future stays small.
                    let landed = Box::pin(watch.during(async {
                        turn.click(x, y).await?;
                        landed(&mut turn).await
                    }))
                    .await?;
                    match landed {
                        Landed::Landed => {
                            drop((gesture, turn));
                            Box::pin(page.settle_gesture(watch)).await;
                            return Ok(());
                        }
                        Landed::Intercepted { observed } => {
                            last = Seen::waiting(Step::ReceivesEvents, observed);
                        }
                    }
                }
                Action::Fill(_) | Action::Press(_) => return Err(mismatched(&seen)),
            },
            Seen::Focused { set_value } => match &request.action {
                Action::Fill(value) if set_value => {
                    if set_picked(&mut turn, value).await? {
                        return Ok(());
                    }
                }
                Action::Fill(value) => return fill(&mut turn, value).await,
                Action::Press(events) => {
                    let mut watch = page.watch_gesture();
                    Box::pin(watch.during(async {
                        for event in events {
                            turn.key(event).await?;
                        }
                        settled(&mut turn).await
                    }))
                    .await?;
                    drop((gesture, turn));
                    Box::pin(page.settle_gesture(watch)).await;
                    return Ok(());
                }
                Action::Click => return Err(mismatched(&seen)),
            },
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
        drop(gesture);
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
/// page's [`PageDriver::gesture`] from this check to [`landed_on`].
pub(crate) async fn check_element(
    page: &PageDriver,
    element: &Element<'_>,
    slice: Duration,
) -> Readiness {
    let argument = check::argument(Target::Handle(element.handle), "pointer", slice);
    page.work().count(Row::ActionScriptSends);
    let Ok(value) = page
        .counting(Row::ActionCheckCommands)
        .call_in(element.session, element.context, CHECK, argument)
        .await
    else {
        return Readiness::Gone;
    };
    match Seen::read(&value) {
        Ok(Seen::Armed { x, y }) => Readiness::Ready { x, y },
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
async fn settled(turn: &mut Turn<'_>) -> Result<(), CallError> {
    match turn.call(SETTLED, serde_json::Value::Null).await {
        Ok(_) | Err(Unanswered::WorldGone) => Ok(()),
        Err(Unanswered::Failed(error)) => Err(error),
    }
}

/// Where the gesture just sent landed; a document gone with it means it
/// landed, since nothing was left to cancel it.
async fn landed(turn: &mut Turn<'_>) -> Result<Landed, CallError> {
    match turn.call(DISARM, serde_json::Value::Null).await {
        Ok(value) => Landed::deserialize(&value).map_err(|error| unreadable(&value, &error)),
        Err(Unanswered::WorldGone) => Ok(Landed::Landed),
        Err(Unanswered::Failed(error)) => Err(error),
    }
}

/// Replaces the selected text of the focused field with `value`, as typed
/// text arrives; an empty value deletes it.
async fn fill(turn: &mut Turn<'_>, value: &str) -> Result<(), CallError> {
    if !value.is_empty() {
        return turn.insert_text(value).await;
    }
    let delete = keys::events("Delete").map_err(|unknown| {
        CallError::protocol(ProtocolFailure::InternalError, unknown.to_string())
    })?;
    for event in &delete {
        turn.key(event).await?;
    }
    Ok(())
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
async fn set_picked(turn: &mut Turn<'_>, value: &str) -> Result<bool, CallError> {
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
    /// The Client's surface, in whose language its errors show targets.
    pub(crate) surface: Surface,
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
    loop {
        let remaining = end.map_or(SLICE, |end| end.saturating_duration_since(clock.now()));
        if remaining.is_zero()
            && let Some(window) = wait.window
        {
            let elapsed = clock.now() - started + window.spent;
            let asked = Asked {
                operation: wait.operation,
                selector: wait.selector,
                surface: wait.surface,
                limit: window.limit,
            };
            return Err(at_deadline(asked, last, elapsed));
        }
        let argument =
            check::wait_argument(wait.selector, step.as_str(), bounds, remaining.min(SLICE));
        // Each of a wait's checks makes the guard's world again, a command of
        // its own (X11).
        page.work().count(Row::ActionScriptSends);
        let checking = page.counting(Row::ActionCheckCommands);
        let Some(value) = checking
            .read_in_page(CHECK, argument, wait.operation)
            .await?
        else {
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
            Seen::Armed { .. } | Seen::Focused { .. } => return Err(mismatched(&value)),
            Seen::Invalid { message } => return Err(unparsed(wait.selector, &message)),
            waiting @ Seen::Waiting { .. } => last = waiting,
        }
    }
}

/// How long a locator wait leaves a document that is not there yet before
/// it checks again.
const RECHECK: Duration = Duration::from_millis(20);

/// What a call at its deadline asked: its operation, its locator's
/// selector, the Client's surface and the setting whose time passed.
struct Asked<'a> {
    operation: &'static str,
    selector: &'a str,
    surface: Surface,
    limit: SettingLimit,
}

/// The error of an action or a locator wait whose deadline passed with
/// `last` its last observation.
fn at_deadline(asked: Asked<'_>, last: Seen, elapsed: Duration) -> CallError {
    let elapsed = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    let target = target::render(asked.surface, asked.selector, None);
    let limit = asked.limit;
    let (step, observed, matches) = match last {
        Seen::Waiting {
            step,
            observed,
            matches,
        } => (step, observed, matches),
        Seen::Armed { .. } | Seen::Focused { .. } | Seen::Ready { .. } | Seen::Invalid { .. } => (
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
                locator: target::render(asked.surface, asked.selector, Some(index)),
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
                "`{}` on {target}: {} elements still matched when `{}` ({} ms, {}) passed: {listed}. Make the locator match one element, or pick one with `nth()`.",
                asked.operation,
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
            "`{}` on {target}: `{}` ({} ms, {}) passed while it waited for its target to be {}: {observed}.",
            asked.operation,
            limit.key,
            limit.value,
            limit.layer.as_str(),
            match step {
                Step::ReceivesEvents => "receiving the event",
                Step::Count => "matched as often as its count allows",
                other => other.as_str(),
            },
        ),
        &DeadlineExceededDetails {
            limit,
            elapsed,
            waited_for: step.waited_for(),
        },
    )
}
