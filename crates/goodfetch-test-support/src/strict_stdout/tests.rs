use std::io::{Cursor, Write as _};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use rstest::rstest;
use serde_json::json;

use super::{Line, StrictStdout, StrictStdoutError, StrictStdoutErrorKind};

/// A reader over the stdout and stderr bytes a child would write.
fn reader(stdout: &[u8], stderr: &[u8]) -> StrictStdout {
    StrictStdout::new(Cursor::new(stdout.to_vec()), Cursor::new(stderr.to_vec()))
}

/// A reader whose line 1 is a valid request and line 2 is `line`.
fn after_a_request(line: &[u8]) -> StrictStdout {
    let mut stdout = br#"{"jsonrpc":"2.0","id":1,"method":"hello"}"#.to_vec();
    stdout.push(b'\n');
    stdout.extend_from_slice(line);
    stdout.push(b'\n');
    reader(&stdout, b"")
}

#[test]
fn a_request_a_notification_and_a_reply_are_yielded_in_order_and_stderr_never_is() {
    let stdout = reader(
        concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{"product":"collie"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","method":"run.progress","params":{"done":1}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":"a","result":{"ok":true}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"no such method"}}"#,
            "\n",
        )
        .as_bytes(),
        b"2026-01-05T09:00:00Z TRACE goodfetch::rpc: read hello\n",
    );

    let messages: Vec<serde_json::Value> = stdout.collect();

    assert_eq!(
        messages,
        [
            json!({"jsonrpc":"2.0","id":1,"method":"hello","params":{"product":"collie"}}),
            json!({"jsonrpc":"2.0","method":"run.progress","params":{"done":1}}),
            json!({"jsonrpc":"2.0","id":"a","result":{"ok":true}}),
            json!({"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"no such method"}}),
        ]
    );
}

#[test]
fn iterating_fails_the_test_at_a_line_that_is_not_json_rpc() {
    let mut stdout = after_a_request(b"engine ready");
    stdout.next();

    let failure = catch_unwind(AssertUnwindSafe(|| stdout.next())).unwrap_err();

    let error = failure.downcast::<StrictStdoutError>().unwrap();
    assert!(
        matches!(error.kind, StrictStdoutErrorKind::NotJson(_)),
        "{error}"
    );
}

#[test]
fn a_line_shows_its_bytes_escaped() {
    let line = Line {
        number: 2,
        bytes: b"\xef\xbb\xbf\x1b[32mINFO\"\xff".to_vec(),
    };

    let shown = line.to_string();

    assert_eq!(shown, r#"line 2: "\xef\xbb\xbf\x1b[32mINFO\"\xff""#);
}

#[test]
fn a_failure_shows_the_stderr_of_the_child() {
    let mut stdout = reader(
        b"engine ready\n",
        b"2026-01-05T09:00:00Z TRACE goodfetch::rpc: read hello\n",
    );

    let error = stdout.try_next().unwrap_err();

    assert_eq!(
        error.stderr,
        "2026-01-05T09:00:00Z TRACE goodfetch::rpc: read hello\n"
    );
}

/// A child's stderr that never goes quiet: a log line every 100 ms, forever,
/// as an engine logging at `trace` mid-Run writes it.
struct Chatty;

impl std::io::Read for Chatty {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        #[expect(
            clippy::disallowed_methods,
            reason = "the stand-in child's stderr, read on the helper's own reader thread"
        )]
        std::thread::sleep(Duration::from_millis(100));
        let line = b"2026-01-05T09:00:00Z TRACE goodfetch::run: fetched\n";
        let written = line.len().min(buffer.len());
        buffer[..written].copy_from_slice(&line[..written]);
        Ok(written)
    }
}

#[test]
fn a_failure_returns_with_the_stderr_so_far_while_the_child_keeps_logging() {
    let (sent, received) = std::sync::mpsc::channel();
    #[expect(
        clippy::disallowed_methods,
        reason = "the test bounds a call that hangs when the helper is broken"
    )]
    std::thread::spawn(move || {
        let mut stdout = StrictStdout::new(Cursor::new(b"engine ready\n".to_vec()), Chatty);
        let _ = sent.send(stdout.try_next());
    });

    let returned = received
        .recv_timeout(super::STDERR_LIMIT + Duration::from_secs(5))
        .expect("try_next returns while the child keeps writing its stderr");

    let error = returned.unwrap_err();
    assert!(
        matches!(error.kind, StrictStdoutErrorKind::NotJson(_)),
        "{error}"
    );
    assert!(
        error.stderr.contains("TRACE goodfetch::run: fetched"),
        "{error}"
    );
}

