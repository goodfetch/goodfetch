//! The peaks file: one JSON line per measured peak, its Product, platform,
//! scenario and bytes, and on Linux how many of them are the executable's
//! own file-backed pages. `measure` appends to it; `check` and `propose` read
//! it.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};
use goodfetch_protocol::catalog::Product;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::platform::Platform;
use super::scenario::Scenario;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Peak {
    pub(crate) product: Product,
    pub(crate) platform: Platform,
    pub(crate) scenario: Scenario,
    /// The peak resident memory, in bytes.
    pub(crate) bytes: u64,
    /// The executable's own file-backed resident pages, in bytes, the
    /// highest sample while it ran; absent on an OS without `/proc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) executable_bytes: Option<u64>,
}

pub(crate) fn read(path: &Path) -> Result<Vec<Peak>> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    text.lines()
        .enumerate()
        .filter(|(_, line)| !line.trim().is_empty())
        .map(|(index, line)| {
            serde_json::from_str(line)
                .with_context(|| format!("reading line {} of {}", index + 1, path.display()))
        })
        .collect()
}

pub(crate) fn append(path: &Path, peak: &Peak) -> Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    let mut line = serde_json::to_string(peak)?;
    line.push('\n');
    crate::output::append(path, &line)
}

/// A name as its type's serde form spells it, such as a TOML key.
pub(crate) fn parse_name<T: DeserializeOwned>(name: &str) -> Result<T> {
    serde_json::from_value(serde_json::Value::String(name.to_owned()))
        .with_context(|| format!("`{name}` is no name this file takes"))
}

/// A Product on the command line, by its mark.
pub(crate) fn parse_product(name: &str) -> Result<Product> {
    parse_name(name)
}
