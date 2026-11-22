//! The corpus check (ADR 0069, Quick suite): every message step of every
//! script in `test-support/fixtures/engine-scripts/` validates against the
//! committed JSON Schema bundle and names a method the catalog or the
//! connection has, so no script sends what the engine could never send. `raw` steps are skipped.
//! The check is shown refusing the fixtures beside it.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use jsonschema::{Registry, Validator};
use serde_json::{Value, json};

use super::{ENGINE_SCRIPTS, Reply, Script, Sent, Step};

/// The committed bundle and catalog, as `xtask codegen` wrote them.
const BUNDLE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../goodfetch-protocol/generated/bundle.json"
);
const CATALOG: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../goodfetch-protocol/generated/catalog.json"
);

/// The URI the bundle is registered under, which its `#/$defs/…`
/// references resolve against.
const BUNDLE_URI: &str = "urn:goodfetch:bundle";

/// The check's own fixtures: scripts it must refuse.
const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/script/fixtures");

/// The codes JSON-RPC predefines for a protocol failure, whose `data.code`
/// names the failure rather than a catalog Error code.
const PROTOCOL_FAILURE_CODES: [i64; 5] = [-32700, -32600, -32601, -32602, -32603];

/// Why a step of a script fails the check.
#[derive(Debug, PartialEq, Eq)]
enum Violation {
    /// An `expect` or a `send` names a method the catalog does not have.
    UnknownMethod { step: usize, method: String },
    /// A message breaks the bundle's schema.
    Schema { step: usize, errors: Vec<String> },
    /// A failure names no Error code of the catalog with its group's class,
    /// and no protocol failure.
    UnknownErrorCode { step: usize, code: Value },
    /// A `reply` or `await_reply` has no request before it to answer.
    NothingToAnswer { step: usize },
}

/// What the next `reply` answers.
enum Answering {
    /// No request: a `reply` has nothing to answer.
    Nothing,
    /// A request of a method the check already refused.
    Unknown,
    /// A request of this method.
    Request(Method),
}

/// Which schemas a method's messages take.
struct Method {
    params: Value,
    result: Option<Value>,
    notification: bool,
}

/// The committed contract a script is checked against.
///
/// The bundle is checked against the JSON Schema meta-schema and indexed
/// once; each schema a message is checked against is compiled once, on first
/// use, so the check's time grows with the schemas the corpus uses, not with
/// its messages.
struct Contract {
    registry: Registry<'static>,
    validators: RefCell<HashMap<String, Validator>>,
    connection: Value,
    catalog: Value,
}

impl Contract {
    fn committed() -> Self {
        let bundle: Value = serde_json::from_slice(&std::fs::read(BUNDLE).unwrap()).unwrap();
        let catalog: Value = serde_json::from_slice(&std::fs::read(CATALOG).unwrap()).unwrap();
        if let Err(error) = jsonschema::meta::validate(&bundle) {
            panic!("the committed bundle is no valid JSON Schema: {error}");
        }
        let connection = bundle["connection"].clone();
        let registry = Registry::new()
            .add(BUNDLE_URI, bundle)
            .unwrap()
            .prepare()
            .unwrap();
        Self {
            registry,
            validators: RefCell::default(),
            connection,
            catalog,
        }
    }

    /// A method the Client sends: `hello`, `$/cancel`, `$/credit` or an
    /// operation.
    fn client_method(&self, name: &str) -> Option<Method> {
        match name {
            "hello" => Some(Method {
                params: self.connection["hello"]["params"].clone(),
                result: Some(self.connection["hello"]["result"].clone()),
                notification: false,
            }),
            "$/cancel" | "$/credit" => Some(Method {
                params: self.connection[name.trim_start_matches("$/")]["params"].clone(),
                result: None,
                notification: true,
            }),
            _ => self.entry("operations", name).map(|operation| Method {
                params: operation["params"].clone(),
                result: Some(operation["result"].clone()),
                notification: false,
            }),
        }
    }

    /// The params of a notification the engine sends: the connection's
    /// `$/events`, `$/chunk`, `$/key` or `$/log`, or an event by its catalog name.
    fn event(&self, name: &str) -> Option<Value> {
        match name {
            // The connection's notifications the engine sends: a sequence's
            // items, a Key-value store's keys, an enqueue's answers and the
            // engine's log events.
            "$/events" | "$/chunk" | "$/record" | "$/key" | "$/enqueued" | "$/log" => {
                Some(self.connection[name.trim_start_matches("$/")]["params"].clone())
            }
            _ => self
                .entry("events", name)
                .map(|event| event["payload"].clone()),
        }
    }

