//! Declared inputs: what no package manager lists, declared in a
//! `third-party.toml` file beside what uses it (the
//! Goodfetch Chromium's third-party inputs, the Clean-up rules' inputs
//! and the block-detection rules' sources), or in a further file given on the command line, such as the list
//! a Chromium build writes.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use serde::Deserialize;

use super::entry::{Ecosystem, Entry, Usage};
use crate::tree;

/// The name of every file of declared inputs in the repository.
const FILE_NAME: &str = "third-party.toml";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    set: Set,
    #[serde(default)]
    input: Vec<Input>,
}

/// The input sets of ADRs 0018, 0035, 0043 and 0047.
#[derive(Deserialize, Clone, Copy)]
#[serde(rename_all = "kebab-case")]
enum Set {
    Chromium,
    CleanUpRules,
    BlockDetectionRules,
}

impl From<Set> for Ecosystem {
    fn from(set: Set) -> Self {
        match set {
            Set::Chromium => Self::Chromium,
            Set::CleanUpRules => Self::CleanUpRules,
            Set::BlockDetectionRules => Self::BlockDetectionRules,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Input {
    name: String,
    /// Its release, or the commit it is pinned to.
    version: String,
    /// The SPDX license expression its authors state; without one the
    /// check refuses it.
    license: Option<String>,
    /// Where its source is; without it the check refuses it.
    source: Option<String>,
    #[serde(rename = "use")]
    usage: Usage,
}

/// The inputs of every `third-party.toml` file of the repository, then of
/// each of `further`.
pub(super) fn entries(root: &Path, further: &[PathBuf]) -> Result<Vec<Entry>> {
    let declared = tree::files(root, "")?
        .into_iter()
        .filter(|file| file.rsplit('/').next() == Some(FILE_NAME))
        .map(|file| root.join(file));
    let mut entries = Vec::new();
    for path in declared.chain(further.iter().cloned()) {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let file: File =
            toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        entries.extend(file.input.into_iter().map(|input| Entry {
            ecosystem: file.set.into(),
            name: input.name,
            version: input.version,
            license: input.license,
            source: input.source,
            usage: input.usage,
        }));
    }
    Ok(entries)
}
