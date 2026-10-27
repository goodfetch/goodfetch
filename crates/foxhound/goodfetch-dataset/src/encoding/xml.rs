//! XML: one UTF-8 document whose root element, named by `export.xml.root`,
//! holds one element per Record, named by `export.xml.record`, each on a
//! line of its own. Each field is an element of its name, or `field` with
//! a `name` attribute where its name is no XML name; a string, a number,
//! `true` and `false` are its text, an object's fields are elements inside
//! it, an array's values `item` elements inside it, and `null` an empty
//! element with `xsi:nil="true"`. Text escapes `&`, `<` and `>`, and a
//! carriage return as `&#13;` so a parser gives it back; an attribute also
//! escapes `"`; a character XML 1.0 cannot hold is written U+FFFD.

use serde_json::Value;

/// The writer of one XML Export.
#[derive(Clone, Debug)]
pub(super) struct Xml {
    pub(super) root: String,
    pub(super) record: String,
}

impl Xml {
    pub(super) fn begin(&self, out: &mut Vec<u8>) {
        let text = format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<{} xmlns:xsi=\"http://www.w3.org/2001/XMLSchema-instance\">\n",
            self.root
        );
        out.extend_from_slice(text.as_bytes());
    }

    pub(super) fn value(&self, record: &Value, out: &mut Vec<u8>) {
        let mut text = String::new();
        element(&self.record, true, record, &mut text);
        text.push('\n');
        out.extend_from_slice(text.as_bytes());
    }

    pub(super) fn end(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(format!("</{}>\n", self.root).as_bytes());
    }
}

/// Writes `value` as an element named `name`, which is a checked XML name
/// when `named` holds, or else becomes `field` with a `name` attribute.
fn element(name: &str, named: bool, value: &Value, out: &mut String) {
    let tag = if named || is_name(name) {
        out.push('<');
        out.push_str(name);
        name
    } else {
        out.push_str("<field name=\"");
        escape(name, true, out);
        out.push('"');
        "field"
    };
    match value {
        Value::Null => {
            out.push_str(" xsi:nil=\"true\"/>");
            return;
        }
        Value::String(text) => {
            out.push('>');
            escape(text, false, out);
        }
        Value::Bool(boolean) => {
            out.push('>');
            out.push_str(if *boolean { "true" } else { "false" });
        }
        Value::Number(number) => {
            out.push('>');
            out.push_str(&number.to_string());
        }
        Value::Object(object) => {
            out.push('>');
            for (inner, value) in object {
                element(inner, false, value, out);
            }
        }
        Value::Array(values) => {
            out.push('>');
            for value in values {
                element("item", true, value, out);
            }
        }
    }
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// Whether `text` is an XML name without a colon: a letter or `_`, then
/// letters, digits, `-`, `.` and `_`.
pub(crate) fn is_name(text: &str) -> bool {
    let mut characters = text.chars();
    characters
        .next()
        .is_some_and(|first| first.is_alphabetic() || first == '_')
        && characters
            .all(|character| character.is_alphanumeric() || matches!(character, '-' | '.' | '_'))
}

/// Writes `text` as XML text, or as an attribute's value when `attribute`
/// holds.
pub(super) fn escape(text: &str, attribute: bool, out: &mut String) {
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if attribute => out.push_str("&quot;"),
            '\r' => out.push_str("&#13;"),
            '\t' if attribute => out.push_str("&#9;"),
            '\n' if attribute => out.push_str("&#10;"),
            '\t' | '\n' => out.push(character),
            '\u{0}'..='\u{1f}' | '\u{fffe}' | '\u{ffff}' => out.push('\u{fffd}'),
            other => out.push(other),
        }
    }
}
