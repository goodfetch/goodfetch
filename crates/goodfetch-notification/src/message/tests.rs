//! An event ID through its constructor: the text a webhook can sign
//! unambiguously, and nothing else.

use rstest::rstest;

use super::{EVENT_ID_MAX, EventId, EventIdInvalid};

/// Letters, digits, `_` and `-` make an event ID, up to [`EVENT_ID_MAX`]
/// bytes; an empty text, a `.`, a space, a non-ASCII letter or a longer
/// text does not.
#[rstest]
#[case::a_standard_webhooks_id("msg_2KWPBgLlAfxdpx2AI54pPJ85f4W", true)]
#[case::dashes("run-failed-0001", true)]
#[case::the_longest(&"a".repeat(EVENT_ID_MAX), true)]
#[case::empty("", false)]
#[case::a_dot("run.failed", false)]
#[case::a_space("run failed", false)]
#[case::a_line_feed("run\nfailed", false)]
#[case::non_ascii("évènement", false)]
#[case::too_long(&"a".repeat(EVENT_ID_MAX + 1), false)]
fn an_event_id_is_the_text_a_webhook_signs_unambiguously(#[case] text: &str, #[case] valid: bool) {
    let read = EventId::new(text);

    if valid {
        assert_eq!(read.unwrap().as_str(), text);
    } else {
        assert_eq!(read, Err(EventIdInvalid));
    }
}
