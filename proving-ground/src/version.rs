//! The Proving ground's version: the workspace version plus the commit it is
//! built from (ADR 0063), recorded by the build script.

/// `<workspace version>+<commit>`.
pub(crate) const VERSION: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    "+",
    env!("GOODFETCH_PROVING_GROUND_COMMIT")
);
