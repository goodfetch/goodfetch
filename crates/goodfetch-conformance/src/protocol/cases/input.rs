//! Input with no target (ADR 0085): `page.mouse`, `page.keyboard` and
//! `page.touchscreen`, Page API cases on the Proving ground's canvas
//! drawing surface (`canvas/2`) and a blank page. `page.mouse` draws a
//! stroke on the canvas, `page.evaluate` reads the drawn pixels back, the
//! oracle verifies both the trusted stroke and the read-back colours, and
//! every `page.mouse.*`, `page.keyboard.*` and `page.touchscreen.tap` call
//! is marked unguarded in Provenance.

use std::time::Duration;

use goodfetch_protocol::journey::TaskId;
use goodfetch_protocol::page::functions::PAGE_EVALUATE;
use goodfetch_protocol::page::input::{
    PAGE_KEYBOARD_DOWN, PAGE_KEYBOARD_INSERT_TEXT, PAGE_KEYBOARD_PRESS, PAGE_KEYBOARD_TYPE,
    PAGE_KEYBOARD_UP, PAGE_MOUSE_CLICK, PAGE_MOUSE_DBLCLICK, PAGE_MOUSE_DOWN, PAGE_MOUSE_MOVE,
    PAGE_MOUSE_UP, PAGE_MOUSE_WHEEL, PAGE_TOUCHSCREEN_TAP,
};
use goodfetch_protocol::page::{PAGE_GOTO, PageId};
use goodfetch_protocol::result_document::OracleOutcome;
use serde_json::{Value, json};

use super::{Checks, Site, finish_run, new_page, poll, run_trace, start_run};
use crate::protocol::connection::{Connection, Gone};
use crate::protocol::target::EngineTarget;
use crate::proving_ground::Submission;

/// How long the oracle is asked again for the canvas stroke Effect posted
/// after `pointerup`.
const EFFECT_WAIT: Duration = Duration::from_secs(10);

/// How long a case lets an action wait: 10 s.
const LONG_WAIT: u64 = 10_000;

/// The Provenance action marking input with no target.
const UNGUARDED_INPUT: &str = "unguarded_input";

/// Reads the canvas's viewport origin and the requested stroke's start and
/// end points in canvas-local CSS pixels from `canvas/2`'s `/surface`.
const SURFACE_GEOMETRY: &str = "() => {
  const surface = document.getElementById('surface');
  const rect = surface.getBoundingClientRect();
  const parse = (selector) => {
    const text = document.querySelector(selector).textContent;
    const match = text.match(/\\((\\d+),\\s*(\\d+)\\)/);
    return [Number(match[1]), Number(match[2])];
  };
  return {
    left: rect.left,
    top: rect.top,
    start: parse('#stroke .start'),
    end: parse('#stroke .end'),
  };
}";

/// Reads the `#rrggbb` colour of the canvas's pixel at `[start, end]`.
const READ_PIXELS: &str = "(points) => {
  const context = document.getElementById('surface').getContext('2d');
  const hex = ([x, y]) => {
    const [r, g, b] = context.getImageData(x, y, 1, 1).data;
    return '#' + [r, g, b].map((c) => c.toString(16).padStart(2, '0')).join('');
  };
  return { start: hex(points.start), end: hex(points.end) };
}";

/// Installs listeners recording trusted `click`, `dblclick` and `wheel`
/// events on the document.
const LISTEN_MOUSE: &str = "() => {
  window.__mouseLog = { clicks: [], dblclicks: [], wheels: [] };
  document.addEventListener('click', (event) => {
    window.__mouseLog.clicks.push({ trusted: event.isTrusted, x: event.clientX, y: event.clientY });
  });
  document.addEventListener('dblclick', (event) => {
    window.__mouseLog.dblclicks.push({ trusted: event.isTrusted, x: event.clientX, y: event.clientY });
  });
  document.addEventListener('wheel', (event) => {
    window.__mouseLog.wheels.push({ trusted: event.isTrusted, dx: event.deltaX, dy: event.deltaY });
  }, { passive: false });
}";

/// Sets up a focused input and a touch pad recording trusted key and touch
/// events.
const KEYBOARD_AND_TOUCH_PAGE: &str = "() => {
  document.body.innerHTML = '<input id=\"field\"><div id=\"pad\" style=\"position:fixed;left:40px;top:40px;width:120px;height:120px;\"></div>';
  window.__inputLog = { keydowns: [], keyups: [], touchstarts: [], touchends: [] };
  const field = document.getElementById('field');
  field.addEventListener('keydown', (event) => {
    window.__inputLog.keydowns.push({ key: event.key, trusted: event.isTrusted });
  });
  field.addEventListener('keyup', (event) => {
    window.__inputLog.keyups.push({ key: event.key, trusted: event.isTrusted });
  });
  const pad = document.getElementById('pad');
  pad.addEventListener('touchstart', (event) => {
    const touch = event.touches[0];
    window.__inputLog.touchstarts.push({ trusted: event.isTrusted, x: touch.clientX, y: touch.clientY });
  });
  pad.addEventListener('touchend', (event) => {
    window.__inputLog.touchends.push({ trusted: event.isTrusted });
  });
  field.focus();
}";

