//! The guard's check script and the guard name each step alike, and
//! press-and-hold dispatches its full CDP sequence and releases on error.
#![expect(
    clippy::disallowed_methods,
    reason = "unit tests seed Entropy directly (ADR 0059)"
)]

use std::sync::Arc;
use std::time::Duration;

use goodfetch_counted_work::Row;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::entropy::Entropy;
use serde_json::{Value, json};
use tokio::sync::oneshot;

use super::check::{CHECK, Seen};
use super::{Action, Landed, human};
use crate::cdp::CdpPage;
use crate::cdp::driver::replay;
use crate::gesture_order::GestureOrder;
use crate::human_input::{PageHumanInput, Point, Shapes, Viewport};
use crate::live_run::played_page;
use crate::page_reads::Turn;
use crate::page_worlds::PageWorlds;

/// Every step `check.js` waits on reads as the guard's `Step` of that
/// name, so a step renamed in the script fails here rather than leave the
/// guard's own reading of it, such as the hiding lift at `visible`, unrun.
#[test]
fn every_step_the_check_script_waits_on_reads_as_the_guards() {
    let named: Vec<&str> = CHECK
        .split("waiting(\"")
        .skip(1)
        .filter_map(|rest| rest.split_once('"').map(|(step, _)| step))
        .collect();

    let unread: Vec<&str> = named
        .iter()
        .copied()
        .filter(|step| {
            !matches!(
                Seen::read(&json!({ "state": "waiting", "step": step, "observed": "" })),
                Ok(Seen::Waiting { step: read, .. }) if read.as_str() == *step
            )
        })
        .collect();

    assert_eq!((named.is_empty(), unread), (false, Vec::<&str>::new()));
}

/// The guard reads a step the script does not name as no step at all.
#[test]
fn a_step_the_guard_does_not_know_does_not_read() {
    let seen = Seen::read(&json!({ "state": "waiting", "step": "shown", "observed": "" }));

    assert!(seen.is_err());
}

/// Press-and-hold checks pointer readiness and routes its command counts
/// to `C3.commands.human_input` when human-like input is on.
#[test]
fn press_and_hold_checks_pointer_readiness_and_routes_human_input_counts() {
    let hold = Action::PressAndHold(Duration::from_millis(600));

    assert_eq!(hold.kind(), "pointer");
    assert_eq!(hold.commands(false), Row::ActionClickCommands);
    assert_eq!(hold.commands(true), Row::ActionHumanInputCommands);
}

/// Drives `human::press_and_hold` over a scripted CDP connection, returning
/// the sequence of `(event_type, buttons)` dispatched by `Input.dispatchMouseEvent`
/// and whether `press_and_hold` succeeded.
async fn run_press_and_hold(fail_drag: bool) -> (Result<Landed, String>, Vec<(String, i64)>) {
    let (driver, mut played, mut connection) = played_page();
    let (mut order, _asks) = GestureOrder::new();
    let (tx, rx) = oneshot::channel();
    order.ask(tx);
    let granted = rx.await.expect("gesture turn granted");
    let mut turn = Turn::granted(
        CdpPage::of(&driver),
        PageWorlds::default(),
        "locator.press_and_hold",
        granted,
    );
    let input = PageHumanInput::new(Entropy::seeded(42), Arc::new(Shapes::GhostCursor));
    let clock = Clock::system();
    let end = clock.now() + Duration::from_secs(5);
    let target = Point { x: 420.0, y: 260.0 };
    let viewport = Viewport {
        width: 1280.0,
        height: 720.0,
    };

    let mut events = Vec::new();
    let mut saw_down = false;
    let mut failed_once = false;
    let outcome = {
        let action = human::press_and_hold(
            &driver,
            &mut turn,
            Some(&input),
            (target, viewport, Duration::from_millis(350)),
            (&clock, end),
        );
        tokio::pin!(action);

        loop {
            tokio::select! {
                biased;
                res = &mut action => break res.map_err(|error| error.code().to_owned()),
                () = &mut connection => panic!("scripted page connection ended early"),
                command = played.command() => {
                    let method = command["method"].as_str().unwrap_or_default().to_owned();
                    match method.as_str() {
                        "Input.dispatchMouseEvent" => {
                            let kind = command["params"]["type"].as_str().unwrap().to_owned();
                            let buttons = command["params"]["buttons"].as_i64().unwrap_or(0);
                            if kind == "mousePressed" {
                                saw_down = true;
                            }
                            events.push((kind.clone(), buttons));
                            if fail_drag && saw_down && kind == "mouseMoved" && !failed_once {
                                failed_once = true;
                                played.send(&[json!({
                                    "id": command["id"],
                                    "sessionId": replay::SESSION,
                                    "error": { "code": -32000, "message": "drag failed" }
                                })]).await;
                            } else {
                                played.send(&[replay::answer(&command, json!({}))]).await;
                            }
                        }
                        "Page.createIsolatedWorld" => {
                            played.send(&[replay::answer(&command, json!({ "executionContextId": 1 }))]).await;
                        }
                        "Runtime.callFunctionOn" => {
                            played.send(&[replay::answer(
                                &command,
                                json!({ "result": { "type": "object", "value": { "state": "landed" } } }),
                            )]).await;
                        }
                        other => {
                            let _ = Value::String(other.to_owned());
                            played.send(&[replay::answer(&command, json!({}))]).await;
                        }
                    }
                }
            }
        }
    };
    drop((turn, driver, played));
    connection.await;
    (outcome, events)
}

/// Human-like press-and-hold sends approach moves, `mousePressed`, dragged
/// micro-tremors with the left button held (`buttons == 1`), and
/// `mouseReleased`; if a drag fails mid-hold, best-effort `mouseReleased`
/// still goes out so no button stays stuck down.
#[test]
fn press_and_hold_dispatches_approach_down_tremors_and_releases_on_error() {
    let runtime = goodfetch_test_support::runtime();
    let (ok, events) = runtime.block_on(run_press_and_hold(false));
    assert!(matches!(ok, Ok(Landed::Landed)), "{ok:?}");
    let down_pos = events
        .iter()
        .position(|(kind, _)| kind == "mousePressed")
        .expect("mousePressed sent");
    let up_pos = events
        .iter()
        .position(|(kind, _)| kind == "mouseReleased")
        .expect("mouseReleased sent");
    assert!(
        down_pos >= 1,
        "expected approach moves before press: {events:?}"
    );
    assert!(
        up_pos > down_pos,
        "expected release after press: {events:?}"
    );
    assert!(
        events[..down_pos]
            .iter()
            .all(|(kind, _)| kind == "mouseMoved"),
        "expected only mouseMoved before press: {events:?}"
    );
    assert!(
        !events[down_pos + 1..up_pos].is_empty()
            && events[down_pos + 1..up_pos]
                .iter()
                .all(|(kind, buttons)| kind == "mouseMoved" && *buttons == 1),
        "expected dragged tremors with buttons=1 during hold: {events:?}"
    );

    let (failed, failed_events) = runtime.block_on(run_press_and_hold(true));
    assert!(failed.is_err(), "expected drag error to surface");
    assert_eq!(
        failed_events.last().map(|(kind, _)| kind.as_str()),
        Some("mouseReleased"),
        "expected best-effort mouseReleased after drag error: {failed_events:?}"
    );
}
