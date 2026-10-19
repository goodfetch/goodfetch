//! A sample result document, whose types `xtask codegen` also generates
//! for the TypeScript and Python helpers that write it (ADR 0063).

use schemars::JsonSchema;
use serde::Serialize;

/// A sample result document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum ResultDocument {
    /// A sample Journey variant's result.
    Journey(JourneyDocument),
}

/// A sample `journey` document.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct JourneyDocument {
    /// The Journey.
    pub journey_id: String,
    /// Its outcome.
    pub outcome: String,
}
