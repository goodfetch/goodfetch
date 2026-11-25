//! `goodfetch-scripted-plugin` (ADR 0033): a process Plugin that answers
//! each call from a script, real code over the plugin protocol on which
//! the engine's Plugin host is tested, never a mock. Written once here, so
//! each test scripts the Plugin it needs rather than writing one.
//!
//! It refuses to run, writing nothing on stdout and exiting 1, unless its
//! environment holds the plugin protocol's cookie, as a Plugin's SDK-free
//! start does. Then it reads its script, the JSON file its one argument
//! names, relative to its working folder or not:
//!
//! ```json
//! {
//!   "hello": { "protocol_version": "0.1", "name": "scripted", … },
//!   "after_hello": [ { "jsonrpc": "2.0", "id": 99, "result": {} } ],
//!   "answers": { "sample_store.put": { "result": { "replaced": false } } }
//! }
//! ```
//!
//! - `hello` is its answer to `hello`, sent as it is;
//! - `after_hello` are messages it sends once that answer is sent, such as
//!   a second answer to `hello` or an answer to no call;
//! - `answers` answer each other call by its method with the `result` or
//!   the `error` given, or leave it unanswered on `{ "silent": true }`; a
//!   method it has no answer for is answered JSON-RPC's method-not-found
//!   error.
//!
//! It records every message it reads and answers three methods of its own
//! whatever the script says:
//! - `scripted.environment`: its whole environment, its working folder,
//!   the handles it holds beyond stdio, asked of the OS one number at a
//!   time, and its process ID;
//! - `scripted.calls`: every message it read before this call;
//! - `scripted.write`: writes `{"path", "text"}`'s text to the file, and
//!   answers `{}`, or the OS's refusal as an error.
//!
//! It exits 0 when its stdin closes, and 2, with the reason on standard
//! error, on a script it cannot read or a line that is no JSON.
//!
//! A test-only executable that no Product ships: it speaks over its own
//! stdio and writes its own stdout, beyond `STYLE.md`'s three writers.
#![expect(
    clippy::disallowed_methods,
    reason = "the test-only scripted Plugin reads its environment and files and speaks over its own stdio, as a Plugin does (ADR 0033)"
)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, BufRead as _, Write as _};
use std::process::ExitCode;

use serde_json::{Map, Value, json};

/// The plugin protocol's cookie, which `goodfetch-protocol` defines: test
/// support depends on no Goodfetch crate (ADR 0062).
const COOKIE_VARIABLE: &str = "GOODFETCH_PLUGIN_COOKIE";

/// The cookie's value.
const COOKIE: &str = "goodfetch-plugin-protocol";

/// JSON-RPC's code for a method the Plugin does not have.
const METHOD_NOT_FOUND: i64 = -32601;

/// JSON-RPC's code for a call it cannot carry out.
const INTERNAL_ERROR: i64 = -32603;

fn main() -> ExitCode {
    if std::env::var_os(COOKIE_VARIABLE).as_deref() != Some(COOKIE.as_ref()) {
        return ExitCode::FAILURE;
    }
    match play() {
        Ok(()) => ExitCode::SUCCESS,
        Err(reason) => {
            let _ = writeln!(io::stderr(), "goodfetch-scripted-plugin: {reason}");
            ExitCode::from(2)
        }
    }
}

/// Plays the script its argument names until its stdin closes.
fn play() -> Result<(), String> {
    let path = std::env::args_os().nth(1).ok_or("no script is named")?;
    let text = fs::read_to_string(&path)
        .map_err(|error| format!("cannot read {}: {error}", path.display()))?;
    let script: Value = serde_json::from_str(&text)
        .map_err(|error| format!("{} is no JSON: {error}", path.display()))?;
    let mut calls: Vec<Value> = Vec::new();
    let (stdin, mut stdout) = (io::stdin().lock(), io::stdout().lock());
    for line in stdin.lines() {
        let line = line.map_err(|error| error.to_string())?;
        if line.trim().is_empty() {
            continue;
        }
        let message: Value =
            serde_json::from_str(&line).map_err(|error| format!("{line:?} is no JSON: {error}"))?;
        let mut sent = Vec::new();
        if let (Some(id), Some(method)) = (message.get("id"), message["method"].as_str()) {
            let answer = answer(&script, method, &message["params"], &calls);
            if answer["silent"] != json!(true) {
                sent.push(with_id(answer, id));
            }
            if method == "hello" {
                sent.extend(
                    script["after_hello"]
                        .as_array()
                        .cloned()
                        .unwrap_or_default(),
                );
            }
        }
        calls.push(message);
        for message in sent {
            let mut line = serde_json::to_vec(&message).map_err(|error| error.to_string())?;
            line.push(b'\n');
            stdout
                .write_all(&line)
                .and_then(|()| stdout.flush())
                .map_err(|error| error.to_string())?;
        }
    }
    Ok(())
}

/// The answer to `method` with `params`, with no id yet.
fn answer(script: &Value, method: &str, params: &Value, calls: &[Value]) -> Value {
    match method {
        "hello" => json!({ "result": script["hello"] }),
        "scripted.environment" => json!({ "result": environment() }),
        "scripted.calls" => json!({ "result": calls }),
        "scripted.write" => {
            let written = match (params["path"].as_str(), params["text"].as_str()) {
                (Some(path), Some(text)) => {
                    fs::write(path, text).map_err(|error| error.to_string())
                }
                _ => Err("`scripted.write` takes a path and a text".to_owned()),
            };
            match written {
                Ok(()) => json!({ "result": {} }),
                Err(reason) => error(INTERNAL_ERROR, &reason),
            }
        }
        _ => script["answers"]
            .get(method)
            .cloned()
            .unwrap_or_else(|| error(METHOD_NOT_FOUND, &format!("no answer to `{method}`"))),
    }
}

fn error(code: i64, message: &str) -> Value {
    json!({ "error": { "code": code, "message": message } })
}

/// `answer` as the reply to the request `id`.
fn with_id(answer: Value, id: &Value) -> Value {
    let mut reply = Map::new();
    reply.insert("jsonrpc".to_owned(), json!("2.0"));
    reply.insert("id".to_owned(), id.clone());
    if let Value::Object(members) = answer {
        reply.extend(members);
    }
    Value::Object(reply)
}

/// Its environment, working folder, handles beyond stdio and process ID.
fn environment() -> Value {
    let variables: BTreeMap<String, String> = std::env::vars_os()
        .map(|(name, value)| {
            (
                name.to_string_lossy().into_owned(),
                value.to_string_lossy().into_owned(),
            )
        })
        .collect();
    let folder = std::env::current_dir()
        .map(|folder| folder.to_string_lossy().into_owned())
        .unwrap_or_default();
    json!({
        "variables": variables,
        "folder": folder,
        "handles": handles(),
        "process": std::process::id(),
    })
}

/// The numbers of the handles it holds beyond stdio, asked of each number
/// in turn: a confined Plugin may not read `/proc`, where Linux lists them.
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "asking whether a handle is open is a raw system call"
)]
fn handles() -> Vec<i32> {
    /// Past the handles a Plugin could inherit: its launcher keeps none.
    const ASKED: i32 = 1024;
    (3..ASKED)
        // SAFETY: `F_GETFD` only reads the handle's flags, of a number that
        // need not be open.
        .filter(|&number| unsafe { libc::fcntl(number, libc::F_GETFD) } != -1)
        .collect()
}

/// None off Unix, where no Plugin runs.
#[cfg(not(unix))]
fn handles() -> Vec<i32> {
    Vec::new()
}
