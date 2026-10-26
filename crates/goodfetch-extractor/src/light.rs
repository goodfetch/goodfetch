//! A DOM's light tree (ADR 0121): a browser serializes each shadow root,
//! open or closed, as a `<template shadowrootmode>` first in its host,
//! which `content()` does not (Playwright's meaning). [`light`] takes each
//! such template out of the serialization, byte for byte otherwise, so
//! `html.raw` is what `content()` returns. The serialization is a browser's,
//! so it is well formed: only comments and raw text elements, whose text
//! may spell a tag, are read past.

/// The elements whose content is text, never markup, until their end tag.
const RAW_TEXT: [&str; 8] = [
    "script", "style", "textarea", "title", "xmp", "iframe", "noembed", "noframes",
];

/// `html`, a browser's serialization of its DOM, without the shadow roots
/// serialized in it.
pub(crate) fn light(html: &str) -> String {
    if !html.contains("shadowrootmode") {
        return html.to_owned();
    }
    let mut kept = String::with_capacity(html.len());
    // The depth of `<template>` elements inside the shadow root being left
    // out, its own included; 0 outside one.
    let mut depth = 0_usize;
    let mut at = 0;
    while let Some(found) = html[at..].find('<') {
        let start = at + found;
        if depth == 0 {
            kept.push_str(&html[at..start]);
        }
        let (end, tag) = read_markup(html, start);
        match tag {
            Tag::Start { name, shadow_root } if name == "template" => {
                if depth > 0 || shadow_root {
                    depth += 1;
                }
            }
            Tag::End { name } if name == "template" && depth > 0 => {
                depth -= 1;
                at = end;
                continue;
            }
            _ => {}
        }
        if depth == 0 {
            kept.push_str(&html[start..end]);
        }
        at = end;
    }
    if depth == 0 {
        kept.push_str(&html[at..]);
    }
    kept
}

enum Tag {
    Start { name: String, shadow_root: bool },
    End { name: String },
    Other,
}

/// The end of the markup that starts at `start` (a `<`), a raw text
/// element's text and end tag included, and what it is.
fn read_markup(html: &str, start: usize) -> (usize, Tag) {
    let rest = &html[start..];
    if let Some(comment) = rest.strip_prefix("<!--") {
        let end = comment
            .find("-->")
            .map_or(html.len(), |at| start + "<!--".len() + at + "-->".len());
        return (end, Tag::Other);
    }
    let Some(close) = tag_end(rest) else {
        return (html.len(), Tag::Other);
    };
    let end = start + close;
    let inside = &rest[1..close - 1];
    if let Some(name) = inside.strip_prefix('/') {
        return (
            end,
            Tag::End {
                name: name.trim().to_ascii_lowercase(),
            },
        );
    }
    let name: String = inside
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect::<String>()
        .to_ascii_lowercase();
    if name.is_empty() {
        return (end, Tag::Other);
    }
    if RAW_TEXT.contains(&name.as_str()) {
        let closing = format!("</{name}");
        let after = html[end..]
            .to_ascii_lowercase()
            .find(&closing)
            .map_or(html.len(), |at| end + at);
        let after = html[after..]
            .find('>')
            .map_or(html.len(), |at| after + at + 1);
        return (after, Tag::Other);
    }
    let shadow_root = name == "template" && has_attribute(&inside[name.len()..], "shadowrootmode");
    (end, Tag::Start { name, shadow_root })
}

/// The length of the tag at the start of `rest` up to its `>`, quoted
/// attribute values read past.
fn tag_end(rest: &str) -> Option<usize> {
    let mut quote = None;
    for (at, c) in rest.char_indices().skip(1) {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), _) if c == open => quote = None,
            (None, '>') => return Some(at + 1),
            _ => {}
        }
    }
    None
}

/// Whether the attributes `attributes` (a start tag's, after its name)
/// name `attribute`, outside their values.
fn has_attribute(attributes: &str, attribute: &str) -> bool {
    let mut quote = None;
    let mut name = String::new();
    for c in attributes.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(open), _) if c == open => quote = None,
            (Some(_), _) => {}
            (None, c) if c.is_whitespace() || c == '=' || c == '/' => {
                if name.eq_ignore_ascii_case(attribute) {
                    return true;
                }
                name.clear();
            }
            (None, c) => name.push(c),
        }
    }
    name.eq_ignore_ascii_case(attribute)
}
