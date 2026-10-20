//! Protocol fuzzing against a fixture server of a fixture schema, in
//! process (spec #327's Seam B): a correct server passes, and a server that
//! accepts one malformed call, changes state on one or answers one with no
//! typed error fails, shrunk to that call.

use std::collections::BTreeMap;
use std::path::Path;

use proptest::test_runner::Config;
use rstest::rstest;
use serde_json::{Map, Value, json};

use super::{
    Broken, Invalidity, ProtocolFuzz, ProtocolFuzzError, ProtocolSchema, ProtocolTransport,
    TransportFailure,
};
use crate::runtime::proptest_config;

const FIXTURES: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/src/protocol_fuzz/fixtures");

/// What the fixture server keeps: its values by key.
type Kept = BTreeMap<String, Value>;

/// The one way the fixture server mishandles a call, if any.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Flaw {
    None,
    /// Takes any number as `fixture.put`'s value, not only one of 0 or
    /// more.
    AcceptsAnyNumberAsValue,
    /// Keeps a `fixture.put` with an unknown property, then refuses it.
    KeepsAPutWithAnUnknownProperty,
    /// Refuses a `fixture.put` with an unknown property as an internal
    /// error.
    RefusesAnUnknownPropertyAsInternal,
    /// Refuses a `fixture.put` with an unknown property without its
    /// `data`.
    RefusesAnUnknownPropertyWithoutData,
    /// Closes the connection on a `fixture.put` with an unknown property.
    ClosesOnAnUnknownProperty,
    /// Answers `fixture.count` with its count as text.
    CountsAsText,
}

/// The fixture schema's server, written by hand from it.
struct FixtureServer {
    flaw: Flaw,
    kept: Kept,
}

impl FixtureServer {
    fn new(flaw: Flaw) -> Self {
        Self {
            flaw,
            kept: Kept::new(),
        }
    }

    fn put(&mut self, id: &Value, params: &Value) -> Option<Value> {
        let Some(object) = params.as_object() else {
            return Some(failure(id, -32602, "invalid_params"));
        };
        let key = object.get("key").and_then(Value::as_str).filter(|key| {
            (1..=8).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_lowercase())
        });
        let value = object.get("value").filter(|value| match self.flaw {
            Flaw::AcceptsAnyNumberAsValue => value.is_number(),
            _ => value.as_u64().is_some(),
        });
        let unknown = object.keys().any(|name| name != "key" && name != "value");
        let (Some(key), Some(value)) = (key, value) else {
            return Some(failure(id, -32602, "invalid_params"));
        };
        if unknown {
            return match self.flaw {
                Flaw::KeepsAPutWithAnUnknownProperty => {
                    self.kept.insert(key.to_owned(), value.clone());
                    Some(failure(id, -32602, "invalid_params"))
                }
                Flaw::RefusesAnUnknownPropertyAsInternal => {
                    Some(failure(id, -32603, "internal_error"))
                }
                Flaw::RefusesAnUnknownPropertyWithoutData => Some(json!({
                    "jsonrpc": "2.0", "id": id, "error": { "code": -32602, "message": "unknown" },
                })),
                Flaw::ClosesOnAnUnknownProperty => None,
                _ => Some(failure(id, -32602, "invalid_params")),
            };
        }
        if self.kept.contains_key(key) {
            return Some(failure(id, -32001, "key_taken"));
        }
        self.kept.insert(key.to_owned(), value.clone());
        Some(success(id, &json!({ "stored": true })))
    }

    fn count(&self, id: &Value, params: &Value) -> Value {
        if !params.as_object().is_some_and(Map::is_empty) {
            return failure(id, -32602, "invalid_params");
        }
        let count = if self.flaw == Flaw::CountsAsText {
            json!(self.kept.len().to_string())
        } else {
            json!(self.kept.len())
        };
        success(id, &json!({ "count": count }))
    }
}

impl ProtocolTransport for FixtureServer {
    fn call(&mut self, request: &Value) -> Result<Value, TransportFailure> {
        let id = &request["id"];
        let params = &request["params"];
        let reply = match request["method"].as_str() {
            Some("fixture.put") => self.put(id, params),
            Some("fixture.count") => Some(self.count(id, params)),
            _ => Some(failure(id, -32601, "method_not_found")),
        };
        reply.ok_or_else(|| TransportFailure::new("closed"))
    }
}

fn success(id: &Value, result: &Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}

fn failure(id: &Value, class: i64, code: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "error": { "code": class, "message": code, "data": { "code": code, "retryable": false } },
    })
}

/// Fuzzing of the fixture schema whose bundle is `bundle`, with valid calls
/// of both its operations.
fn fixture_fuzz(bundle: &str) -> ProtocolFuzz {
    let fixtures = Path::new(FIXTURES);
    let schema =
        ProtocolSchema::read(&fixtures.join(bundle), &fixtures.join("catalog.json")).unwrap();
    ProtocolFuzz::new(schema).with_valid_calls(&["fixture.put", "fixture.count"])
}

/// Proptest's configuration on the test's seed, with enough cases that a
/// flaw one call in a few hundred meets is found on every seed, and no
/// regression file: these failures are the tests' expected outcome.
fn config() -> Config {
    Config {
        cases: 256,
        failure_persistence: None,
        ..proptest_config()
    }
}

