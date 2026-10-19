//! What each generator gives the inventory: one Feature as its source names
//! it, before the inventory gives it its area and Products.

use anyhow::anyhow;

use super::files::{Kind, Status, Unreadable};

pub(super) struct Entry {
    pub(super) kind: Kind,
    /// Its ID after `<kind>:`.
    pub(super) name: String,
    /// Its area, for a declared Feature; a generated one takes its area
    /// from `features/areas.toml`.
    pub(super) area: Option<u32>,
    pub(super) milestone: Option<String>,
    pub(super) status: Status,
    /// The Products that carry it.
    pub(super) carriers: Carriers,
}

/// Which Products carry a Feature (ADR 0094).
#[derive(Clone, Copy)]
pub(super) enum Carriers {
    /// Collie and Foxhound, which contains Collie.
    Both,
    /// Foxhound alone.
    Foxhound,
    /// Foxhound alone when ADR 0087's generated table of Foxhound's names
    /// lists it under this section, both otherwise.
    Table(&'static str),
}

/// A file a generator could not read, as the error `xtask codegen` stops on.
pub(super) fn unreadable(error: Unreadable, path: &str) -> anyhow::Error {
    match error {
        Unreadable::Io(error) => error,
        Unreadable::Schema(message) => anyhow!("{path} breaks its schema: {message}"),
    }
}