/// A case of input with no target, by its ID in the page suite's case list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Case {
    MouseDrawsOnCanvas,
    KeyboardAndTouchscreenUnguarded,
}

impl Case {
    /// The case `id` names, if it is one of these lines.
    pub(super) fn from_id(id: &str) -> Option<Self> {
        Some(match id {
            "mouse-draws-on-the-canvas-surface-and-evaluate-reads-the-pixels-back" => {
                Self::MouseDrawsOnCanvas
            }
            "keyboard-and-touchscreen-send-trusted-input-and-provenance-marks-each-unguarded" => {
                Self::KeyboardAndTouchscreenUnguarded
            }
            _ => return None,
        })
    }

    /// Runs the case on the open Connection against `site`.
    pub(super) fn run(
        self,
        connection: &mut Connection,
        target: &EngineTarget,
        site: &Site,
        checks: &mut Checks,
    ) -> Result<(), Gone> {
        match self {
            Self::MouseDrawsOnCanvas => mouse_draws_on_canvas(connection, target, site, checks),
            Self::KeyboardAndTouchscreenUnguarded => {
                keyboard_and_touchscreen(connection, target, site, checks)
            }
        }
    }
}

/// Calls an input operation and checks that it answered `{}`.
fn send_input(
    connection: &mut Connection,
    operation: &str,
    params: &Value,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let answer = connection.call(operation, params)?;
    checks.check(
        format!("`{operation}` answers an empty result"),
        answer["result"] == json!({}),
    );
    Ok(())
}

/// The `operation` names of Provenance entries that marked an unguarded
/// input on `page_id` in `run_id`.
fn unguarded_operations(
    target: &EngineTarget,
    run_id: &str,
    page_id: &PageId,
    checks: &mut Checks,
) -> Vec<String> {
    checks
        .run_trace(run_trace::provenance(target, run_id))
        .into_iter()
        .filter(|entry| entry["action"] == UNGUARDED_INPUT && entry["page_id"] == page_id.0)
        .filter_map(|entry| entry["operation"].as_str().map(str::to_owned))
        .collect()
}

/// Draws the requested stroke on `canvas/2`'s `/surface`, reads back the
/// pixel colours at its ends via `page.evaluate`, and judges the task
/// against the Proving ground's oracle.
fn draw_canvas_stroke(
    connection: &mut Connection,
    site: &Site,
    task: &str,
    page_id: &PageId,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let geometry = connection.call(
        PAGE_EVALUATE.name,
        &json!({ "page_id": page_id, "function": SURFACE_GEOMETRY, "world": "main" }),
    )?;
    let value = &geometry["result"]["value"];
    let (left, top) = (
        value["left"].as_f64().unwrap_or(0.0),
        value["top"].as_f64().unwrap_or(0.0),
    );
    let (sx, sy) = (
        value["start"][0].as_f64().unwrap_or(0.0),
        value["start"][1].as_f64().unwrap_or(0.0),
    );
    let (ex, ey) = (
        value["end"][0].as_f64().unwrap_or(0.0),
        value["end"][1].as_f64().unwrap_or(0.0),
    );
    send_input(
        connection,
        PAGE_MOUSE_MOVE.name,
        &json!({ "page_id": page_id, "x": left + sx, "y": top + sy }),
        checks,
    )?;
    send_input(
        connection,
        PAGE_MOUSE_DOWN.name,
        &json!({ "page_id": page_id, "button": "left" }),
        checks,
    )?;
    send_input(
        connection,
        PAGE_MOUSE_MOVE.name,
        &json!({ "page_id": page_id, "x": left + ex, "y": top + ey, "steps": 6 }),
        checks,
    )?;
    send_input(
        connection,
        PAGE_MOUSE_UP.name,
        &json!({ "page_id": page_id, "button": "left" }),
        checks,
    )?;
    let pixels = connection.call(
        PAGE_EVALUATE.name,
        &json!({
            "page_id": page_id,
            "function": READ_PIXELS,
            "arg": { "start": [sx, sy], "end": [ex, ey] },
            "world": "main",
        }),
    )?;
    let colours = &pixels["result"]["value"];
    let record = json!({
        "id": "stroke",
        "start": colours["start"],
        "end": colours["end"],
    });
    let id = TaskId::try_from(task.to_owned()).expect("invariant: valid task ID");
    let verified = poll::try_until(
        EFFECT_WAIT,
        Duration::from_millis(100),
        || {
            site.proving_ground
                .judge(&Submission {
                    task: &id,
                    status: "success",
                    records: std::slice::from_ref(&record),
                })
                .map(|answer| answer.outcome)
        },
        |outcome| *outcome == OracleOutcome::Verified,
    )
    .map_err(|why| {
        site.unread(format!(
            "the Proving ground's oracle judges `{task}` ({why})"
        ));
    })
    .ok();
    checks.check(
        "the oracle verifies the trusted mouse stroke and the read-back pixel colours",
        verified == Some(OracleOutcome::Verified),
    );
    Ok(())
}

