//! The one way `xtask` runs Git: in a folder of the caller's choosing,
//! without the variables that point Git at a repository (`GIT_DIR`,
//! `GIT_WORK_TREE`, `GIT_INDEX_FILE`, …), so Git finds the repository from
//! that folder, as it does in a shell, even when a Git hook started `xtask`
//! with them set for the hook's own repository. A failure says which: Git
//! could not start, Git refused, or its output is no UTF-8 text.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-process ban targets async crates (STYLE.md)"
)]

use std::fmt;
use std::path::Path;
use std::process::Command;

/// The variables that name a repository or a part of one, which Git itself
/// clears before it enters another repository (`git rev-parse
/// --local-env-vars`, Git 2.53).
const REPOSITORY_VARIABLES: [&str; 15] = [
    "GIT_ALTERNATE_OBJECT_DIRECTORIES",
    "GIT_CONFIG",
    "GIT_CONFIG_PARAMETERS",
    "GIT_CONFIG_COUNT",
    "GIT_OBJECT_DIRECTORY",
    "GIT_DIR",
    "GIT_WORK_TREE",
    "GIT_IMPLICIT_WORK_TREE",
    "GIT_GRAFT_FILE",
    "GIT_INDEX_FILE",
    "GIT_NO_REPLACE_OBJECTS",
    "GIT_REPLACE_REF_BASE",
    "GIT_PREFIX",
    "GIT_SHALLOW_FILE",
    "GIT_COMMON_DIR",
];

/// Why a Git command gave no output.
#[derive(Debug)]
pub(crate) enum Error {
    /// Git could not be started.
    Start {
        arguments: String,
        source: std::io::Error,
    },
    /// Git ran and exited unsuccessfully, saying why on its standard error.
    Refused { arguments: String, stderr: String },
    /// Git's output is no UTF-8 text.
    NotText { arguments: String },
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Start { arguments, .. } => write!(formatter, "running git {arguments}"),
            Self::Refused { arguments, stderr } => {
                write!(formatter, "git {arguments} failed: {stderr}")
            }
            Self::NotText { arguments } => {
                write!(formatter, "git {arguments}'s output is no UTF-8 text")
            }
        }
    }
}

impl std::error::Error for Error {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Start { source, .. } => Some(source),
            Self::Refused { .. } | Self::NotText { .. } => None,
        }
    }
}

/// `git` as a command run in `folder`, without the repository variables:
/// for a caller that sets more of its environment or reads its status
/// itself.
pub(crate) fn command(folder: &Path) -> Command {
    let mut command = Command::new("git");
    command.current_dir(folder);
    for name in REPOSITORY_VARIABLES {
        command.env_remove(name);
    }
    command
}

/// What `git <arguments>` in `folder` prints on its standard output.
pub(crate) fn bytes(folder: &Path, arguments: &[&str]) -> Result<Vec<u8>, Error> {
    let joined = || arguments.join(" ");
    let output = command(folder)
        .args(arguments)
        .output()
        .map_err(|source| Error::Start {
            arguments: joined(),
            source,
        })?;
    if !output.status.success() {
        return Err(Error::Refused {
            arguments: joined(),
            stderr: String::from_utf8_lossy(&output.stderr).trim().to_owned(),
        });
    }
    Ok(output.stdout)
}

/// What `git <arguments>` in `folder` prints, as text.
pub(crate) fn text(folder: &Path, arguments: &[&str]) -> Result<String, Error> {
    String::from_utf8(bytes(folder, arguments)?).map_err(|_| Error::NotText {
        arguments: arguments.join(" "),
    })
}

/// The one line `git <arguments>` in `folder` prints, without its line
/// end: an object name, a path.
pub(crate) fn line(folder: &Path, arguments: &[&str]) -> Result<String, Error> {
    Ok(text(folder, arguments)?.trim().to_owned())
}

/// Whether `git <arguments>` in `folder` succeeds, for a question Git
/// answers by its status alone (`cat-file -e`, `rev-parse --verify`).
pub(crate) fn succeeds(folder: &Path, arguments: &[&str]) -> Result<bool, Error> {
    match bytes(folder, arguments) {
        Ok(_) => Ok(true),
        Err(Error::Refused { .. }) => Ok(false),
        Err(error) => Err(error),
    }
}
