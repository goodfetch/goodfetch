//! What opens and governs a Connection: `hello`, with the protocol's
//! `major.minor`, the surface, the token and the Product the Client needs
//! with its Product version (ADRs 0024, 0066 and 0095); `$/cancel` (ADR
//! 0038); the Error codes they answer with; and those a Client's own runtime
//! raises about its Connection, which no engine sends (ADRs 0078 and 0079),
//! among them the refusal of handlers on an edge runtime (ADR 0049).
//! Neither `hello` nor `$/cancel` is an operation of the catalog: every
//! Product's engine speaks both.
//!
//! Each field records the protocol minor that added it, as `arrived_in` in
//! its schema (ADR 0096).
//!
//! Size: one contract, what every Connection speaks whatever its Product:
//! its opening, its cancel and the Error codes with their details, kept
//! whole so the codes' details stand beside the calls that answer them.

use std::borrow::Cow;
use std::cmp::Ordering;
use std::fmt;
use std::net::{Ipv4Addr, SocketAddr};
use std::num::NonZeroU64;
use std::path::PathBuf;

use schemars::{JsonSchema, Schema, SchemaGenerator, json_schema};
use serde::{Deserialize, Serialize};

use crate::catalog::{
    Entries, ErrorCodeDefinition, ErrorGroup, PROTOCOL_VERSION, Product, ProtocolVersion,
};
use crate::jsonrpc::RequestId;
use crate::log::LogLevel;
use crate::open_value::UnknownValue;
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
    /// The catalog's own names: Integrations and the HTTP
    /// listener.
    Catalog,
}

impl Surface {
    /// Every value, in declaration order.
    pub const ALL: [Self; 9] = [
        Self::SdkTypescript,
        Self::SdkPython,
        Self::Rust,
        Self::Cli,
        Self::Mcp,
        Self::Kennel,
        Self::Cloud,
        Self::BrowserHost,
        Self::Worker,
    ];

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
            | Self::Kennel
            | Self::Cloud
            | Self::BrowserHost => false,
        }
    }

    /// The form a Client of this surface writes names in when its `hello`
    /// declares none (ADR 0099): an SDK's own language, the CLI's and the
    /// MCP server's, TypeScript for a Script sandbox worker, which runs Code
    /// mode, and the catalog's for every other surface.
    pub fn form(self) -> Form {
        match self {
            Self::SdkTypescript | Self::Worker => Form::Typescript,
            Self::SdkPython => Form::Python,
            Self::Rust => Form::Rust,
            Self::Cli => Form::Cli,
            Self::Mcp => Form::Tool,
            Self::Kennel | Self::Cloud | Self::BrowserHost => Form::Catalog,
        }
    }

    /// Whether a person drives a Client of this surface, so the engine
    /// admits from it the operations only a person may call
    /// (`Reach::PersonOnly`): the CLI and the Kennel.
    pub fn is_a_person(self) -> bool {
        match self {
            Self::Cli | Self::Kennel => true,
            Self::SdkTypescript
            | Self::SdkPython
            | Self::Rust
            | Self::Mcp
            | Self::Cloud
            | Self::BrowserHost
            | Self::Worker => false,
        }
    }

    /// Whether the Runs a Connection of this surface starts or resumes on
    /// a Daemon outlive that Connection: each CLI command is a Connection
    /// of its own (ADRs 0024 and 0088), and so are a person's controls from
    /// the Kennel (ADR 0141).
    pub fn runs_outlive_connection(self) -> bool {
        match self {
            Self::Cli | Self::Kennel => true,
            Self::SdkTypescript
            | Self::SdkPython
            | Self::Rust
            | Self::Mcp
            | Self::Cloud
            | Self::BrowserHost
            | Self::Worker => false,
        }
    }

    /// Its name on the wire.
    pub fn as_str(self) -> &'static str {
        crate::wire_name::of(&self)
    }
}

/// A Product version, `major.minor.patch` with an optional pre-release
/// (ADR 0090). Reported, never compared across a Connection (ADR 0095);
/// on one machine it orders by Semantic Versioning's precedence, so the
/// newest installed executable can be picked (ADR 0098), and it names its
/// [`VersionLine`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ProductVersion {
    text: String,
    major: u64,
    minor: u64,
}

