//! The index removal files a page's plain CSS rules in (spec #2164), so
//! that an element is tested only against the rules it could match. A rule
//! is filed under one key of its rightmost compound, the one its subject
//! must match: its ID, else a class, else an attribute, else its tag, and a
//! rule with none of them, or one whose text this does not key exactly,
//! under no key, which every element is tested against. A selector list is
//! filed under each member's key.
//!
//! An attribute keys a rule by its name and, when the compound compares one
//! with a value, by that comparison (issue 2191): an element is tested
//! against it only when its attribute of that name holds the value, starts
//! or ends with it or holds it anywhere, as the operator asks. A word
//! (`~=`) is looked for anywhere and a dash match (`|=`) as a start, which
//! each admit every value the selector could match.
//!
//! The index only narrows: the match stays `Matcher`'s. Keys and values are
//! folded to ASCII lower case on both sides, which finds every rule a
//! case-sensitive or a case-insensitive match could accept, and never adds
//! a removal.

use std::collections::BTreeMap;

use cssparser::{ParseError, Parser, ParserInput, Token};
use dom_query::NodeRef;

/// What a rule is filed under.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Key {
    Id(String),
    Class(String),
    /// An attribute's name, and the value it is compared with, if any.
    Attribute(String, Option<Value>),
    Tag(String),
    /// No key: the rule is tested against every element.
    Universal,
}

/// The value an attribute selector compares its attribute with, folded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Value {
    pub(crate) operator: Operator,
    pub(crate) text: String,
}

/// How an attribute's value is compared with a rule's, as the index reads
/// it: each admits at least the values the selector's operator matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Operator {
    /// `=`.
    Equals,
    /// `^=`, and `|=`, whose value is the attribute's or starts it.
    Prefix,
    /// `$=`.
    Suffix,
    /// `*=`, and `~=`, whose value is one of the attribute's words.
    Contains,
}

impl Value {
    /// Whether an attribute holding `value`, folded, could match.
    fn admits(&self, value: &str) -> bool {
        let text = self.text.as_str();
        match self.operator {
            Operator::Equals => value == text,
            Operator::Prefix => value.starts_with(text),
            Operator::Suffix => value.ends_with(text),
            Operator::Contains => value.contains(text),
        }
    }
}

/// The rules of one page, by position in rule order, under their keys.
#[derive(Default)]
pub(crate) struct Index {
    ids: BTreeMap<String, Vec<usize>>,
    classes: BTreeMap<String, Vec<usize>>,
    attributes: BTreeMap<String, Attribute>,
    tags: BTreeMap<String, Vec<usize>>,
    universal: Vec<usize>,
}

/// The rules keyed by one attribute's name: those keyed by the name alone,
/// and those that compare it with a value, with their values.
#[derive(Default)]
struct Attribute {
    rules: Vec<usize>,
    compared: Vec<(Value, usize)>,
}

