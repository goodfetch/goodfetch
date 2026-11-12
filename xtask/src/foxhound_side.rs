//! Foxhound's side of the repository (ADR 0094): the paths whose changes reach
//! Foxhound only. `xtask affected` selects only Foxhound's tests for a change
//! confined to them, and the same paths group `CHANGELOG.md`'s entries.

/// `crates/foxhound/` (the sink processes included, ADR 0087) and Foxhound's
/// own SDK files.
const FOLDERS: [&str; 3] = [
    "crates/foxhound/",
    "sdks/typescript/foxhound/",
    "sdks/python/foxhound/",
];

/// Whether `path`, relative to the repository root with `/` separators, is on
/// Foxhound's side.
pub(crate) fn contains(path: &str) -> bool {
    FOLDERS.iter().any(|folder| path.starts_with(folder))
}