/// A Product version's line: its major, or during `0.y` previews its `0.y`
/// minor, since each preview minor counts as a major (ADR 0030). Lines
/// order as their versions do, and each has its own per-major folder in
/// Goodfetch home, named by its text: `1` for any `1.x.y`, `0.4` for any
/// `0.4.y`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct VersionLine {
    major: u64,
    /// The preview's minor when `major` is 0, else 0.
    minor: u64,
}

impl VersionLine {
    /// The line a per-major folder's name, `1` or `0.4`, names: `None` for
    /// a name that is no line's, such as `0`, `01` or `1.2`.
    pub fn parse(name: &str) -> Option<Self> {
        let number = |part: &str| {
            let canonical = !part.is_empty()
                && part.bytes().all(|byte| byte.is_ascii_digit())
                && (part == "0" || !part.starts_with('0'));
            canonical.then(|| part.parse().ok()).flatten()
        };
        match name.split_once('.') {
            Some(("0", minor)) => Some(Self {
                major: 0,
                minor: number(minor)?,
            }),
            Some(_) => None,
            None => match number(name)? {
                0 => None,
                major => Some(Self { major, minor: 0 }),
            },
        }
    }

    /// Whether it is a `0.y` preview's.
    pub fn is_preview(self) -> bool {
        self.major == 0
    }

    /// Its major: 0 for a preview.
    pub fn major(self) -> u64 {
        self.major
    }

    /// The preview's minor, which counts as its major; 0 for a release.
    pub fn minor(self) -> u64 {
        self.minor
    }

    /// The line after it: the next `0.y` during previews, else the next
    /// major; `None` past `u64::MAX`.
    pub fn next(self) -> Option<Self> {
        if self.is_preview() {
            Some(Self {
                major: 0,
                minor: self.minor.checked_add(1)?,
            })
        } else {
            Some(Self {
                major: self.major.checked_add(1)?,
                minor: 0,
            })
        }
    }
}

impl fmt::Display for VersionLine {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.is_preview() {
            write!(formatter, "0.{}", self.minor)
        } else {
            write!(formatter, "{}", self.major)
        }
    }
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

    /// Its line: its major, or its `0.y` minor during previews (ADR 0030).
    pub fn line(&self) -> VersionLine {
        VersionLine {
            major: self.major,
            minor: if self.major == 0 { self.minor } else { 0 },
        }
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
        let line = self.line();
        let (base, number) = if line.is_preview() {
            (preview_base, line.minor())
        } else {
            (release_base, line.major())
        };
        let port = u16::try_from(number).map_or(release_base, |number| base.saturating_add(number));
        SocketAddr::from((Ipv4Addr::LOCALHOST, port))
    }

    /// Its pre-release, the text after the first `-`, if any.
    fn pre_release(&self) -> Option<&str> {
        self.text
            .split_once('-')
            .map(|(_, pre_release)| pre_release)
    }

    /// Its patch number's text, canonical digits.
    fn patch(&self) -> &str {
        let core = self
            .text
            .split_once('-')
            .map_or(self.text.as_str(), |(core, _)| core);
        core.rsplit('.').next().unwrap_or(core)
    }
}

impl Ord for ProductVersion {
    /// Semantic Versioning's precedence: the numbers, then a release above
    /// its pre-releases, which compare identifier by identifier; versions
    /// of equal precedence order by their text, so the order agrees with
    /// equality.
    fn cmp(&self, other: &Self) -> Ordering {
        // A canonical number's text orders by its length, then its digits,
        // however long it is.
        let (patch, other_patch) = (self.patch(), other.patch());
        (self.major, self.minor)
            .cmp(&(other.major, other.minor))
            .then_with(|| (patch.len(), patch).cmp(&(other_patch.len(), other_patch)))
            .then_with(|| match (self.pre_release(), other.pre_release()) {
                (None, None) => Ordering::Equal,
                (None, Some(_)) => Ordering::Greater,
                (Some(_), None) => Ordering::Less,
                (Some(left), Some(right)) => pre_release_order(left, right),
            })
            .then_with(|| self.text.cmp(&other.text))
    }
}

