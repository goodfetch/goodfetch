use rstest::rstest;
use serde_json::{Value, json};

use super::{Message, RequestId};

#[rstest]
#[case::request(json!({"jsonrpc": "2.0", "id": 1, "method": "engine.info", "params": {}}), "request")]
#[case::request_with_a_text_id(json!({"jsonrpc": "2.0", "id": "a", "method": "hello"}), "request")]
#[case::notification(json!({"jsonrpc": "2.0", "method": "$/cancel", "params": {"id": 1}}), "notification")]
#[case::response(json!({"jsonrpc": "2.0", "id": 1, "result": {}}), "response")]
#[case::error(json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": "m", "data": {"code": "parse_error", "retryable": false}}}), "error")]
fn each_message_reads_as_its_kind(#[case] message: Value, #[case] kind: &str) {
    let read: Message = serde_json::from_value(message).unwrap();

    let read_kind = match read {
        Message::Request(_) => "request",
        Message::Notification(_) => "notification",
        Message::Response(_) => "response",
        Message::Error(_) => "error",
    };
    assert_eq!(read_kind, kind);
}

#[rstest]
#[case::another_version(json!({"jsonrpc": "1.0", "id": 1, "method": "hello"}))]
#[case::unknown_member(json!({"jsonrpc": "2.0", "id": 1, "method": "hello", "extra": 1}))]
#[case::object_id(json!({"jsonrpc": "2.0", "id": {}, "method": "hello"}))]
#[case::batch(json!([{"jsonrpc": "2.0", "id": 1, "method": "hello"}]))]
fn a_message_outside_the_protocol_is_refused(#[case] message: Value) {
    assert!(serde_json::from_value::<Message>(message).is_err());
}

#[test]
fn an_error_keeps_its_details_beside_its_code() {
    let read: Message = serde_json::from_value(json!({
        "jsonrpc": "2.0",
        "id": 7,
        "error": {"code": -32006, "message": "m", "data": {"code": "not_in_product", "retryable": false, "fix": "f"}},
    }))
    .unwrap();

    let Message::Error(error) = read else {
        panic!("not an error: {read:?}");
    };
    assert_eq!(error.id, Some(RequestId::Number(7)));
    assert_eq!(error.error.data.code, "not_in_product");
    assert_eq!(error.error.data.details["fix"], json!("f"));
}
