use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use rstest::rstest;

use super::{DecodeError, Frame, Framer, Incoming, MAX_DEPTH};

fn decode(line: &[u8]) -> Result<Incoming, DecodeError> {
    super::decode(line, &goodfetch_counted_work::CountedWork::default())
}

fn frames(chunks: &[&[u8]], limit: usize) -> Vec<Frame> {
    let mut framer = Framer::new(limit);
    let mut cut = Vec::new();
    for chunk in chunks {
        framer.feed(chunk, &mut cut);
    }
    cut.extend(framer.finish());
    cut
}

#[test]
fn lines_split_across_chunks_come_out_whole_in_order() {
    let cut = frames(&[b"{\"a\":1}\n{\"b\"", b":2}\n\n  \n{\"c\":3}"], 64);

    assert_eq!(
        cut,
        [
            Frame::Line(b"{\"a\":1}".to_vec()),
            Frame::Line(b"{\"b\":2}".to_vec()),
            Frame::Line(b"{\"c\":3}".to_vec()),
        ]
    );
}

#[test]
fn a_line_past_the_limit_is_one_too_large_frame_and_the_next_line_reads() {
    let cut = frames(&[b"0123456789", b"0123456789\n{}\n"], 16);

    assert_eq!(cut, [Frame::TooLarge, Frame::Line(b"{}".to_vec())]);
}

#[test]
fn a_line_at_the_limit_reads() {
    let cut = frames(&[b"0123456789abcdef\n"], 16);

    assert_eq!(cut, [Frame::Line(b"0123456789abcdef".to_vec())]);
}

#[rstest]
#[case::request(
    br#"{"jsonrpc":"2.0","id":1,"method":"engine.info","params":{}}"#,
    "request"
)]
#[case::notification(
    br#"{"jsonrpc":"2.0","method":"$/cancel","params":{"id":1}}"#,
    "notification"
)]
#[case::reply(br#"{"jsonrpc":"2.0","id":7,"result":{"records":[]}}"#, "reply")]
#[case::failure(
    br#"{"jsonrpc":"2.0","id":7,"error":{"code":-32001,"message":"no","data":{"code":"handler_failed","retryable":true}}}"#,
    "reply"
)]
fn a_message_decodes_as_its_kind(#[case] line: &[u8], #[case] kind: &str) {
    let decoded = decode(line).unwrap();

    let decoded_kind = match decoded {
        Incoming::Request(_) => "request",
        Incoming::Notification(_) => "notification",
        Incoming::Reply { .. } => "reply",
    };
    assert_eq!(decoded_kind, kind);
}

#[rstest]
#[case::not_json(b"{not json", ProtocolFailure::ParseError, None)]
#[case::batch(
    br#"[{"jsonrpc":"2.0","id":1,"method":"hello"}]"#,
    ProtocolFailure::InvalidRequest,
    None
)]
#[case::not_an_object(b"12", ProtocolFailure::InvalidRequest, None)]
#[case::no_method_and_no_answer(
    br#"{"jsonrpc":"2.0","id":4}"#,
    ProtocolFailure::InvalidRequest,
    Some(RequestId::Number(4))
)]
#[case::wrong_version(br#"{"jsonrpc":"1.0","id":"x","method":"hello"}"#, ProtocolFailure::InvalidRequest, Some(RequestId::Text("x".into())))]
fn a_line_that_is_no_message_answers_its_protocol_failure(
    #[case] line: &[u8],
    #[case] failure: ProtocolFailure,
    #[case] id: Option<RequestId>,
) {
    let error = decode(line).unwrap_err();

    assert_eq!((error.failure, error.id), (failure, id));
}

/// A Client's failure that names no Error code reads as the handler's own
/// failure, `handler_failed`, which a retry may not help.
#[test]
fn a_failure_reply_without_an_error_code_reads_as_the_handlers_own() {
    let decoded = decode(br#"{"jsonrpc":"2.0","id":3,"error":{"code":-1,"message":"boom"}}"#);

    let Ok(Incoming::Reply {
        id,
        answer: Err(error),
    }) = decoded
    else {
        panic!("not a failure reply: {decoded:?}");
    };
    assert_eq!(
        (id, error.data.code.as_str(), error.data.retryable),
        (RequestId::Number(3), "handler_failed", false)
    );
}

#[test]
fn nesting_past_the_limit_is_too_deep() {
    let line = format!("{}{}", "[".repeat(MAX_DEPTH + 1), "]".repeat(MAX_DEPTH + 1));

    assert_eq!(
        decode(line.as_bytes()).unwrap_err().failure,
        ProtocolFailure::MessageTooDeep
    );
}

#[test]
fn brackets_inside_strings_are_no_nesting() {
    let line = format!(
        r#"{{"jsonrpc":"2.0","method":"m","params":{{"text":"{}\"{}"}}}}"#,
        "[".repeat(MAX_DEPTH * 2),
        "{".repeat(MAX_DEPTH * 2)
    );

    assert!(decode(line.as_bytes()).is_ok());
}

mod json_rpc_framing {
    goodfetch_test_support::fuzz_replay!(super::super::fuzz_framing, "fuzz/json_rpc_framing");
}
