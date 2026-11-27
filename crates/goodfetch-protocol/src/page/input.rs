//! Input with no target (ADR 0085): Playwright's `page.keyboard`,
//! `page.mouse` and `page.touchscreen`, for a canvas, a map or a key for
//! whatever has focus (pitfall B14). Each call is a Gesture, sent in its
//! page's turn, and passes no guard, since it names no element: Provenance
//! marks each one unguarded (ADR 0019). A point is in CSS pixels from the
//! top-left corner of the page's viewport. The page's keyboard and mouse
//! keep their state from one call to the next, as a person's do: the keys
//! and buttons held down, which every later input carries, and where the
//! mouse is. No call waits for a navigation it starts, as in Playwright.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::{ARRIVED_IN, Acted, PageId};
use crate::catalog::{Operation, OperationClass, OperationKind, Reach, Stability, schema_of};

/// What `page.keyboard.down` and `page.keyboard.up` take: the page, and one
/// key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyboardKeyParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// One key, by Playwright's key names, such as `Shift`, `ArrowLeft` or
    /// `a`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
}

/// What `page.keyboard.press` takes: the page, a key or a combination, and
/// how long each key stays down.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyboardPressParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// A key or a combination, such as `Enter`, `a` or `Shift+Tab`, by
    /// Playwright's key names.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub key: String,
    /// The milliseconds between its keys going down and coming up; none
    /// for no wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub delay: Option<u64>,
}

/// What `page.keyboard.type` takes: the page, the text, and how long each
/// key stays down.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyboardTypeParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The text, typed one character at a time.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
    /// The milliseconds each key stays down, or a character with no key
    /// waits before it is inserted; none for no wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub delay: Option<u64>,
}

/// What `page.keyboard.insert_text` takes: the page and the text.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct KeyboardInsertTextParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The text, inserted whole.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub text: String,
}

/// A mouse button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    /// The left button, the default.
    #[default]
    Left,
    /// The right button.
    Right,
    /// The middle button.
    Middle,
}

/// What `page.mouse.move` takes: the page, the point, and how many moves
/// the mouse makes to it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MouseMoveParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The point's distance from the viewport's left edge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub x: f64,
    /// The point's distance from the viewport's top edge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub y: f64,
    /// The moves, evenly spaced from where the mouse is to the point, the
    /// last on the point; none for one, as `0` is. At most 1000.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(max = 1000), extend("arrived_in" = "0.1"))]
    pub steps: Option<u32>,
}

/// What `page.mouse.down` and `page.mouse.up` take: the page, the button,
/// and which of a run of clicks it is.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MouseButtonParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The button; none for the left one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub button: Option<MouseButton>,
    /// Which of a run of clicks it is, from 1, as `UIEvent.detail`; none
    /// for 1.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1), extend("arrived_in" = "0.1"))]
    pub click_count: Option<u32>,
}

/// What `page.mouse.click` takes: the page, the point, the button, how many
/// clicks, and how long the button stays down.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MouseClickParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The point's distance from the viewport's left edge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub x: f64,
    /// The point's distance from the viewport's top edge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub y: f64,
    /// The button; none for the left one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub button: Option<MouseButton>,
    /// How many clicks, one after the other; none for 1. At most 10.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(range(min = 1, max = 10), extend("arrived_in" = "0.1"))]
    pub click_count: Option<u32>,
    /// The milliseconds the button stays down at each click, and between
    /// two clicks; none for no wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub delay: Option<u64>,
}

/// What `page.mouse.dblclick` takes: the page, the point, the button, and
/// how long the button stays down.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MouseDblclickParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The point's distance from the viewport's left edge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub x: f64,
    /// The point's distance from the viewport's top edge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub y: f64,
    /// The button; none for the left one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub button: Option<MouseButton>,
    /// The milliseconds the button stays down at each click, and between
    /// the two; none for no wait.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub delay: Option<u64>,
}

/// What `page.mouse.wheel` takes: the page, and how far the wheel scrolls.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct MouseWheelParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// How far it scrolls right, in CSS pixels; a negative value scrolls
    /// left.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub delta_x: f64,
    /// How far it scrolls down, in CSS pixels; a negative value scrolls
    /// up.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub delta_y: f64,
}

