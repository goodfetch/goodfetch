//! The `canvas` family: a button drawn on a `<canvas>` (`/button`), which
//! no DOM selector reaches since it has no element, role or text, beside
//! a decoy drawn the same way; and a drawing surface (`/surface`) a tool
//! draws on with the mouse and reads back through `evaluate`. The pages'
//! script reports each click (on the button, the decoy or elsewhere on the
//! canvas, a miss) and each stroke to the site, with the browser's
//! `isTrusted`, which the oracle judges from.

use std::collections::BTreeMap;

use super::document;
use crate::exchange::Reply;
use crate::generator;
use crate::task::{self, Effect, Entry, Expected, Kind, Record, Task};

pub(super) const HOST: &str = "canvas.test";
pub(super) const FAMILY: &str = "canvas";
pub(super) const TASKS: u32 = 2;

/// The canvas's size, in CSS pixels, which its backing store matches.
const WIDTH: u64 = 480;
const HEIGHT: u64 = 320;

/// Where the button page pins its canvas, in CSS pixels from the page's
/// top-left corner, below its heading, so the Fake model knows where a
/// screenshot shows the button.
const CANVAS_LEFT: u64 = 16;
const CANVAS_TOP: u64 = 160;

/// A drawn button's size.
const BUTTON_WIDTH: u64 = 128;
const BUTTON_HEIGHT: u64 = 48;

/// The margin a drawn button keeps from the canvas's edges and middle.
const MARGIN: u64 = 24;

/// The side of a cell of the surface's grid, in which a stroke's ends are
/// reported, so a stroke a pixel or two off its point still ends in it.
const CELL: u64 = 10;

/// The generator places of the family's values.
const FAMILY_PLACE: u64 = 22;
const SIDE_PLACE: u64 = 1;
const LEFT_PLACE: u64 = 2;
const TOP_PLACE: u64 = 3;
const LABEL_PLACE: u64 = 4;
const INK_PLACE: u64 = 5;
const POINT_PLACE: u64 = 6;

const LABELS: [&str; 8] = [
    "Accept", "Begin", "Confirm", "Continue", "Enter", "Proceed", "Start", "Submit",
];

/// The surface's inks, each as `#rrggbb`.
const INKS: [&str; 6] = [
    "#b91c1c", "#1d4ed8", "#047857", "#7c3aed", "#c2410c", "#0f766e",
];

const SCRIPT: &str = include_str!("../../pages/browser/canvas.js");

/// A rectangle drawn on the canvas, in CSS pixels from its top-left corner.
struct Drawn {
    left: u64,
    top: u64,
    label: &'static str,
}

/// The button and the decoy for `seed`, each in its own half of the canvas.
fn scene(seed: u64) -> (Drawn, Drawn) {
    let draw = |place, bound| generator::below(seed, &[FAMILY_PLACE, place], bound);
    let half = WIDTH / 2;
    let spread = half - 2 * MARGIN - BUTTON_WIDTH;
    let place = |left_of_half: u64, which: u64| Drawn {
        left: left_of_half + MARGIN + draw(LEFT_PLACE + 10 * which, spread),
        top: MARGIN + draw(TOP_PLACE + 10 * which, HEIGHT - 2 * MARGIN - BUTTON_HEIGHT),
        label: "",
    };
    let button_side = draw(SIDE_PLACE, 2);
    let labels = generator::distinct(seed, &[FAMILY_PLACE, LABEL_PLACE], LABELS.len(), 2);
    let button = Drawn {
        label: LABELS[labels[0]],
        ..place(button_side * half, 0)
    };
    let decoy = Drawn {
        label: LABELS[labels[1]],
        ..place((1 - button_side) * half, 1)
    };
    (button, decoy)
}

/// The surface's ink for `seed`.
fn ink(seed: u64) -> &'static str {
    let index = generator::below(seed, &[FAMILY_PLACE, INK_PLACE], INKS.len() as u64);
    INKS[usize::try_from(index).expect("invariant: an ink index is below 6")]
}

/// The stroke's two points for `seed`, each the middle of a grid cell: one
/// in the canvas's left half and one in its right.
fn points(seed: u64) -> ((u64, u64), (u64, u64)) {
    let cell = |place, from: u64, cells: u64| {
        (from + generator::below(seed, &[FAMILY_PLACE, POINT_PLACE, place], cells)) * CELL
            + CELL / 2
    };
    let rows = HEIGHT / CELL - 6;
    let start = (cell(1, 3, WIDTH / CELL / 2 - 6), cell(2, 3, rows));
    let end = (
        cell(3, WIDTH / CELL / 2 + 3, WIDTH / CELL / 2 - 6),
        cell(4, 3, rows),
    );
    (start, end)
}

/// A rectangle's place as the script reads it: left, top, width, height.
fn geometry(drawn: &Drawn) -> String {
    format!(
        "{} {} {BUTTON_WIDTH} {BUTTON_HEIGHT}",
        drawn.left, drawn.top
    )
}

