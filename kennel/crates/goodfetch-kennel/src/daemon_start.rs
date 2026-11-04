//! The "Start Daemon" action (ADR 0141): the installed Product's `daemon
//! start`, run by the absolute path its executable recorded in the home's
//! installed executables' list (ADR 0098), with no shell. A Kennel started
//! through `LaunchServices` has a minimal `PATH`, so the Kennel never looks a
//! Product up there. The Kennel runs it only when the person chooses it.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::executables::{ExecutableKind, InstalledExecutable, InstalledExecutables};

use crate::home::{Home, read_if_present};

/// A Product's executable that can run its major's Daemon.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledProduct {
    /// Which Product it is.
    pub product: Product,
    /// Its absolute path.
    pub path: PathBuf,
}

/// Why a `daemon start` failed.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum StartFailure {
    /// The executable could not be started.
    #[error("`daemon start` could not run: {0}")]
    NotRun(String),
    /// It ended with a failure, with its exit status when it had one.
    #[error("`daemon start` exited with status {}", .code.map_or_else(|| "unknown".to_owned(), |code| code.to_string()))]
    Exited {
        /// The exit status.
        code: Option<i32>,
    },
}

/// The installed Product whose `daemon start` the Kennel offers: a
/// Product's executable still at its recorded path, size and modification
/// time, Foxhound's first, since it contains Collie, then the one seen
/// last. None when the list is missing, unreadable or names no such
/// executable. Blocks.
pub fn find(home: &Home) -> Option<InstalledProduct> {
    let bytes = read_if_present(&home.installed_executables_file()).ok()??;
    let list: InstalledExecutables = serde_json::from_slice(&bytes).ok()?;
    list.executables
        .into_iter()
        .filter(|entry| entry.kind == ExecutableKind::ProductExecutable)
        .filter(|entry| Path::new(&entry.path).is_absolute() && still_at_path(entry))
        .max_by(|one, other| {
            (one.product == Product::Foxhound)
                .cmp(&(other.product == Product::Foxhound))
                .then_with(|| one.last_seen.cmp(&other.last_seen))
        })
        .map(|entry| InstalledProduct {
            product: entry.product,
            path: PathBuf::from(entry.path),
        })
}

/// Whether the file at the entry's path is still the executable it
/// recorded, by its size and its modification time in whole seconds, as
/// the engine records them.
#[expect(
    clippy::disallowed_methods,
    reason = "called from `find`, on the tracker's blocking threads (STYLE.md)"
)]
fn still_at_path(entry: &InstalledExecutable) -> bool {
    let Ok(metadata) = std::fs::metadata(&entry.path) else {
        return false;
    };
    let modified = metadata
        .modified()
        .ok()
        .and_then(|modified| jiff::Timestamp::try_from(modified).ok())
        .map(jiff::Timestamp::as_second);
    metadata.len() == entry.size && modified == Some(entry.modified_at.as_second())
}

/// Runs `product`'s `daemon start` and waits for it to end: the command
/// itself waits until the Daemon is ready, and the Daemon outlives it and
/// the Kennel.
///
/// # Cancel safety
/// Dropping the future stops the wait, never the command.
pub async fn start(product: &InstalledProduct) -> Result<(), StartFailure> {
    let mut command = tokio::process::Command::new(&product.path);
    command
        .args(["daemon", "start"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[expect(
        clippy::disallowed_methods,
        reason = "the Kennel starts one child process, the Product's `daemon start` the person chose (ADR 0141), which the Daemon outlives"
    )]
    let child = command.spawn();
    let status = child
        .map_err(|error| StartFailure::NotRun(error.to_string()))?
        .wait()
        .await
        .map_err(|error| StartFailure::NotRun(error.to_string()))?;
    if status.success() {
        Ok(())
    } else {
        Err(StartFailure::Exited {
            code: status.code(),
        })
    }
}
