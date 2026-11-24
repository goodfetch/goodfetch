//! `goodfetch-toy-plugin` (#171): a process Plugin of Collie's sample Plugin
//! kind, `sample_store`, real code over the plugin protocol (ADR 0033) on
//! which the conformance harness's process Plugin target is proved. It
//! keeps values by key in memory and answers `sample_store.put` with
//! whether the value replaced one.
//!
//! It refuses to run, writing nothing on stdout, unless its environment
//! holds the plugin protocol's cookie, then answers `hello` with the kind
//! and its capabilities `put`, `replace` and `newer_params`. Two flags turn
//! it into the toy Plugins a suite must not prove:
//! - `--forget` forgets every value, so a second `put` under a key still
//!   answers `replaced: false`, which breaks the declared `replace`;
//! - `--leave-out <capability>` leaves that capability out of its `hello`.
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

use goodfetch_protocol::plugin::{
    COOKIE, COOKIE_VARIABLE, DeclaredKind, HELLO, PLUGIN_PROTOCOL_VERSION, PluginHelloResult,
};
use goodfetch_protocol::sample::{SampleStorePutParams, SampleStorePutResult};
use serde_json::{Value, json};

/// The kind it implements.
const KIND: &str = "sample_store";

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
        if let Some(answer) = plugin.answer(&message) {
            writeln!(stdout, "{answer}").map_err(Failure::Io)?;
            stdout.flush().map_err(Failure::Io)?;
        }
    }
    Ok(())
}

/// The toy Plugin's state: what it declares and the values it keeps.
struct Plugin {
    forget: bool,
    left_out: BTreeSet<String>,
    values: BTreeMap<String, String>,
}

impl Plugin {
    /// Reads `--forget` and `--leave-out <capability>`.
    fn from_arguments(arguments: impl Iterator<Item = OsString>) -> Result<Self, Failure> {
        let mut plugin = Self {
            forget: false,
            left_out: BTreeSet::new(),
            values: BTreeMap::new(),
        };
        let mut arguments = arguments;
        while let Some(argument) = arguments.next() {
            match argument.to_str() {
                Some("--forget") => plugin.forget = true,
                Some("--leave-out") => {
                    let capability = arguments
                        .next()
                        .and_then(|capability| capability.into_string().ok())
                        .ok_or(Failure::Usage)?;
                    plugin.left_out.insert(capability);
                }
                _ => return Err(Failure::Usage),
            }
        }
        Ok(plugin)
    }

    /// Its answer to `message`: none to a notification.
    fn answer(&mut self, message: &Value) -> Option<Value> {
        let id = message.get("id")?.clone();
        let method = message.get("method").and_then(Value::as_str);
        let params = message.get("params").cloned().unwrap_or(Value::Null);
        let outcome = match method {
            Some(HELLO) => Ok(self.hello()),
            Some(PUT) => self.put(params),
            _ => Err((METHOD_NOT_FOUND, "the toy Plugin has no such method")),
        };
        Some(match outcome {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, reason)) => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": { "code": code, "message": reason },
            }),
        })
    }

    fn hello(&self) -> Value {
        let capabilities = CAPABILITIES
            .iter()
            .filter(|capability| !self.left_out.contains(**capability))
            .map(|capability| (*capability).to_owned())
            .collect();
        serde_json::to_value(PluginHelloResult {
            protocol_version: PLUGIN_PROTOCOL_VERSION,
            name: env!("CARGO_BIN_NAME").to_owned(),
            version: env!("CARGO_PKG_VERSION").to_owned(),
            kinds: vec![DeclaredKind {
                kind: KIND.to_owned(),
                capabilities,
            }],
        })
        .expect("invariant: a `hello` result serializes")
    }

    fn put(&mut self, params: Value) -> Result<Value, (i64, &'static str)> {
        let params: SampleStorePutParams = serde_json::from_value(params)
            .map_err(|_| (INVALID_PARAMS, "no `sample_store.put` params"))?;
        let replaced = if self.forget {
            false
        } else {
            self.values.insert(params.key, params.value).is_some()
        };
        Ok(serde_json::to_value(SampleStorePutResult { replaced })
            .expect("invariant: a `put` result serializes"))
    }
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
                "usage: goodfetch-toy-plugin [--forget] [--leave-out <capability>]…"
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
