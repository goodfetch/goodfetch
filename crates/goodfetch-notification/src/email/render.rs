//! How an email renders a Notification: one message whose subject is the
//! title, with a plain-text part and a minimal HTML part, each holding the
//! title, the text and the link, the link without the user and password it
//! may carry. Its `Message-ID` is the event ID at the sender's domain, so a
//! retried delivery is the same message to the reader's mail client.

use std::fmt::Write as _;
use std::time::{SystemTime, UNIX_EPOCH};

use mail_builder::MessageBuilder;
use mail_builder::headers::date::Date;
use url::Url;

use super::Address;
use crate::message::{Message, html_escaped};

/// What an email that reports a Notification says: its subject and its
/// two parts, as a provider's HTTP API takes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Parts {
    /// The title, on one line.
    pub(super) subject: String,
    /// The plain-text part.
    pub(super) text: String,
    /// The HTML part.
    pub(super) html: String,
}

/// The subject and the two parts that report `message`.
pub(super) fn parts(message: &Message) -> Parts {
    let link = message.link_without_credentials();
    Parts {
        subject: one_line(&message.title),
        text: text(message, link.as_ref()),
        html: html(message, link.as_ref()),
    }
}

/// The message that reports `message`, from `from` to each of `to`, dated
/// `at`.
pub(super) fn message(
    message: &Message,
    from: &Address,
    to: &[Address],
    at: SystemTime,
) -> Vec<u8> {
    let Parts {
        subject,
        text,
        html,
    } = parts(message);
    let seconds = at.duration_since(UNIX_EPOCH).map_or(0, |since| {
        i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
    });
    MessageBuilder::new()
        .from(from.as_str())
        .to(to.iter().map(Address::as_str).collect::<Vec<_>>())
        .subject(subject)
        .date(Date::new(seconds))
        .message_id(format!("{}@{}", message.event_id.as_str(), from.domain()))
        .text_body(text)
        .html_body(html)
        .write_to_vec()
        .expect("invariant: writing a message to memory cannot fail")
}

/// `text` on one line, as a header holds it: each control character, a
/// line break among them, a space.
fn one_line(text: &str) -> String {
    text.chars()
        .map(|character| {
            if character.is_control() {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// `text` as a part holds it: each control character but a line break and
/// a tab a space. `mail-builder` writes a part's other control characters,
/// NUL among them, as they come, which a message may not carry (RFC 5322,
/// section 2.3); it turns each bare line feed into a line break.
fn printable(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .map(|character| {
            if character.is_control() && !matches!(character, '\n' | '\t') {
                ' '
            } else {
                character
            }
        })
        .collect()
}

/// The plain-text part: the title, the text and the link, each a
/// paragraph.
fn text(message: &Message, link: Option<&Url>) -> String {
    let mut part = format!(
        "{}\r\n\r\n{}\r\n",
        printable(&message.title),
        printable(&message.text)
    );
    if let Some(link) = link {
        let _ = write!(part, "\r\n{link}\r\n");
    }
    part
}

/// The HTML part: the title as a heading, the text as a paragraph and the
/// link as an anchor, each element and the link on a line of its own so
/// the part needs no line folding.
fn html(message: &Message, link: Option<&Url>) -> String {
    let mut part = format!(
        "<!DOCTYPE html>\r\n<html>\r\n<body>\r\n<h1>\r\n{}\r\n</h1>\r\n<p>\r\n{}\r\n</p>\r\n",
        html_escaped(&printable(&message.title)),
        html_escaped(&printable(&message.text))
    );
    if let Some(link) = link {
        let link = html_escaped(link.as_str());
        let _ = write!(
            part,
            "<p>\r\n<a href=\"{link}\">\r\n{link}\r\n</a>\r\n</p>\r\n"
        );
    }
    part.push_str("</body>\r\n</html>\r\n");
    part
}
