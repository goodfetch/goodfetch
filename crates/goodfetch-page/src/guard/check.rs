//! The guard's check and arm as the engine sends and reads it: the script
//! the guard's world calls, the argument it takes, and what it answers
//! (ADR 0019). Pure, so the check-and-arm round trip's benchmark builds the
//! very command the guard sends (ADR 0071).

use std::time::Duration;

use serde::Deserialize;
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
/// conditions (`pointer`, `text` or `fill`), and how long it may wait in
/// the page before it answers.
pub(crate) fn argument(target: Target<'_>, kind: &str, slice: Duration) -> Value {
    let mut argument = json!({
        "action": kind,
        "events": CLICK_EVENTS,
        "slice": u64::try_from(slice.as_millis()).unwrap_or(u64::MAX),
    });
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
    },
    /// A text action's target is ready and has the focus, its text
    /// selected for `fill`.
    Focused {
        /// For `fill`, whether the target is an input whose value a
        /// person picks rather than types, which the guard sets.
        #[serde(default)]
        set_value: bool,
    },
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
    /// The check's answer, `value`, as what it saw.
    pub(crate) fn read(value: &Value) -> Result<Self, serde_json::Error> {
        Self::deserialize(value)
    }
}

/// A condition the check waits on, by the name `check.js` gives it: a step
/// it names otherwise does not read, so the script and the guard cannot
/// part on a name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
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
    /// Its name, as `check.js` and `deadline_exceeded`'s `waited_for` give
    /// it.
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Count => "count",
            Self::Detached => "detached",
            Self::Unique => "unique",
            Self::Hidden => "hidden",
            Self::Attached => "attached",
            Self::Visible => "visible",
            Self::Enabled => "enabled",
            Self::Editable => "editable",
            Self::Stable => "stable",
            Self::ReceivesEvents => "receives_events",
        }
    }
}