/// Exercises `page.mouse.click`, `page.mouse.dblclick` and
/// `page.mouse.wheel` and checks that each delivers a trusted DOM event.
fn exercise_click_dblclick_wheel(
    connection: &mut Connection,
    page_id: &PageId,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let listened = connection.call(
        PAGE_EVALUATE.name,
        &json!({ "page_id": page_id, "function": LISTEN_MOUSE, "world": "main" }),
    )?;
    checks.check(
        "mouse event listeners are installed",
        listened.get("result").is_some(),
    );
    send_input(
        connection,
        PAGE_MOUSE_CLICK.name,
        &json!({ "page_id": page_id, "x": 12.0, "y": 14.0 }),
        checks,
    )?;
    send_input(
        connection,
        PAGE_MOUSE_DBLCLICK.name,
        &json!({ "page_id": page_id, "x": 16.0, "y": 18.0 }),
        checks,
    )?;
    send_input(
        connection,
        PAGE_MOUSE_WHEEL.name,
        &json!({ "page_id": page_id, "delta_x": 0.0, "delta_y": 24.0 }),
        checks,
    )?;
    let log = connection.call(
        PAGE_EVALUATE.name,
        &json!({ "page_id": page_id, "function": "() => window.__mouseLog", "world": "main" }),
    )?;
    let log = &log["result"]["value"];
    checks.check(
        "`page.mouse.click` delivers a trusted click",
        log["clicks"]
            .as_array()
            .is_some_and(|clicks| clicks.iter().any(|event| event["trusted"] == true)),
    );
    checks.check(
        "`page.mouse.dblclick` delivers a trusted dblclick",
        log["dblclicks"]
            .as_array()
            .is_some_and(|dbls| dbls.iter().any(|event| event["trusted"] == true)),
    );
    checks.check(
        "`page.mouse.wheel` delivers a trusted wheel",
        log["wheels"].as_array().is_some_and(|wheels| {
            wheels
                .iter()
                .any(|event| event["trusted"] == true && event["dy"].as_f64() == Some(24.0))
        }),
    );
    Ok(())
}

/// `page.mouse` draws on the Proving ground's canvas page (`canvas/2`),
/// `page.evaluate` reads the drawn pixels back, the oracle verifies both
/// the trusted stroke and the read-back colours, `click`/`dblclick`/`wheel`
/// deliver trusted events, and Provenance marks each call unguarded.
fn mouse_draws_on_canvas(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let (task, run_id) = ("canvas/2", "conformance-input-mouse-canvas");
    let Some(url) = site.start_url(task).map(str::to_owned) else {
        checks.check(
            format!("the Proving ground serves the task `{task}`"),
            false,
        );
        return Ok(());
    };
    start_run(
        connection,
        site,
        run_id,
        &json!({ "timeout": { "action": LONG_WAIT } }),
        checks,
    )?;
    let page_id = if let Some(page) = new_page(connection, run_id, checks)? {
        let page_id = page.page_id;
        let went = connection.call(PAGE_GOTO.name, &json!({ "page_id": page_id, "url": url }))?;
        checks.check(
            format!("`{}` reaches `{task}`", PAGE_GOTO.name),
            went["result"]["response"]["status"] == 200,
        );
        draw_canvas_stroke(connection, site, task, &page_id, checks)?;
        exercise_click_dblclick_wheel(connection, &page_id, checks)?;
        Some(page_id)
    } else {
        None
    };
    finish_run(connection, run_id)?;
    if let Some(page_id) = page_id {
        let recorded = unguarded_operations(target, run_id, &page_id, checks);
        checks.check(
            "Provenance marks every `page.mouse.*` call unguarded",
            recorded
                == [
                    PAGE_MOUSE_MOVE.name,
                    PAGE_MOUSE_DOWN.name,
                    PAGE_MOUSE_MOVE.name,
                    PAGE_MOUSE_UP.name,
                    PAGE_MOUSE_CLICK.name,
                    PAGE_MOUSE_DBLCLICK.name,
                    PAGE_MOUSE_WHEEL.name,
                ],
        );
    }
    Ok(())
}

