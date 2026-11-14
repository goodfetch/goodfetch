//! The index removal files a page's plain CSS rules in (spec #2164), so
//! that an element is tested only against the rules it could match. A rule
//! is filed under one key of its rightmost compound, the one its subject
//! must match: its ID, else a class, else an attribute's name, else its
//! tag, and a rule with none of them, or one whose text this does not key
//! exactly, under no key, which every element is tested against. A selector
//! list is filed under each member's key.
//!
//! The index only narrows: the match stays `Matcher`'s. Keys are folded to
//! ASCII lower case on both sides, which finds every rule a case-sensitive
//! or a case-insensitive match could accept, and never adds a removal.

use std::collections::BTreeMap;

use cssparser::{ParseError, Parser, ParserInput, Token};
use dom_query::NodeRef;

/// What a rule is filed under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum Key {
    Id(String),
    Class(String),
    Attribute(String),
    Tag(String),
    /// No key: the rule is tested against every element.
    Universal,
}

/// The rules of one page, by position in rule order, under their keys.
#[derive(Default)]
pub(super) struct Index {
    ids: BTreeMap<String, Vec<usize>>,
    classes: BTreeMap<String, Vec<usize>>,
    attributes: BTreeMap<String, Vec<usize>>,
    tags: BTreeMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

impl Index {
    /// The index of the rules whose selectors are `selectors`, in rule
    /// order.
    pub(super) fn of<'a>(selectors: impl Iterator<Item = &'a str>) -> Self {
        let mut index = Self::default();
        for (position, selector) in selectors.enumerate() {
            for key in keys(selector) {
                let bucket = match key {
                    Key::Id(id) => index.ids.entry(id).or_default(),
                    Key::Class(class) => index.classes.entry(class).or_default(),
                    Key::Attribute(name) => index.attributes.entry(name).or_default(),
                    Key::Tag(tag) => index.tags.entry(tag).or_default(),
                    Key::Universal => &mut index.universal,
                };
                // Two members of one list may share a key.
                if bucket.last() != Some(&position) {
                    bucket.push(position);
                }
            }
        }
        index
    }

    /// Into `candidates`, the positions of the rules `node` could match, in
    /// rule order, each once: those filed under its ID, its classes, its
    /// attributes' names and its tag, and those under no key.
    pub(super) fn candidates(&self, node: &NodeRef<'_>, candidates: &mut Vec<usize>) {
        candidates.clear();
        candidates.extend_from_slice(&self.universal);
        node.query(|node| {
            let Some(element) = node.as_element() else {
                return;
            };
            filed(&self.tags, &element.name.local, candidates);
            for attribute in &element.attrs {
                let name: &str = &attribute.name.local;
                filed(&self.attributes, name, candidates);
                if name.eq_ignore_ascii_case("id") {
                    filed(&self.ids, &attribute.value, candidates);
                } else if name.eq_ignore_ascii_case("class") {
                    for class in attribute.value.split_ascii_whitespace() {
                        filed(&self.classes, class, candidates);
                    }
                }
            }
        });
        candidates.sort_unstable();
        candidates.dedup();
    }
}

/// Adds to `candidates` the rules `bucket` files under `key`, folded as
/// the keys are.
fn filed(bucket: &BTreeMap<String, Vec<usize>>, key: &str, candidates: &mut Vec<usize>) {
    let rules = if key.bytes().any(|byte| byte.is_ascii_uppercase()) {
        bucket.get(&key.to_ascii_lowercase())
    } else {
        bucket.get(key)
    };
    if let Some(rules) = rules {
        candidates.extend_from_slice(rules);
    }
}

/// The parts of a compound selector that key it.
#[derive(Default)]
struct Compound {
    id: Option<String>,
    class: Option<String>,
    attribute: Option<String>,
    tag: Option<String>,
    /// How many simple selectors it holds so far.
    parts: usize,
}

