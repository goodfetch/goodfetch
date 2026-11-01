use bytes::Bytes;
use serde_json::json;

use super::{Framer, FramingError, frame};

#[test]
fn messages_split_across_reads_and_joined_in_one_read_come_out_whole() {
    let mut framer = Framer::default();
    framer.push(b"{\"id\":1}\0{\"id\"");
    assert_eq!(
        framer.next_frame(),
        Ok(Some(Bytes::from_static(b"{\"id\":1}")))
    );
    assert_eq!(framer.next_frame(), Ok(None));
    framer.push(b":2}\0{\"id\":3}\0");
    let joined = (
        framer.next_frame(),
        framer.next_frame(),
        framer.next_frame(),
    );
    // No byte of them is left: the next message comes out alone.
    framer.push(b"{\"id\":4}\0");
    assert_eq!(
        (joined, framer.next_frame()),
        (
            (
                Ok(Some(Bytes::from_static(b"{\"id\":2}"))),
                Ok(Some(Bytes::from_static(b"{\"id\":3}"))),
                Ok(None),
            ),
            Ok(Some(Bytes::from_static(b"{\"id\":4}")))
        )
    );
}

/// A message past the limit is dropped through its NUL, naming the command
/// it answers, and the messages after it read whole (finding 563-1: one
/// oversized answer ended the browser's whole connection).
#[test]
fn a_message_past_the_limit_is_dropped_naming_the_command_it_answers_and_the_next_reads() {
    let mut framer = Framer::with_limit(16);
    framer.push(b"{\"id\":42,\"result\":{\"body\":\"");
    let dropped = framer.next_frame();
    framer.push(&[b'a'; 64]);
    let while_dropping = framer.next_frame();
    framer.push(b"\"}}\0{\"id\":43}\0");

    assert_eq!(
        (
            dropped,
            while_dropping,
            framer.next_frame(),
            framer.next_frame()
        ),
        (
            Err(FramingError::TooLarge {
                answering: Some(42)
            }),
            Ok(None),
            Ok(Some(Bytes::from_static(b"{\"id\":43}"))),
            Ok(None),
        )
    );
}

#[test]
fn an_event_past_the_limit_is_dropped_answering_no_command() {
    let mut framer = Framer::with_limit(8);
    framer.push(b"{\"method\":\"Runtime.consoleAPICalled\"");
    let dropped = framer.next_frame();
    framer.push(b"}\0{\"id\":1}\0");

    assert_eq!(
        (dropped, framer.next_frame()),
        (
            Err(FramingError::TooLarge { answering: None }),
            Ok(Some(Bytes::from_static(b"{\"id\":1}"))),
        )
    );
}

#[test]
fn a_sent_message_is_its_json_and_one_nul() {
    assert_eq!(frame(&json!({ "id": 7 })), b"{\"id\":7}\0".to_vec());
}
