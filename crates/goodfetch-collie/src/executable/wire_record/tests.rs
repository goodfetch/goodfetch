//! The wire record through its two functions: the bytes a message records
//! after the shared redactions, and what a record holds at each level.
#![expect(
    clippy::disallowed_types,
    reason = "the test shares what the subscriber wrote with the test through a Mutex"
)]

use std::sync::{Arc, Mutex};

use tracing::level_filters::LevelFilter;

use super::{PATTERNS, SENT, record_wire, redacted_len};

/// What a subscriber wrote, shared with the test.
#[derive(Clone, Default)]
struct Written(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for Written {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// What `record_wire` writes for `line`, sent, under a subscriber whose
/// most verbose level is `level`.
fn recorded(level: LevelFilter, line: &str) -> String {
    let written = Written::default();
    let writer = written.clone();
    let subscriber = tracing_subscriber::fmt()
        .with_max_level(level)
        .with_ansi(false)
        .with_writer(move || writer.clone())
        .finish();
    tracing::subscriber::with_default(subscriber, || record_wire(SENT, line.as_bytes()));
    String::from_utf8(written.0.lock().unwrap().clone()).unwrap()
}

/// A `run.progress` answer at `instant`, from a page on `port`, of the Run
/// `run_id`.
fn answer(port: u16, instant: &str, run_id: &str) -> String {
    format!(
        r#"{{"jsonrpc":"2.0","id":3,"result":{{"run_id":"{run_id}","at":"{instant}","url":"http://127.0.0.1:{port}/a"}}}}"#
    )
}

/// Two messages that differ only by a loopback port, an instant and a ULID
/// record the same bytes, so a protocol-count snapshot repeats run to run.
#[test]
fn messages_that_differ_only_by_a_port_an_instant_and_a_ulid_record_the_same_bytes() {
    let first = answer(
        8080,
        "2026-11-22T05:16:34.123Z",
        "01JD7Q3K9V8X2M4N6P8R0T2W4Y",
    );
    let second = answer(
        54321,
        "2027-01-02T03:04:05+01:00",
        "7ZZZZZZZZZZZZZZZZZZZZZZZZZ",
    );

    assert_ne!(first.len(), second.len());
    assert_eq!(redacted_len(&first), redacted_len(&second));
}

/// A message with one more field records more bytes.
#[test]
fn a_message_with_one_more_field_records_more_bytes() {
    let message = answer(8080, "2026-11-22T05:16:34Z", "01JD7Q3K9V8X2M4N6P8R0T2W4Y");
    let more = message.replacen(r#""id":3,"#, r#""id":3,"extra":true,"#, 1);

    assert_eq!(
        redacted_len(&more),
        redacted_len(&message) + r#""extra":true,"#.len()
    );
}

/// At `trace`, a record names the message's direction, kind, method and
/// bytes, and never its content: the Daemon's token in a `hello` appears
/// in no record.
#[test]
fn a_record_holds_no_content_of_its_message_not_even_the_daemons_token() {
    let token = "the-daemons-token-in-this-hello";
    let hello = format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"hello","params":{{"token":"{token}","surface":"cli"}}}}"#
    );

    let traced = recorded(LevelFilter::TRACE, &hello);

    assert!(
        traced.contains(r#"direction="sent" kind="request" method="hello" bytes="#),
        "{traced}"
    );
    assert!(!traced.contains(token), "{traced}");
    assert!(!traced.contains("surface"), "{traced}");
}

/// With `trace` off, a message is neither recorded nor redacted: the
/// patterns are never compiled.
#[test]
fn with_trace_off_a_message_is_neither_recorded_nor_redacted() {
    let hello = r#"{"jsonrpc":"2.0","id":1,"method":"hello","params":{}}"#;

    let traced = recorded(LevelFilter::DEBUG, hello);

    assert_eq!((traced.as_str(), PATTERNS.get().is_none()), ("", true));
}
