//! What opens and governs a Connection: `hello`, with the protocol's
//! `major.minor`, the surface, the token and the Product the Client needs
//! with its Product version (ADRs 0024, 0066 and 0095); `$/cancel` (ADR
//! 0038); the Error codes they answer with; and those a Client's own runtime
//! raises about its Connection, which no engine sends (ADRs 0078 and 0079).
//! Neither `hello` nor `$/cancel` is an operation of the catalog: every
//! Product's engine speaks both.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).

use std::borrow::Cow;
use std::fmt;
use std::net::SocketAddr;
use std::path::PathBuf;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};
use crate::jsonrpc::RequestId;

const ARRIVED_IN: ProtocolVersion = ProtocolVersion { major: 0, minor: 1 };

/// The method every Connection opens with.
pub const HELLO: &str = "hello";

/// The notification that cancels a call in flight.
pub const CANCEL: &str = "$/cancel";

/// The surface a `hello` comes from (ADR 0024).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Surface {
    /// The TypeScript SDK.
    SdkTypescript,
    /// The Python SDK.
    SdkPython,
    /// A Rust program, through a Product's crate.
    Rust,
    /// The CLI, as a Client of a Daemon.
    Cli,
    /// The MCP server, as a Client of a Daemon.
    Mcp,
    /// The Console.
    Console,
    /// Goodfetch Cloud's gateway, or the Stub gateway, which a Collie engine
    /// takes as given to word `not_in_product`'s `fix` (ADRs 0095, 0096).
    Cloud,
    /// A Browser host, which is never a Client and needs no Product
    /// (ADR 0095).
    BrowserHost,
    /// A Script sandbox worker.
    Worker,
}

impl Surface {
    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SdkTypescript => "sdk_typescript",
            Self::SdkPython => "sdk_python",
            Self::Rust => "rust",
            Self::Cli => "cli",
            Self::Mcp => "mcp",
            Self::Console => "console",
            Self::Cloud => "cloud",
            Self::BrowserHost => "browser_host",
            Self::Worker => "worker",
        }
    }
}

/// A Product version, `major.minor.patch` with an optional pre-release
/// (ADR 0090). Reported, never compared (ADR 0095).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProductVersion {
    text: String,
    major: u64,
}

/// Text that is no Product version.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is no Product version `major.minor.patch`")]
pub struct MalformedProductVersion(String);

impl ProductVersion {
    /// Reads `text`, such as `1.4.0` or `1.5.0-rc.1`.
    pub fn parse(text: &str) -> Result<Self, MalformedProductVersion> {
        let malformed = || MalformedProductVersion(text.to_owned());
        let (core, pre_release) = match text.split_once('-') {
            Some((core, pre_release)) => (core, Some(pre_release)),
            None => (text, None),
        };
        let numbers: Vec<&str> = core.split('.').collect();
        let canonical = |part: &&str| {
            !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (*part == "0" || !part.starts_with('0'))
        };
        if numbers.len() != 3 || !numbers.iter().all(canonical) {
            return Err(malformed());
        }
        if let Some(pre_release) = pre_release {
            let identifier = |part: &str| {
                !part.is_empty()
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            };
            if !pre_release.split('.').all(identifier) {
                return Err(malformed());
            }
        }
        let major = numbers[0].parse().map_err(|_| malformed())?;
        Ok(Self {
            text: text.to_owned(),
            major,
        })
    }

    /// Its product major.
    pub fn major(&self) -> u64 {
        self.major
    }

    /// The version as written.
    pub fn as_str(&self) -> &str {
        &self.text
    }
}

impl fmt::Display for ProductVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.text)
    }
}

impl Serialize for ProductVersion {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.text)
    }
}

impl<'de> Deserialize<'de> for ProductVersion {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = Cow::<str>::deserialize(deserializer)?;
        Self::parse(&text).map_err(serde::de::Error::custom)
    }
}

impl JsonSchema for ProductVersion {
    fn schema_name() -> Cow<'static, str> {
        "ProductVersion".into()
    }

    fn json_schema(_generator: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "description": "A Product version, `major.minor.patch` with an optional pre-release (ADR 0090).",
            "type": "string",
            "pattern": "^(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)\\.(0|[1-9][0-9]*)(-[0-9A-Za-z-]+(\\.[0-9A-Za-z-]+)*)?$",
        })
    }
}

/// What a `hello` says.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct HelloParams {
    /// The protocol version the sender speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The surface it comes from.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub surface: Surface,
    /// The Daemon's token; an Attached engine needs none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub token: Option<String>,
    /// The Product the Client needs, which every surface but a Browser
    /// host's must name (ADR 0095).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub needs: Option<Product>,
    /// The sender's own Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
}

/// What an engine answers to a `hello` it accepts.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct HelloResult {
    /// The protocol version the engine speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub protocol_version: ProtocolVersion,
    /// The engine's Product.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// The engine's Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// An Attached engine's private endpoint, reported to the Client of its
    /// stdio alone: where that Client's `workers=N` handler processes, and a
    /// Daemon forwarding a live action, connect (ADRs 0024 and 0031).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub private_endpoint: Option<PrivateEndpoint>,
}

/// Where a Daemon or an Attached engine's private endpoint listens (ADR
/// 0024): a `0600` Unix socket on macOS and Linux, loopback TCP on
/// Windows.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum DaemonEndpoint {
    /// A Unix socket's path.
    Unix(#[schemars(with = "String")] PathBuf),
    /// A loopback address and port.
    Tcp(#[schemars(with = "String")] SocketAddr),
}

impl fmt::Display for DaemonEndpoint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unix(path) => write!(formatter, "{}", path.display()),
            Self::Tcp(address) => write!(formatter, "{address}"),
        }
    }
}

