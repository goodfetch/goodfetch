//! The counted figures a job saves for the results store (ADR 0071):
//! `perf-figures.json`, written into the job's JUnit folder so the ingest
//! keeps it with the JUnit files (ADR 0064), and read back by the drift
//! check. A figure with a ceiling, such as a resident-memory peak from the
//! Release suite's check, carries it.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};

use super::compare::Row;
use super::figure::Figure;

/// The file's name inside a JUnit artifact.
pub(crate) const FILE_NAME: &str = "perf-figures.json";

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Figures {
    pub(crate) figures: Vec<Recorded>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(deny_unknown_fields)]
pub(crate) struct Recorded {
    pub(crate) subject: String,
    pub(crate) figure: Figure,
    pub(crate) value: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) ceiling: Option<u64>,
}

impl Recorded {
    pub(crate) fn id(&self) -> String {
        format!("{} {}", self.subject, self.figure)
    }
}

/// Saves the head's figures of `rows` at `path`.
pub(crate) fn save(path: &Path, rows: &[Row]) -> Result<()> {
    let figures = Figures {
        figures: rows
            .iter()
            .map(|row| Recorded {
                subject: row.measured.subject.clone(),
                figure: row.measured.figure,
                value: row.measured.head,
                ceiling: row.limit.ceiling,
            })
            .collect(),
    };
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(path, serde_json::to_vec_pretty(&figures)?)
        .with_context(|| format!("writing {}", path.display()))
}

pub(crate) fn parse(bytes: &[u8]) -> Result<Figures> {
    serde_json::from_slice(bytes).context("parsing a saved perf-figures.json")
}