impl PartialOrd for ProductVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Semantic Versioning's precedence of two pre-releases: identifier by
/// identifier, numbers numerically and below words, words by their bytes,
/// and a shorter list below a longer one it begins.
fn pre_release_order(left: &str, right: &str) -> Ordering {
    let mut left_parts = left.split('.');
    let mut right_parts = right.split('.');
    loop {
        match (left_parts.next(), right_parts.next()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(left), Some(right)) => {
                let order = match (left.parse::<u64>(), right.parse::<u64>()) {
                    (Ok(left), Ok(right)) => left.cmp(&right),
                    (Ok(_), Err(_)) => Ordering::Less,
                    (Err(_), Ok(_)) => Ordering::Greater,
                    (Err(_), Err(_)) => left.cmp(right),
                };
                if order != Ordering::Equal {
                    return order;
                }
            }
        }
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
    /// The form its user writes names in (ADR 0099), which every SDK
    /// declares; without one, the engine takes its surface's.
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
    /// The Connection's own settings, nested by key: the run layer at the
    /// connection scope, which every Run it starts inherits beneath its
    /// own (ADR 0083). A key the connection scope does not take is refused
    /// with `setting_not_applicable`, and the `hello` with it.
    #[serde(default, skip_serializing_if = "serde_json::Map::is_empty")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub settings: serde_json::Map<String, serde_json::Value>,
}

impl HelloParams {
    /// The `hello` every Client builds on: the protocol version this
    /// crate speaks, `surface`, its own `product_version`, and every other
    /// field at its default, no form, token, Product, log level, relayed
    /// surface, keep-alive or settings, which a caller sets by struct update.
    pub fn new(surface: Surface, product_version: ProductVersion) -> Self {
        Self {
            protocol_version: PROTOCOL_VERSION,
            surface,
            form: None,
            token: None,
            needs: None,
            product_version,
            log_level: None,
            client_surface: None,
            keep_daemon_alive: false,
            settings: serde_json::Map::new(),
        }
    }
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
    /// How many calls the engine keeps in flight on this Connection: one
    /// more is refused with `too_many_calls`, so a Client sends at most
    /// this many at once and queues the rest in the order they were made
    /// (ADR 0038; issue 2529). Absent from an engine that sets no cap, such
    /// as the Scripted engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max_calls_in_flight: Option<NonZeroU64>,
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
    pub waited_for: WaitedFor,
    /// The enclosing call whose timeout ended it, when that call's end came
    /// before its own (ADR 0080): `limit` is then that call's timeout.
    /// Absent when its own timeout ended it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub ended_by: Option<EnclosingCall>,
    /// The last state seen before it ended, such as a navigation under way
    /// on its page, when the engine saw one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub observed: Option<String>,
}

/// The enclosing call a timeout belongs to, as `deadline_exceeded`'s
/// `ended_by` names it (ADR 0080): the calls made inside a call inherit its
/// end, so none outlasts it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EnclosingCall {
    /// Its operation, such as `crawl_request.handle`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub operation: String,
    /// Its request's id on its Connection.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub call_id: RequestId,
}