impl Index {
    /// The index of the rules whose selectors are `selectors`, in rule
    /// order.
    pub(crate) fn of<'a>(selectors: impl Iterator<Item = &'a str>) -> Self {
        let mut index = Self::default();
        for (position, selector) in selectors.enumerate() {
            for key in keys(selector) {
                let bucket = match key {
                    Key::Id(id) => index.ids.entry(id).or_default(),
                    Key::Class(class) => index.classes.entry(class).or_default(),
                    Key::Attribute(name, None) => {
                        &mut index.attributes.entry(name).or_default().rules
                    }
                    Key::Attribute(name, Some(value)) => {
                        // Two members of one list may share a key; the
                        // candidates are deduplicated.
                        let attribute = index.attributes.entry(name).or_default();
                        attribute.compared.push((value, position));
                        continue;
                    }
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

    /// About how many bytes it holds.
    pub(crate) fn bytes(&self) -> usize {
        let buckets = [&self.ids, &self.classes, &self.tags];
        let filed: usize = buckets
            .into_iter()
            .flat_map(BTreeMap::iter)
            .map(|(key, rules)| {
                size_of::<(String, Vec<usize>)>()
                    + key.capacity()
                    + rules.capacity() * size_of::<usize>()
            })
            .sum();
        let attributes: usize = self
            .attributes
            .iter()
            .map(|(name, attribute)| {
                size_of::<(String, Attribute)>()
                    + name.capacity()
                    + attribute.rules.capacity() * size_of::<usize>()
                    + attribute.compared.capacity() * size_of::<(Value, usize)>()
                    + attribute
                        .compared
                        .iter()
                        .map(|(value, _)| value.text.capacity())
                        .sum::<usize>()
            })
            .sum();
        size_of::<Self>() + filed + attributes + self.universal.capacity() * size_of::<usize>()
    }

    /// Into `candidates`, the positions of the rules `node` could match, in
    /// rule order, each once: those filed under its ID, its classes, its
    /// attributes' names, the values its attributes could match and its
    /// tag, and those under no key.
    pub(crate) fn candidates(&self, node: &NodeRef<'_>, candidates: &mut Vec<usize>) {
        candidates.clear();
        candidates.extend_from_slice(&self.universal);
        node.query(|node| {
            let Some(element) = node.as_element() else {
                return;
            };
            filed(&self.tags, &element.name.local, candidates);
            for attribute in &element.attrs {
                let name: &str = &attribute.name.local;
                attributed(&self.attributes, (name, &attribute.value), candidates);
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

/// Adds to `candidates` the rules `bucket` files under the attribute
/// `name`: those keyed by the name alone, and those whose value `value`
/// could match, folded as the keys are.
fn attributed(
    bucket: &BTreeMap<String, Attribute>,
    (name, value): (&str, &str),
    candidates: &mut Vec<usize>,
) {
    if bucket.is_empty() {
        return;
    }
    let attribute = if name.bytes().any(|byte| byte.is_ascii_uppercase()) {
        bucket.get(&name.to_ascii_lowercase())
    } else {
        bucket.get(name)
    };
    let Some(attribute) = attribute else {
        return;
    };
    candidates.extend_from_slice(&attribute.rules);
    if attribute.compared.is_empty() {
        return;
    }
    let folded;
    let value = if value.bytes().any(|byte| byte.is_ascii_uppercase()) {
        folded = value.to_ascii_lowercase();
        folded.as_str()
    } else {
        value
    };
    candidates.extend(
        attribute
            .compared
            .iter()
            .filter(|(compared, _)| compared.admits(value))
            .map(|(_, at)| *at),
    );
}

/// The parts of a compound selector that key it.
#[derive(Default)]
struct Compound {
    id: Option<String>,
    class: Option<String>,
    /// Its first attribute compared with a value, else its first one.
    attribute: Option<(String, Option<Value>)>,
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
        } else if let Some((name, value)) = self.attribute {
            Key::Attribute(name, value)
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
pub(crate) fn keys(selector: &str) -> Vec<Key> {
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
            Token::SquareBracketBlock => match attribute(&mut parser) {
                Some(attribute) => {
                    if compound
                        .attribute
                        .as_ref()
                        .is_none_or(|(_, value)| value.is_none())
                    {
                        compound.attribute = Some(attribute);
                    }
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

/// The attribute the block `parser` just entered selects by: its name,
/// folded, and the value it is compared with, folded, when the block reads
/// as one; none when the block names a namespace or holds no name first.
fn attribute(parser: &mut Parser<'_, '_>) -> Option<(String, Option<Value>)> {
    parser
        .parse_nested_block(|block| -> Result<_, ParseError<'_, ()>> {
            let name = match block.next() {
                Ok(Token::Ident(name)) => name.to_ascii_lowercase(),
                _ => return Ok(None),
            };
            let next = block.next_including_whitespace().cloned();
            if matches!(next, Ok(Token::Delim('|'))) {
                while block.next().is_ok() {}
                return Ok(None);
            }
            let next = match next {
                Ok(Token::WhiteSpace(_)) => block.next().cloned(),
                next => next,
            };
            let operator = match next {
                Ok(Token::Delim('=')) => Some(Operator::Equals),
                Ok(Token::PrefixMatch | Token::DashMatch) => Some(Operator::Prefix),
                Ok(Token::SuffixMatch) => Some(Operator::Suffix),
                Ok(Token::SubstringMatch | Token::IncludeMatch) => Some(Operator::Contains),
                _ => None,
            };
            let value = operator.and_then(|operator| match block.next() {
                Ok(Token::Ident(text) | Token::QuotedString(text)) => Some(Value {
                    operator,
                    text: text.to_ascii_lowercase(),
                }),
                _ => None,
            });
            // The block is read to its end, as its parser asks.
            while block.next().is_ok() {}
            Ok(Some((name, value)))
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
