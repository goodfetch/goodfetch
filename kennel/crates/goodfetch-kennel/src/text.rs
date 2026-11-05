//! The Text function (ADR 0141): it makes text from the Daemon, from a
//! Run's program or from a crawled site safe to show in the menu, the
//! window or a desktop notice. Every shown string passes through it.
//!
//! - **Plain text.** Nothing is rendered: HTML-looking text is shown as it
//!   was written, and no URL becomes a link.
//! - **No control or bidirectional characters.** White space, line breaks
//!   included, becomes one space, and the ends are trimmed. Every other
//!   control character (C0, DEL, C1), every bidirectional mark, embedding,
//!   override and isolate, and the zero-width characters are stripped, so a
//!   hostile page can neither reorder what the Kennel shows nor hide text
//!   in it.
//! - **Secret values masked.** The engine already masks the Secret values
//!   a Run resolved, as `%name%`, in all it returns (ADR 0014). The Kennel
//!   cannot know any other value, so it masks the shapes credentials take
//!   in URLs, messages and Error text: a URL's password, the value of a
//!   credential's query parameter or `name=value` pair (`token`,
//!   `password`, `api_key`, a signed URL's signature, …), and the
//!   credentials after `Authorization:`. Each is shown as `••••`, which
//!   tells nothing of its length.
//! - **Truncated** to the characters its place may take, after masking,
//!   so a cut never shows the start of a secret: a cut text ends with `…`.

#[cfg(test)]
mod tests;

/// What a masked value is shown as.
const MASK: &str = "••••";

/// The names of a `name=value` pair whose value is a credential, in lower
/// case.
const CREDENTIAL_NAMES: &[&str] = &[
    "access_token",
    "api_key",
    "apikey",
    "auth",
    "client_secret",
    "id_token",
    "key",
    "passwd",
    "password",
    "pwd",
    "refresh_token",
    "secret",
    "sig",
    "signature",
    "token",
    "x-amz-credential",
    "x-amz-security-token",
    "x-amz-signature",
    "x-goog-credential",
    "x-goog-signature",
];

/// `text` made safe to show, in at most `most` characters.
pub fn shown(text: &str, most: usize) -> String {
    truncated(masked(&plain(text)), most)
}

/// `text` with white space collapsed to one space, the ends trimmed, and
/// every other control, bidirectional or zero-width character stripped.
fn plain(text: &str) -> String {
    let mut plain = String::with_capacity(text.len());
    let mut space = false;
    for character in text.chars() {
        if character.is_whitespace() {
            space = !plain.is_empty();
        } else if !hidden(character) {
            if space {
                plain.push(' ');
                space = false;
            }
            plain.push(character);
        }
    }
    plain
}

/// Whether `character` is never shown: a control character, a
/// bidirectional formatting character or a zero-width one.
fn hidden(character: char) -> bool {
    character.is_control()
        || matches!(
            character,
            // The Arabic letter mark, the left-to-right and right-to-left
            // marks, the embeddings and overrides, and the isolates.
            '\u{61c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}'
            // The zero-width space, non-joiner and joiner, the word
            // joiner and the byte order mark.
            | '\u{200b}'..='\u{200d}' | '\u{2060}' | '\u{feff}'
        )
}

/// `text` with its credentials masked.
fn masked(text: &str) -> String {
    let mut masked = String::with_capacity(text.len());
    let mut rest = text;
    while let Some((kept, value)) = next_credential(rest) {
        masked.push_str(&rest[..kept]);
        masked.push_str(MASK);
        rest = &rest[kept + value..];
    }
    masked.push_str(rest);
    masked
}

/// The next credential value in `text`: the bytes kept before it and its
/// length in bytes.
fn next_credential(text: &str) -> Option<(usize, usize)> {
    [url_password(text), named_value(text), authorization(text)]
        .into_iter()
        .flatten()
        .min_by_key(|(start, _)| *start)
}

/// A URL's password: `scheme://user:password@host`.
fn url_password(text: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(found) = text[from..].find("://") {
        let authority_start = from + found + 3;
        let authority = &text[authority_start..];
        let authority_end = authority
            .find(['/', '?', '#', ' '])
            .unwrap_or(authority.len());
        let authority = &authority[..authority_end];
        if let Some(at) = authority.rfind('@')
            && let Some(colon) = authority[..at].find(':')
            && colon + 1 < at
        {
            return Some((authority_start + colon + 1, at - colon - 1));
        }
        from = authority_start;
    }
    None
}

/// The value of a credential's `name=value` pair, whose name is a whole
/// word of [`CREDENTIAL_NAMES`], ignoring case.
fn named_value(text: &str) -> Option<(usize, usize)> {
    let mut from = 0;
    while let Some(found) = text[from..].find('=') {
        let equals = from + found;
        let name_start = text[..equals]
            .rfind(|character: char| !name_character(character))
            .map_or(0, |before| before + 1);
        let name = &text[name_start..equals];
        let value = value_length(&text[equals + 1..]);
        if value > 0
            && CREDENTIAL_NAMES
                .iter()
                .any(|credential| name.eq_ignore_ascii_case(credential))
        {
            return Some((equals + 1, value));
        }
        from = equals + 1;
    }
    None
}

/// The credentials after `Authorization:`, with its scheme, such as
/// `Bearer` or `Basic`, kept.
fn authorization(text: &str) -> Option<(usize, usize)> {
    const HEADER: &str = "authorization:";
    let lower = text.to_ascii_lowercase();
    let mut from = 0;
    while let Some(found) = lower[from..].find(HEADER) {
        let after = from + found + HEADER.len();
        let start = after + leading_spaces(&text[after..]);
        let first = value_length(&text[start..]);
        if first > 0 {
            let next = start + first;
            let gap = leading_spaces(&text[next..]);
            let second = value_length(&text[next + gap..]);
            let scheme = text[start..next]
                .chars()
                .all(|character| character.is_ascii_alphabetic());
            return Some(if scheme && gap > 0 && second > 0 {
                (next + gap, second)
            } else {
                (start, first)
            });
        }
        from = after;
    }
    None
}

/// Whether `character` can be part of a pair's name.
fn name_character(character: char) -> bool {
    character.is_ascii_alphanumeric() || matches!(character, '_' | '-')
}

/// The length in bytes of the value at the start of `text`, up to the
/// next space or separator.
fn value_length(text: &str) -> usize {
    text.find([' ', '&', '#', ';', ',', '"', '\'', '<', '>'])
        .unwrap_or(text.len())
}

/// How many spaces begin `text`, in bytes.
fn leading_spaces(text: &str) -> usize {
    text.len() - text.trim_start_matches(' ').len()
}

/// `text` in at most `most` characters, ending with `…` when it was cut.
fn truncated(text: String, most: usize) -> String {
    if text.chars().count() <= most {
        return text;
    }
    let mut cut: String = text.chars().take(most.saturating_sub(1)).collect();
    cut.push('…');
    cut
}
