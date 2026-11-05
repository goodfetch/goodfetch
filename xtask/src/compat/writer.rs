//! Each format's writer: the `compat-<format>` example of the workspace
//! package that owns the format, which runs the format's fixed scenarios
//! through its current writers, with ADR 0059's fixed `Entropy` seed and test
//! `Clock` start, into the folder given as its only argument (ADR 0066).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::Command;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::registry::Format;
use super::tree::{self, Tree};
use crate::metadata;

/// The file `xtask compat` adds to each set, which no writer may write.
pub(super) const DESCRIPTION: &str = "format.json";

/// The writers the workspace has, by the example's name.
pub(super) struct Writers {
    /// The workspace's `compat-<format>` examples.
    examples: BTreeSet<String>,
}

impl Writers {
    pub(super) fn find(cargo: &OsStr, root: &Path) -> Result<Self> {
        let mut examples = BTreeSet::new();
        for package in metadata::packages(cargo, root)? {
            for target in package.targets {
                if target.kind.iter().any(|kind| kind == "example")
                    && target.name.starts_with("compat-")
                {
                    examples.insert(target.name);
                }
            }
        }
        Ok(Self { examples })
    }

    pub(super) fn has(&self, format: &Format) -> bool {
        self.examples.contains(&example(format))
    }

    /// The `head/` set each of `formats`' writers produces now, with the
    /// format's description added, in `formats`' order.
    ///
    /// The writers build in one `cargo build` over the workspace, so the
    /// crates they share build once instead of once per package, with the
    /// features the Quick suite's build of the workspace gives them, which
    /// it then reuses, and the examples link in parallel; then they all run
    /// at once. A failure reports the first failing format in order.
    pub(super) fn write(
        &self,
        cargo: &OsStr,
        root: &Path,
        formats: &[&Format],
    ) -> Result<Vec<Tree>> {
        let mut examples = Vec::new();
        for format in formats {
            let example = example(format);
            if !self.examples.contains(&example) {
                bail!(
                    "`{}` has no writer: add a `{example}` example to the package that owns it",
                    format.name
                );
            }
            examples.push(example);
        }
        if formats.is_empty() {
            return Ok(Vec::new());
        }
        let executables = build(cargo, root, &examples)?;
        std::thread::scope(|scope| {
            let runs: Vec<_> = formats
                .iter()
                .zip(&examples)
                .map(|(format, example)| {
                    let executable = &executables[example];
                    scope.spawn(move || run(root, format, example, executable))
                })
                .collect();
            runs.into_iter()
                .map(|handle| {
                    handle
                        .join()
                        .expect("invariant: a writer's thread does not panic")
                })
                .collect()
        })
    }
}

/// Cargo's JSON message for a built target, of which `build` reads the
/// examples' executables.
#[derive(Deserialize)]
struct Artifact {
    reason: String,
    target: Option<ArtifactTarget>,
    executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct ArtifactTarget {
    name: String,
    kind: Vec<String>,
}

/// Builds `examples` in one `cargo build` over the workspace, and gives
/// each example's executable.
fn build(cargo: &OsStr, root: &Path, examples: &[String]) -> Result<BTreeMap<String, PathBuf>> {
    let output = crate::child_cargo::command(cargo)
        .args([
            "build",
            "--quiet",
            "--message-format",
            "json-render-diagnostics",
            "--workspace",
        ])
        .args(
            examples
                .iter()
                .flat_map(|example| ["--example", example.as_str()]),
        )
        .current_dir(root)
        .output()
        .context("building the writers")?;
    if !output.status.success() {
        bail!(
            "building the writers ({}) failed: {}",
            examples.join(", "),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let mut executables = BTreeMap::new();
    for line in String::from_utf8_lossy(&output.stdout).lines() {
        let Ok(artifact) = serde_json::from_str::<Artifact>(line) else {
            continue;
        };
        let (Some(target), Some(executable)) = (artifact.target, artifact.executable) else {
            continue;
        };
        if artifact.reason == "compiler-artifact"
            && target.kind.iter().any(|kind| kind == "example")
        {
            executables.insert(target.name, executable);
        }
    }
    for example in examples {
        if !executables.contains_key(example) {
            bail!("cargo build reported no executable for {example}");
        }
    }
    Ok(executables)
}

/// The set `format`'s writer, built at `executable`, writes into a new
/// folder, with the format's description added.
fn run(root: &Path, format: &Format, example: &str, executable: &Path) -> Result<Tree> {
    let folder = tempfile::tempdir().context("creating a folder for the writer")?;
    let output = Command::new(executable)
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

pub(super) fn example(format: &Format) -> String {
    format!("compat-{}", format.name)
}
