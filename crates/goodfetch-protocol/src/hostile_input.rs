//! The Error codes of hostile input a caller or a configuration hands the
//! engine (ADR 0067): `header_value_invalid`, for a CR, an LF or a NUL in a
//! header value the engine would send, refused before it reaches the wire
//! or CDP; and `path_unsafe`, for a name or path from outside input that
//! would leave the folder the engine writes it in, or is no safe file name
//! on some OS, refused on every OS alike.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, ProtocolVersion};

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// `header_value_invalid` (ADR 0067).
pub const HEADER_VALUE_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "header_value_invalid",
    description: "A header value the engine would send holds a CR, an LF or a NUL, which could end the header and start another, so it is refused before it reaches the wire or the browser. The details name the header and the character, never the value. Remove the character from the value.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A character no header value may hold.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum HeaderCharacter {
    /// A carriage return, U+000D.
    Cr,
    /// A line feed, U+000A.
    Lf,
    /// A NUL, U+0000.
    Nul,
}

/// What `header_value_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct HeaderValueInvalidDetails {
    /// The header's name.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// The first character its value may not hold.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub character: HeaderCharacter,
}

/// `path_unsafe` (ADR 0067).
pub const PATH_UNSAFE: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "path_unsafe",
    description: "A name or path from outside input would leave the folder the engine writes it in, or is no safe file name on Linux, macOS or Windows: a `..` or `.` part, an absolute, drive, UNC, verbatim or device path, a name Windows reserves for a device, an alternate data stream, a character Windows forbids or a control character, a trailing dot or space, a part longer than 255 bytes, an empty part or a name that is not in Unicode's composed form. It is refused on every OS alike. The details name it and its hazard.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What makes a name or path unsafe.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum PathHazard {
    /// A `..` part, which climbs out of the folder.
    ParentTraversal,
    /// A path from the root.
    Absolute,
    /// A Windows drive, absolute or drive-relative.
    Drive,
    /// A Windows verbatim path, `\\?\`.
    Verbatim,
    /// A Windows device path, `\\.\`.
    DeviceNamespace,
    /// A Windows UNC path.
    Unc,
    /// A name Windows reserves for a device, with or without an extension.
    ReservedName,
    /// A trailing dot or space, which Windows removes.
    TrailingDotOrSpace,
    /// An NTFS alternate data stream, after a colon.
    AlternateDataStream,
    /// A character Windows forbids in a name: `<`, `>`, `"`, `|`, `?` or `*`.
    ForbiddenCharacter,
    /// A control character or a NUL.
    ControlCharacter,
    /// A name not in Unicode's composed form (NFC), which APFS opens as the
    /// composed name it equals.
    Normalisation,
    /// A part longer than the 255 bytes a file system's name holds.
    Overlong,
    /// An empty part, or a `.` part.
    DotName,
    /// A folder separator where one name alone is taken.
    Separator,
}

/// What `path_unsafe` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PathUnsafeDetails {
    /// The name or path, as given.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub name: String,
    /// What makes it unsafe.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub hazard: PathHazard,
}

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[HEADER_VALUE_INVALID, PATH_UNSAFE],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
