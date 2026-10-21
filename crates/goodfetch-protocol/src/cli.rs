//! What a CLI command prints under `--json` when it fails (ADR 0069): one
//! JSON value, `{ "error": … }`, holding the Error code, its group, whether
//! it is retryable, the message, the operation and the code's own details.
//! A command that succeeds prints its operation's result, the catalog's
//! type.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::catalog::ErrorGroup;

/// What a command prints under `--json` when it fails.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CommandFailed {
    /// The failure.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub error: CommandError,
}

/// A failed command's error: what a Client's error object holds, with the
/// operation the command called.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CommandError {
    /// The Error code, or the name of the protocol failure.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub code: String,
    /// The group of the Error code, named for its remedy (ADR 0079).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub group: ErrorGroup,
    /// Whether the same call may succeed when made again.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub retryable: bool,
    /// What went wrong, for people; never matched on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub message: String,
    /// The operation the command called, by its canonical name, such as
    /// `run.finish`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub operation: String,
    /// The Error code's own details, such as `fix`.
    #[serde(flatten)]
    pub details: Map<String, Value>,
}
