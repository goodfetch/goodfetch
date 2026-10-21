//! The guard (ADRs 0019 and 0085): a locator action waits until its
//! target is unique and actionable, acts with trusted input, and replays a
//! gesture that landed elsewhere, until its window, `timeout.action`,
//! passes. The conditions are uniqueness, visibility, enabled, stable and
//! receiving the event for a pointer action; visible and enabled for a text
//! action; and editable too for `fill`. Each check and the arming of the
//! interception listener are one round trip into the guard's isolated
//! world, and stability is waited for only while an animation or a
//! transition runs. The locator is resolved again at every check, so a
//! re-render that replaces its element changes nothing. At the deadline the
//! last observation decides: still ambiguous raises `target_not_unique`
//! with every match, anything else `deadline_exceeded` naming the step it
//! waited for.

mod keys;
mod target;

use std::time::Duration;

use goodfetch_cdp::generated::input::DispatchKeyEventParams;
use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_protocol::connection::{DEADLINE_EXCEEDED, DeadlineExceededDetails, Surface};
use goodfetch_protocol::jsonrpc::ProtocolFailure;
use goodfetch_protocol::page::{TARGET_NOT_UNIQUE, TargetMatch, TargetNotUniqueDetails};
use goodfetch_protocol::settings::SettingLimit;
use serde::Deserialize;
use serde_json::json;

use crate::driver::{PageDriver, Turn, Unanswered};

/// The check and arm, a function the guard's world calls.
const CHECK: &str = include_str!("guard/check.js");

/// The disarm, which says where the gesture landed.
const DISARM: &str = include_str!("guard/disarm.js");

/// The longest one check waits in the page before it answers, so that a
/// navigation that replaced the document, or the call's cancellation, is
/// seen within it.
const SLICE: Duration = Duration::from_secs(1);

/// The events of a click the interception listener checks.
const CLICK_EVENTS: [&str; 5] = ["pointerdown", "mousedown", "pointerup", "mouseup", "click"];

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
}

/// How long an action may wait: `timeout.action`'s value and where it was
/// set.
pub(crate) struct Window {
    /// The setting, its value and its layer.
    pub(crate) limit: SettingLimit,
    /// Its duration.
    pub(crate) duration: Duration,
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
}

/// What one check saw.
#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Seen {
    /// The target is ready: for a pointer action, armed, and the point to
    /// press.
    Ready { x: Option<f64>, y: Option<f64> },
    /// It is not, at the step `step`.
    Waiting {
        step: String,
        observed: String,
        #[serde(default)]
        matches: Vec<String>,
    },
    /// The selector is no CSS the page parses.
    Invalid { message: String },
}

/// Where a gesture landed.
#[derive(Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
enum Landed {
    Landed,
    Intercepted { observed: String },
}

/// Performs `request` on `page`, through the guard, timed by `clock`.
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
    let end = started + request.window.duration;
    let mut turn = page.turn(request.operation).await;
    let mut last = Seen::Waiting {
        step: "attached".to_owned(),
        observed: "the action waited its turn on the page".to_owned(),
        matches: Vec::new(),
    };
    loop {
        let remaining = end.saturating_duration_since(clock.now());
        if remaining.is_zero() {
            let elapsed = clock.now() - started;
            return Err(at_deadline(&request, last, elapsed));
        }
        let argument = json!({
            "selector": request.selector,
            "action": request.action.kind(),
            "events": CLICK_EVENTS,
            "slice": u64::try_from(remaining.min(SLICE).as_millis()).unwrap_or(u64::MAX),
        });
        let seen = match turn.call(CHECK, argument).await {
            Ok(value) => read::<Seen>(&value)?,
            Err(Unanswered::WorldGone) => continue,
            Err(Unanswered::Failed(error)) => return Err(error),
        };
        match seen {
            Seen::Ready { x, y } => match &request.action {
                Action::Click => {
                    turn.click(x.unwrap_or_default(), y.unwrap_or_default())
                        .await?;
                    match landed(&mut turn).await? {
                        Landed::Landed => return Ok(()),
                        Landed::Intercepted { observed } => {
                            last = Seen::Waiting {
                                step: "receives_events".to_owned(),
                                observed,
                                matches: Vec::new(),
                            };
                        }
                    }
                }
                Action::Fill(value) => return fill(&mut turn, value).await,
                Action::Press(events) => {
                    for event in events {
                        turn.key(event).await?;
                    }
                    return Ok(());
                }
            },
            Seen::Invalid { message } => {
                return Err(CallError::protocol(
                    ProtocolFailure::InvalidParams,
                    format!(
                        "`{}` is no CSS selector the page parses: {message}",
                        request.selector
                    ),
                ));
            }
            waiting @ Seen::Waiting { .. } => last = waiting,
        }
    }
}

/// Where the gesture just sent landed; a document gone with it means it
/// landed, since nothing was left to cancel it.
async fn landed(turn: &mut Turn<'_>) -> Result<Landed, CallError> {
    match turn.call(DISARM, serde_json::Value::Null).await {
        Ok(value) => read::<Landed>(&value),
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

/// The guard's answer as the type the script returns.
fn read<T: for<'de> Deserialize<'de>>(value: &serde_json::Value) -> Result<T, CallError> {
    T::deserialize(value).map_err(|error| {
        CallError::protocol(
            ProtocolFailure::InternalError,
            format!("the guard's script answered {value}, which does not read: {error}"),
        )
    })
}

/// The error of an action whose deadline passed with `last` its last
/// observation.
fn at_deadline(request: &Request<'_>, last: Seen, elapsed: Duration) -> CallError {
    let elapsed = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
    let target = target::render(request.surface, request.selector, None);
    let limit = request.window.limit.clone();
    let (step, observed, matches) = match last {
        Seen::Waiting {
            step,
            observed,
            matches,
        } => (step, observed, matches),
        Seen::Ready { .. } | Seen::Invalid { .. } => (
            "receives_events".to_owned(),
            "the target was ready".to_owned(),
            Vec::new(),
        ),
    };
    if step == "unique" {
        let matches: Vec<TargetMatch> = matches
            .into_iter()
            .enumerate()
            .map(|(index, element)| TargetMatch {
                element,
                locator: target::render(request.surface, request.selector, Some(index)),
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
                request.operation,
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
            request.operation,
            limit.key,
            limit.value,
            limit.layer.as_str(),
            match step.as_str() {
                "receives_events" => "receiving the event",
                other => other,
            },
        ),
        &DeadlineExceededDetails {
            limit,
            elapsed,
            waited_for: format!("target_{step}"),
        },
    )
}
