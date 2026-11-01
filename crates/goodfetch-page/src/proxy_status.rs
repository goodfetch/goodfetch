//! The `Proxy-Status` field (RFC 9209) as a structured field list (RFC
//! 8941): its members, each an intermediary's name and its `;key=value`
//! parameters, a value a token or a quoted string. The proxy-or-site rule
//! ([`crate::proxy`]) and a hosted Proxy provider's translation point
//! ([`crate::proxy_provider`]) read the field through it, so it is parsed
//! one way.

/// The header an intermediary names its own errors in (RFC 9209), as the
/// browser gives a response's headers, lower-cased.
pub(crate) const PROXY_STATUS: &str = "proxy-status";

/// The last member of a `Proxy-Status` field: the intermediary nearest the
/// browser appends its own (RFC 9209).
pub(crate) fn nearest(field: &str) -> Option<&str> {
    split_outside_quotes(field, ',').pop()
}

/// A member's intermediary name, its item before any parameter.
pub(crate) fn name(member: &str) -> &str {
    split_outside_quotes(member, ';')
        .first()
        .map_or("", |item| item.trim())
}

/// The value of the parameter `name` of one member, unquoted; none when it
/// has none or an empty one.
pub(crate) fn parameter(member: &str, name: &str) -> Option<String> {
    split_outside_quotes(member, ';')
        .into_iter()
        .skip(1)
        .find_map(|parameter| {
            let (key, value) = parameter.split_once('=')?;
            (key.trim() == name).then(|| unquoted(value.trim()))
        })
        .filter(|value| !value.is_empty())
}

/// `text` split at each `separator` outside a quoted string.
fn split_outside_quotes(text: &str, separator: char) -> Vec<&str> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut escaped = false;
    for (at, character) in text.char_indices() {
        match character {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            _ if character == separator && !quoted => {
                parts.push(&text[start..at]);
                start = at + character.len_utf8();
            }
            _ => {}
        }
    }
    parts.push(&text[start..]);
    parts
}

/// A token as it is, or a quoted string without its quotes and escapes.
fn unquoted(value: &str) -> String {
    let Some(inner) = value
        .strip_prefix('"')
        .and_then(|rest| rest.strip_suffix('"'))
    else {
        return value.to_owned();
    };
    let mut text = String::with_capacity(inner.len());
    let mut escaped = false;
    for character in inner.chars() {
        if escaped || character != '\\' {
            text.push(character);
            escaped = false;
        } else {
            escaped = true;
        }
    }
    text
}
