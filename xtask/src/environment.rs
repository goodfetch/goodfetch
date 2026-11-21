//! What every command reads from its process, which `main` alone gathers
//! (ADR 0059): the workspace's folder, the Cargo that runs `xtask`,
//! GitHub's token, whether the host has a LazyFS build, the cap on a counts
//! benchmark's Callgrind processes, the Goodfetch
//! variables a measured executable must not inherit and the bytes the
//! environment takes in a child's arguments area.

use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};

/// The variable a benchmark reads its cap on the Callgrind processes it runs
/// at once from: test support's `callgrind::bounded` reads it (#2520).
pub(crate) const CALLGRIND_AT_ONCE_VARIABLE: &str = "GOODFETCH_COUNTS_CALLGRIND_AT_ONCE";

pub(crate) struct Environment {
    /// The names of the process's `GOODFETCH_*` variables, which a
    /// measured executable runs without, as shipped: an engine stops on one
    /// it does not know.
    pub(crate) goodfetch_variables: Vec<OsString>,
    /// The bytes the process's variables take in the area `execve` copies
    /// for a child: each name, `=`, value and NUL, and its pointer.
    pub(crate) variables_bytes: usize,
    /// The workspace's folder: the current folder.
    pub(crate) root: PathBuf,
    /// The Cargo that runs `xtask`, or the one on `PATH`.
    pub(crate) cargo: OsString,
    /// `GITHUB_TOKEN`, for GitHub's API; never taken on the command line.
    pub(crate) github_token: Option<String>,
    /// Whether `GOODFETCH_LAZYFS` names a LazyFS build, as `just
    /// lazyfs-test` sets it on Linux.
    pub(crate) lazyfs: bool,
    /// `GOODFETCH_COUNTS_CALLGRIND_AT_ONCE`, as given: the cap on the
    /// Callgrind processes a counts benchmark runs at once (#2520).
    pub(crate) callgrind_at_once: Option<OsString>,
    /// Whether `CI` is set: CI runs every part, so a missing tool fails
    /// there rather than listing its part as skipped (ADR 0061).
    pub(crate) ci: bool,
    /// The folders of `PATH`, where a tool a command runs is looked for.
    pub(crate) programs: Vec<PathBuf>,
}

impl Environment {
    /// What `main` gathers from the process's `variables`, read once.
    pub(crate) fn of(variables: &[(OsString, OsString)]) -> Self {
        let variable = |name: &str| {
            variables
                .iter()
                .find(|(found, _)| found == name)
                .map(|(_, value)| value.clone())
        };
        Self {
            root: PathBuf::from("."),
            cargo: variable("CARGO").unwrap_or_else(|| OsString::from("cargo")),
            github_token: variable("GITHUB_TOKEN")
                .and_then(|token| token.into_string().ok())
                .filter(|token| !token.is_empty()),
            lazyfs: variable("GOODFETCH_LAZYFS").is_some(),
            callgrind_at_once: variable(CALLGRIND_AT_ONCE_VARIABLE),
            ci: variable("CI").is_some_and(|value| !value.is_empty()),
            programs: variable("PATH")
                .map(|path| std::env::split_paths(&path).collect())
                .unwrap_or_default(),
            goodfetch_variables: variables
                .iter()
                .map(|(name, _)| name)
                .filter(|name| is_goodfetch(name))
                .cloned()
                .collect(),
            variables_bytes: variables
                .iter()
                .map(|(name, value)| name.len() + value.len() + 2 + size_of::<usize>())
                .sum(),
        }
    }

    /// A path given on the command line, from the workspace's folder.
    pub(crate) fn path(&self, path: &Path) -> PathBuf {
        self.root.join(path)
    }

    pub(crate) fn token(&self) -> Option<&str> {
        self.github_token.as_deref()
    }

    /// Whether the program `name` is in a folder of `PATH`.
    pub(crate) fn has_program(&self, name: &str) -> bool {
        self.programs
            .iter()
            .any(|folder| folder.join(name).is_file())
    }
}

/// Whether `name` is a Goodfetch variable.
fn is_goodfetch(name: &OsStr) -> bool {
    name.to_string_lossy().starts_with("GOODFETCH_")
}
