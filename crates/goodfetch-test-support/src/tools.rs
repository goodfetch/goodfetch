//! The test tool manifest (ADRs 0062 and 0094): the JSON file, written by
//! `just test` after it builds them, that holds the absolute paths of the
//! commit's executables a test runs. `GOODFETCH_TEST_TOOLS` names it. A
//! test takes Collie's executable unless it needs Foxhound, and one that
//! measures an executable takes each.
#![expect(
    clippy::disallowed_methods,
    reason = "test support reads the manifest outside any async worker (ADR 0062)"
)]

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

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
    /// `goodfetch-stub-gateway`, the Stub gateway the Console conformance
    /// suite runs against (ADR 0096).
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

impl Tools {
    /// Reads the manifest at `manifest`, which `None` says is not set.
    pub fn read(manifest: Option<&Path>) -> Result<Self, ToolsError> {
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

/// The manifest `GOODFETCH_TEST_TOOLS` names.
pub fn tools() -> Result<Tools, ToolsError> {
    let variables = Variables::from_process();
    Tools::read(variables.get(TOOLS_VARIABLE).map(Path::new))
}

/// Why a test cannot find an executable: each names `just test`, which
/// builds them and writes the manifest.
#[derive(Debug, thiserror::Error)]
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
}
