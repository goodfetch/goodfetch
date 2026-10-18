//! Each format's writer: the `compat-<format>` example of the workspace
//! package that owns the format, which runs the format's fixed scenarios
//! through its current writers, with ADR 0059's fixed `Entropy` seed and test
//! `Clock` start, into the folder given as its only argument (ADR 0066).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::path::Path;
use std::process::Command;

use anyhow::{Context as _, Result, bail};

use super::registry::Format;
use super::tree::{self, Tree};
use crate::metadata;

/// The file `xtask compat` adds to each set, which no writer may write.
pub(super) const DESCRIPTION: &str = "format.json";

/// The writers the workspace has, by the example's name.
pub(super) struct Writers {
    /// The package of each `compat-<format>` example.
    packages: BTreeMap<String, String>,
}

impl Writers {
    pub(super) fn find(cargo: &OsStr, root: &Path) -> Result<Self> {
        let mut packages = BTreeMap::new();
        for package in metadata::packages(cargo, root)? {
            for target in package.targets {
                if target.kind.iter().any(|kind| kind == "example")
                    && target.name.starts_with("compat-")
                {
                    packages.insert(target.name, package.name.clone());
                }
            }
        }
        Ok(Self { packages })
    }

    pub(super) fn has(&self, format: &Format) -> bool {
        self.packages.contains_key(&example(format))
    }

    /// The `head/` set the format's writer produces now, with the format's
    /// description added.
    pub(super) fn write(&self, cargo: &OsStr, root: &Path, format: &Format) -> Result<Tree> {
        let example = example(format);
        let Some(package) = self.packages.get(&example) else {
            bail!(
                "`{}` has no writer: add a `{example}` example to the package that owns it",
                format.name
            );
        };
        let folder = tempfile::tempdir().context("creating a folder for the writer")?;
        let output = Command::new(cargo)
            .args([
                "run",
                "--quiet",
                "--package",
                package,
                "--example",
                &example,
                "--",
            ])
            .arg(folder.path())
            .current_dir(root)
            .output()
            .with_context(|| format!("running {example}"))?;
        if !output.status.success() {
            bail!(
                "{example} failed: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        let mut set = tree::read(folder.path())?;
        if set.contains_key(DESCRIPTION) {
            bail!("{example} wrote {DESCRIPTION}, a name `xtask compat` keeps for itself");
        }
        let mut description = serde_json::to_string_pretty(&format.description())?;
        description.push('\n');
        set.insert(DESCRIPTION.to_owned(), description.into_bytes());
        Ok(set)
    }
}

pub(super) fn example(format: &Format) -> String {
    format!("compat-{}", format.name)
}
