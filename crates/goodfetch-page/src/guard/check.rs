//! The guard's check and arm as the engine sends and reads it: the script
//! the guard's world calls, the argument it takes, and what it answers
//! (ADR 0019). Pure, so the check-and-arm round trip's benchmark builds the
//! very command the guard sends (ADR 0071).

use std::time::Duration;

use goodfetch_protocol::connection::WaitedFor;
use goodfetch_protocol::page::waits::ElementState;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

/// The check and arm, a function the guard's world calls.
pub(crate) const CHECK: &str = include_str!("check.js");

/// The events of a click the interception listener checks.
const CLICK_EVENTS: [&str; 5] = ["pointerdown", "mousedown", "pointerup", "mouseup", "click"];

/// What a check looks at.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Target<'a> {
    /// A CSS selector's one match.
    Selector(&'a str),
    /// The element another engine script put in the world's registry of
    /// guard targets under this handle.
    Handle(u64),
}

/// The check's argument: its target, the kind of action, which decides the
/// conditions (`pointer`, `text`, `fill` or `scroll`), how long it may wait
/// in the page before it answers, and whether an off-screen target should
/// ask the engine to scroll with wheel notches instead of teleporting into
/// view.
pub(crate) fn argument(
    target: Target<'_>,
    kind: &str,
    slice: Duration,
    human_scroll: bool,
) -> Value {
    let mut argument = json!({
        "action": kind,
        "events": CLICK_EVENTS,
        "slice": u64::try_from(slice.as_millis()).unwrap_or(u64::MAX),
    });
    if human_scroll {
        argument["human_scroll"] = json!(true);
    }
    match target {
        Target::Selector(selector) => argument["selector"] = json!(selector),
        Target::Handle(handle) => argument["handle"] = json!(handle),
    }
    argument
}

/// A locator wait's check's argument: the selector, the state its
/// matches are waited for in, `attached`, `detached`, `visible`, `hidden`
/// or `count` within `min` and `max`, and how long it may wait in the page
/// before it answers.
pub(crate) fn wait_argument(
    selector: &str,
    state: &str,
    (min, max): (Option<u64>, Option<u64>),
    slice: Duration,
) -> Value {
    let mut argument = json!({
        "selector": selector,
        "action": state,
        "slice": u64::try_from(slice.as_millis()).unwrap_or(u64::MAX),
    });
    if let Some(min) = min {
        argument["min"] = json!(min);
    }
    if let Some(max) = max {
        argument["max"] = json!(max);
    }
    argument
}

const fn default_width() -> f64 {
    1280.0
}

const fn default_height() -> f64 {
    720.0
}

/// What one check saw.
#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum Seen {
    /// A pointer action's target is ready, the interception listener
    /// armed, at the point to press.
    Armed {
        /// The point's distance from the viewport's left, in CSS pixels.
        x: f64,
        /// Its distance from the viewport's top.
        y: f64,
        /// The viewport's width, in CSS pixels.
        #[serde(default = "default_width")]
        width: f64,
        /// The viewport's height, in CSS pixels.
        #[serde(default = "default_height")]
        height: f64,
    },
    /// A pointer action's target lies outside the viewport under
    /// human-like input: scroll by `(dx, dy)` using wheel notches first.
    Scroll {
        dx: f64,
        dy: f64,
        #[serde(default = "default_width")]
        width: f64,
        #[serde(default = "default_height")]
        height: f64,
    },
    /// A text action's target is ready and has the focus, its text
    /// selected for `fill`.
    Focused {
        /// For `fill`, whether the target is an input whose value a
        /// person picks rather than types, which the guard sets.
        #[serde(default)]
        set_value: bool,
        /// For `fill`, whether the target is a password input
        /// (`<input type="password">`), read from the same check with no
        /// extra CDP command (ADR 0006, ticket #2447).
        #[serde(default)]
        password: bool,
        /// The URL of the frame that owns the focused element, read from the
        /// same check with no extra CDP command so a Secret placeholder checks
        /// its bound domains against the target frame's origin (ADR 0092).
        #[serde(default)]
        frame_url: String,
    },
    /// A scroll's target, the last match, is visible and was scrolled to
    /// the top of the view.
    Scrolled,
    /// A locator wait's matches are in the state it waits for.
    Ready {
        /// For `count`, how many elements matched.
        count: Option<u64>,
    },
    /// It is not, at the step `step`.
    Waiting {
        /// The condition that did not hold, such as `visible` or `unique`.
        step: Step,
        /// What the page showed instead.
        observed: String,
        /// Every element that matched, when there were several.
        #[serde(default)]
        matches: Vec<String>,
    },
    /// The selector is no CSS the page parses.
    Invalid {
        /// The page's own words.
        message: String,
    },
}

impl Seen {
    /// A wait at `step`, the page showing `observed`, listing no matches.
    pub(crate) fn waiting(step: Step, observed: impl Into<String>) -> Self {
        Self::Waiting {
            step,
            observed: observed.into(),
            matches: Vec::new(),
        }
    }

    /// The check's answer, `value`, as what it saw.
    pub(crate) fn read(value: &Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

/// A condition the check waits on, by the name `check.js` gives it: a step
/// it names otherwise does not read, so the script and the guard cannot
/// part on a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Step {
    /// A count wait's matches are not as many as its bounds allow.
    Count,
    /// A `detached` wait still matches.
    Detached,
    /// More than one element matches.
    Unique,
    /// A `hidden` wait's match is visible.
    Hidden,
    /// Nothing matches.
    Attached,
    /// The match is not visible.
    Visible,
    /// The match is disabled.
    Enabled,
    /// The match is not editable.
    Editable,
    /// The match is moving.
    Stable,
    /// Another element would receive the event.
    ReceivesEvents,
}

impl Step {
    /// The step a locator's wait for `state` waits at.
    pub(crate) fn waiting_for(state: ElementState) -> Self {
        match state {
            ElementState::Attached => Self::Attached,
            ElementState::Detached => Self::Detached,
            ElementState::Visible => Self::Visible,
            ElementState::Hidden => Self::Hidden,
        }
    }

    /// Its name, as `check.js` gives it.
    pub(crate) fn as_str(self) -> &'static str {
        goodfetch_protocol::wire_name::of(&self)
    }

    /// What a call that waited for this check to pass waited for, as its
    /// `deadline_exceeded` names it.
    pub(crate) fn waited_for(self) -> WaitedFor {
        match self {
            Self::Count => WaitedFor::TARGET_COUNT,
            Self::Detached => WaitedFor::TARGET_DETACHED,
            Self::Unique => WaitedFor::TARGET_UNIQUE,
            Self::Hidden => WaitedFor::TARGET_HIDDEN,
            Self::Attached => WaitedFor::TARGET_ATTACHED,
            Self::Visible => WaitedFor::TARGET_VISIBLE,
            Self::Enabled => WaitedFor::TARGET_ENABLED,
            Self::Editable => WaitedFor::TARGET_EDITABLE,
            Self::Stable => WaitedFor::TARGET_STABLE,
            Self::ReceivesEvents => WaitedFor::TARGET_RECEIVES_EVENTS,
        }
    }
}
