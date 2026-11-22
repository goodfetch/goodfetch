//! The test tool manifest (ADRs 0062 and 0094): the JSON file, written by
//! `just test` after it builds them, that holds the absolute paths of the
//! commit's executables a test runs. `GOODFETCH_TEST_TOOLS` names it. A
//! test takes Collie's executable unless it needs Foxhound, and one that
//! measures an executable takes each.
//!
//! Beside the manifest, `xtask test-tools build` writes its input record,
//! `<manifest>.inputs.json`: the tree the tools were built from and the
//! size, modification time and content digest of each source Cargo
//! compiled into them.
//! Reading the manifest compares the record with the tree, so tools built
//! before a merge, a `git switch` or an edit, or from another tree, are
//! refused at their first use rather than run. A manifest with no record,
//! as a test writes for stand-in programs, describes no build and is read
//! as it is.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads the manifest outside any async worker (ADR 0062)"
)]

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::UNIX_EPOCH;

use data_encoding::HEXLOWER;
use sha2::{Digest as _, Sha256};

use crate::variables::{TOOLS_VARIABLE, Variables};

/// The commit's executables, each present when its crate exists in the
/// workspace. Tests of a Product's executable crate use its
/// `CARGO_BIN_EXE_` path instead.
#[derive(
    Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize, schemars::JsonSchema,
)]
#[serde(deny_unknown_fields)]
pub struct Tools {
    /// `goodfetch-collie`, Collie's executable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PathBuf")]
    pub collie: Option<PathBuf>,
    /// `goodfetch-foxhound`, Foxhound's executable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PathBuf")]
    pub foxhound: Option<PathBuf>,
    /// The Proving ground's executable.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PathBuf")]
    pub proving_ground: Option<PathBuf>,
    /// `goodfetch-test-env`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PathBuf")]
    pub test_env: Option<PathBuf>,
    /// `goodfetch-scripted-engine`, the Scripted engine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PathBuf")]
    pub scripted_engine: Option<PathBuf>,
    /// `goodfetch-conformance`, whose `read` loads a result document
    /// through `goodfetch-protocol`'s reader.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PathBuf")]
    pub conformance: Option<PathBuf>,
    /// `goodfetch-stub-gateway`, the Stub gateway the Cloud-readiness
    /// conformance suite runs against (ADR 0096).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "PathBuf")]
    pub stub_gateway: Option<PathBuf>,
}

/// One executable of the manifest.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tool {
    /// `goodfetch-collie`.
    Collie,
    /// `goodfetch-foxhound`.
    Foxhound,
    /// The Proving ground.
    ProvingGround,
    /// `goodfetch-test-env`.
    TestEnv,
    /// `goodfetch-scripted-engine`.
    ScriptedEngine,
    /// `goodfetch-conformance`.
    Conformance,
    /// `goodfetch-stub-gateway`.
    StubGateway,
}

impl fmt::Display for Tool {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Collie => "collie",
            Self::Foxhound => "foxhound",
            Self::ProvingGround => "proving_ground",
            Self::TestEnv => "test_env",
            Self::ScriptedEngine => "scripted_engine",
            Self::Conformance => "conformance",
            Self::StubGateway => "stub_gateway",
        })
    }
}

/// A Goodfetch Product, whose executable a test runs: Collie's unless the
/// test needs Foxhound (ADR 0094).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub enum Product {
    /// Collie.
    #[default]
    Collie,
    /// Foxhound.
    Foxhound,
}

impl Product {
    /// Its executable in the manifest.
    pub fn tool(self) -> Tool {
        match self {
            Self::Collie => Tool::Collie,
            Self::Foxhound => Tool::Foxhound,
        }
    }
}

/// The tree this crate was compiled from, which is the tree of the test
/// that links it: Cargo rebuilds a test with its sources, and not the
/// tools.
const TREE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../..");

/// What a [`ToolsError`] of tools not built from this tree says to run.
const REBUILD: &str =
    "rebuild them with `just _test-tools`, or run the tests with `just test`, which does";

