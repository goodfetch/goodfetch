//! RSS 2.0 and Atom (RFC 4287) feeds: one UTF-8 document named after the
//! Dataset, one item or entry per Record, its title, link, date and summary
//! taken from the fields the `export.feed.*` settings name. A field's text
//! is a string's own, or another value's JSON text. A date is read as RFC
//! 3339 or RFC 2822 and written as RSS's RFC 2822 `pubDate` or Atom's RFC
//! 3339 `published` and `updated`; an entry with no such date is `updated`
//! when its Record was committed, and the feed's `updated` is its latest
//! entry's, written after them (Atom orders no feed child), or the epoch
//! when it has none. An item's `guid` and an entry's ID are its link, or
//! the feed's ID with the Record's number. A Record read past a Wall gets a
//! category of the scheme `urn:goodfetch:wall` naming its kind.

use jiff::Timestamp;
use jiff::fmt::rfc2822;
use jiff::tz::TimeZone;
use serde_json::Value;

use super::xml::escape;

/// The scheme of a Wall marker's category.
const WALL_SCHEME: &str = "urn:goodfetch:wall";

/// The fields an item or entry is taken from, `export.feed.*`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FeedFields {
    /// Its title, `export.feed.title`.
    pub title: String,
    /// Its link, `export.feed.link`.
    pub link: String,
    /// Its date, `export.feed.published`.
    pub published: String,
    /// Its summary, `export.feed.summary`.
    pub summary: String,
}

impl Default for FeedFields {
    fn default() -> Self {
        Self {
            title: "title".to_owned(),
            link: "url".to_owned(),
            published: "published".to_owned(),
            summary: "summary".to_owned(),
        }
    }
}

/// Which feed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Flavor {
    Rss,
    Atom,
}

/// The writer of one feed Export.
#[derive(Clone, Debug)]
pub(super) struct Feed {
    pub(super) flavor: Flavor,
    pub(super) fields: FeedFields,
    pub(super) title: String,
    pub(super) id: String,
    pub(super) updated: Option<Timestamp>,
}

/// One item's or entry's values.
struct Item {
    title: Option<String>,
    link: Option<String>,
    published: Option<Timestamp>,
    summary: Option<String>,
    wall: Option<String>,
}

impl Feed {
    pub(super) fn begin(&self, out: &mut Vec<u8>) {
        let mut text = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
        match self.flavor {
            Flavor::Rss => {
                text.push_str("<rss version=\"2.0\">\n<channel>\n");
                line("title", &self.title, &mut text);
                line("link", &self.id, &mut text);
                line("description", &self.title, &mut text);
            }
            Flavor::Atom => {
                text.push_str("<feed xmlns=\"http://www.w3.org/2005/Atom\">\n");
                line("title", &self.title, &mut text);
                line("id", &self.id, &mut text);
                text.push_str("<author>");
                tagged("name", &self.title, &mut text);
                text.push_str("</author>\n");
            }
        }
        out.extend_from_slice(text.as_bytes());
    }

