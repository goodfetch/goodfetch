//! `goodfetch-toy-plugin` (#171): a process Plugin of Collie's sample Plugin
//! kind, `sample_store`, real code over the plugin protocol (ADR 0033) on
//! which the conformance harness's process Plugin target is proved. It
//! keeps values by key in memory and answers `sample_store.put` with
//! whether the value replaced one.
//!
//! It refuses to run, writing nothing on stdout, unless its environment
//! holds the plugin protocol's cookie, then answers `hello` with the newest
//! plugin protocol version it shares with the starter, the kind and its
//! capabilities `put`, `replace` and `newer_params`, at network level
//! `none`; sharing none, it answers `plugin_protocol_unsupported`. Flags
//! turn it into the toy Plugins a suite must not prove, or the harness must
//! refuse:
//! - `--forget` forgets every value, so a second `put` under a key still
//!   answers `replaced: false`, which breaks the declared `replace`;
//! - `--leave-out <capability>` leaves that capability out of its `hello`;
//! - `--protocol <min> <max>` speaks those plugin protocol versions rather
//!   than the engine's own;
//! - `--claim-kind <kind>` declares that kind too, which no manifest
//!   approved;
//! - `--speak-first` sends a notification before its answer to `hello`.
//!
//! It exits 0 when its stdin closes, and 1, with the reason on standard
//! error, on a line that is no JSON-RPC message.
//!
//! A test-only executable that no Product ships: it speaks over its own
//! stdio and writes its own stdout, beyond `STYLE.md`'s three writers.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsString;
use std::fmt;
use std::io::{self, BufRead as _, Write as _};

use goodfetch_protocol::catalog::ProtocolVersion;
use goodfetch_protocol::jsonrpc::{ErrorData, ErrorObject};
use goodfetch_protocol::plugin::{
    COOKIE, COOKIE_VARIABLE, DeclaredKind, HELLO, NetworkLevel, PLUGIN_PROTOCOL,
    PLUGIN_PROTOCOL_UNSUPPORTED, PluginHelloParams, PluginHelloResult, PluginKindName,
    PluginProtocolRange,
};
use goodfetch_protocol::sample::{SampleStorePutParams, SampleStorePutResult};
use serde_json::{Map, Value, json};

/// The kind it implements.
const KIND: PluginKindName = PluginKindName::from_static("sample_store");

/// The method of the kind that keeps a value.
const PUT: &str = "sample_store.put";

/// The capabilities of the kind it declares unless one is left out.
const CAPABILITIES: [&str; 3] = ["put", "replace", "newer_params"];

/// JSON-RPC's code for a method the Plugin does not have.
const METHOD_NOT_FOUND: i64 = -32601;

/// JSON-RPC's code for params a method cannot read.
const INVALID_PARAMS: i64 = -32602;

fn main() -> Result<(), Failure> {
    #[expect(
        clippy::disallowed_methods,
        reason = "the toy Plugin's `main` reads the cookie its starter sets, as a Plugin's `main` does (ADR 0033)"
    )]
    let cookie = std::env::var_os(COOKIE_VARIABLE);
    if cookie.as_deref() != Some(COOKIE.as_ref()) {
        return Err(Failure::NoCookie);
    }
    let mut plugin = Plugin::from_arguments(std::env::args_os().skip(1))?;
    #[expect(
        clippy::disallowed_methods,
        reason = "the test-only toy Plugin speaks the plugin protocol over its own stdio (ADR 0033)"
    )]
    let (stdin, mut stdout) = (io::stdin().lock(), io::stdout().lock());
    for line in stdin.lines() {
        let line = line.map_err(Failure::Io)?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Value =
            serde_json::from_str(&line).map_err(|source| Failure::NoMessage { line, source })?;
        for answer in plugin.answer(&message) {
            writeln!(stdout, "{answer}").map_err(Failure::Io)?;
        }
        stdout.flush().map_err(Failure::Io)?;
    }
    Ok(())
}

/// Why a call is refused: a JSON-RPC error object.
type Refusal = Value;

/// The toy Plugin's state: what it declares and the values it keeps.
struct Plugin {
    forget: bool,
    left_out: BTreeSet<String>,
    protocol: PluginProtocolRange,
    claimed: Vec<PluginKindName>,
    speak_first: bool,
    values: BTreeMap<String, String>,
}

