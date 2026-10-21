//! Page bodies as handles (ADRs 0024 and 0031): a body is a file in its
//! Run's folder, named by the digest of its bytes. A Client on the same
//! machine reads the file at the handle's path; any other reads it through
//! `page.body`, which sends it in `$/chunk` notifications, one per credit
//! the Client grants the call with `$/credit`, and answers once the last
//! chunk is sent. The rest of a page's entries join with area 5.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, Operation, OperationClass, OperationKind,
    ProtocolVersion, Stability, schema_of,
};
use crate::jsonrpc::RequestId;
use crate::run::RunId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The notification that carries one chunk of a body `page.body` reads.
pub const CHUNK: &str = "$/chunk";

/// The most bytes one chunk carries before its Base64 encoding.
pub const CHUNK_BYTES: usize = 64 * 1024;

/// The characters of a body's ID: the lowercase hex SHA-256 of its bytes.
pub const BODY_ID_PATTERN: &str = "^[0-9a-f]{64}$";

/// A page body as a handle: its ID, the file that holds it and its size.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BodyHandle {
    /// The Run whose folder holds it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Its ID, the lowercase hex SHA-256 of its bytes.
    #[schemars(extend("arrived_in" = "0.1", "pattern" = BODY_ID_PATTERN))]
    pub body: String,
    /// The file that holds it, which a Client on the same machine reads.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub path: String,
    /// Its size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
}

/// What `page.body` takes: the body to read.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PageBodyParams {
    /// The Run whose folder holds it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// Its ID.
    #[schemars(extend("arrived_in" = "0.1", "pattern" = BODY_ID_PATTERN))]
    pub body: String,
}

/// What `page.body` answers once it sent the last chunk.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct PageBodyRead {
    /// The body's size in bytes.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub size: u64,
}

/// What `$/chunk` carries: the next bytes of the body a call reads.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct Chunk {
    /// The id of the `page.body` call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
    /// Where its bytes start in the body.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub offset: u64,
    /// Its bytes, in Base64 (RFC 4648, with padding).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub data: String,
}

/// What `body_not_found` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct BodyNotFoundDetails {
    /// The Run.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub run_id: RunId,
    /// The body's ID.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub body: String,
}

/// A body no Run's folder holds.
pub const BODY_NOT_FOUND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "body_not_found",
    description: "The Run's folder holds no body of this ID: the Run or the body is unknown, or the ID is not a body's.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// `page.body`.
pub const PAGE_BODY: Operation = Operation {
    name: "page.body",
    description: "Reads a page body by its handle, for a Client that cannot read the handle's file: sends the body in `$/chunk` notifications of at most 64 KiB, one per credit granted with `$/credit`, then answers its size.",
    class: OperationClass::Read,
    kind: OperationKind::Sequence,
    params: schema_of::<PageBodyParams>,
    result: schema_of::<PageBodyRead>,
    events: &[],
    cancellable: true,
    stability: Stability::Experimental,
    deprecated: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[PAGE_BODY],
    events: &[],
    handler_requests: &[],
    error_codes: &[BODY_NOT_FOUND],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};
