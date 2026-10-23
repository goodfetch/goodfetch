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

/// What one check saw.
#[derive(Debug, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub(crate) enum Seen {
    /// The target is ready: for a pointer action, armed, and the point to
    /// press.
    Ready {
        /// The point's distance from the viewport's left, in CSS pixels.
        x: Option<f64>,
        /// Its distance from the viewport's top.
        y: Option<f64>,
    },
    /// It is not, at the step `step`.
    Waiting {
        /// The condition that did not hold, such as `visible` or `unique`.
        step: String,
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