/// A manifest's input record, which `xtask test-tools build` writes beside
/// it: what its tools were built from.
#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRecord {
    /// The tree's folder.
    root: PathBuf,
    /// Each source of the tools, as it was when they were built.
    inputs: Vec<Input>,
}

/// One source: its path from the tree's folder, its size in bytes (none for
/// a folder, whose size Linux and Windows count each their own way), its
/// modification time in nanoseconds since the Unix epoch and the SHA-256 of
/// its content in lowercase hex (none for a folder).
#[derive(serde::Deserialize)]
struct Input(PathBuf, u64, u64, Option<String>);

/// The nanoseconds of the coarsest clock a source's time is read through:
/// Windows keeps file times in 100 ns ticks, so Wine running a test built for
/// it reads the time `xtask` recorded to the nanosecond on Linux in those
/// ticks (issue 2767).
const TICK: u64 = 100;

/// The size and modification time of the file at `path`, as an [`Input`]
/// holds them; `None` when it has none to read.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let metadata = std::fs::metadata(path).ok()?;
    let modified = metadata.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    let size = if metadata.is_dir() { 0 } else { metadata.len() };
    Some((size, u64::try_from(modified.as_nanos()).ok()?))
}

/// Whether the file at `path` is as `input` recorded it: its size, and its
/// modification time to the [`TICK`] or, when only the time moved, its
/// content, as a check that writes a file back with its own bytes moves its
/// time alone (issue 3236). A folder has no content to compare.
fn unchanged(path: &Path, Input(_, size, modified, digest): &Input) -> bool {
    let Some((now_size, now_modified)) = stamp(path) else {
        return false;
    };
    if now_size != *size {
        return false;
    }
    if now_modified / TICK == modified / TICK {
        return true;
    }
    digest.as_ref().is_some_and(|digest| {
        std::fs::read(path)
            .is_ok_and(|content| HEXLOWER.encode(&Sha256::digest(content)) == *digest)
    })
}

/// Where the input record of the manifest at `manifest` is.
fn record_path(manifest: &Path) -> PathBuf {
    manifest.with_extension("inputs.json")
}

/// Fails when the tools of the manifest at `manifest` were built from
/// another tree than `tree`, or before a source of theirs changed; a
/// manifest with no input record passes.
fn check_built_from(manifest: &Path, tree: &Path) -> Result<(), ToolsError> {
    let path = record_path(manifest);
    let text = match std::fs::read(&path) {
        Ok(text) => text,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(ToolsError::Unreadable { path, source }),
    };
    let record: InputRecord =
        serde_json::from_slice(&text).map_err(|source| ToolsError::Invalid { path, source })?;
    // One tree has several spellings: `tree` through `..`, a link's.
    let tree = std::fs::canonicalize(tree).unwrap_or_else(|_| tree.to_owned());
    if record.root != tree && std::fs::canonicalize(&record.root).ok().as_ref() != Some(&tree) {
        return Err(ToolsError::OtherTree {
            manifest: manifest.to_owned(),
            built_from: record.root,
            tree,
        });
    }
    match record
        .inputs
        .into_iter()
        .find(|input| !unchanged(&tree.join(&input.0), input))
    {
        Some(Input(input, ..)) => Err(ToolsError::Stale {
            manifest: manifest.to_owned(),
            input,
        }),
        None => Ok(()),
    }
}

impl Tools {
    /// Reads the manifest at `manifest`, which `None` says is not set, and
    /// fails when its tools were not built from this tree as it is.
    pub fn read(manifest: Option<&Path>) -> Result<Self, ToolsError> {
        Self::read_built_from(manifest, Path::new(TREE))
    }

    /// Reads the manifest at `manifest`, which `None` says is not set, and
    /// fails when its input record says its tools were built from another
    /// tree than `tree`, or before a source of theirs in it changed.
    pub fn read_built_from(manifest: Option<&Path>, tree: &Path) -> Result<Self, ToolsError> {
        let tools = Self::parse(manifest)?;
        if let Some(manifest) = manifest {
            check_built_from(manifest, tree)?;
        }
        Ok(tools)
    }

