//! The Error code of a program's own schema rules that raised instead of
//! rejecting (ADR 0082): wherever an SDK runs a schema's native validation
//! (Zod's `.refine()`, a Pydantic `@field_validator`), only the library's
//! own rejection counts as one, and any other exception stops the call at
//! once. The SDK raises it in the program's process, so it never crosses
//! the wire; it is declared here so each SDK generates its class. In Rust
//! a `Deserialize` that returns an error is a rejection, and a panic
//! unwinds in the caller's own call, so a Rust program never sees it.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// A schema's native validation that raised something other than a
/// rejection (ADRs 0082 and 0079).
pub const SCHEMA_VALIDATOR_FAILED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "schema_validator_failed",
    description: "A schema's own validation, run by the SDK in the program's process, raised something other than a rejection, so the call stopped at once: only the library's own rejection (Standard Schema issues, Pydantic's `ValidationError`) is one, and a bug is never retried or reported as a value the schema refused. The message always names the exception, and the original is the error's cause. The details name the schema by its JSON Schema `title` (`null` without one), the field path where known (`null` otherwise), and the exception's type and message. Fix the validator so that it rejects with the library's own issue, or never raises.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `schema_validator_failed` details: the schema, where in the value,
/// and the exception, as `hook_failed`'s details name a Hook's.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct SchemaValidatorFailedDetails {
    /// The schema's JSON Schema `title`, or none when it has none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub schema: Option<String>,
    /// Where in the value the validator ran, a JSON Pointer (RFC 6901),
    /// or none when the library does not say.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: Option<String>,
    /// The exception's type name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exception_type: String,
    /// The exception's message.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exception_message: String,
}

/// The Error code of a schema's validation that raised.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[SCHEMA_VALIDATOR_FAILED],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
