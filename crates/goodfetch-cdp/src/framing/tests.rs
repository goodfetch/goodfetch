use serde_json::json;

use super::{Framer, FramingError, frame};

#[test]
fn messages_split_across_reads_and_joined_in_one_read_come_out_whole() {
    let mut framer = Framer::default();
    framer.push(b"{\"id\":1}\0{\"id\"");
    assert_eq!(framer.next_frame(), Ok(Some(b"{\"id\":1}".to_vec())));
    assert_eq!(framer.next_frame(), Ok(None));
    framer.push(b":2}\0{\"id\":3}\0");
    assert_eq!(
        (
            framer.next_frame(),
            framer.next_frame(),
            framer.next_frame(),
            framer.has_partial()
        ),
        (
            Ok(Some(b"{\"id\":2}".to_vec())),
            Ok(Some(b"{\"id\":3}".to_vec())),
            Ok(None),
            false
        )
    );
}

#[test]
fn a_message_without_its_nul_past_the_limit_ends_the_connection() {
    let mut framer = Framer::with_limit(8);
    framer.push(b"{\"id\":1}");
    assert_eq!(framer.next_frame(), Ok(None));
    framer.push(b" ");
    assert_eq!(framer.next_frame(), Err(FramingError::TooLarge));
}

#[test]
fn a_sent_message_is_its_json_and_one_nul() {
    assert_eq!(frame(&json!({ "id": 7 })), b"{\"id\":7}\0".to_vec());
}
