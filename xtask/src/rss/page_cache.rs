//! Dropping an executable from the OS's page cache (spec #2165): `just
//! compare` and `just rss` drop each executable they measure before each
//! trial, so a trial pages its executable in by its own faults, as a first
//! start does, rather than finding it resident from its build's write or an
//! earlier trial. On Linux, `posix_fadvise(POSIX_FADV_DONTNEED)` over the
//! whole file, which needs no root, after `fdatasync` has written back the
//! pages its build left dirty, which the advice keeps. Elsewhere nothing is
//! dropped, and each command says so ([`NOT_DROPPED`]). `xtask rss measure
//! --page-cache cached` reads each executable through instead, so a run
//! finds every page of it in the page cache, as a start soon after another
//! does ([`PageCache`]).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: it opens the executable with a blocking call (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};

/// Where an executable's pages are when a run of it starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum PageCache {
    /// Dropped from the page cache, so the run pages its executable in by
    /// its own faults, as a first start does.
    #[default]
    Dropped,
    /// Read through once, so every page of the executable is in the page
    /// cache when the run maps it, as at a start soon after another.
    Cached,
}

impl PageCache {
    /// Leaves `path`'s pages where this mode says, while no process runs it.
    pub(crate) fn prepare(self, path: &Path) -> Result<()> {
        match self {
            Self::Dropped => drop_cached(path),
            Self::Cached => read_through(path),
        }
    }
}

/// Reads `path` whole, which leaves each of its pages in the page cache.
fn read_through(path: &Path) -> Result<()> {
    let mut file =
        std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    std::io::copy(&mut file, &mut std::io::sink())
        .with_context(|| format!("reading {} into the page cache", path.display()))?;
    Ok(())
}

/// Whether this OS drops a file from its page cache.
pub(crate) const DROPS: bool = cfg!(target_os = "linux");

/// What a command prints where [`DROPS`] is false.
pub(crate) const NOT_DROPPED: &str = "This OS drops no executable from the page cache: a trial may find its executable's pages resident.";

/// Drops `path`'s pages from the page cache: those of a process that maps
/// it stay, so it is dropped while no process runs it.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "the advice goes through the OS's C interface: posix_fadvise"
)]
pub(crate) fn drop_cached(path: &Path) -> Result<()> {
    use std::os::fd::AsRawFd as _;

    let file = std::fs::File::open(path).with_context(|| format!("opening {}", path.display()))?;
    file.sync_data()
        .with_context(|| format!("writing back {}", path.display()))?;
    // SAFETY: the descriptor is `file`'s, open for the whole call.
    let error = unsafe { libc::posix_fadvise(file.as_raw_fd(), 0, 0, libc::POSIX_FADV_DONTNEED) };
    if error != 0 {
        return Err(std::io::Error::from_raw_os_error(error))
            .with_context(|| format!("dropping {} from the page cache", path.display()));
    }
    Ok(())
}

/// Drops nothing: this OS has no `posix_fadvise`.
#[cfg(not(target_os = "linux"))]
pub(crate) fn drop_cached(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(test)]
mod tests;
