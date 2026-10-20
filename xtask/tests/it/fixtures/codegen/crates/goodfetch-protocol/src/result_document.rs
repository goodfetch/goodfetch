//! A sample result document and case log entry, whose types `xtask codegen`
//! also generates for the TypeScript and Python helpers that write them
//! (ADRs 0063 and 0069).

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

/// A sample case log entry.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CaseLogEntry {
    /// The case's ID.
    pub case: String,
}
