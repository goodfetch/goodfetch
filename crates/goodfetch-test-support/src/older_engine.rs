//! The older engine of a skew job (ADR 0066): the executable of a released
//! minor that opens what the current build wrote.

use std::path::{Path, PathBuf};

use crate::variables::{OLDER_ENGINE_VARIABLE, Variables};

/// The older engine a test opens the current build's files with: the one
/// `GOODFETCH_TEST_OLDER_ENGINE` names when it is set, as a skew job of a
/// released minor sets it, else `built`, the commit's own executable of the
/// same Product, so the tests run with no release too.
pub fn older_engine(built: impl AsRef<Path>) -> PathBuf {
    named(&Variables::from_process(), built.as_ref())
}

fn named(variables: &Variables, built: &Path) -> PathBuf {
    variables
        .get(OLDER_ENGINE_VARIABLE)
        .map_or_else(|| built.to_owned(), PathBuf::from)
}

#[cfg(test)]
mod tests;
