//! ADR 0030's run-time warning for a deprecated or Experimental catalog
//! entry (ADR 0099): each generated member of one calls it, each
//! hand-written member serving one reads its markers from the catalog, and
//! it warns once per process per name and marker, through `tracing`, the
//! host's channel (ADR 0078). The type-level markers, `#[deprecated]` and
//! the rustdoc line, are the generated member's own.

use std::collections::BTreeSet;
use std::sync::PoisonError;

use goodfetch_protocol::catalog::Stability;
use goodfetch_protocol::entries::MODULES;

use crate::logging;
use crate::sync::Mutex;

/// Which of an entry's markers a warning is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Marker {
    Deprecated,
    Experimental,
}

/// The names this process has warned of, with the marker each was for.
static WARNED: Mutex<BTreeSet<(Marker, &'static str)>> = Mutex::new(BTreeSet::new());

/// Warns, the first time in the process for each marker, that the entry
/// `name` is Experimental, when `experimental` says so, and that it is
/// deprecated, when it has a `deprecation`: its deprecation's one text,
/// `Since <version>: <note>`, which the TypeScript and Python SDKs' markers
/// carry too.
pub(crate) fn warn(name: &'static str, experimental: bool, deprecation: Option<&str>) {
    if experimental && first(Marker::Experimental, name) {
        tracing::warn!(
            target: logging::CLIENT,
            entry = name,
            "`{name}` is Experimental: outside semver until it is promoted (ADR 0030)."
        );
    }
    if let Some(text) = deprecation
        && first(Marker::Deprecated, name)
    {
        tracing::warn!(target: logging::CLIENT, entry = name, "`{name}` is deprecated. {text}");
    }
}

/// Warns, the first time in the process, of each marker the catalog gives
/// the operation `name`: what a hand-written member serving it calls
/// first, as a generated member calls [`warn`].
pub(crate) fn warn_operation(name: &'static str) {
    let operation = MODULES
        .into_iter()
        .flat_map(|entries| entries.operations)
        .find(|operation| operation.name == name)
        .expect("invariant: a hand-written member serves a catalog operation");
    warn(
        name,
        operation.stability == Stability::Experimental,
        operation
            .deprecated
            .map(|deprecated| deprecated.message())
            .as_deref(),
    );
}

/// Whether this is the first warning of `marker` for `name`.
fn first(marker: Marker, name: &'static str) -> bool {
    WARNED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert((marker, name))
}

#[cfg(test)]
mod tests;
