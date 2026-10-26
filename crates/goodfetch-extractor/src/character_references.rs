//! A raw attribute value's or text's character references decoded as the
//! HTML standard's tokenizer decodes them, for a pass that reads markup
//! without a tokenizer that decodes it (lol-html passes text and
//! attribute values through as written): numeric references with their
//! replacements, the named ones by the longest name the standard lists,
//! and an attribute's legacy reference without its `;` left as written
//! before `=` or a letter or digit.

use html5ever::data::{C1_REPLACEMENTS, NAMED_ENTITIES};

/// Where a raw value stands, which changes how it decodes.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Place {
    /// An attribute's value.
    Attribute,
    /// Text.
    Text,
}

/// `raw`, as written in `place`, decoded.
pub(crate) fn decode(raw: &str, place: Place) -> String {
    // The input stream's own preprocessing: CR LF and a lone CR become LF.
    let raw = raw.replace("\r\n", "\n").replace('\r', "\n");
    let mut decoded = String::with_capacity(raw.len());
    let mut rest = raw.as_str();
    while let Some(at) = rest.find('&') {
        decoded.push_str(&rest[..at]);
        let after = &rest[at + 1..];
        let (read, used) = if let Some(number) = after.strip_prefix('#') {
            numeric(number).map_or((None, 0), |(character, used)| (Some(character), used + 1))
        } else {
            named(after, place)
        };
        if let Some(characters) = read {
            decoded.push_str(&characters);
            rest = &after[used..];
        } else {
            decoded.push('&');
            rest = after;
        }
    }
    decoded.push_str(rest);
    match place {
        Place::Attribute => decoded.replace('\0', "\u{fffd}"),
        // The tree builder drops a NUL in text.
        Place::Text => decoded.replace('\0', ""),
    }
}

/// The character of a numeric reference whose `#` came before `after`,
/// with how many bytes of `after` it used; `None` when no digit follows.
fn numeric(after: &str) -> Option<(String, usize)> {
    let (radix, digits_at) = match after.as_bytes().first() {
        Some(b'x' | b'X') => (16, 1),
        _ => (10, 0),
    };
    let digits = after[digits_at..]
        .bytes()
        .take_while(|byte| char::from(*byte).is_digit(radix))
        .count();
    if digits == 0 {
        return None;
    }
    let value = after[digits_at..digits_at + digits]
        .chars()
        .filter_map(|digit| digit.to_digit(radix))
        // Past the last code point it stays past it, without overflowing.
        .fold(0_u32, |value, digit| {
            value
                .saturating_mul(radix)
                .saturating_add(digit)
                .min(0x11_0000)
        });
    let end = digits_at + digits;
    let used = if after[end..].starts_with(';') {
        end + 1
    } else {
        end
    };
    let character = match value {
        0 | 0xD800..=0xDFFF | 0x11_0000.. => '\u{fffd}',
        0x80..=0x9F => C1_REPLACEMENTS
            [usize::try_from(value - 0x80).expect("invariant: a C1 control's offset fits a usize")]
        .or_else(|| char::from_u32(value))
        .unwrap_or('\u{fffd}'),
        _ => char::from_u32(value).unwrap_or('\u{fffd}'),
    };
    Some((character.to_string(), used))
}

/// The characters of the named reference that begins `after`, the longest
/// name the standard lists, with how many bytes of `after` it used.
fn named(after: &str, place: Place) -> (Option<String>, usize) {
    let mut longest = None;
    for (at, character) in after.char_indices() {
        let end = at + character.len_utf8();
        match NAMED_ENTITIES.get(&after[..end]) {
            Some(&(first, second)) => {
                if first != 0 {
                    longest = Some((end, first, second));
                }
                if character == ';' {
                    break;
                }
            }
            None => break,
        }
    }
    let Some((end, first, second)) = longest else {
        return (None, 0);
    };
    let legacy = !after[..end].ends_with(';');
    let next = after[end..].chars().next();
    if place == Place::Attribute
        && legacy
        && next.is_some_and(|next| next == '=' || next.is_ascii_alphanumeric())
    {
        return (None, 0);
    }
    let characters = [first, second]
        .into_iter()
        .filter(|&point| point != 0)
        .filter_map(char::from_u32)
        .collect();
    (Some(characters), end)
}