/// What a call past its deadline waited for, as `deadline_exceeded` names
/// it: a value of a type both Products share, open as ADR 0099 has such
/// types, so Foxhound adds its own (`goodfetch_protocol::foxhound`'s) and a
/// reader keeps one it does not know. Each of Collie's values is an
/// associated constant.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(transparent)]
#[schemars(transparent)]
pub struct WaitedFor(Cow<'static, str>);

impl WaitedFor {
    /// A response's first byte.
    pub const FIRST_BYTE: Self = Self::named("first_byte");
    /// A navigation's end.
    pub const NAVIGATION: Self = Self::named("navigation");
    /// A request the page sends.
    pub const REQUEST: Self = Self::named("request");
    /// A response to a request.
    pub const RESPONSE: Self = Self::named("response");
    /// A request's end.
    pub const FINISHED_REQUEST: Self = Self::named("finished request");
    /// A response's whole body.
    pub const RESPONSE_BODY: Self = Self::named("response body");
    /// The page's URL to match.
    pub const URL: Self = Self::named("url");
    /// A page function to return a truthy value.
    pub const FUNCTION: Self = Self::named("function");
    /// A navigation's commit.
    pub const COMMIT: Self = Self::named("commit");
    /// The document's `DOMContentLoaded`.
    pub const DOM_CONTENT_LOADED: Self = Self::named("domcontentloaded");
    /// The document's `load`.
    pub const LOAD: Self = Self::named("load");
    /// The page's network to be idle.
    pub const NETWORK_IDLE: Self = Self::named("networkidle");
    /// The browser's answer to a command.
    pub const BROWSER_ANSWER: Self = Self::named("browser_answer");
    /// A Run's end.
    pub const RUN_END: Self = Self::named("Run's end");
    /// A Run's next safe point.
    pub const RUN_SAFE_POINT: Self = Self::named("Run's next safe point");
    /// A call's answer, which an enclosing call's end came before.
    pub const ANSWER: Self = Self::named("answer");
    /// The engine's answer to `hello`, which opening a Connection waits
    /// for within `timeout.connect` (ADR 0083).
    pub const HELLO: Self = Self::named("hello");
    /// A place among the Connection's calls in flight, which a call the
    /// Client queued past the engine's cap waits for before it is sent
    /// (ADR 0038).
    pub const CALLS_IN_FLIGHT: Self = Self::named("calls_in_flight");
    /// A locator's matches to number what its count allows.
    pub const TARGET_COUNT: Self = Self::named("target_count");
    /// A locator's target to be detached.
    pub const TARGET_DETACHED: Self = Self::named("target_detached");
    /// A locator to match one element.
    pub const TARGET_UNIQUE: Self = Self::named("target_unique");
    /// A locator's target to be hidden.
    pub const TARGET_HIDDEN: Self = Self::named("target_hidden");
    /// A locator's target to be attached.
    pub const TARGET_ATTACHED: Self = Self::named("target_attached");
    /// A locator's target to be visible.
    pub const TARGET_VISIBLE: Self = Self::named("target_visible");
    /// A locator's target to be enabled.
    pub const TARGET_ENABLED: Self = Self::named("target_enabled");
    /// A locator's target to be editable.
    pub const TARGET_EDITABLE: Self = Self::named("target_editable");
    /// A locator's target to stop moving.
    pub const TARGET_STABLE: Self = Self::named("target_stable");
    /// A locator's target to be the one that receives the event.
    pub const TARGET_RECEIVES_EVENTS: Self = Self::named("target_receives_events");

    /// The value named `name`, for a Product's own constant.
    pub const fn named(name: &'static str) -> Self {
        Self(Cow::Borrowed(name))
    }

    /// Its name on the wire.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for WaitedFor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// Who stopped a Run as an abort, the `by` of its `abort` stop reason; a
/// call the stop cancels names it as its `by` (ADR 0113).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum StoppedBy {
    /// A Client, through `run.abort`.
    Client,
    /// The program, such as through `run.stop` (ADR 0102).
    Program,
    /// The browser, outside every Client.
    Browser,
}

/// Who stopped a call that `call_cancelled` ended (ADR 0113): the
/// vocabulary of an Agent's `abort` (ADR 0100), plus `browser`, open
/// (ADR 0079).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum CancelledBy {
    /// A person, through a Client that acts for them (ADR 0031).
    Person,
    /// Another Client, such as through `$/cancel`.
    Client,
    /// The program, closing a parent value from another task or thread.
    Program,
    /// The browser, outside every Client: a page's window closed there, or
    /// the browser quit.
    Browser,
    /// Who a newer engine names that this reader does not know, kept as
    /// received.
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

impl CancelledBy {
    /// Its name on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown(value) => value.as_str(),
            known => crate::wire_name::of(known),
        }
    }
}

/// Whoever stopped a Run stopped the calls in flight under it.
impl From<StoppedBy> for CancelledBy {
    fn from(by: StoppedBy) -> Self {
        match by {
            StoppedBy::Client => Self::Client,
            StoppedBy::Program => Self::Program,
            StoppedBy::Browser => Self::Browser,
        }
    }
}