    fn entry(&self, section: &str, name: &str) -> Option<&Value> {
        self.catalog[section]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| entry["name"] == name)
    }

    /// The catalog's numeric class of an Error code, through its group.
    fn error_class(&self, code: &Value) -> Option<&Value> {
        self.catalog["error_codes"]
            .as_array()
            .unwrap()
            .iter()
            .find(|entry| &entry["name"] == code)
            .map(|entry| &entry["json_rpc_class"])
    }

    /// What `schema`, a reference into the bundle as the catalog and the
    /// connection give every schema, finds wrong with `instance`.
    fn errors(&self, schema: &Value, instance: &Value) -> Vec<String> {
        let reference = match schema.as_object() {
            Some(object) if object.len() == 1 => object["$ref"].as_str(),
            _ => None,
        }
        .filter(|reference| reference.starts_with("#/"))
        .unwrap_or_else(|| panic!("a schema that is no reference into the bundle: {schema}"));
        let mut validators = self.validators.borrow_mut();
        let validator = validators.entry(reference.to_owned()).or_insert_with(|| {
            jsonschema::options()
                .with_registry(&self.registry)
                .build(&json!({
                    "$schema": "https://json-schema.org/draft/2020-12/schema",
                    "$ref": format!("{BUNDLE_URI}{reference}"),
                }))
                .unwrap()
        });
        validator
            .iter_errors(instance)
            .map(|error| format!("{}: {error}", error.instance_path()))
            .collect()
    }

    fn envelope(&self, definition: &str, message: &Value) -> Vec<String> {
        self.errors(&json!({ "$ref": format!("#/$defs/{definition}") }), message)
    }
}

/// Every way `script` breaks the contract.
fn violations(script: &Script, contract: &Contract) -> Vec<Violation> {
    let mut found = Vec::new();
    let mut answering = Answering::Nothing;
    let mut sent_request = false;
    for (step, kind) in script.steps.iter().enumerate() {
        let mut schema_errors = Vec::new();
        match kind {
            Step::Expect(message) => {
                let Some(method) = contract.client_method(&message.method) else {
                    found.push(Violation::UnknownMethod {
                        step,
                        method: message.method.clone(),
                    });
                    answering = Answering::Unknown;
                    continue;
                };
                let mut wire = json!({ "jsonrpc": "2.0", "method": message.method });
                if !method.notification {
                    wire["id"] = json!(0);
                }
                if let Some(params) = &message.params {
                    wire["params"] = params.clone();
                }
                let envelope = if method.notification {
                    "Notification"
                } else {
                    "Request"
                };
                schema_errors.extend(contract.envelope(envelope, &wire));
                schema_errors.extend(contract.errors(
                    &method.params,
                    message.params.as_ref().unwrap_or(&Value::Null),
                ));
                // A notification leaves the request before it to answer, as
                // the player does.
                if !method.notification {
                    answering = Answering::Request(method);
                }
            }
            Step::Reply(answer) => {
                let method = match std::mem::replace(&mut answering, Answering::Nothing) {
                    Answering::Request(method) => method,
                    Answering::Unknown => continue,
                    Answering::Nothing => {
                        found.push(Violation::NothingToAnswer { step });
                        continue;
                    }
                };
                schema_errors.extend(answer_errors(contract, answer, method.result.as_ref()));
                if let Some(violation) = unknown_error_code(contract, answer, step) {
                    found.push(violation);
                }
            }
            Step::Send(Sent::Request(call)) => {
                let mut wire = json!({ "jsonrpc": "2.0", "id": 0, "method": call.method });
                if let Some(params) = &call.params {
                    wire["params"] = params.clone();
                }
                schema_errors.extend(contract.envelope("Request", &wire));
                sent_request = true;
            }
            Step::Send(Sent::Notification(call)) => {
                let Some(payload) = contract.event(&call.method) else {
                    found.push(Violation::UnknownMethod {
                        step,
                        method: call.method.clone(),
                    });
                    continue;
                };
                let mut wire = json!({ "jsonrpc": "2.0", "method": call.method });
                if let Some(params) = &call.params {
                    wire["params"] = params.clone();
                }
                schema_errors.extend(contract.envelope("Notification", &wire));
                schema_errors.extend(
                    contract.errors(&payload, call.params.as_ref().unwrap_or(&Value::Null)),
                );
            }
            Step::AwaitReply(answer) => {
                if !std::mem::take(&mut sent_request) {
                    found.push(Violation::NothingToAnswer { step });
                    continue;
                }
                schema_errors.extend(answer_errors(contract, answer, None));
            }
            // The next Client starts with nothing to answer, as the player
            // does.
            Step::Drop {} => {
                answering = Answering::Nothing;
                sent_request = false;
            }
            Step::Raw(_) | Step::Stall {} | Step::Close {} | Step::Exit(_) => {}
        }
        if !schema_errors.is_empty() {
            found.push(Violation::Schema {
                step,
                errors: schema_errors,
            });
        }
    }
    found
}