    /// Writes the item or entry of the Record numbered `number`, committed
    /// at `committed_at`, whose value is `record`.
    pub(super) fn value(
        &mut self,
        number: u64,
        record: &Value,
        committed_at: &str,
        out: &mut Vec<u8>,
    ) {
        let item = Item {
            title: text_of(record, &self.fields.title),
            link: text_of(record, &self.fields.link),
            published: text_of(record, &self.fields.published).and_then(|text| date(&text)),
            summary: text_of(record, &self.fields.summary),
            wall: record
                .get("#wall")
                .and_then(|marker| marker.get("kind"))
                .and_then(Value::as_str)
                .map(str::to_owned),
        };
        let id = item
            .link
            .clone()
            .unwrap_or_else(|| format!("{}:{number}", self.id));
        let mut text = String::new();
        match self.flavor {
            Flavor::Rss => {
                text.push_str("<item>");
                // RSS 2.0 wants a title or a description in every item.
                match (&item.title, &item.summary) {
                    (Some(title), _) => tagged("title", title, &mut text),
                    (None, None) => tagged("title", "", &mut text),
                    (None, Some(_)) => {}
                }
                if let Some(link) = &item.link {
                    tagged("link", link, &mut text);
                }
                if let Some(summary) = &item.summary {
                    tagged("description", summary, &mut text);
                }
                if let Some(published) = item.published {
                    let date = rfc2822::DateTimePrinter::new()
                        .zoned_to_string(&published.to_zoned(TimeZone::UTC))
                        .expect("invariant: a timestamp from year 1 prints in RFC 2822");
                    tagged("pubDate", &date, &mut text);
                }
                text.push_str(if item.link.is_some() {
                    "<guid isPermaLink=\"true\">"
                } else {
                    "<guid isPermaLink=\"false\">"
                });
                escape(&id, false, &mut text);
                text.push_str("</guid>");
                if let Some(kind) = &item.wall {
                    text.push_str("<category domain=\"");
                    text.push_str(WALL_SCHEME);
                    text.push_str("\">");
                    escape(kind, false, &mut text);
                    text.push_str("</category>");
                }
                text.push_str("</item>\n");
            }
            Flavor::Atom => {
                let updated = item
                    .published
                    .or_else(|| date(committed_at))
                    .unwrap_or_default();
                self.updated = Some(self.updated.map_or(updated, |latest| latest.max(updated)));
                text.push_str("<entry>");
                tagged("id", &id, &mut text);
                tagged(
                    "title",
                    item.title.as_deref().unwrap_or_default(),
                    &mut text,
                );
                if let Some(link) = &item.link {
                    text.push_str("<link href=\"");
                    escape(link, true, &mut text);
                    text.push_str("\"/>");
                }
                if let Some(published) = item.published {
                    tagged("published", &published.to_string(), &mut text);
                }
                tagged("updated", &updated.to_string(), &mut text);
                if let Some(summary) = &item.summary {
                    tagged("summary", summary, &mut text);
                }
                if let Some(kind) = &item.wall {
                    text.push_str("<category scheme=\"");
                    text.push_str(WALL_SCHEME);
                    text.push_str("\" term=\"");
                    escape(kind, true, &mut text);
                    text.push_str("\"/>");
                }
                text.push_str("</entry>\n");
            }
        }
        out.extend_from_slice(text.as_bytes());
    }

    pub(super) fn end(&self, out: &mut Vec<u8>) {
        let mut text = String::new();
        match self.flavor {
            Flavor::Rss => text.push_str("</channel>\n</rss>\n"),
            Flavor::Atom => {
                line(
                    "updated",
                    &self.updated.unwrap_or_default().to_string(),
                    &mut text,
                );
                text.push_str("</feed>\n");
            }
        }
        out.extend_from_slice(text.as_bytes());
    }
}

/// Writes `<name>text</name>`.
fn tagged(name: &str, text: &str, out: &mut String) {
    out.push('<');
    out.push_str(name);
    out.push('>');
    escape(text, false, out);
    out.push_str("</");
    out.push_str(name);
    out.push('>');
}

/// Writes `<name>text</name>` on a line of its own.
fn line(name: &str, text: &str, out: &mut String) {
    tagged(name, text, out);
    out.push('\n');
}

/// The text of `record`'s field `name`: a string's own, another value's
/// JSON text; none when it lacks it or holds `null`.
fn text_of(record: &Value, name: &str) -> Option<String> {
    match record.get(name)? {
        Value::Null => None,
        Value::String(text) => Some(text.clone()),
        other => Some(other.to_string()),
    }
}

/// `text` read as an RFC 3339 or RFC 2822 date and time.
fn date(text: &str) -> Option<Timestamp> {
    text.parse::<Timestamp>()
        .ok()
        .or_else(|| rfc2822::parse(text).ok().map(|zoned| zoned.timestamp()))
}