/// What was closed or stopped under a call that `call_cancelled` ended
/// (ADR 0113), open (ADR 0079).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
#[schemars(extend("open" = true), transform = crate::open_value::known_members)]
#[non_exhaustive]
pub enum ClosedValue {
    /// Its page.
    Page,
    /// Its page's Session.
    Session,
    /// Its page's browser.
    Browser,
    /// Its Run, stopped.
    Run,
    /// Its Connection.
    Connection,
    /// What a newer engine names that this reader does not know, kept as
    /// received.
    #[serde(untagged)]
    #[schemars(skip)]
    Unknown(UnknownValue),
}

impl ClosedValue {
    /// Its name on the wire.
    pub fn as_str(&self) -> &str {
        match self {
            Self::Unknown(value) => value.as_str(),
            known => crate::wire_name::of(known),
        }
    }
}

/// What `call_cancelled` details (ADR 0113).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct CallCancelledDetails {
    /// Who stopped the call.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub by: CancelledBy,
    /// What was closed or stopped under it, when something was rather than
    /// the call itself cancelled.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub closed: Option<ClosedValue>,
    /// Whom the stopping Client acts for (ADR 0031), when known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(extend("arrived_in" = "0.1"))]
    pub person: Option<String>,
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

/// A Connection, or a value under it, used in a process other than the one
/// that opened it, such as a forked child (ADR 0078), raised by the SDK
/// runtime itself, which leaves the opening process's engine untouched.
pub const CONNECTION_FORKED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "connection_forked",
    description: "The call was made in another process than the one that opened its Connection, such as a child created by `fork`: a Connection and every Run, Session or Page under it belong to the process that opened them. Connect in the child process.",
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

/// A call an SDK runtime's own bounds refused unsent (ADRs 0079 and 0081).
pub const CONNECTION_OVERLOADED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "connection_overloaded",
    description: "The Connection already holds as many calls waiting for their answer, or as many bytes waiting to be written to the engine, as its bounds allow, so the call was refused at once and never sent; the calls made before it go on. The details name the bound, `pending_calls` or `queued_write_bytes`, and its value. Wait for earlier calls to end, or raise `connection.max_pending_calls` or `connection.max_queued_write_bytes` in `connect()`'s `settings`.",
    group: ErrorGroup::Refused,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// A new call refused by a Connection that holds as many calls in flight
/// as the engine serves on one (ADR 0038; issue 2312).
pub const TOO_MANY_CALLS: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "too_many_calls",
    description: "The Connection already has as many calls in flight as the engine serves on one, which `max` gives, so the engine refused this call at once and did nothing for it; the calls in flight go on, and the engine still reads their `$/cancel`, their `$/credit` and the replies to its own requests. Make the call again once an earlier call ended, cancel the calls no longer needed, or open another Connection.",
    group: ErrorGroup::Refused,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// What `too_many_calls` names: how many calls in flight the engine serves
/// on one Connection.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct TooManyCallsDetails {
    /// How many calls one Connection keeps in flight.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub max: u64,
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

/// An Attached engine that ended before its Connection was closed (ADRs
/// 0078 and 0079), raised by the SDK runtime.
pub const ENGINE_EXITED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "engine_exited",
    description: "The engine ended before its Connection was closed, so the call has no answer. An Attached engine's details carry its exit code or signal and the last 64 KiB of its stderr; a linked engine's carry none.",
    group: ErrorGroup::Unavailable,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The evidence `engine_exited` carries of how the engine ended (ADR
/// 0078). Each field is always sent: a linked engine, which runs in the
/// program's own process, has no exit code or signal, both `null`, and no
/// stderr of its own, an empty tail.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EngineExitedDetails {
    /// The engine's exit code; `null` when a signal ended it, or when it
    /// is linked.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub exit_code: Option<i32>,
    /// The signal that ended it, such as `SIGKILL`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub signal: Option<String>,
    /// The last 64 KiB of its stderr, cut where a character starts.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub stderr_tail: String,
}

impl EngineExitedDetails {
    /// The evidence when nothing tells how the engine ended, as of a linked
    /// engine: no exit code, no signal and an empty tail.
    #[must_use]
    pub const fn without_evidence() -> Self {
        Self {
            exit_code: None,
            signal: None,
            stderr_tail: String::new(),
        }
    }
}

