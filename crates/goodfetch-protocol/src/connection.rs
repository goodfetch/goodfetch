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
use std::net::{Ipv4Addr, SocketAddr};
use std::path::PathBuf;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::catalog::{Entries, ErrorCodeDefinition, ErrorGroup, Product, ProtocolVersion};
use crate::jsonrpc::RequestId;
use crate::log::LogLevel;
use crate::settings::SettingLimit;

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
    /// The CLI, as a Client of its own Attached engine or of a Daemon.
    Cli,
    /// The MCP server, as a Client of a Daemon.
    Mcp,
    /// The Console.
    Console,
    /// The Kennel, a person surface that observes the Daemon: its Runs
    /// outlive its Connection, which holds the Daemon neither awake nor
    /// back from a handover unless its `hello` sets `keep_daemon_alive`
    /// (ADR 0141).
    Kennel,
    /// Goodfetch Cloud's gateway, or the Stub gateway, which a Collie engine
    /// takes as given to word `not_in_product`'s `fix` (ADRs 0095, 0096).
    /// Its `hello` names in `client_surface` the surface of the Client it
    /// relays, which every other rule of the Connection follows.
    Cloud,
    /// A Browser host, which is never a Client and needs no Product
    /// (ADR 0095).
    BrowserHost,
    /// A Script sandbox worker.
    Worker,
}

/// The form a Client's user writes names in (ADR 0099), which the engine
/// renders the names and targets of its messages in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum Form {
    /// The TypeScript SDK, and Code mode.
    Typescript,
    /// The Python SDK.
    Python,
    /// A Rust program.
    Rust,
    /// The CLI.
    Cli,
    /// The MCP server and the Framework adapters.
    Tool,
    /// The catalog's own names: the Console, Integrations and the HTTP
    /// listener.
    Catalog,
}

impl Surface {
    /// Whether a Client of this surface acts for a model, which sets none of
    /// the settings that send a Secret reference's value to a destination
    /// they name (ADR 0092): the MCP server, and a Script sandbox worker
    /// running Code mode.
    pub fn acts_for_model(self) -> bool {
        match self {
            Self::Mcp | Self::Worker => true,
            Self::SdkTypescript
            | Self::SdkPython
            | Self::Rust
            | Self::Cli
            | Self::Console
            | Self::Kennel
            | Self::Cloud
            | Self::BrowserHost => false,
        }
    }

    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::SdkTypescript => "sdk_typescript",
            Self::SdkPython => "sdk_python",
            Self::Rust => "rust",
            Self::Cli => "cli",
            Self::Mcp => "mcp",
            Self::Console => "console",
            Self::Kennel => "kennel",
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
    minor: u64,
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
        let minor = numbers[1].parse().map_err(|_| malformed())?;
        Ok(Self {
            text: text.to_owned(),
            major,
            minor,
        })
    }

    /// Its product major.
    pub fn major(&self) -> u64 {
        self.major
    }

    /// Its major and minor, as the formats' rules compare them (ADRs 0030
    /// and 0098); a number past `u16::MAX` reads as `u16::MAX`.
    pub fn major_minor(&self) -> ProtocolVersion {
        let number = |number: u64| u16::try_from(number).unwrap_or(u16::MAX);
        ProtocolVersion {
            major: number(self.major),
            minor: number(self.minor),
        }
    }

    /// The version as written.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// A listener's address on loopback that belongs to this version's
    /// major alone: port `release_base` plus the major, or `preview_base`
    /// plus the minor during `0.y` previews, where each minor counts as a
    /// major (ADR 0030).
    pub(crate) fn loopback_of_its_major(&self, release_base: u16, preview_base: u16) -> SocketAddr {
        let number = |number: u64| u16::try_from(number).ok();
        let port = match (number(self.major), number(self.minor)) {
            (Some(0), Some(minor)) => preview_base.saturating_add(minor),
            (Some(major), _) => release_base.saturating_add(major),
            (None, _) => release_base,
        };
        SocketAddr::from((Ipv4Addr::LOCALHOST, port))
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
    /// The form its user writes names in (ADR 0099); none until each
    /// surface names its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub form: Option<Form>,
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
    /// The level of the engine's log events the Client receives as `$/log`
    /// notifications, until `connection.set_log_level` changes it; none,
    /// as `off`, receives none (ADR 0078).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub log_level: Option<LogLevel>,
    /// On a gateway's `hello` (surface `cloud`), the surface of the Client
    /// whose Connection it relays, such as `mcp` for a model's: the engine
    /// holds the Connection to that surface's rules, a model's settings
    /// included (ADR 0092), and the gateway's mark only words
    /// `not_in_product` (ADR 0095). None on the gateway's own Connection;
    /// any other surface names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub client_surface: Option<Surface>,
    /// On a Kennel's `hello` (surface `kennel`), asks a Daemon to count the
    /// Connection as a Client that keeps it alive, holding off its idle
    /// stop and a handover to its major's pick as every other surface's
    /// Connection does (ADR 0141). Read only from a `kennel` Connection,
    /// and ignored on any other surface's.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub keep_daemon_alive: bool,
}

/// One engine process, new each time an engine starts (ADR 0031): the
/// positions of its events count within it, so a position another engine
/// instance gave means nothing to it.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct EngineInstance(String);

impl EngineInstance {
    /// The engine instance `text` names.
    pub fn new(text: impl Into<String>) -> Self {
        Self(text.into())
    }

