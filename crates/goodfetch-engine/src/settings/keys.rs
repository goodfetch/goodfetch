//! The keys of the settings schema, read from the catalog's entries with
//! the Product each one's mark gives (ADRs 0087 and 0095): finding a key,
//! telling a prefix of keys from an unknown name, and the closest key to a
//! misspelt one.

use goodfetch_protocol::catalog::{Product, Setting};
use goodfetch_protocol::entries::MODULES;

/// A key of the schema and the smallest Product that honours it.
#[derive(Clone, Copy)]
pub(crate) struct Key {
    pub(crate) setting: &'static Setting,
    pub(crate) product: Product,
}

/// Every key of either Product, in declaration order.
pub(crate) fn all() -> impl Iterator<Item = Key> {
    MODULES.into_iter().flat_map(|entries| {
        let product = Product::of_module(entries.defined_in);
        entries
            .settings
            .iter()
            .map(move |setting| Key { setting, product })
    })
}

/// The key named `name`.
pub(crate) fn find(name: &str) -> Option<Key> {
    all().find(|key| key.setting.name == name)
}

/// Whether `path` is the first segments of at least one key.
pub(crate) fn is_prefix(path: &str) -> bool {
    all().any(|key| {
        key.setting
            .name
            .strip_prefix(path)
            .is_some_and(|rest| rest.starts_with('.'))
    })
}

/// The key closest to `name`, when one is within a third of its length in
/// edits (at least two).
pub(crate) fn closest(name: &str) -> Option<&'static str> {
    nearest(name, all().map(|key| key.setting.name))
}

/// The name of `candidates` closest to `name`, when one is within a third of
/// its length in edits (at least two).
pub(crate) fn nearest(
    name: &str,
    candidates: impl IntoIterator<Item = &'static str>,
) -> Option<&'static str> {
    let allowed = (name.chars().count() / 3).max(2);
    candidates
        .into_iter()
        .map(|candidate| (distance(name, candidate), candidate))
        .filter(|(edits, _)| *edits <= allowed)
        .min()
        .map(|(_, candidate)| candidate)
}

/// Levenshtein's distance between `left` and `right`, by characters.
fn distance(left: &str, right: &str) -> usize {
    let right: Vec<char> = right.chars().collect();
    let mut previous: Vec<usize> = (0..=right.len()).collect();
    for (row, left) in left.chars().enumerate() {
        let mut current = vec![row + 1];
        for (column, right) in right.iter().enumerate() {
            let substitution = previous[column] + usize::from(left != *right);
            let insertion = current[column] + 1;
            let deletion = previous[column + 1] + 1;
            current.push(substitution.min(insertion).min(deletion));
        }
        previous = current;
    }
    previous[right.len()]
}