/// What `page.touchscreen.tap` takes: the page and the point.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TouchscreenTapParams {
    /// The page.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub page_id: PageId,
    /// The point's distance from the viewport's left edge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub x: f64,
    /// The point's distance from the viewport's top edge.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub y: f64,
}

/// An input operation: an Effect, a Gesture of its page, that answers
/// [`Acted`] once its input was sent.
const fn input(
    name: &'static str,
    description: &'static str,
    params: fn(&mut schemars::SchemaGenerator) -> schemars::Schema,
) -> Operation {
    Operation {
        name,
        description,
        class: OperationClass::Effect,
        kind: OperationKind::RoundTrip,
        reach: Reach::AllSurfaces,
        params,
        result: schema_of::<Acted>,
        events: &[],
        cancellable: true,
        input: None,
        bound: None,
        stability: Stability::Experimental,
        deprecated: None,
        arrived_in: ARRIVED_IN,
    }
}

/// `page.keyboard.down`.
pub const PAGE_KEYBOARD_DOWN: Operation = input(
    "page.keyboard.down",
    "Presses a key down, as a keyboard does, on whatever has focus, and holds it until `page.keyboard.up` releases it: a modifier held so is held for every later input. A key already down repeats. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<KeyboardKeyParams>,
);

/// `page.keyboard.up`.
pub const PAGE_KEYBOARD_UP: Operation = input(
    "page.keyboard.up",
    "Releases a key, as a keyboard does, on whatever has focus. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<KeyboardKeyParams>,
);

/// `page.keyboard.press`.
pub const PAGE_KEYBOARD_PRESS: Operation = input(
    "page.keyboard.press",
    "Presses a key or a combination, as a keyboard does, on whatever has focus: each key goes down in order, then comes up in the reverse order, with the keys `page.keyboard.down` holds. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<KeyboardPressParams>,
);

/// `page.keyboard.type`.
pub const PAGE_KEYBOARD_TYPE: Operation = input(
    "page.keyboard.type",
    "Types a text into whatever has focus, one character at a time: a character a US keyboard has is pressed as a key, a line break as `Enter`, and any other is inserted as an input method commits it. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<KeyboardTypeParams>,
);

/// `page.keyboard.insert_text`.
pub const PAGE_KEYBOARD_INSERT_TEXT: Operation = input(
    "page.keyboard.insert_text",
    "Inserts a text where the focused element's selection is, as an input method commits it: an `input` event, and no key event. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<KeyboardInsertTextParams>,
);

/// `page.mouse.move`.
pub const PAGE_MOUSE_MOVE: Operation = input(
    "page.mouse.move",
    "Moves the mouse to a point of the page's viewport, through `steps` moves, with the buttons and keys held down. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<MouseMoveParams>,
);

/// `page.mouse.down`.
pub const PAGE_MOUSE_DOWN: Operation = input(
    "page.mouse.down",
    "Presses a mouse button where the mouse is, and holds it until `page.mouse.up` releases it. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<MouseButtonParams>,
);

/// `page.mouse.up`.
pub const PAGE_MOUSE_UP: Operation = input(
    "page.mouse.up",
    "Releases a mouse button where the mouse is. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<MouseButtonParams>,
);

/// `page.mouse.click`.
pub const PAGE_MOUSE_CLICK: Operation = input(
    "page.mouse.click",
    "Moves the mouse to a point of the page's viewport and clicks there: the button goes down and up `click_count` times. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<MouseClickParams>,
);

/// `page.mouse.dblclick`.
pub const PAGE_MOUSE_DBLCLICK: Operation = input(
    "page.mouse.dblclick",
    "Moves the mouse to a point of the page's viewport and double-clicks there: two clicks, the second a `dblclick`. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<MouseDblclickParams>,
);

/// `page.mouse.wheel`.
pub const PAGE_MOUSE_WHEEL: Operation = input(
    "page.mouse.wheel",
    "Turns the mouse wheel where the mouse is: a `wheel` event, which scrolls what is under the mouse unless the page cancels it. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<MouseWheelParams>,
);

/// `page.touchscreen.tap`.
pub const PAGE_TOUCHSCREEN_TAP: Operation = input(
    "page.touchscreen.tap",
    "Taps a point of the page's viewport with one finger: a touch starts there and ends. Marked unguarded in Provenance: no element is checked (ADR 0085).",
    schema_of::<TouchscreenTapParams>,
);