/// A label as the script reads it, its characters' code points, so no
/// text of the page holds it.
fn code_points(label: &str) -> String {
    label
        .chars()
        .map(|character| u32::from(character).to_string())
        .collect::<Vec<_>>()
        .join(" ")
}

/// A `GET` of `path` on task `ordinal`'s host.
pub(super) fn page(seed: u64, ordinal: u32, path: &str) -> Option<Reply> {
    match (ordinal, path) {
        (1, "/button") => {
            let (button, decoy) = scene(seed);
            Some(Reply::html(document(
                "Canvas",
                &format!(
                    "<style>#canvas {{ position: absolute; left: {CANVAS_LEFT}px; top: {CANVAS_TOP}px; }}</style>\n"
                ),
                &format!(
                    "<h1>Canvas</h1>\n<p>Presses: <output id=\"presses\">0</output></p>\n<canvas id=\"canvas\" width=\"{WIDTH}\" height=\"{HEIGHT}\"></canvas>\n"
                ),
                &format!(
                    "<script data-button=\"{}\" data-button-label=\"{}\" data-decoy=\"{}\" data-decoy-label=\"{}\">\n{SCRIPT}</script>\n",
                    geometry(&button),
                    code_points(button.label),
                    geometry(&decoy),
                    code_points(decoy.label),
                ),
            )))
        }
        (2, "/surface") => {
            let ((x1, y1), (x2, y2)) = points(seed);
            Some(Reply::html(document(
                "Drawing surface",
                "<style>canvas { touch-action: none; }</style>\n",
                &format!(
                    "<h1>Drawing surface</h1>\n<p id=\"stroke\">Draw one stroke from <span class=\"start\">({x1}, {y1})</span> to <span class=\"end\">({x2}, {y2})</span>, in CSS pixels from the surface's top-left corner.</p>\n<p>Strokes: <output id=\"strokes\">0</output></p>\n<canvas id=\"surface\" width=\"{WIDTH}\" height=\"{HEIGHT}\"></canvas>\n"
                ),
                &format!(
                    "<script data-ink=\"{}\" data-cell=\"{CELL}\">\n{SCRIPT}</script>\n",
                    ink(seed)
                ),
            )))
        }
        _ => None,
    }
}

/// Where the vision rung clicks on task `ordinal`'s page: the middle of the
/// drawn button in the page's CSS pixels; `None` for the surface, which
/// draws nothing to point at.
pub(super) fn point(seed: u64, ordinal: u32) -> Option<(u64, u64)> {
    (ordinal == 1).then(|| {
        let (button, _) = scene(seed);
        (
            CANVAS_LEFT + button.left + BUTTON_WIDTH / 2,
            CANVAS_TOP + button.top + BUTTON_HEIGHT / 2,
        )
    })
}

/// The family's tasks for `seed`. Ordinals are append-only (ADR 0063).
pub(super) fn tasks(seed: u64, port: Option<u16>) -> Vec<Task> {
    let ((x1, y1), (x2, y2)) = points(seed);
    let cell = |x: u64, y: u64| format!("{},{}", x / CELL, y / CELL);
    let ink = ink(seed).to_owned();
    let entry = |ordinal: u32, path: &str, description: &'static str| Entry {
        id: format!("{FAMILY}/{ordinal}"),
        family: FAMILY,
        kind: Kind::Act,
        start_url: format!(
            "{}{path}",
            task::origin(&task::task_host(ordinal, HOST), port)
        ),
        description,
        account: None,
        time_steps: None,
    };
    vec![
        Task {
            host: task::task_host(1, HOST),
            entry: entry(
                1,
                "/button",
                "Click, once, the button this page's canvas asks for: the canvas draws two \
                 buttons and, above them, the label of the one to press. Both are pixels on the \
                 canvas, with no element, role or text a DOM selector could reach, and a click \
                 anywhere on the canvas but on that button is a miss.",
            ),
            expected: Expected::effects(vec![Effect::Event {
                event: "click".to_owned(),
                target: "button".to_owned(),
                trusted: true,
                details: BTreeMap::new(),
            }]),
        },
        Task {
            host: task::task_host(2, HOST),
            entry: entry(
                2,
                "/surface",
                "Draw with the mouse the one stroke this page asks for on its drawing surface, \
                 between the two points it names, then read the surface back through \
                 `evaluate` and return one Record with the fields `id` (`stroke`), `start` and \
                 `end`: the colour of the canvas's pixel at each of the two points, as \
                 `#rrggbb`.",
            ),
            expected: Expected {
                records: vec![Record::from([
                    ("id", "stroke".to_owned()),
                    ("start", ink.clone()),
                    ("end", ink),
                ])],
                effects: vec![Effect::Event {
                    event: "stroke".to_owned(),
                    target: "surface".to_owned(),
                    trusted: true,
                    details: BTreeMap::from([
                        ("from".to_owned(), cell(x1, y1)),
                        ("to".to_owned(), cell(x2, y2)),
                    ]),
                }],
                ..Expected::default()
            },
        },
    ]
}
