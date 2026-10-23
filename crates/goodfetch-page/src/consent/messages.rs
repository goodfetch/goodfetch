//! What the Consent prompt runner reports through its world's binding, and
//! the functions the engine calls in that world (ADR 0048). Pure, so each
//! message's reading is tested without a browser.

use serde::{Deserialize, Serialize};

/// The binding the runner reports through, added to the guard's world of
/// every frame by the world's name.
pub(crate) const BINDING: &str = "__goodfetch_consent__";

/// Starts the runner in its world, reporting through the binding its
/// argument names; `absent` when the document's world never ran the
/// bundle.
pub(crate) const START: &str = "function start(binding) { const runner = globalThis[Symbol.for(\"goodfetch.consent\")]; return runner === undefined ? \"absent\" : runner.start(binding); }";

/// Gives the runner the engine's answer to one of its requests.
pub(crate) const ANSWER: &str = "function answer({ id, value }) { return globalThis[Symbol.for(\"goodfetch.consent\")]?.answer(id, value) ?? false; }";

/// Starts the runner's detection again, on a route change of a single-page
/// app.
pub(crate) const RESTART: &str = "function restart() { return globalThis[Symbol.for(\"goodfetch.consent\")]?.restart() ?? \"absent\"; }";

/// One message of the runner.
#[derive(Debug, PartialEq, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub(crate) enum Message {
    /// A trusted click, through the guard, on the element its world's
    /// registry of guard targets holds under `handle`; answered with
    /// whether it landed.
    Click {
        /// The request's ID, which the answer names.
        id: u64,
        /// The element's handle.
        handle: u64,
    },
    /// A snippet to run in the frame's main world; answered with its
    /// result as a boolean.
    Eval {
        /// The request's ID.
        id: u64,
        /// The snippet's name in autoconsent's library.
        snippet: String,
        /// Its source, a call expression.
        code: String,
    },
    /// The whole text of the document's hiding sheet; answered once set.
    Sheet {
        /// The request's ID.
        id: u64,
        /// Its CSS.
        text: String,
    },
    /// What the runner did with a prompt.
    Outcome(Outcome),
}

/// What the runner did with one prompt, as Provenance records it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) struct Outcome {
    /// The document's URL.
    pub(crate) url: String,
    /// What it did.
    pub(crate) action: Action,
    /// The rule that matched: a vendor rule's name, or `heuristic`.
    pub(crate) rule: String,
    /// Whether the prompt was seen gone afterwards, when the runner looked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) verified: Option<bool>,
}

/// What the runner did with a prompt.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Action {
    /// It refused it.
    Refused,
    /// It hid a prompt with no refusal that leaves the content readable.
    Hidden,
    /// It left a prompt with no refusal that covers the content, for the
    /// wall test.
    Left,
    /// Its refusal did not go through; the next step was tried.
    Failed,
}

impl Message {
    /// The message `payload` carries; `None` for one the runner never
    /// sends.
    pub(crate) fn read(payload: &str) -> Option<Self> {
        serde_json::from_str(payload).ok()
    }
}

#[cfg(test)]
mod tests;
