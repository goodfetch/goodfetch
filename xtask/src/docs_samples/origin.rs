//! The online Proving ground's origin, which the docs show, and what an
//! extracted sample holds in its place: samples run against a local Proving
//! ground, never the online one (ADR 0068), whose origin their runner gives
//! in `GOODFETCH_DOCS_ORIGIN`.

/// The online Proving ground's origin, on a subdomain of `goodfetch.dev`
/// (ADRs 0025 and 0032).
pub(super) const ORIGIN: &str = "https://proving-ground.goodfetch.dev";

/// What an extracted sample holds in the origin's place, for its runner to
/// replace with the local Proving ground's.
pub(super) const PLACEHOLDER: &str = "__GOODFETCH_DOCS_ORIGIN__";

/// The hidden line a `rust` sample that writes the origin in a string
/// literal starts with: the literal becomes the macro's call, so the
/// doctest reads the origin from `GOODFETCH_DOCS_ORIGIN` when it compiles.
pub(super) const RUST_MACRO: &str = "# macro_rules! goodfetch_docs_origin { ($path:literal) => { concat!(env!(\"GOODFETCH_DOCS_ORIGIN\"), $path) }; }";

/// Whether `text` writes the origin.
pub(super) fn holds(text: &str) -> bool {
    text.contains(ORIGIN)
}

/// `text` with the placeholder for each origin.
pub(super) fn in_text(text: &str) -> String {
    text.replace(ORIGIN, PLACEHOLDER)
}

/// A line of Rust with each string literal that starts with the origin, on
/// one line, written as `goodfetch_docs_origin!("<rest>")`, and the
/// placeholder for any other origin, as in a comment or a raw string; and
/// whether it called the macro.
pub(super) fn in_rust(line: &str) -> (String, bool) {
    let opening = format!("\"{ORIGIN}");
    let mut rest = line;
    let mut written = String::new();
    let mut called = false;
    while let Some(start) = rest.find(&opening) {
        let plain = rest[..start]
            .chars()
            .next_back()
            .is_none_or(|before| !before.is_alphanumeric() && before != '#' && before != '_');
        let after = &rest[start + opening.len()..];
        if let Some(end) = closing_quote(after).filter(|_| plain) {
            written.push_str(&rest[..start]);
            written.push_str("goodfetch_docs_origin!(\"");
            written.push_str(&after[..=end]);
            written.push(')');
            rest = &after[end + 1..];
            called = true;
        } else {
            written.push_str(&rest[..start + opening.len()]);
            rest = after;
        }
    }
    written.push_str(rest);
    (in_text(&written), called)
}

/// The byte index of the first `"` in `text` that no backslash escapes.
fn closing_quote(text: &str) -> Option<usize> {
    let mut escaped = false;
    for (index, character) in text.char_indices() {
        match character {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return Some(index),
            _ => escaped = false,
        }
    }
    None
}
