//! An Attached engine over stdio: `hello`, refused before any other message
//! and then closed, `engine.info`, `$/cancel` and every failure's shape,
//! read through `StrictStdout` with the engine's `log.level` at `trace`,
//! so a log line on stdout fails the test (ADRs 0024, 0066 and 0095).
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use goodfetch_protocol::catalog::Product;
use goodfetch_test_client::{Attached, Hello, hello, newer_minor};
use goodfetch_test_support::TestEnv;
use serde_json::{Value, json};

use crate::COLLIE;

fn request(id: i64, method: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": {} })
}

/// Covers: story:3-1, story:3-3, command:attached-engine
#[test]
fn an_attached_engine_answers_hello_over_stdio_and_stops_at_the_end_of_its_input() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);

    engine.send(&hello(Product::Collie));
    let mut reply = engine.next().unwrap();
    let private_endpoint = reply["result"]
        .as_object_mut()
        .unwrap()
        .remove("private_endpoint");
    let engine_instance = reply["result"]
        .as_object_mut()
        .unwrap()
        .remove("engine_instance");

    assert_eq!(
        (
            reply,
            private_endpoint.is_some_and(|it| it.is_object()),
            engine_instance.is_some_and(|it| it.is_string())
        ),
        (
            json!({
                "jsonrpc": "2.0",
                "id": 0,
                "result": { "protocol_version": "0.1", "product": "collie", "product_version": "0.0.0" },
            }),
            true,
            true
        )
    );
    assert_eq!(engine.finish(), (vec![], true));
}

/// Covers: story:3-132, error:protocol_version_unsupported
#[test]
fn a_hello_of_a_newer_minor_is_refused_before_any_other_message_and_stdout_closes() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);

    engine.send(
        &Hello::needing(Product::Collie)
            .protocol_version(newer_minor())
            .message(),
    );
    let reply = engine.next().unwrap();
    let after = engine.next();

    assert_eq!(
        (&reply["error"]["data"]["code"], after),
        (&json!("protocol_version_unsupported"), None)
    );
    assert_eq!(engine.finish(), (vec![], true));
}

/// Covers: story:3-135, error:not_in_product
#[test]
fn collies_engine_refuses_a_client_that_needs_foxhound_and_stdout_closes() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);

    engine.send(&hello(Product::Foxhound));
    let reply = engine.next().unwrap();
    let after = engine.next();

    assert_eq!(
        (
            &reply["error"]["data"]["code"],
            &reply["error"]["data"]["needs"],
            after
        ),
        (&json!("not_in_product"), &json!("foxhound"), None)
    );
    assert_eq!(engine.finish(), (vec![], true));
}

/// Covers: operation:engine.info
#[test]
fn engine_info_answers_collie_and_its_versions() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    engine.send(&request(1, "engine.info"));
    let reply = engine.next().unwrap();

    assert_eq!(
        reply["result"],
        json!({
            "product": "collie",
            "product_version": "0.0.0",
            "protocol_version": "0.1",
            "product_major": 0,
        })
    );
    let _ = engine.finish();
}

/// Covers: story:3-136, error:not_in_product
#[test]
fn an_operation_only_foxhound_carries_is_refused_naming_foxhound_and_its_install() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    engine.send(&request(1, "sample.crawl"));
    let reply = engine.next().unwrap();

    assert_eq!(
        reply["error"],
        json!({
            "code": -32006,
            "message": "`sample.crawl` is part of Foxhound, and this engine is Collie 0.0.0.",
            "data": {
                "code": "not_in_product",
                "retryable": false,
                "product": "collie",
                "product_version": "0.0.0",
                "needs": "foxhound",
                "requested": "sample.crawl",
                "fix": "Run it with Foxhound, which contains everything Collie does (`cargo install goodfetch-foxhound --locked --features executable`), or connect to a Foxhound engine.",
                "install": "cargo install goodfetch-foxhound --locked --features executable",
            },
        })
    );
    let _ = engine.finish();
}

/// Covers: story:3-6
#[test]
fn cancel_of_a_finished_call_leaves_the_connection_answering() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();
    engine.send(&request(1, "engine.info"));
    let _ = engine.next();

    engine.send(&json!({ "jsonrpc": "2.0", "method": "$/cancel", "params": { "id": 1 } }));
    engine.send(&request(2, "engine.info"));
    let reply = engine.next().unwrap();

    assert_eq!(
        (&reply["id"], &reply["result"]["product"]),
        (&json!(2), &json!("collie"))
    );
    assert_eq!(engine.finish(), (vec![], true));
}

/// How many calls [`an_id_is_free_again_once_its_call_was_answered`] sends
/// under one id: enough that a reply written before its call left the
/// engine's calls in flight shows up on most runs of the multi-threaded
/// runtime.
const REUSES: usize = 5_000;

/// A call's id is free again once its answer arrived (JSON-RPC 2.0): a
/// Client that sends each call under the id of the one it just got the
/// answer of, as the SDKs' and the tests' Clients do, gets every call
/// answered, never refused as still in flight.
/// Covers: story:3-1
#[test]
fn an_id_is_free_again_once_its_call_was_answered() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    let refused: Vec<Value> = (0..REUSES)
        .filter_map(|_| {
            engine.send(&request(1, "engine.info"));
            engine.next().unwrap().get("error").cloned()
        })
        .collect();

    assert_eq!(refused, Vec::<Value>::new());
    assert_eq!(engine.finish(), (vec![], true));
}

/// Covers: story:3-7
#[test]
fn a_failure_carries_its_error_code_and_retryable_flag_under_its_json_rpc_class() {
    let env = TestEnv::new();
    let mut engine = Attached::start(&env, COLLIE);
    engine.send(&hello(Product::Collie));
    let _ = engine.next();

    engine.send(&request(1, "page.levitate"));
    let reply = engine.next().unwrap();

    assert_eq!(
        reply["error"],
        json!({
            "code": -32601,
            "message": "no Product carries the operation `page.levitate`",
            "data": { "code": "method_not_found", "retryable": false },
        })
    );
    let _ = engine.finish();
}
