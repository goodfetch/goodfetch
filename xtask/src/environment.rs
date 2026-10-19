//! What every command reads from its process, which `main` alone gathers
//! (ADR 0059): the workspace's folder, the Cargo that runs `xtask` and
//! GitHub's token.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

pub(crate) struct Environment {
    /// The workspace's folder: the current folder.
    pub(crate) root: PathBuf,
    /// The Cargo that runs `xtask`, or the one on `PATH`.
    pub(crate) cargo: OsString,
    /// `GITHUB_TOKEN`, for GitHub's API; never taken on the command line.
    pub(crate) github_token: Option<String>,
}

impl Environment {
    /// A path given on the command line, from the workspace's folder.
    pub(crate) fn path(&self, path: &Path) -> PathBuf {
        self.root.join(path)
    }

    pub(crate) fn token(&self) -> Option<&str> {
        self.github_token.as_deref()
    }
}