    /// Its text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl JsonSchema for EngineInstance {
    fn schema_name() -> Cow<'static, str> {
        "EngineInstance".into()
    }

    fn json_schema(_: &mut SchemaGenerator) -> Schema {
        json_schema!({
            "type": "string",
            "minLength": 1,
            "description": "One engine process, new each time an engine starts: the positions of its events count within it. A Client compares it for equality only.",
        })
    }
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
    /// The engine process, new each time an engine starts: the positions
    /// of its events count within it, so a Client that meets a new one
    /// resyncs (ADR 0031).
    #[schemars(extend("arrived_in" = "0.1"))]
    pub engine_instance: EngineInstance,
    /// An Attached engine's private endpoint, reported to the Client of its
    /// stdio alone: where that Client's `workers=N` handler processes, and a
    /// Daemon forwarding a live action, connect (ADRs 0024 and 0031).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub private_endpoint: Option<PrivateEndpoint>,
    /// The sign-in link of the Console an Attached engine serves for its
    /// script, which asked for one with a Console address, reported to the
    /// Client of its stdio alone: `<url>#token=<token>`, whose fragment the
    /// page swaps once for its cookie (ADRs 0024 and 0031).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub console_link: Option<String>,
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
    /// The command that installs the Product `needs` from the engine's own
    /// channel, which `fix` names; none behind Goodfetch Cloud's gateway,
    /// where an install would not help (ADR 0096).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub install: Option<String>,
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

/// A call past its deadline (ADR 0080, as ADR 0083 amends it): one Error
/// code whatever the phase, whose `limit` names the timeout's setting.
pub const DEADLINE_EXCEEDED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "deadline_exceeded",
    description: "The call's deadline passed before what it waited for came, such as a response's first byte or a navigation. Its `limit` names the timeout's setting, its value and the layer that set it: raise that setting, or wait for the right thing.",
    group: ErrorGroup::DeadlineExceeded,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `deadline_exceeded` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct DeadlineExceededDetails {
    /// The timeout's setting, its value in milliseconds and the layer that
    /// set it.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub limit: SettingLimit,
    /// How long the call waited, in milliseconds.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub elapsed: u64,
    /// What it waited for, such as `first_byte` or `navigation`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub waited_for: String,
}

/// Who stopped a call (ADR 0113).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StoppedBy {
    /// A Client, such as through `$/cancel`.
    Client,
    /// The program, such as through `run.stop` (ADR 0102).
    Program,
    /// The browser, outside every Client: a page's window closed there, or
    /// the browser quit (ADR 0113).
    Browser,
}

/// What was closed under a call that `call_cancelled` ended (ADR 0113).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ClosedValue {
    /// Its page.
    Page,
    /// Its page's Session.
    Session,
    /// Its page's browser.
    Browser,
}

/// What `call_cancelled` details.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallCancelledDetails {
    /// Who stopped the call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by: StoppedBy,
    /// What was closed under it, when something was rather than the call
    /// itself cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub closed: Option<ClosedValue>,
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

/// A transition a lifecycle table refuses that only a bug in Goodfetch can
/// cause (ADRs 0079 and 0128), logged at `error` with the same details.
pub const INTERNAL: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "internal",
    description: "A bug in Goodfetch: an object such as a Session or a page received an event its state cannot receive. Report it with the object, the state and the event its details carry.",
    group: ErrorGroup::Internal,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `internal` details: the object whose table refused the event, its
/// state and the event, as the engine names them.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct InternalDetails {
    /// The object, such as `session` or `page`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub object: String,
    /// Its state.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub state: String,
    /// The event refused.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub event: String,
}

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

/// What `engine_not_found` details: where the SDK looked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum EngineNotFoundDetails {
    /// The Product's engine-path variable named a path that is no
    /// executable file.
    Path {
        /// The path the variable named.
        #[schemars(extend("arrived_in" = "0.1"))]
        path: String,
    },
    /// No engine-path variable was set, and the Product's platform package
    /// is not installed.
    Package {
        /// The platform package's name, such as
        /// `@goodfetch/collie-linux-x64`.
        #[schemars(extend("arrived_in" = "0.1"))]
        package: String,
    },
}

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

/// A Connection to an engine that dropped while a call waited for its
/// answer (ADRs 0031 and 0079), raised by the Client's own runtime, as the
/// Console's, which then connects again.
pub const CONNECTION_LOST: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "connection_lost",
    description: "The Connection to the engine dropped before the call was answered, so whether it took effect is unknown. The Client connects again by itself; make the call again once it has, after checking that an Effect did not happen.",
    group: ErrorGroup::Unavailable,
    retryable: true,
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

/// What `engine_output_invalid` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EngineOutputInvalidDetails {
    /// The line the engine wrote, without its line end.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub line: String,
}

/// The refusal of an answer only a person gives (ADRs 0011 and 0041): an
/// operation the catalog marks [`Reach::PersonOnly`](crate::catalog::Reach)
/// asked by anything but the Console, the Kennel or an interactive CLI
/// prompt (ADR 0141).
pub const PERSON_REQUIRED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "person_required",
    description: "Only a person gives this answer, in the Console, the Kennel or an interactive CLI prompt: no agent surface, SDK, script or command run under `--json` or without a terminal may. Answer it in the Console or the Kennel, or run the command in a terminal.",
    group: ErrorGroup::Refused,
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
        DEADLINE_EXCEEDED,
        CLOSED,
        INTERNAL,
        ENGINE_NOT_FOUND,
        ENGINE_EXITED,
        CONNECTION_LOST,
        ENGINE_OUTPUT_INVALID,
        PERSON_REQUIRED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[],
    hook_stages: &[],
    shared_type_values: &[],
};

#[cfg(test)]
mod tests;