#[rstest]
#[case::plain_log_line(b"2026-01-05T09:00:00Z INFO engine ready")]
#[case::ansi_escape(b"\x1b[32mINFO\x1b[0m engine ready")]
#[case::ansi_escape_before_a_message(b"\x1b[0m{\"jsonrpc\":\"2.0\",\"method\":\"run.progress\"}")]
#[case::truncated_message(b"{\"jsonrpc\":\"2.0\",\"method\":\"run.prog")]
#[case::utf8_bom_before_a_message(b"\xef\xbb\xbf{\"jsonrpc\":\"2.0\",\"method\":\"run.progress\"}")]
#[case::empty_line(b"")]
#[case::space_before_a_message(b" {\"jsonrpc\":\"2.0\",\"method\":\"run.progress\"}")]
#[case::carriage_return_after_a_message(b"{\"jsonrpc\":\"2.0\",\"method\":\"run.progress\"}\r")]
fn a_line_that_is_not_one_json_value_is_refused_naming_it(#[case] line: &[u8]) {
    let mut stdout = after_a_request(line);
    stdout.try_next().unwrap();

    let error = stdout.try_next().unwrap_err();

    assert!(
        matches!(error.kind, StrictStdoutErrorKind::NotJson(_)),
        "{error}"
    );
    assert_eq!(
        error.kind.line(),
        Some(&Line {
            number: 2,
            bytes: line.to_vec()
        })
    );
}

#[test]
fn a_line_that_is_not_utf8_is_refused_naming_it() {
    let line = b"{\"jsonrpc\":\"2.0\",\"method\":\"run.progress\",\"params\":[\"\xff\"]}";
    let mut stdout = after_a_request(line);
    stdout.try_next().unwrap();

    let error = stdout.try_next().unwrap_err();

    assert!(
        matches!(error.kind, StrictStdoutErrorKind::NotUtf8(_)),
        "{error}"
    );
    assert_eq!(
        error.kind.line(),
        Some(&Line {
            number: 2,
            bytes: line.to_vec()
        })
    );
}

#[rstest]
#[case::a_whole_message(br#"{"jsonrpc":"2.0","method":"run.progress"}"#)]
#[case::a_truncated_message(br#"{"jsonrpc":"2.0","met"#)]
fn a_partial_line_at_the_end_of_stdout_is_refused_naming_it(#[case] partial: &[u8]) {
    let mut bytes = br#"{"jsonrpc":"2.0","id":1,"method":"hello"}"#.to_vec();
    bytes.push(b'\n');
    bytes.extend_from_slice(partial);
    let mut stdout = reader(&bytes, b"");
    stdout.try_next().unwrap();

    let error = stdout.try_next().unwrap_err();

    assert!(
        matches!(error.kind, StrictStdoutErrorKind::PartialLine(_)),
        "{error}"
    );
    assert_eq!(
        error.kind.line(),
        Some(&Line {
            number: 2,
            bytes: partial.to_vec()
        })
    );
}

#[rstest]
#[case::another_json_object(br#"{"address":"127.0.0.1:4000"}"#)]
#[case::no_version(br#"{"id":1,"method":"hello"}"#)]
#[case::another_version(br#"{"jsonrpc":"1.0","id":1,"method":"hello"}"#)]
#[case::a_method_that_is_not_a_string(br#"{"jsonrpc":"2.0","id":1,"method":7}"#)]
#[case::params_that_are_neither_object_nor_array(
    br#"{"jsonrpc":"2.0","method":"run.progress","params":"done"}"#
)]
#[case::an_id_that_is_an_object(br#"{"jsonrpc":"2.0","id":{},"method":"hello"}"#)]
#[case::a_reply_with_a_result_and_an_error(
    br#"{"jsonrpc":"2.0","id":1,"result":1,"error":{"code":1,"message":"m"}}"#
)]
#[case::a_reply_with_neither(br#"{"jsonrpc":"2.0","id":1}"#)]
#[case::a_reply_without_an_id(br#"{"jsonrpc":"2.0","result":1}"#)]
#[case::an_error_without_a_code(br#"{"jsonrpc":"2.0","id":1,"error":{"message":"m"}}"#)]
#[case::an_error_without_a_message(br#"{"jsonrpc":"2.0","id":1,"error":{"code":1}}"#)]
#[case::a_request_with_a_result(br#"{"jsonrpc":"2.0","id":1,"method":"hello","result":1}"#)]
#[case::an_unknown_member(br#"{"jsonrpc":"2.0","method":"run.progress","extra":1}"#)]
#[case::a_batch(br#"[{"jsonrpc":"2.0","method":"run.progress"}]"#)]
#[case::a_number(b"42")]
fn a_json_line_that_is_not_a_json_rpc_message_is_refused_naming_it(#[case] line: &[u8]) {
    let mut stdout = after_a_request(line);
    stdout.try_next().unwrap();

    let error = stdout.try_next().unwrap_err();

    assert!(
        matches!(error.kind, StrictStdoutErrorKind::NotJsonRpc(_)),
        "{error}"
    );
    assert_eq!(
        error.kind.line(),
        Some(&Line {
            number: 2,
            bytes: line.to_vec()
        })
    );
}

#[test]
fn a_child_that_writes_nothing_is_quiet_and_a_line_written_meanwhile_is_kept_for_the_next_read() {
    let (stdout, mut writer) = std::io::pipe().unwrap();
    let mut stdout = StrictStdout::new(stdout, Cursor::new(Vec::new()));

    let quiet = stdout.is_quiet_for(Duration::from_millis(50));
    writer
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n")
        .unwrap();
    let written = stdout.is_quiet_for(Duration::from_secs(10));
    let kept = stdout.next();

    assert_eq!(
        (quiet, written, kept),
        (
            true,
            false,
            Some(json!({"jsonrpc":"2.0","id":1,"result":{}}))
        )
    );
}
