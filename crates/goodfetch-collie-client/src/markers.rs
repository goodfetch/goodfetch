//! ADR 0030's run-time warning for a deprecated or Experimental catalog
//! entry (ADR 0099): each generated member of one calls it, and it warns
//! once per process per name and marker, through `tracing`, the host's
//! channel (ADR 0078). The type-level markers, `#[deprecated]` and the
//! rustdoc line, are the generated member's own.

use std::collections::BTreeSet;
use std::sync::PoisonError;

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
pub(crate) fn warn(name: &'static str, experimental: bool, deprecation: Option<&'static str>) {
    if experimental && first(Marker::Experimental, name) {
        tracing::warn!(
            entry = name,
            "`{name}` is Experimental: outside semver until it is promoted (ADR 0030)."
        );
    }
    if let Some(text) = deprecation
        && first(Marker::Deprecated, name)
    {
        tracing::warn!(entry = name, "`{name}` is deprecated. {text}");
    }
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
