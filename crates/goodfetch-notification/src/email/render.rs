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
use crate::message::Message;

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
    let link = message.link.as_ref().map(without_credentials);
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

/// `url` without its user and password, which no email carries.
fn without_credentials(url: &Url) -> Url {
    let mut clean = url.clone();
    // Only a URL that cannot be a base refuses either, and it carries
    // neither.
    let _ = clean.set_username("");
    let _ = clean.set_password(None);
    clean
}

/// `text` on one line, as a header holds it.
fn one_line(text: &str) -> String {
    text.replace(['\r', '\n'], " ")
}

/// The plain-text part: the title, the text and the link, each a
/// paragraph.
fn text(message: &Message, link: Option<&Url>) -> String {
    let mut part = format!("{}\r\n\r\n{}\r\n", message.title, message.text);
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
        escaped(&message.title),
        escaped(&message.text)
    );
    if let Some(link) = link {
        let link = escaped(link.as_str());
        let _ = write!(
            part,
            "<p>\r\n<a href=\"{link}\">\r\n{link}\r\n</a>\r\n</p>\r\n"
        );
    }
    part.push_str("</body>\r\n</html>\r\n");
    part
}

/// `text` with HTML's special characters escaped.
fn escaped(text: &str) -> String {
    text.chars().fold(
        String::with_capacity(text.len()),
        |mut escaped, character| {
            match character {
                '&' => escaped.push_str("&amp;"),
                '<' => escaped.push_str("&lt;"),
                '>' => escaped.push_str("&gt;"),
                '"' => escaped.push_str("&quot;"),
                '\'' => escaped.push_str("&#39;"),
                other => escaped.push(other),
            }
            escaped
        },
    )
}
