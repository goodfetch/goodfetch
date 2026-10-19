//! A sample Journey file's type, whose schema `xtask codegen` also writes
//! on its own to `journeys/journey.schema.json` (ADR 0063).

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// A sample Journey.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Journey {
    /// Its ID.
    pub id: String,
    /// What the person achieves.
    pub goal: String,
}