impl Plugin {
    /// Reads `--forget`, `--leave-out <capability>`, `--protocol <min>
    /// <max>`, `--claim-kind <kind>` and `--speak-first`.
    fn from_arguments(arguments: impl Iterator<Item = OsString>) -> Result<Self, Failure> {
        let mut plugin = Self {
            forget: false,
            left_out: BTreeSet::new(),
            protocol: PLUGIN_PROTOCOL,
            claimed: Vec::new(),
            speak_first: false,
            values: BTreeMap::new(),
        };
        let mut arguments = arguments;
        let text = |argument: OsString| argument.into_string().map_err(|_| Failure::Usage);
        let value = |arguments: &mut dyn Iterator<Item = OsString>| {
            arguments.next().ok_or(Failure::Usage).and_then(text)
        };
        let version = |text: String| text.parse::<ProtocolVersion>().map_err(|_| Failure::Usage);
        while let Some(argument) = arguments.next() {
            match text(argument)?.as_str() {
                "--forget" => plugin.forget = true,
                "--speak-first" => plugin.speak_first = true,
                "--leave-out" => {
                    plugin.left_out.insert(value(&mut arguments)?);
                }
                "--protocol" => {
                    plugin.protocol = PluginProtocolRange {
                        min: version(value(&mut arguments)?)?,
                        max: version(value(&mut arguments)?)?,
                    };
                }
                "--claim-kind" => {
                    let kind =
                        PluginKindName::new(&value(&mut arguments)?).ok_or(Failure::Usage)?;
                    plugin.claimed.push(kind);
                }
                _ => return Err(Failure::Usage),
            }
        }
        Ok(plugin)
    }

    /// Its answers to `message`, in order: none to a notification.
    fn answer(&mut self, message: &Value) -> Vec<Value> {
        let Some(id) = message.get("id").cloned() else {
            return Vec::new();
        };
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let mut answers = Vec::new();
        let outcome = match method {
            Some(HELLO) => {
                if self.speak_first {
                    answers.push(json!({ "jsonrpc": "2.0", "method": "$/log", "params": {} }));
                }
                self.hello(params)
            }
            Some(PUT) => self.put(params),
            _ => Err(protocol_failure(
                METHOD_NOT_FOUND,
                "the toy Plugin has no such method",
            )),
        };
        answers.push(match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err(error) => json!({ "jsonrpc": "2.0", "id": id, "error": error }),
        });
        answers
    }

    /// Its `hello` result in the newest version it shares with the starter,
    /// or `plugin_protocol_unsupported` when it shares none.
    fn hello(&self, params: Value) -> Result<Value, Refusal> {
        let params: PluginHelloParams = serde_json::from_value(params)
            .map_err(|_| protocol_failure(INVALID_PARAMS, "no `hello` params"))?;
        let Some(protocol_version) = self.protocol.newest_shared(params.protocol) else {
            let refusal = ErrorObject {
                code: PLUGIN_PROTOCOL_UNSUPPORTED.group.json_rpc_class(),
                message: format!(
                    "the toy Plugin speaks plugin protocol {}, and the starter {}",
                    self.protocol, params.protocol
                ),
                data: ErrorData {
                    code: PLUGIN_PROTOCOL_UNSUPPORTED.name.to_owned(),
                    retryable: PLUGIN_PROTOCOL_UNSUPPORTED.retryable,
                    details: Map::new(),
                },
            };
            return Err(serde_json::to_value(refusal).expect("invariant: an error serializes"));
        };
        let capabilities = CAPABILITIES
            .iter()
            .filter(|capability| !self.left_out.contains(**capability))
            .map(|capability| (*capability).to_owned())
            .collect();
        let claimed = self.claimed.iter().map(|&kind| DeclaredKind {
            kind,
            capabilities: Vec::new(),
        });
        Ok(serde_json::to_value(PluginHelloResult {
            protocol_version,
            name: env!("CARGO_BIN_NAME").to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            kinds: std::iter::once(DeclaredKind {
                kind: KIND,
                capabilities,
            })
            .chain(claimed)
            .collect(),
            hook_stages: BTreeSet::new(),
            network_level: NetworkLevel::None,
        })
        .expect("invariant: a `hello` result serializes"))
    }

    fn put(&mut self, params: Value) -> Result<Value, Refusal> {
        let params: SampleStorePutParams = serde_json::from_value(params)
            .map_err(|_| protocol_failure(INVALID_PARAMS, "no `sample_store.put` params"))?;
        let replaced = if self.forget {
            false
        } else {
            self.values.insert(params.key, params.value).is_some()
        };
        Ok(serde_json::to_value(SampleStorePutResult { replaced })
            .expect("invariant: a `put` result serializes"))
    }
}

/// A protocol failure's error object, with one of JSON-RPC's predefined
/// codes.
fn protocol_failure(code: i64, reason: &str) -> Refusal {
    json!({ "code": code, "message": reason })
}

/// Why the toy Plugin stopped.
enum Failure {
    NoCookie,
    Usage,
    Io(io::Error),
    NoMessage {
        line: String,
        source: serde_json::Error,
    },
}

impl fmt::Display for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoCookie => write!(
                formatter,
                "this is a Goodfetch Plugin, started by the engine or the conformance harness: {COOKIE_VARIABLE} is not set"
            ),
            Self::Usage => write!(
                formatter,
                "usage: goodfetch-toy-plugin [--forget] [--leave-out <capability>]… [--protocol <min> <max>] [--claim-kind <kind>]… [--speak-first]"
            ),
            Self::Io(source) => write!(formatter, "stdio failed: {source}"),
            Self::NoMessage { line, source } => {
                write!(formatter, "`{line}` is no JSON-RPC message: {source}")
            }
        }
    }
}

/// `main`'s error goes to standard error through `Debug`.
impl fmt::Debug for Failure {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