impl Compound {
    /// Its most specific key.
    fn key(self) -> Key {
        if let Some(id) = self.id {
            Key::Id(id)
        } else if let Some(class) = self.class {
            Key::Class(class)
        } else if let Some(name) = self.attribute {
            Key::Attribute(name)
        } else if let Some(tag) = self.tag {
            Key::Tag(tag)
        } else {
            Key::Universal
        }
    }
}

/// The key of each member of the selector list `selector`: that of its
/// rightmost compound, read from the selector's tokens, and
/// [`Key::Universal`] for a member holding anything this does not read
/// exactly: a namespace, a pseudo-element, which moves the subject, or a
/// token no selector holds.
pub(super) fn keys(selector: &str) -> Vec<Key> {
    let mut input = ParserInput::new(selector);
    let mut parser = Parser::new(&mut input);
    let mut keys = Vec::new();
    let mut compound = Compound::default();
    // Whether the member read so far is keyed exactly.
    let mut exact = true;
    // Whether white space followed the compound's last part: a descendant
    // combinator when another part follows.
    let mut spaced = false;
    let key = |compound: Compound, exact: bool| {
        if exact {
            compound.key()
        } else {
            Key::Universal
        }
    };
    while let Ok(token) = parser.next_including_whitespace().cloned() {
        match token {
            Token::WhiteSpace(_) => {
                spaced = true;
                continue;
            }
            Token::Comma => {
                keys.push(key(std::mem::take(&mut compound), exact));
                (exact, spaced) = (true, false);
                continue;
            }
            Token::Delim('>' | '+' | '~') => {
                compound = Compound::default();
                spaced = false;
                continue;
            }
            _ => {}
        }
        if spaced && compound.parts > 0 {
            compound = Compound::default();
        }
        spaced = false;
        compound.parts += 1;
        match token {
            Token::Ident(tag) if compound.parts == 1 => {
                compound.tag = Some(tag.to_ascii_lowercase());
            }
            Token::Delim('*') if compound.parts == 1 => {}
            Token::IDHash(id) => {
                compound.id.get_or_insert_with(|| id.to_ascii_lowercase());
            }
            Token::Delim('.') => match parser.next_including_whitespace() {
                Ok(Token::Ident(class)) => {
                    let class = class.to_ascii_lowercase();
                    compound.class.get_or_insert(class);
                }
                _ => exact = false,
            },
            Token::SquareBracketBlock => match attribute_name(&mut parser) {
                Some(name) => {
                    compound.attribute.get_or_insert(name);
                }
                None => exact = false,
            },
            Token::Colon => exact &= pseudo_class(&mut parser),
            _ => exact = false,
        }
    }
    keys.push(key(compound, exact));
    keys
}

/// The name of the attribute the block `parser` just entered selects by,
/// folded; none when the block names a namespace or holds no name first.
fn attribute_name(parser: &mut Parser<'_, '_>) -> Option<String> {
    parser
        .parse_nested_block(|block| -> Result<_, ParseError<'_, ()>> {
            let name = match block.next() {
                Ok(Token::Ident(name)) => name.to_ascii_lowercase(),
                _ => return Ok(None),
            };
            let namespaced = matches!(block.next_including_whitespace(), Ok(Token::Delim('|')));
            // The block is read to its end, as its parser asks.
            while block.next().is_ok() {}
            Ok((!namespaced).then_some(name))
        })
        .ok()
        .flatten()
}

/// Whether what follows the colon `parser` just read is a pseudo-class,
/// which narrows the compound it is in and keys nothing; a pseudo-element
/// is none, in either of its spellings.
fn pseudo_class(parser: &mut Parser<'_, '_>) -> bool {
    match parser.next_including_whitespace() {
        Ok(Token::Ident(name)) => !["before", "after", "first-line", "first-letter"]
            .iter()
            .any(|element| name.eq_ignore_ascii_case(element)),
        // Its arguments are skipped with its block.
        Ok(Token::Function(_)) => true,
        _ => false,
    }
}

#[cfg(test)]
mod tests;
