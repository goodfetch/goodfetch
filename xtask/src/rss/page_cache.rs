//! Dropping an executable from the OS's page cache (spec #2165): `just
//! compare` and `just rss` drop each executable they measure before each
//! trial, so a trial pages its executable in by its own faults, as a first
//! start does, rather than finding it resident from its build's write or an
//! earlier trial. On Linux, `posix_fadvise(POSIX_FADV_DONTNEED)` over the
//! whole file, which needs no root, after `fdatasync` has written back the
//! pages its build left dirty, which the advice keeps. Elsewhere nothing is
//! dropped, and each command says so ([`NOT_DROPPED`]).
#![cfg_attr(
    target_os = "linux",
    expect(
        clippy::disallowed_methods,
        reason = "xtask is synchronous: it opens the executable with a blocking call (STYLE.md)"
    )
)]

use std::path::Path;

use anyhow::Result;

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

    use anyhow::Context as _;

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
