//! A test tool manifest's input record, `<manifest>.inputs.json` beside it:
//! the workspace's folder and the size, modification time and content
//! digest of each source its tools were built from, which
//! `goodfetch-test-support` compares with
//! the tree before a test runs a tool, so tools built before a merge or from
//! another tree are refused (issue 1631).
//!
//! The sources are what the dep-info file Cargo writes beside each
//! executable names inside the workspace: Rust sources, included files, and
//! what build scripts watch, a watched folder with every file under it.
//! With them go `Cargo.lock`, the toolchain and Cargo configuration files
//! and the manifests of the packages the sources are in. Git's own files and
//! the target folder are left out: a commit or another worktree's fetch
//! changes no source, and a build writes the target folder.
//! `goodfetch-test-support`'s `tools()` is the record's reader, and
//! `xtask/tests/it/test_tools.rs` holds the two to one format.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::child_cargo;

/// Cargo's lockfile, at the workspace's top.
const LOCKFILE: &str = "Cargo.lock";

/// The files at the workspace's top that every build reads.
const BUILD_FILES: [&str; 4] = [
    "Cargo.toml",
    LOCKFILE,
    "rust-toolchain.toml",
    ".cargo/config.toml",
];

/// The workspace a record is of.
pub(super) struct Workspace<'a> {
    /// Its folder, an absolute path: Cargo names sources from it.
    pub(super) folder: &'a Path,
    /// Its target folder, as Cargo resolves it.
    pub(super) target_directory: &'a Path,
    /// Each member's manifest.
    pub(super) package_manifests: &'a [PathBuf],
}

/// What a manifest's tools were built from.
#[derive(Serialize)]
pub(super) struct InputRecord {
    /// The workspace's folder.
    root: PathBuf,
    /// Each source of the tools, as it was when they were built.
    inputs: Vec<Input>,
}

/// One source: its path from the workspace's folder, its parts joined
/// with `/` on every OS, its size in bytes, its modification time in
/// nanoseconds since the Unix epoch and the SHA-256 of its content in
/// lowercase hex, none for a folder. The reader compares the digest only
/// when the size matches and the time does not: a check that writes a file
/// back with its own bytes moves its time alone (issue 3236). A list, not a
/// map, since every test process that uses a tool parses each one.
#[derive(Serialize)]
struct Input(String, u64, u64, Option<String>);

impl InputRecord {
    /// The record of `executables`, which a Cargo build that began at
    /// `started` made in `workspace`: the sources each one's dep-info file
    /// names, as they are now.
    ///
    /// # Errors
    ///
    /// An executable without a dep-info file, or whose file names no source
    /// in the workspace: a record without its sources would pass tools of
    /// any age. A source but the lockfile modified after `started`: Cargo
    /// may have compiled it as it was before, and the record would call the
    /// tools built from it as it is now. Cargo itself writes the lockfile
    /// when a build resolves anew, and builds from what it wrote.
    pub(super) fn of(
        workspace: &Workspace<'_>,
        executables: &[&Path],
        started: SystemTime,
    ) -> Result<Self> {
        let folder = workspace.folder;
        let mut sources = BTreeSet::new();
        for executable in executables {
            let dep_info = executable.with_extension("d");
            let text = std::fs::read_to_string(&dep_info)
                .with_context(|| format!("reading {}", dep_info.display()))?;
            let named: Vec<PathBuf> = child_cargo::dep_info_paths(&text)
                .into_iter()
                .map(PathBuf::from)
                .filter(|path| !path.starts_with(workspace.target_directory))
                .filter(|path| !path.components().any(|part| part.as_os_str() == ".git"))
                .filter_map(|path| path.strip_prefix(folder).map(Path::to_owned).ok())
                .collect();
            if named.is_empty() {
                bail!(
                    "{} names no source of {} inside {}",
                    dep_info.display(),
                    executable.display(),
                    folder.display()
                );
            }
            for source in named {
                under(folder, &source, &mut sources)?;
                sources.insert(source);
            }
        }
        let manifests: Vec<PathBuf> = workspace
            .package_manifests
            .iter()
            .filter_map(|manifest| manifest.strip_prefix(folder).ok())
            .filter(|manifest| {
                manifest
                    .parent()
                    .is_some_and(|package| sources.iter().any(|source| source.starts_with(package)))
            })
            .map(Path::to_owned)
            .collect();
        sources.extend(manifests);
        if !sources.is_empty() {
            sources.extend(
                BUILD_FILES
                    .iter()
                    .map(PathBuf::from)
                    .filter(|file| folder.join(file).exists()),
            );
        }
        let inputs = sources
            .into_iter()
            .map(|source| stamp(folder, &source, started))
            .collect::<Result<_>>()?;
        Ok(Self {
            root: folder.to_owned(),
            inputs,
        })
    }

    /// Writes the record beside the manifest at `manifest`.
    pub(super) fn write_beside(&self, manifest: &Path) -> Result<()> {
        let out = manifest.with_extension("inputs.json");
        let mut text = serde_json::to_string(self)?;
        text.push('\n');
        std::fs::write(&out, text).with_context(|| format!("writing {}", out.display()))
    }
}

/// Adds to `sources` every file and folder under `source`, a path from
/// `folder`, when it is a folder: a build script that watches one reads the
/// files in it, and a file changed in place leaves the folder's time as it
/// was.
fn under(folder: &Path, source: &Path, sources: &mut BTreeSet<PathBuf>) -> Result<()> {
    let path = folder.join(source);
    if !path.is_dir() {
        return Ok(());
    }
    for entry in std::fs::read_dir(&path).with_context(|| format!("reading {}", path.display()))? {
        let entry = entry.with_context(|| format!("reading {}", path.display()))?;
        let inside = source.join(entry.file_name());
        under(folder, &inside, sources)?;
        sources.insert(inside);
    }
    Ok(())
}

/// `source`, a path from `folder`, with its size, modification time and
/// digest; a folder with no size, since Linux and Windows count a folder's
/// size each their own way, and Windows' reading of a record made on Linux
/// would find every watched folder changed (issue 2767), and with no
/// digest.
///
/// A file's content is read before its time, so the digest recorded is
/// never of content newer than the time beside it: a write between the two
/// shows as a time after `started`, which fails for every source but the
/// lockfile.
fn stamp(folder: &Path, source: &Path, started: SystemTime) -> Result<Input> {
    let path = folder.join(source);
    let digest = if path.is_dir() {
        None
    } else {
        let content =
            std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        Some(format!("{:x}", Sha256::digest(&content)))
    };
    let metadata =
        std::fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
    let modified = metadata
        .modified()
        .with_context(|| format!("reading {}'s modification time", path.display()))?;
    if modified > started && source != Path::new(LOCKFILE) {
        bail!(
            "{} changed while the test tools were building: build them again",
            source.display()
        );
    }
    let nanoseconds = modified
        .duration_since(UNIX_EPOCH)
        .ok()
        .and_then(|since| u64::try_from(since.as_nanos()).ok())
        .with_context(|| format!("{}'s modification time is out of range", path.display()))?;
    let size = if metadata.is_dir() { 0 } else { metadata.len() };
    Ok(Input(slashed(source), size, nanoseconds, digest))
}

/// `source`'s parts joined with `/`, which Windows reads as a separator
/// too: a record names each source one way on every OS, where Windows
/// joins a path's parts with `\`.
fn slashed(source: &Path) -> String {
    source
        .components()
        .map(|part| part.as_os_str().to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}

#[cfg(test)]
mod tests;
