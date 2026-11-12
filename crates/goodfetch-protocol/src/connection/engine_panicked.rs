//! `engine_panicked` (ADR 0148): the Error code of whatever a panic in the
//! engine ended, what it names as ended, and its details, which never
//! carry the panic's own text.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::ARRIVED_IN;
use crate::catalog::{ErrorCodeDefinition, ErrorGroup};

/// What a panic in the engine ended (ADR 0148): the answer to a call whose
/// task panicked, and the code of whatever else a panic ended.
pub const ENGINE_PANICKED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "engine_panicked",
    description: "A bug in Goodfetch made the engine panic, which ended what `ended` names and only that: for `call`, this call, while its Connection, its Run and its Page go on. Making the same call again is likely to hit the same bug. Report it with the crash report `crash_report` names, a file in the engine's crash-report folder on its machine.",
    group: ErrorGroup::Internal,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What a panic in the engine ended (ADR 0148).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PanicEnded {
    /// One call; its Connection goes on.
    Call,
    /// One attempt of a crawl.
    Attempt,
    /// A Page.
    Page,
    /// A Session, with its Pages.
    Session,
    /// A browser, treated as crashed.
    Browser,
    /// A Run, which ends `failed`.
    Run,
    /// A Connection, which ends as when its Client goes away.
    Connection,
    /// The engine, which stops.
    Engine,
}

impl PanicEnded {
    /// Every value, in declaration order.
    pub const ALL: [Self; 8] = [
        Self::Call,
        Self::Attempt,
        Self::Page,
        Self::Session,
        Self::Browser,
        Self::Run,
        Self::Connection,
        Self::Engine,
    ];

    /// Its name on the wire.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// What `engine_panicked` details: what ended, never the panic's own text,
/// which stays in the crash report.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnginePanickedDetails {
    /// What the panic ended.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended: PanicEnded,
    /// The ID of what ended, when it has one, such as a Run's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: Option<String>,
    /// The crash report's file name, when one was written.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub crash_report: Option<String>,
}
