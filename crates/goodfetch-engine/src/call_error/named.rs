//! The names a failure's message shows that each Client spells in its own
//! form (ADR 0099): a setting key, a parameter and an event. The module
//! that writes the message seldom knows the Connection it goes to, so it
//! writes each such name marked, as [`setting`], [`parameter`] and
//! [`event`] display it, and the failure keeps the marked text beside its
//! canonical one: the RPC layer renders it in the form of the Connection
//! that receives it, and every other reader, a log, a stored outcome or a
//! command of the executable, reads the canonical names.
//!
//! A mark is the name between two characters of Unicode's Private Use Area,
//! which no message of the engine writes otherwise; text that only looks
//! like a mark, as a page's text might, renders as it is.

use std::borrow::Cow;
use std::fmt;

use goodfetch_protocol::catalog::Event;
use goodfetch_protocol::connection::Form;
use goodfetch_protocol::naming;
use serde_json::Value;

/// Opens a mark, followed by its kind's letter.
const OPEN: char = '\u{E000}';
/// Closes a mark.
const CLOSE: char = '\u{E001}';
/// Parts an event's name from its payload's type.
const PAYLOAD: char = '\u{E002}';

/// The kind of a marked name, as its letter writes it.
#[derive(Clone, Copy)]
enum Kind {
    Setting,
    Parameter,
    Event,
}

impl Kind {
    fn letter(self) -> char {
        match self {
            Self::Setting => 's',
            Self::Parameter => 'p',
            Self::Event => 'e',
        }
    }

    fn of(letter: char) -> Option<Self> {
        match letter {
            's' => Some(Self::Setting),
            'p' => Some(Self::Parameter),
            'e' => Some(Self::Event),
            _ => None,
        }
    }
}

/// A name a message shows, marked for its Client's form: written into the
/// message with `format!`, as `` "`{}`" ``.
pub struct Named<'a> {
    kind: Kind,
    name: &'a str,
    /// An event's payload type, which names Rust's marker of an event whose
    /// own name would end in `Error`; empty for the other kinds.
    payload: Cow<'a, str>,
}

impl fmt::Display for Named<'_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{OPEN}{}{}", self.kind.letter(), self.name)?;
        if !self.payload.is_empty() {
            write!(formatter, "{PAYLOAD}{}", self.payload)?;
        }
        write!(formatter, "{CLOSE}")
    }
}

/// The setting `key` (ADR 0083), as its Client sets it: `timeout.first_byte`
/// reads `timeout.firstByte` in TypeScript and `timeout::FIRST_BYTE` in
/// Rust.
pub fn setting(key: &str) -> Named<'_> {
    Named {
        kind: Kind::Setting,
        name: key,
        payload: Cow::Borrowed(""),
    }
}

/// The parameter `name` of a call, as its Client passes it: `max_redirects`
/// reads `maxRedirects` in TypeScript.
pub fn parameter(name: &str) -> Named<'_> {
    Named {
        kind: Kind::Parameter,
        name,
        payload: Cow::Borrowed(""),
    }
}

/// The catalog's `event`, as its Client's listener names it:
/// `page.request_finished` reads `requestfinished` in TypeScript and Python
/// and `events::RequestFinished` in Rust, `page.page_error`
/// `events::WebError`, its payload's type.
pub fn event(event: &Event) -> Named<'static> {
    let (name, payload) = naming::listened(event);
    Named {
        kind: Kind::Event,
        name,
        payload: payload.map_or(Cow::Borrowed(""), Cow::Owned),
    }
}

/// Whether `text` holds a mark.
pub(crate) fn marked(text: &str) -> bool {
    text.contains(OPEN)
}