/// Sends keyboard and touchscreen inputs to the prepared page and verifies
/// the resulting field value and trusted DOM events.
fn exercise_keyboard_and_touchscreen(
    connection: &mut Connection,
    page_id: &PageId,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let setup = connection.call(
        PAGE_EVALUATE.name,
        &json!({ "page_id": page_id, "function": KEYBOARD_AND_TOUCH_PAGE, "world": "main" }),
    )?;
    checks.check(
        "the input and touch pad page is prepared",
        setup.get("result").is_some(),
    );
    for (operation, params) in [
        (
            PAGE_KEYBOARD_DOWN.name,
            json!({ "page_id": page_id, "key": "Shift" }),
        ),
        (
            PAGE_KEYBOARD_PRESS.name,
            json!({ "page_id": page_id, "key": "a" }),
        ),
        (
            PAGE_KEYBOARD_UP.name,
            json!({ "page_id": page_id, "key": "Shift" }),
        ),
        (
            PAGE_KEYBOARD_TYPE.name,
            json!({ "page_id": page_id, "text": "bc!" }),
        ),
        (
            PAGE_KEYBOARD_INSERT_TEXT.name,
            json!({ "page_id": page_id, "text": "é" }),
        ),
        (
            PAGE_TOUCHSCREEN_TAP.name,
            json!({ "page_id": page_id, "x": 100.0, "y": 100.0 }),
        ),
    ] {
        send_input(connection, operation, &params, checks)?;
    }
    let observed = connection.call(
        PAGE_EVALUATE.name,
        &json!({
            "page_id": page_id,
            "function": "() => ({ value: document.getElementById('field').value, log: window.__inputLog })",
            "world": "main",
        }),
    )?;
    let observed = &observed["result"]["value"];
    checks.check(
        "keyboard calls type into the focused field with trusted key events",
        observed["value"] == "Abc!é"
            && observed["log"]["keydowns"].as_array().is_some_and(|keys| {
                !keys.is_empty() && keys.iter().all(|event| event["trusted"] == true)
            })
            && observed["log"]["keyups"].as_array().is_some_and(|keys| {
                !keys.is_empty() && keys.iter().all(|event| event["trusted"] == true)
            }),
    );
    checks.check(
        "`page.touchscreen.tap` fires trusted `touchstart` and `touchend` at the tapped point",
        observed["log"]["touchstarts"]
            .as_array()
            .is_some_and(|starts| {
                starts.len() == 1
                    && starts[0]["trusted"] == true
                    && starts[0]["x"] == 100
                    && starts[0]["y"] == 100
            })
            && observed["log"]["touchends"]
                .as_array()
                .is_some_and(|ends| ends.len() == 1 && ends[0]["trusted"] == true),
    );
    Ok(())
}

/// `page.keyboard` and `page.touchscreen` send trusted input to the page
/// and Provenance marks every call unguarded.
fn keyboard_and_touchscreen(
    connection: &mut Connection,
    target: &EngineTarget,
    site: &Site,
    checks: &mut Checks,
) -> Result<(), Gone> {
    let run_id = "conformance-input-keyboard-touchscreen";
    start_run(
        connection,
        site,
        run_id,
        &json!({ "timeout": { "action": LONG_WAIT } }),
        checks,
    )?;
    let page_id = if let Some(page) = new_page(connection, run_id, checks)? {
        let page_id = page.page_id;
        exercise_keyboard_and_touchscreen(connection, &page_id, checks)?;
        Some(page_id)
    } else {
        None
    };
    finish_run(connection, run_id)?;
    if let Some(page_id) = page_id {
        let recorded = unguarded_operations(target, run_id, &page_id, checks);
        checks.check(
            "Provenance marks every `page.keyboard.*` and `page.touchscreen.tap` call unguarded",
            recorded
                == [
                    PAGE_KEYBOARD_DOWN.name,
                    PAGE_KEYBOARD_PRESS.name,
                    PAGE_KEYBOARD_UP.name,
                    PAGE_KEYBOARD_TYPE.name,
                    PAGE_KEYBOARD_INSERT_TEXT.name,
                    PAGE_TOUCHSCREEN_TAP.name,
                ],
        );
    }
    Ok(())
}
