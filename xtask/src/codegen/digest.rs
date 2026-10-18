//! The digests of each outside generator's input and output, recorded when
//! `xtask codegen` runs it, so `--check` judges its output without running
//! it: a different input means the types changed and the output was not
//! regenerated; a different output means someone edited it.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest as _, Sha256};

#[derive(Serialize, Deserialize)]
struct Record {
    input: String,
    output: String,
}

/// The recorded digests, by output path.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(super) struct Digests(BTreeMap<String, Record>);

pub(super) enum Verdict {
    /// Generated from today's input and unchanged since.
    Current,
    /// Generated from another input, or never.
    Stale,
    /// Changed after its generator wrote it, or gone.
    Edited,
}

impl Digests {
    /// The committed digests; none when the file does not exist yet.
    pub(super) fn read(path: &Path) -> Result<Self> {
        match std::fs::read(path) {
            Ok(bytes) => serde_json::from_slice(&bytes)
                .with_context(|| format!("reading {}", path.display())),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
        }
    }

    pub(super) fn record(&mut self, output_path: &str, input: &[u8], output: &[u8]) {
        self.0.insert(
            output_path.to_owned(),
            Record {
                input: sha256(input),
                output: sha256(output),
            },
        );
    }

    pub(super) fn verify(&self, output_path: &str, input: &[u8], output: Option<&[u8]>) -> Verdict {
        let Some(record) = self.0.get(output_path) else {
            return Verdict::Stale;
        };
        if record.input != sha256(input) {
            return Verdict::Stale;
        }
        match output {
            Some(output) if record.output == sha256(output) => Verdict::Current,
            _ => Verdict::Edited,
        }
    }

    pub(super) fn to_json(&self) -> Result<Value> {
        Ok(serde_json::to_value(self)?)
    }
}

fn sha256(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}
