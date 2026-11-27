use goodfetch_protocol::jsonrpc::RequestId;
use goodfetch_protocol::plugin::net::PLUGIN_FETCH_FAILED;
use rstest::rstest;
use serde_json::{Value, json};

use super::{call_back_line, served};
use crate::call_error::CallError;
use crate::framing::{self, Incoming};

/// The call `line` holds, a request the Plugin sends.
fn call_in(line: &Value) -> framing::Call {
    match framing::decode(line.to_string().into_bytes()) {
        Ok(Incoming::Request { call, .. }) => call,
        other => panic!("{line} is no request: {other:?}"),
    }
}

/// What the line answering the call back `call-back-1` holds.
fn answer_in(line: &[u8]) -> Value {
    let answer: Value = serde_json::from_slice(line).unwrap();
    assert_eq!(answer["id"], json!("call-back-1"), "{answer}");
    answer
}

#[rstest]
#[case::a_method_it_does_not_serve(json!({ "method": "net.resolve", "params": {} }), "method_not_found")]
#[case::no_params(json!({ "method": "net.fetch" }), "invalid_params")]
#[case::params_that_are_not_its(json!({ "method": "net.fetch", "params": { "call_id": 1 } }), "invalid_params")]
fn a_call_back_the_engine_cannot_serve_is_refused_by_its_protocol_failure(
    #[case] mut request: Value,
    #[case] code: &str,
) {
    request["jsonrpc"] = json!("2.0");
    request["id"] = json!("call-back-1");

    let error = served(&call_in(&request)).unwrap_err();

    assert_eq!(error.code(), code);
}

#[test]
fn a_net_fetch_is_served_with_the_call_it_names() {
    let request = json!({
        "jsonrpc": "2.0",
        "id": "call-back-1",
        "method": "net.fetch",
        "params": { "call_id": 7, "method": "GET", "url": "https://example.com/" },
    });

    let params = served(&call_in(&request)).unwrap();

    assert_eq!(params.call_id, RequestId::Number(7));
}

#[test]
fn a_call_back_cancelled_before_its_answer_is_answered_call_cancelled() {
    let line = call_back_line(RequestId::Text("call-back-1".into()), None);

    assert_eq!(
        answer_in(&line)["error"]["data"]["code"],
        json!("call_cancelled")
    );
}

#[test]
fn a_call_back_that_failed_is_answered_with_its_error() {
    let failed = CallError::new(&PLUGIN_FETCH_FAILED, "no response", &());

    let line = call_back_line(RequestId::Text("call-back-1".into()), Some(Err(failed)));

    assert_eq!(
        answer_in(&line)["error"]["data"]["code"],
        json!("plugin_fetch_failed")
    );
}
