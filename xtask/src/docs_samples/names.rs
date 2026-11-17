//! The names check (ADR 0097): no code block of a Product's rendering of a
//! docs page carries the other Product's names, `console` blocks and MCP
//! pairs included, except in a part marked as naming that Product. A source
//! page that writes `goodfetch-collie` instead of its marker thus fails in
//! Foxhound's rendering.

use crate::check::Finding;
use crate::products::Product;

use super::blocks::Block;

/// The finding kind of a name of the other Product.
const KIND: &str = "docs-names";

/// A finding for each line of `blocks`, of `rendering`'s rendering of
/// `page`, that carries a name of the other Product.
pub(super) fn check(page: &str, rendering: Product, blocks: &[Block]) -> Vec<Finding> {
    let other = rendering.other();
    blocks
        .iter()
        .filter(|block| block.names != Some(other))
        .flat_map(|block| &block.body)
        .filter_map(|line| {
            let carried: Vec<&str> = other
                .names()
                .by_marker()
                .into_iter()
                .map(|(_, name)| name)
                .filter(|name| carries(&line.text, name))
                .collect();
            (!carried.is_empty()).then(|| {
                Finding::new(
                    KIND,
                    format!("{}:{page}:{}", rendering.key(), line.source),
                    format!(
                        "{:?}'s rendering carries {:?}'s {}; write the name's marker, or mark the part as naming {:?}",
                        rendering,
                        other,
                        carried
                            .iter()
                            .map(|name| format!("`{name}`"))
                            .collect::<Vec<_>>()
                            .join(", "),
                        other
                    ),
                )
            })
        })
        .collect()
}

/// Whether `text` holds `name` as a whole name: neither a letter, a digit
/// nor `_` touches it, so `goodfetch_foxhound` is not in
/// `goodfetch_foxhound_extra`, but `@goodfetch/collie` is in
/// `@goodfetch/collie-client`, Collie's too.
fn carries(text: &str, name: &str) -> bool {
    let word = |character: Option<char>| {
        character.is_some_and(|character| character.is_alphanumeric() || character == '_')
    };
    text.match_indices(name).any(|(start, _)| {
        !word(text[..start].chars().next_back()) && !word(text[start + name.len()..].chars().next())
    })
}