/// What the bundle finds wrong with an answer, as the wire carries it.
fn answer_errors(contract: &Contract, answer: &Reply, result: Option<&Value>) -> Vec<String> {
    match answer {
        Reply::Result(value) => {
            let wire = json!({ "jsonrpc": "2.0", "id": 0, "result": value });
            let mut errors = contract.envelope("Response", &wire);
            if let Some(schema) = result {
                errors.extend(contract.errors(schema, value));
            }
            errors
        }
        Reply::Error(value) => {
            let wire = json!({ "jsonrpc": "2.0", "id": 0, "error": value });
            contract.envelope("ErrorResponse", &wire)
        }
    }
}

/// A failure whose Error code the catalog lacks, or whose class is not its
/// group's; a protocol failure names its own code.
fn unknown_error_code(contract: &Contract, answer: &Reply, step: usize) -> Option<Violation> {
    let Reply::Error(error) = answer else {
        return None;
    };
    let class = &error["code"];
    if class
        .as_i64()
        .is_some_and(|class| PROTOCOL_FAILURE_CODES.contains(&class))
    {
        return None;
    }
    let code = &error["data"]["code"];
    (contract.error_class(code) != Some(class)).then(|| Violation::UnknownErrorCode {
        step,
        code: code.clone(),
    })
}

/// Every script file under `folder`, sorted.
fn scripts_under(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![folder.to_owned()];
    while let Some(current) = pending.pop() {
        for entry in std::fs::read_dir(&current).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                pending.push(path);
            } else if path
                .extension()
                .is_some_and(|extension| extension == "json")
                && path
                    .file_name()
                    .is_some_and(|name| name != "script.schema.json")
            {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn fixture(name: &str) -> Script {
    Script::read(&Path::new(FIXTURES).join(name)).unwrap()
}

#[test]
fn every_script_of_the_corpus_meets_the_committed_bundle() {
    let contract = Contract::committed();
    let scripts = scripts_under(Path::new(ENGINE_SCRIPTS));

    let found: Vec<(PathBuf, Vec<Violation>)> = scripts
        .iter()
        .map(|path| {
            (
                path.clone(),
                violations(&Script::read(path).unwrap(), &contract),
            )
        })
        .filter(|(_, violations)| !violations.is_empty())
        .collect();

    assert_ne!(scripts, Vec::<PathBuf>::new());
    assert_eq!(found, Vec::new());
}

#[test]
fn a_reply_that_breaks_the_bundles_schema_fails_the_check() {
    let contract = Contract::committed();

    let found = violations(&fixture("reply-breaks-the-schema.json"), &contract);

    assert!(
        matches!(found.as_slice(), [Violation::Schema { step: 1, .. }]),
        "{found:?}"
    );
}

#[test]
fn an_expect_naming_an_unknown_method_fails_the_check() {
    let contract = Contract::committed();

    let found = violations(&fixture("expect-names-an-unknown-method.json"), &contract);

    assert_eq!(
        found,
        vec![Violation::UnknownMethod {
            step: 2,
            method: "engine.reboot".to_owned()
        }]
    );
}

#[test]
fn a_failure_naming_an_unknown_error_code_fails_the_check() {
    let contract = Contract::committed();

    let found = violations(
        &fixture("reply-names-an-unknown-error-code.json"),
        &contract,
    );

    assert_eq!(
        found,
        vec![Violation::UnknownErrorCode {
            step: 1,
            code: json!("hello_refused")
        }]
    );
}

#[test]
fn an_events_batch_that_breaks_the_bundles_schema_fails_the_check() {
    let contract = Contract::committed();

    let found = violations(&fixture("events-batch-breaks-the-schema.json"), &contract);

    assert!(
        matches!(found.as_slice(), [Violation::Schema { step: 2, .. }]),
        "{found:?}"
    );
}

#[test]
fn a_raw_step_is_skipped_by_the_check() {
    let contract = Contract::committed();

    let found = violations(&fixture("raw-is-not-json-rpc.json"), &contract);

    assert_eq!(found, Vec::new());
}
