//! How recently a build read what the free-space sweep may remove: when a
//! file's times prove no build read it since a moment under `relatime`,
//! which files a run re-arms, and the order and the stop of what goes under
//! the line.

use std::fs::Metadata;
use std::path::PathBuf;
use std::time::SystemTime;

use anyhow::Result;

use super::{RELATIME_LAG, Swept};

/// A file's times, as the free-space sweep reads them.
#[derive(Clone, Copy, Debug)]
pub(super) struct Times {
    pub(super) accessed: SystemTime,
    pub(super) modified: SystemTime,
    /// The last change of the file or of its metadata.
    pub(super) changed: SystemTime,
}

impl Times {
    pub(super) fn of(metadata: &Metadata) -> Result<Self> {
        Ok(Self {
            accessed: metadata.accessed()?,
            modified: metadata.modified()?,
            changed: changed(metadata)?,
        })
    }

    /// Whether no process has read or written the file since `since`, as
    /// `relatime` records reads: a read moves the access time when it is no
    /// later than the last change, or a day old.
    pub(super) fn unread_since(self, since: SystemTime) -> bool {
        // Unread since its last change, which came before `since`.
        let armed = self.accessed < self.changed && self.changed <= since;
        // Any read since `since` would have found the access time a day old.
        let lagged = since
            .checked_sub(RELATIME_LAG)
            .is_some_and(|lag| self.accessed.max(self.modified) < lag);
        armed || lagged
    }

    /// Whether a read since the last change left the access time where the
    /// next read may not move it for a day.
    pub(super) fn needs_rearm(self) -> bool {
        self.accessed >= self.changed
    }
}

/// What the free-space sweep may remove: a unit's hashed files, then its
/// fingerprint, or a file with no unit, or an incremental folder.
#[derive(Debug)]
pub(super) struct Candidate {
    /// When a build last read it, as far as the file system tells.
    pub(super) last: SystemTime,
    /// Its target's index.
    pub(super) target: usize,
    pub(super) paths: Vec<PathBuf>,
    /// The unit's index in the fingerprints, for a unit.
    pub(super) unit: Option<usize>,
}

/// Takes `candidates`, least recently read first, each while `under_line`
/// says its target's file system is still under the line.
pub(super) fn least_read_first(
    candidates: &mut [Candidate],
    mut under_line: impl FnMut(usize, &[Swept]) -> Result<bool>,
    mut take: impl FnMut(&Candidate, &mut [Swept]) -> Result<()>,
    swept: &mut [Swept],
) -> Result<()> {
    candidates.sort_by_key(|candidate| candidate.last);
    for candidate in candidates.iter() {
        if under_line(candidate.target, swept)? {
            take(candidate, swept)?;
        }
    }
    Ok(())
}

/// The last change of a file or of its metadata.
#[cfg(unix)]
fn changed(metadata: &Metadata) -> Result<SystemTime> {
    use std::os::unix::fs::MetadataExt as _;
    use std::time::Duration;

    use anyhow::Context as _;

    let seconds = u64::try_from(metadata.ctime()).context("a change before 1970")?;
    let nanos = u32::try_from(metadata.ctime_nsec()).context("a change's nanoseconds")?;
    Ok(SystemTime::UNIX_EPOCH + Duration::new(seconds, nanos))
}

#[cfg(not(unix))]
fn changed(_metadata: &Metadata) -> Result<SystemTime> {
    anyhow::bail!("--free-gb reads a file's last change, which only a Unix host gives")
}

#[cfg(test)]
mod tests;