/// An Attached engine's private endpoint (ADRs 0024 and 0031): where it
/// listens, a `0600` Unix socket on macOS and Linux or loopback TCP on
/// Windows, and the `0600` file holding the token that each Connection's
/// `hello` there carries, as a Daemon's does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct PrivateEndpoint {
    /// Where it listens.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub address: DaemonEndpoint,
    /// The path of the file holding its token.
    #[schemars(with = "String", extend("arrived_in" = "0.1"))]
    pub token_file: PathBuf,
}

/// What `$/cancel` names: the call to stop.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct CancelParams {
    /// The call's id.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub id: RequestId,
}

/// The refusal of a Client's protocol version (ADRs 0024 and 0066).
pub const PROTOCOL_VERSION_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "protocol_version_unsupported",
    description: "The engine does not speak the protocol version the Client's `hello` names: it accepts its own major with the same or an older minor. Upgrade the older side.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: Some(11),
    arrived_in: ARRIVED_IN,
};

/// What `protocol_version_unsupported` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct ProtocolVersionUnsupportedDetails {
    /// The protocol version the engine speaks.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub engine_protocol_version: ProtocolVersion,
    /// The protocol version the Client's `hello` named.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub client_protocol_version: ProtocolVersion,
    /// What to upgrade: the error's `fix` (ADR 0079).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
    /// Whether the refusing engine is a Daemon handing over to its major's
    /// pick, which a Client of the major waits for and connects to (ADR
    /// 0098).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub handover: bool,
}

/// A request for what the running Product lacks (ADRs 0087 and 0095).
pub const NOT_IN_PRODUCT: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "not_in_product",
    description: "The running engine's Product does not carry what was asked: a Client that needs Foxhound reached a Collie engine, or a call, a setting or a value only Foxhound carries reached one. Run it with Foxhound, which contains everything Collie does, or connect to a Foxhound engine.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: Some(10),
    arrived_in: ARRIVED_IN,
};

/// What `not_in_product` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct NotInProductDetails {
    /// The running engine's Product.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product: Product,
    /// The running engine's Product version.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub product_version: ProductVersion,
    /// The Product that carries what was asked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub needs: Product,
    /// What was asked, such as an operation's name; `None` for a `hello`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub requested: Option<String>,
    /// What to do instead.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub fix: String,
    /// Whether the refusing engine is a Daemon handing over to its major's
    /// pick, which a Client of the major waits for and connects to (ADR
    /// 0098).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub handover: bool,
}

/// The answer to a call someone other than its caller stopped, a
/// `$/cancel` included (ADRs 0038, 0080 and 0113).
pub const CALL_CANCELLED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "call_cancelled",
    description: "The call was stopped in flight, at its next cancellation point: its Client sent `$/cancel`, or what it needed was closed under it.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Who stopped a call (ADR 0113).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StoppedBy {
    /// A Client, such as through `$/cancel`.
    Client,
}

/// What `call_cancelled` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallCancelledDetails {
    /// Who stopped the call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by: StoppedBy,
}

/// A call on a value its program already closed (ADR 0078), raised by the
/// SDK runtime itself.
pub const CLOSED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "closed",
    description: "The call was made on a value its program had closed: a Connection, or a Run, Session or Page under it. A value never reopens: connect again, or start the Run again by its Run ID.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// No engine executable to start, raised by an SDK's `connect()` (ADRs
/// 0024, 0078 and 0090).
pub const ENGINE_NOT_FOUND: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "engine_not_found",
    description: "`connect()` found no engine executable to start: the Product's engine-path variable names no executable, or the Product's platform package is not installed. The `fix` names the variable or the package.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// An Attached engine that ended before its Connection was closed (ADRs
/// 0078 and 0079), raised by the SDK runtime.
pub const ENGINE_EXITED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "engine_exited",
    description: "The Attached engine ended before its Connection was closed, so the call has no answer. Its details carry the exit code or signal and the last 64 KiB of its stderr.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `engine_exited` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct EngineExitedDetails {
    /// The engine's exit code; `None` when a signal ended it or the
    /// Connection's transport has none.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exit_code: Option<i32>,
    /// The signal that ended it, by name, such as `SIGKILL`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signal: Option<String>,
    /// The last 64 KiB of its stderr.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stderr_tail: String,
}

/// An engine that wrote what is no JSON-RPC message on the Connection
/// (ADRs 0024 and 0078), raised by the SDK runtime.
pub const ENGINE_OUTPUT_INVALID: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "engine_output_invalid",
    description: "The engine wrote a line that is not one JSON-RPC message on the Connection, such as a log line on an Attached engine's stdout, so the Client ended the Connection. A bug in Goodfetch: report it with the line its details carry.",
    group: ErrorGroup::Internal,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Every entry this module defines.
pub const ENTRIES: Entries = Entries {
    defined_in: module_path!(),
    operations: &[],
    events: &[],
    handler_requests: &[],
    error_codes: &[
        PROTOCOL_VERSION_UNSUPPORTED,
        NOT_IN_PRODUCT,
        CALL_CANCELLED,
        CLOSED,
        ENGINE_NOT_FOUND,
        ENGINE_EXITED,
        ENGINE_OUTPUT_INVALID,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