/// A Connection to an engine that dropped while a call waited for its
/// answer (ADRs 0031 and 0079), raised by the Client's own runtime, which
/// then connects again.
pub const CONNECTION_LOST: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "connection_lost",
    description: "The Connection to the engine dropped before the call was answered, so whether it took effect is unknown. The Client connects again by itself; make the call again once it has, after checking that an Effect did not happen.",
    group: ErrorGroup::Unavailable,
    retryable: true,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

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

/// The refusal of an answer only a person gives (ADRs 0011 and 0041): an
/// operation the catalog marks [`Reach::PersonOnly`](crate::catalog::Reach)
/// asked by anything but the Kennel or an interactive CLI
/// prompt (ADR 0141).
pub const PERSON_REQUIRED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "person_required",
    description: "Only a person gives this answer: no agent surface, SDK, script or command run under `--json` or without a terminal may. Run the Operation's command in a terminal, where the CLI has one, or answer it in the Kennel where the Kennel offers the Operation.",
    group: ErrorGroup::Refused,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// Handler code that must outlive its request, asked for on an edge
/// runtime (ADR 0049), which ends what it runs with the request: raised by
/// an SDK before any message, so no engine sends it.
pub const EDGE_HANDLERS_UNSUPPORTED: ErrorCodeDefinition = ErrorCodeDefinition {
    name: "edge_handlers_unsupported",
    description: "The program runs on an edge runtime, Cloudflare Workers, Vercel's edge runtime or Deno Deploy, which ends what it runs with its request, and asked for code the engine would call after it: a crawl's handlers, an SDK handler, a Hook in code or a lent page. Nothing was sent. Run that code as a Recipe on the Daemon, and start or poll its Run from the edge function.",
    group: ErrorGroup::Program,
    retryable: false,
    exit_code: None,
    arrived_in: ARRIVED_IN,
};

/// The edge runtime a program runs on (ADR 0049), told by its marker.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum EdgeRuntime {
    /// Cloudflare Workers (`workerd`), whose `navigator.userAgent` is
    /// `Cloudflare-Workers`.
    CloudflareWorkers,
    /// Vercel's edge runtime, which sets the `EdgeRuntime` global.
    VercelEdge,
    /// Deno Deploy, whose environment holds `DENO_DEPLOYMENT_ID`.
    DenoDeploy,
}

/// What `edge_handlers_unsupported` details.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct EdgeHandlersUnsupportedDetails {
    /// The edge runtime the program runs on.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub runtime: EdgeRuntime,
    /// The call that asked for the handlers, by its catalog name, such as
    /// `crawl.start`.
    #[schemars(extend("arrived_in" = "0.1"))]
    pub operation: String,
}

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
        CONNECTION_FORKED,
        INTERNAL,
        ENGINE_PANICKED,
        CONNECTION_OVERLOADED,
        TOO_MANY_CALLS,
        ENGINE_NOT_FOUND,
        ENGINE_EXITED,
        CONNECTION_LOST,
        ENGINE_OUTPUT_INVALID,
        PERSON_REQUIRED,
        EDGE_HANDLERS_UNSUPPORTED,
    ],
    plugin_kinds: &[],
    metrics: &[],
    settings: &[
        TIMEOUT_HELLO,
        NETWORK_MAX_HELLO_SIZE,
        CONNECTION_MAX_PENDING_CALLS,
        CONNECTION_MAX_QUEUED_WRITE_BYTES,
        CONNECTION_MAX_LISTENER_QUEUE,
    ],
    hook_stages: &[],
    shared_type_values: &[],
};

mod bounds;
mod engine_panicked;
mod opening;

pub use bounds::{
    CONNECTION_MAX_LISTENER_QUEUE, CONNECTION_MAX_PENDING_CALLS, CONNECTION_MAX_QUEUED_WRITE_BYTES,
    applies_client_bounds,
};
pub use engine_panicked::{ENGINE_PANICKED, EnginePanickedDetails, PanicEnded};
pub use opening::{NETWORK_MAX_HELLO_SIZE, TIMEOUT_HELLO};

#[cfg(test)]
mod tests;
