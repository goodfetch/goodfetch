//! HTML read as its blocks of text, the oracle's way to judge an HTML
//! value (spec #580): a document or fragment is reduced to the text of
//! each block, in document order, its white space collapsed, so a
//! serializer's choices (a browser's DOM or the bytes served, attribute
//! order, a wrapper element, an absolute link for a relative one) never
//! matter and a lost or extra paragraph, cell or list item does. Its head,
//! scripts, styles and templates hold no text a person reads. The same
//! reading judges a text value, white space aside.
#![expect(
    clippy::disallowed_types,
    reason = "ADR 0140: the Proving ground, on no Shared path, depends on no Goodfetch crate, so its parses go uncounted (ADR 0032)"
)]

use dom_query::{Document, NodeRef};

/// The elements that hold no text a person reads.
const NOT_TEXT: [&str; 6] = ["head", "script", "style", "noscript", "template", "title"];

/// The elements that run inside a block, which every other element is.
const INLINE: [&str; 30] = [
    "a", "abbr", "b", "bdi", "bdo", "cite", "code", "data", "del", "dfn", "em", "font", "i", "img",
    "ins", "kbd", "label", "mark", "q", "s", "samp", "small", "span", "strong", "sub", "sup",
    "time", "u", "var", "wbr",
];

/// Whether `submitted` holds the blocks of text `expected` holds.
pub(crate) fn same_html(expected: &str, submitted: &str) -> bool {
    blocks(expected) == blocks(submitted)
}

/// Whether `submitted` is the text `expected` is, white space aside: each
/// run of it one space, none at either end.
pub(crate) fn same_text(expected: &str, submitted: &str) -> bool {
    collapsed(expected) == collapsed(submitted)
}

/// The text of each block of `html`, in document order.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0140: a selector or pattern compiled on no counted row yet; the Shared paths audit moves it onto its counted function"
)]
pub(crate) fn blocks(html: &str) -> Vec<String> {
    let document = Document::from(html);
    let mut blocks = Blocks::default();
    for node in document.select("html").nodes() {
        blocks.read(node);
    }
    blocks.end();
    blocks.done
}

#[derive(Default)]
struct Blocks {
    done: Vec<String>,
    current: String,
}

impl Blocks {
    fn read(&mut self, node: &NodeRef<'_>) {
        if node.is_text() {
            self.current.push_str(&node.text());
            return;
        }
        if !node.is_element() {
            return;
        }
        let name = node.node_name().unwrap_or_default();
        if NOT_TEXT.contains(&name.as_ref()) {
            return;
        }
        let block = !INLINE.contains(&name.as_ref());
        if block {
            self.end();
        }
        for child in node.children_it(false) {
            self.read(&child);
        }
        if block {
            self.end();
        }
    }

    fn end(&mut self) {
        let block = collapsed(&self.current);
        if !block.is_empty() {
            self.done.push(block);
        }
        self.current.clear();
    }
}

fn collapsed(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests;
