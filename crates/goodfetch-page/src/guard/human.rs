//! Human-like CDP input dispatch inside the guard's deadlines (ADR 0035;
//! spec #599, #1018): curved pointer approaches, wheel scrolling, press-and-
//! hold micro-tremors and key-by-key typing with corrected slips.

use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::clock::Clock;
use goodfetch_protocol::jsonrpc::ProtocolFailure;

use super::{Landed, landed};
use crate::cdp::driver::PageDriver;
use crate::cdp::{CdpPage, keys};
use crate::human_input::{KeyKind, PageHumanInput, Point, Viewport};
use crate::page_reads::Turn;

/// The most time a human-like approach move or wheel scroll takes of an
/// action's remaining deadline, keeping it well under the guard's 10 s arm
/// expiry while remaining natural.
const HUMAN_MOTION_CAP: Duration = Duration::from_secs(2);

/// Sleeps `delay` bounded by `end` on `clock`.
async fn pace(clock: &Clock, end: tokio::time::Instant, delay: Duration) {
    let wait = delay.min(end.saturating_duration_since(clock.now()));
    if !wait.is_zero() {
        clock.sleep(wait).await;
    }
}

/// Moves the pointer along a human-like curve onto `target`, presses and
/// releases as one atomic burst, then reads where the click landed.
pub(super) async fn click(
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
pub(super) async fn scroll(
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
pub(super) async fn press_and_hold(
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
pub(super) async fn fill(
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
