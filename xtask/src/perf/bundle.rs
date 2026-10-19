//! size-limit's reports (`size-limit --json`): an array of entries, each
//! with its `name` and its brotli `size` in bytes, measured once on the base
//! and once on the head (ADR 0071).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use super::figure::{Figure, Measured};

#[derive(Deserialize)]
struct Entry {
    name: String,
    size: u64,
}

/// An entry's subject in the table and in findings.
pub(crate) fn subject(name: &str) -> String {
    format!("size-limit/{name}")
}

fn entries(path: &Path) -> Result<BTreeMap<String, u64>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let entries: Vec<Entry> =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(entries
        .into_iter()
        .map(|entry| (entry.name, entry.size))
        .collect())
}

/// The subjects of the entries of the report at `path`.
pub(crate) fn entry_subjects(path: &Path) -> Result<BTreeSet<String>> {
    Ok(entries(path)?.keys().map(|name| subject(name)).collect())
}

/// Every head entry, with the base's size of the entry of the same name;
/// with no base report, no entry has a base.
pub(crate) fn read(base: Option<&Path>, head: &Path) -> Result<Vec<Measured>> {
    let base = base.map(entries).transpose()?.unwrap_or_default();
    Ok(entries(head)?
        .into_iter()
        .map(|(name, size)| Measured {
            subject: subject(&name),
            figure: Figure::Brotli,
            base: base.get(&name).copied(),
            head: size,
        })
        .collect())
}
