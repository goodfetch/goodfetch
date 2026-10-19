//! Fuzz replay (ADR 0067): a fuzz target's plain body run over every
//! committed seed and every past crasher, as an ordinary `#[test]` that
//! `fuzz_replay!` writes, so the replay runs on every OS, Windows included,
//! where cargo-fuzz does not build.
#![expect(
    clippy::disallowed_methods,
    reason = "the replay reads its folders inside a test, outside any async worker (ADR 0062)"
)]

use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

/// The folder of a target's committed seeds, inside its target folder.
pub const SEEDS_FOLDER: &str = "seeds";

/// The folder of a target's past crashers, inside its target folder.
pub const CRASHERS_FOLDER: &str = "crashers";

/// The files a replay ran the body over, seeds first, each folder in name
/// order.
#[derive(Debug, PartialEq, Eq)]
pub struct Replayed {
    /// Each file replayed.
    pub files: Vec<PathBuf>,
}

/// Why a replay failed.
#[derive(Debug, PartialEq, Eq)]
pub enum FuzzReplayError {
    /// The target folder has no seeds folder, or it holds no file.
    NoSeeds {
        /// The seeds folder.
        folder: PathBuf,
    },
    /// A folder or a file could not be read.
    Unreadable {
        /// The folder or file.
        path: PathBuf,
        /// The error's kind.
        kind: io::ErrorKind,
    },
    /// The body panicked on these files, in replay order.
    Crashed {
        /// Each file the body panicked on.
        files: Vec<PathBuf>,
    },
}

impl fmt::Display for FuzzReplayError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSeeds { folder } => write!(
                formatter,
                "the fuzz target has no committed seed in {} (ADR 0067)",
                folder.display()
            ),
            Self::Unreadable { path, kind } => {
                write!(formatter, "cannot read {}: {kind}", path.display())
            }
            Self::Crashed { files } => {
                write!(formatter, "the fuzz target's body panicked on")?;
                for file in files {
                    write!(formatter, "\n  {}", file.display())?;
                }
                Ok(())
            }
        }
    }
}

impl std::error::Error for FuzzReplayError {}

/// Runs `body` over every file of `target`'s seeds folder, then of its
/// crashers folder, each in name order, and fails naming each file the body
/// panicked on. A missing or empty crashers folder replays nothing; a
/// missing or empty seeds folder fails.
///
/// # Errors
///
/// [`FuzzReplayError`] when the seeds folder is missing or empty, a file
/// cannot be read, or the body panicked on a file.
pub fn replay_fuzz_target(
    target: impl AsRef<Path>,
    body: impl Fn(&[u8]),
) -> Result<Replayed, FuzzReplayError> {
    let target = target.as_ref();
    let seeds = target.join(SEEDS_FOLDER);
    let mut files = match files_in(&seeds)? {
        Some(files) if !files.is_empty() => files,
        _ => return Err(FuzzReplayError::NoSeeds { folder: seeds }),
    };
    files.extend(files_in(&target.join(CRASHERS_FOLDER))?.unwrap_or_default());

    let mut crashed = Vec::new();
    for file in &files {
        let input = std::fs::read(file).map_err(|error| FuzzReplayError::Unreadable {
            path: file.clone(),
            kind: error.kind(),
        })?;
        // Named before it runs, so an input that aborts the process, such as
        // a stack overflow, is still named in the test's output.
        #[expect(
            clippy::print_stderr,
            reason = "the replay names each input in the test's output (ADR 0067)"
        )]
        {
            eprintln!("goodfetch-fuzz-replay={}", file.display());
        }
        if catch_unwind(AssertUnwindSafe(|| body(&input))).is_err() {
            crashed.push(file.clone());
        }
    }
    if crashed.is_empty() {
        Ok(Replayed { files })
    } else {
        Err(FuzzReplayError::Crashed { files: crashed })
    }
}

/// The paths in `folder`, in name order, or `None` when it does not exist.
fn files_in(folder: &Path) -> Result<Option<Vec<PathBuf>>, FuzzReplayError> {
    let unreadable = |error: io::Error| FuzzReplayError::Unreadable {
        path: folder.to_owned(),
        kind: error.kind(),
    };
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(unreadable(error)),
    };
    let mut files = entries
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(unreadable)?;
    files.sort();
    Ok(Some(files))
}

#[cfg(test)]
mod tests;