/// Whether a string of `value` holds a mark.
pub(crate) fn marked_value(value: &Value) -> bool {
    match value {
        Value::String(text) => marked(text),
        Value::Array(values) => values.iter().any(marked_value),
        Value::Object(members) => members.values().any(marked_value),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

/// `text` with each mark rendered in `form`, the catalog's own names for
/// [`Form::Catalog`].
pub(crate) fn render(text: &str, form: Form) -> Cow<'_, str> {
    if !marked(text) {
        return Cow::Borrowed(text);
    }
    let mut rendered = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(OPEN) {
        rendered.push_str(&rest[..start]);
        let after = &rest[start + OPEN.len_utf8()..];
        match spelled(after, form) {
            Some((name, length)) => {
                rendered.push_str(&name);
                rest = &after[length..];
            }
            None => {
                rendered.push(OPEN);
                rest = after;
            }
        }
    }
    rendered.push_str(rest);
    Cow::Owned(rendered)
}

/// `value` with each of its strings rendered in `form`.
pub(crate) fn render_value(value: &Value, form: Form) -> Value {
    match value {
        Value::String(text) => Value::String(render(text, form).into_owned()),
        Value::Array(values) => Value::Array(
            values
                .iter()
                .map(|member| render_value(member, form))
                .collect(),
        ),
        Value::Object(members) => Value::Object(
            members
                .iter()
                .map(|(key, member)| (key.clone(), render_value(member, form)))
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) => value.clone(),
    }
}

/// The mark `after` opens with, past its opening character, spelled in
/// `form`, with the length it takes in `after`; `None` when it is no mark.
fn spelled(after: &str, form: Form) -> Option<(String, usize)> {
    let mut characters = after.chars();
    let kind = Kind::of(characters.next()?)?;
    let body = characters.as_str();
    let end = body.find(CLOSE)?;
    let (name, payload) = body[..end]
        .split_once(PAYLOAD)
        .unwrap_or((&body[..end], ""));
    let is_name = !name.is_empty()
        && name
            .chars()
            .all(|character| matches!(character, 'a'..='z' | '0'..='9' | '_' | '.'));
    if !is_name
        || !payload
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
    {
        return None;
    }
    let spelled = match kind {
        Kind::Setting => naming::setting(form, name),
        Kind::Parameter => naming::parameter(form, name),
        Kind::Event => naming::event(form, name, payload),
    };
    Some((spelled, 1 + end + CLOSE.len_utf8()))
}

#[cfg(test)]
mod tests {
    use goodfetch_protocol::connection::Form;
    use goodfetch_protocol::page::events::{PAGE_PAGE_ERROR, PAGE_REQUEST_FINISHED};
    use rstest::rstest;

    use super::{event, parameter, render, setting};

    /// Each kind of name renders in each SDK's form, and canonically for
    /// the catalog, around the text the message writes itself.
    #[rstest]
    #[case::setting_typescript(Form::Typescript, "`timeout.firstByte` passed")]
    #[case::setting_python(Form::Python, "`timeout.first_byte` passed")]
    #[case::setting_rust(Form::Rust, "`timeout::FIRST_BYTE` passed")]
    #[case::setting_catalog(Form::Catalog, "`timeout.first_byte` passed")]
    fn a_marked_setting_renders_in_the_form(#[case] form: Form, #[case] expected: &str) {
        let text = format!("`{}` passed", setting("timeout.first_byte"));

        assert_eq!(render(&text, form), expected);
    }

    #[rstest]
    #[case::parameter_typescript(Form::Typescript, "`maxRedirects` is 30")]
    #[case::parameter_python(Form::Python, "`max_redirects` is 30")]
    #[case::parameter_rust(Form::Rust, "`max_redirects` is 30")]
    #[case::parameter_catalog(Form::Catalog, "`max_redirects` is 30")]
    fn a_marked_parameter_renders_in_the_form(#[case] form: Form, #[case] expected: &str) {
        let text = format!("`{}` is 30", parameter("max_redirects"));

        assert_eq!(render(&text, form), expected);
    }

    #[rstest]
    #[case::event_typescript(Form::Typescript, "`pageerror` and `requestfinished`")]
    #[case::event_python(Form::Python, "`pageerror` and `requestfinished`")]
    #[case::event_rust(Form::Rust, "`events::WebError` and `events::RequestFinished`")]
    #[case::event_catalog(Form::Catalog, "`page_error` and `request_finished`")]
    fn a_marked_event_renders_in_the_form(#[case] form: Form, #[case] expected: &str) {
        let text = format!(
            "`{}` and `{}`",
            event(&PAGE_PAGE_ERROR),
            event(&PAGE_REQUEST_FINISHED)
        );

        assert_eq!(render(&text, form), expected);
    }

    /// Text that only looks like a mark, as a page's own text may, keeps
    /// every character.
    #[rstest]
    #[case::unclosed("a \u{E000}sopen")]
    #[case::unknown_kind("a \u{E000}xname\u{E001}")]
    #[case::no_name("a \u{E000}s\u{E001}")]
    #[case::not_a_name("a \u{E000}sName With Spaces\u{E001}")]
    fn text_that_is_no_mark_renders_as_it_is(#[case] text: &str) {
        assert_eq!(render(text, Form::Typescript), text);
    }
}