    /// The manifest at `manifest`, whatever its tools were built from.
    fn parse(manifest: Option<&Path>) -> Result<Self, ToolsError> {
        let path = manifest.ok_or(ToolsError::Unset)?;
        let text = std::fs::read(path).map_err(|source| ToolsError::Unreadable {
            path: path.to_owned(),
            source,
        })?;
        serde_json::from_slice(&text).map_err(|source| ToolsError::Invalid {
            path: path.to_owned(),
            source,
        })
    }

    /// The path of one executable.
    pub fn path(&self, tool: Tool) -> Result<&Path, ToolsError> {
        let path = match tool {
            Tool::Collie => &self.collie,
            Tool::Foxhound => &self.foxhound,
            Tool::ProvingGround => &self.proving_ground,
            Tool::TestEnv => &self.test_env,
            Tool::ScriptedEngine => &self.scripted_engine,
            Tool::Conformance => &self.conformance,
            Tool::StubGateway => &self.stub_gateway,
        };
        path.as_deref().ok_or(ToolsError::Missing(tool))
    }
}

/// The manifest `GOODFETCH_TEST_TOOLS` names, whose tools were built from
/// this tree as it is: its input record is compared with the tree at the
/// process's first call, and a manifest that passed is not compared again.
pub fn tools() -> Result<Tools, ToolsError> {
    /// The manifest whose tools this process found built from this tree.
    static BUILT_FROM_THIS_TREE: OnceLock<PathBuf> = OnceLock::new();
    let variables = Variables::from_process();
    let manifest = variables.get(TOOLS_VARIABLE).map(Path::new);
    let tools = Tools::parse(manifest)?;
    if let Some(manifest) = manifest
        && BUILT_FROM_THIS_TREE.get().map(PathBuf::as_path) != Some(manifest)
    {
        check_built_from(manifest, Path::new(TREE))?;
        // Lost to another thread's same answer, or to another manifest's,
        // which then is compared at each call.
        let _ = BUILT_FROM_THIS_TREE.set(manifest.to_owned());
    }
    Ok(tools)
}

/// Why a test cannot find an executable, or may not run it: each names
/// `just test`, which builds them and writes the manifest.
#[derive(thiserror::Error)]
#[non_exhaustive]
pub enum ToolsError {
    /// `GOODFETCH_TEST_TOOLS` is not set.
    #[error(
        "{TOOLS_VARIABLE} is not set: run the tests with `just test`, which builds the commit's executables and writes the test tool manifest"
    )]
    Unset,
    /// The manifest cannot be read.
    #[error("the test tool manifest {} cannot be read ({source}): run the tests with `just test`, which writes it", path.display())]
    Unreadable {
        /// Its path.
        path: PathBuf,
        /// Why.
        source: io::Error,
    },
    /// The manifest is not one.
    #[error("the test tool manifest {} is invalid ({source}): run the tests with `just test`, which writes it", path.display())]
    Invalid {
        /// Its path.
        path: PathBuf,
        /// Why.
        source: serde_json::Error,
    },
    /// The manifest lists no such executable.
    #[error(
        "the test tool manifest lists no {0}: run the tests with `just test`, which builds every executable the workspace has"
    )]
    Missing(Tool),
    /// The manifest's tools were built from another tree.
    #[error("the test tools of {} were built from {}, not from this tree, {}: {REBUILD}", manifest.display(), built_from.display(), tree.display())]
    OtherTree {
        /// The manifest's path.
        manifest: PathBuf,
        /// The tree its input record names.
        built_from: PathBuf,
        /// The tree of the test.
        tree: PathBuf,
    },
    /// A source of the manifest's tools changed since they were built.
    #[error("the test tools of {} are older than their sources ({} changed since they were built): {REBUILD}", manifest.display(), input.display())]
    Stale {
        /// The manifest's path.
        manifest: PathBuf,
        /// The first changed source, from the tree's folder.
        input: PathBuf,
    },
}

/// Its message: most tests unwrap `tools()`, and the panic then says what to
/// run rather than the variant's name.
impl fmt::Debug for ToolsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}
