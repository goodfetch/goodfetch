use goodfetch_protocol::jsonrpc::{ProtocolFailure, RequestId};
use rstest::rstest;

use super::{DecodeError, Frame, Framer, Incoming, MAX_DEPTH};

fn decode(line: &[u8]) -> Result<Incoming, DecodeError> {
    super::decode(line.to_vec())
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
fn a_first_line_past_the_opening_limit_is_too_large() {
    let mut framer = Framer::opening(8, 64);
    let mut cut = Vec::new();

    framer.feed(b"0123456789\n", &mut cut);

    assert_eq!(cut, [Frame::TooLarge]);
}

#[test]
fn lines_after_the_first_read_up_to_the_whole_limit() {
    let mut framer = Framer::opening(8, 64);
    let mut cut = Vec::new();

    framer.feed(b"\n  \n{\"a\":1}\n0123456789abcdef\n", &mut cut);

    assert_eq!(
        cut,
        [
            Frame::Line(b"{\"a\":1}".to_vec()),
            Frame::Line(b"0123456789abcdef".to_vec()),
        ]
    );
}

#[test]
fn the_framer_says_it_is_skipping_a_line_past_its_limit_before_the_line_ends() {
    let mut framer = Framer::opening(8, 64);
    let mut cut = Vec::new();

    framer.feed(b"0123", &mut cut);
    let before = framer.skipping();
    framer.feed(b"456789", &mut cut);

    assert!(!before);
    assert!(framer.skipping());
    assert_eq!(cut, Vec::<Frame>::new());
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
        Incoming::Request { .. } => "request",
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
#[case::no_version(
    br#"{"id":2,"method":"hello"}"#,
    ProtocolFailure::InvalidRequest,
    Some(RequestId::Number(2))
)]
#[case::unknown_member(
    br#"{"jsonrpc":"2.0","id":2,"method":"hello","extra":1}"#,
    ProtocolFailure::InvalidRequest,
    Some(RequestId::Number(2))
)]
#[case::notification_with_a_result(
    br#"{"jsonrpc":"2.0","method":"$/credit","result":1}"#,
    ProtocolFailure::InvalidRequest,
    None
)]
#[case::method_no_string(
    br#"{"jsonrpc":"2.0","id":2,"method":7}"#,
    ProtocolFailure::InvalidRequest,
    Some(RequestId::Number(2))
)]
#[case::id_no_number_or_string(
    br#"{"jsonrpc":"2.0","id":null,"method":"hello"}"#,
    ProtocolFailure::InvalidRequest,
    None
)]
fn a_line_that_is_no_message_answers_its_protocol_failure(
    #[case] line: &[u8],
    #[case] failure: ProtocolFailure,
    #[case] id: Option<RequestId>,
) {
    let error = decode(line).unwrap_err();

    assert_eq!((error.failure, error.id), (failure, id));
}

/// An id reads as a whole number or as a string, a string of digits as a
/// string; any other id reads as none.
#[rstest]
#[case::a_number("7", Some(RequestId::Number(7)))]
#[case::a_negative_number("-3", Some(RequestId::Number(-3)))]
#[case::a_string("\"x\"", Some(RequestId::Text("x".into())))]
#[case::a_string_of_digits("\"7\"", Some(RequestId::Text("7".into())))]
#[case::a_string_with_an_escape("\"a\x5cu0062\"", Some(RequestId::Text("ab".into())))]
#[case::a_decimal("1.5", None)]
#[case::a_number_past_i64("9223372036854775808", None)]
#[case::a_boolean("true", None)]
fn an_id_reads_as_a_number_or_a_string(#[case] id: &str, #[case] read: Option<RequestId>) {
    let line = format!(r#"{{"jsonrpc":"1.0","id":{id},"method":"hello"}}"#);

    assert_eq!(decode(line.as_bytes()).unwrap_err().id, read);
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

/// A request's method and params are read from where they are in its line,
/// an escaped method included, and `null` params are none.
#[rstest]
#[case::params(
    br#"{"jsonrpc":"2.0","id":1,"method":"page.goto","params": {"url":"a"} }"#,
    "page.goto",
    Some(r#"{"url":"a"}"#)
)]
#[case::escaped_method(
    br#"{"params":[1],"method":"page.\u0067oto","id":1,"jsonrpc":"2.0"}"#,
    "page.goto",
    Some("[1]")
)]
#[case::null_params(
    br#"{"jsonrpc":"2.0","id":1,"method":"engine.info","params":null}"#,
    "engine.info",
    None
)]
#[case::last_member_wins(br#"{"jsonrpc":"2.0","id":1,"method":"a","method":"b"}"#, "b", None)]
fn a_request_reads_its_method_and_params_from_its_line(
    #[case] line: &[u8],
    #[case] method: &str,
    #[case] params: Option<&str>,
) {
    let Ok(Incoming::Request { id, call }) = decode(line) else {
        panic!("no request");
    };

    assert_eq!(
        (id, call.method(), call.params()),
        (RequestId::Number(1), method, params)
    );
}

/// A reply's result is read where it is, into the type its reader asks for.
#[test]
fn a_reply_reads_its_result_from_its_line() {
    let Ok(Incoming::Reply {
        answer: Ok(result), ..
    }) = decode(br#"{"jsonrpc":"2.0","id":7,"result":{"records":[1, 2]}}"#)
    else {
        panic!("no result");
    };

    assert_eq!(result.json(), r#"{"records":[1, 2]}"#);
    assert_eq!(
        result.read::<serde_json::Value>().unwrap(),
        serde_json::json!({ "records": [1, 2] })
    );
}

/// A handler's answer keeps each Record's fields in the order its program
/// wrote them, which a parsed `Value` would sort.
#[test]
fn records_in_a_reply_keep_their_fields_in_the_order_written() {
    let Ok(Incoming::Reply {
        answer: Ok(result), ..
    }) = decode(
        br#"{"jsonrpc":"2.0","id":7,"result":{"records":[{"dataset":"books","data":{"title":"Dune","price":9}}]}}"#,
    )
    else {
        panic!("no result");
    };

    let handled = result
        .read::<goodfetch_protocol::foxhound::crawl::CrawlRequestHandled>()
        .unwrap();

    assert_eq!(
        handled.records[0].data.json(),
        r#"{"title":"Dune","price":9}"#
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