fn fuzz_server(bundle: &str, flaw: Flaw) -> Result<(), ProtocolFuzzError<Kept>> {
    fixture_fuzz(bundle).run(
        config(),
        || FixtureServer::new(flaw),
        |server| server.kept.clone(),
    )
}

#[test]
fn a_correct_server_passes() {
    let fuzzed = fuzz_server("bundle.json", Flaw::None);

    assert_eq!(fuzzed, Ok(()));
}

#[test]
fn a_server_that_accepts_a_malformed_call_fails_shrunk_to_that_call() {
    let fuzzed = fuzz_server("bundle.json", Flaw::AcceptsAnyNumberAsValue);

    let Err(ProtocolFuzzError::Broken { calls, broken }) = fuzzed else {
        panic!("the property held or was abandoned: {fuzzed:?}");
    };
    assert_eq!(
        (
            calls.len(),
            calls[0].method(),
            calls[0].invalidity(),
            broken
        ),
        (
            1,
            "fixture.put",
            Some(&Invalidity::RefusedValue {
                property: "value".to_owned()
            }),
            Broken::Accepted {
                reply: json!({ "jsonrpc": "2.0", "id": 1, "result": { "stored": true } })
            }
        )
    );
}

#[test]
fn a_server_that_changes_state_on_a_malformed_call_fails_shrunk_to_that_call() {
    let fuzzed = fuzz_server("bundle.json", Flaw::KeepsAPutWithAnUnknownProperty);

    let Err(ProtocolFuzzError::Broken { calls, broken }) = fuzzed else {
        panic!("the property held or was abandoned: {fuzzed:?}");
    };
    assert_eq!(
        (
            calls.len(),
            calls[0].method(),
            matches!(
                calls[0].invalidity(),
                Some(Invalidity::UnknownProperty { .. })
            ),
            matches!(&broken, Broken::StateChanged { before, after } if before.is_empty() && after.len() == 1),
        ),
        (1, "fixture.put", true, true),
        "{broken:?}"
    );
}

#[rstest]
#[case::refused_as_internal(
    Flaw::RefusesAnUnknownPropertyAsInternal,
    Broken::Internal {
        reply: json!({
            "jsonrpc": "2.0",
            "id": 1,
            "error": {
                "code": -32603,
                "message": "internal_error",
                "data": { "code": "internal_error", "retryable": false },
            },
        }),
    }
)]
#[case::refused_without_data(
    Flaw::RefusesAnUnknownPropertyWithoutData,
    Broken::Untyped {
        reply: json!({ "jsonrpc": "2.0", "id": 1, "error": { "code": -32602, "message": "unknown" } }),
    }
)]
#[case::closed(
    Flaw::ClosesOnAnUnknownProperty,
    Broken::Transport { failure: TransportFailure::new("closed") }
)]
fn a_server_that_answers_a_malformed_call_with_no_typed_refusal_fails_shrunk_to_that_call(
    #[case] flaw: Flaw,
    #[case] expected: Broken<Kept>,
) {
    let fuzzed = fuzz_server("bundle.json", flaw);

    let Err(ProtocolFuzzError::Broken { calls, broken }) = fuzzed else {
        panic!("the property held or was abandoned: {fuzzed:?}");
    };
    assert_eq!(
        (
            calls.len(),
            calls[0].method(),
            matches!(
                calls[0].invalidity(),
                Some(Invalidity::UnknownProperty { .. })
            ),
            broken,
        ),
        (1, "fixture.put", true, expected)
    );
}

#[test]
fn a_server_whose_valid_answer_breaks_its_result_schema_fails_shrunk_to_that_call() {
    let fuzzed = fuzz_server("bundle.json", Flaw::CountsAsText);

    let Err(ProtocolFuzzError::Broken { calls, broken }) = fuzzed else {
        panic!("the property held or was abandoned: {fuzzed:?}");
    };
    assert_eq!(
        (
            calls.len(),
            calls[0].method(),
            calls[0].invalidity(),
            broken
        ),
        (
            1,
            "fixture.count",
            None,
            Broken::ResultBreaksSchema {
                reply: json!({ "jsonrpc": "2.0", "id": 1, "result": { "count": "0" } })
            }
        )
    );
}

/// The correct server was written for `bundle.json`; the bundle that also
/// requires `label` makes the generated calls lack it, which that server
/// accepts.
#[test]
fn a_schema_change_in_the_bundle_changes_the_generated_calls() {
    let fuzzed = fuzz_server("bundle-label-required.json", Flaw::None);

    let Err(ProtocolFuzzError::Broken { calls, broken }) = fuzzed else {
        panic!("the property held or was abandoned: {fuzzed:?}");
    };
    assert_eq!(
        (
            calls.len(),
            calls[0].method(),
            calls[0].invalidity(),
            broken
        ),
        (
            1,
            "fixture.put",
            Some(&Invalidity::MissingProperty {
                property: "label".to_owned()
            }),
            Broken::Accepted {
                reply: json!({ "jsonrpc": "2.0", "id": 1, "result": { "stored": true } })
            }
        )
    );
}

#[test]
fn a_missing_bundle_is_unreadable() {
    let fixtures = Path::new(FIXTURES);

    let read = ProtocolSchema::read(
        &fixtures.join("absent.json"),
        &fixtures.join("catalog.json"),
    );

    assert_eq!(
        read.map(|_| ()),
        Err(super::SchemaUnreadable::Unreadable {
            path: fixtures.join("absent.json"),
            kind: std::io::ErrorKind::NotFound,
        })
    );
}
