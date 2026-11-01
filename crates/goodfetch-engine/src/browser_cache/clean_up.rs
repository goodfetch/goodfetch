//! The clean-up's rule (ADR 0098): an entry is deleted only when nothing
//! installed pins it, it has gone unused for `browser_cache.max_unused_age`,
//! and no process holds it; an entry this engine cannot read, and a cache it
//! cannot write, are never touched. Runs on the tasks module's blocking
//! policy.

use std::collections::{BTreeMap, BTreeSet};
use std::io;
use std::path::Path;
use std::time::Duration;

use goodfetch_protocol::browser_cache::{ArtifactId, NotCleaned};

use super::store::{self, Locking, Read};

/// `browser_cache.max_unused_age` (ADR 0098): 30 days by default, or `None`
/// for `null`, which turns the automatic clean-up off.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MaxUnusedAge(Option<Duration>);

impl MaxUnusedAge {
    /// The default, 30 days.
    pub const DEFAULT: Duration = Duration::from_hours(30 * 24);

    /// The setting's value: a duration, or `None` for `null`.
    pub fn new(value: Option<Duration>) -> Self {
        Self(value)
    }

    /// The age the automatic clean-up uses, or `None` when it is off.
    pub fn automatic(self) -> Option<Duration> {
        self.0
    }

    /// The age `browser clean` and the count of installed executables use:
    /// the setting's, or the default when it is `null`.
    pub fn counting(self) -> Duration {
        self.0.unwrap_or(Self::DEFAULT)
    }
}

impl Default for MaxUnusedAge {
    fn default() -> Self {
        Self(Some(Self::DEFAULT))
    }
}

/// One entry of the list of installed executables, as the clean-up reads it
/// (ADR 0098).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstalledExecutable {
    /// Whether its executable is still at its recorded path, at its
    /// recorded version.
    pub still_at_path: bool,
    /// When it was last seen.
    pub last_seen: jiff::Timestamp,
    /// The artifacts its version pins.
    pub pins: Vec<ArtifactId>,
}

/// The artifacts nothing may delete: those every executable that counts as
/// installed pins, and those an installed Plugin declares (ADRs 0033 and
/// 0098). An entry counts while its executable is still at its path, or was
/// seen within `max_unused_age`.
pub fn kept_artifacts(
    installed: &[InstalledExecutable],
    declared_by_plugins: &[ArtifactId],
    now: jiff::Timestamp,
    max_unused_age: MaxUnusedAge,
) -> BTreeSet<ArtifactId> {
    let age = max_unused_age.counting();
    installed
        .iter()
        .filter(|executable| executable.still_at_path || within(executable.last_seen, now, age))
        .flat_map(|executable| executable.pins.iter().cloned())
        .chain(declared_by_plugins.iter().cloned())
        .collect()
}

/// Whether `then` is less than `age` before `now`; a time after `now` is.
fn within(then: jiff::Timestamp, now: jiff::Timestamp, age: Duration) -> bool {
    let elapsed = now.duration_since(then);
    elapsed.is_negative() || elapsed.unsigned_abs() < age
}

/// What one clean-up is asked to do.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanUp {
    /// The artifacts nothing may delete, from [`kept_artifacts`].
    pub kept: BTreeSet<ArtifactId>,
    /// The time it runs at.
    pub now: jiff::Timestamp,
    /// How long an unpinned entry may go unused.
    pub max_unused_age: Duration,
    /// Whether it only lists what it would delete.
    pub dry_run: bool,
}

/// What a clean-up did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CleanUpReport {
    /// What it deleted, or would delete under a dry run, in order.
    pub deleted: Vec<ArtifactId>,
    /// The bytes each entry of `deleted` took on disk.
    pub bytes: BTreeMap<ArtifactId, u64>,
    /// Why it touched nothing, when it did not run.
    pub not_cleaned: Option<NotCleaned>,
}

impl CleanUpReport {
    /// The bytes the deleted entry `id` took, or `None` when it deleted no
    /// such entry.
    pub fn bytes_of(&self, id: &ArtifactId) -> Option<u64> {
        self.bytes.get(id).copied()
    }
}

/// Runs `request` over the cache at `root`.
pub(super) fn run(root: &Path, request: &CleanUp) -> io::Result<CleanUpReport> {
    if !store::writable(root) {
        return Ok(CleanUpReport {
            deleted: Vec::new(),
            bytes: BTreeMap::new(),
            not_cleaned: Some(NotCleaned::CacheNotWritable),
        });
    }
    if !request.dry_run {
        store::finish_deletions(root)?;
    }
    let mut deleted = Vec::new();
    let mut bytes = BTreeMap::new();
    for (id, folder) in store::entry_folders(root)? {
        if request.kept.contains(&id) {
            continue;
        }
        let Read::Entry(entry) = store::read_entry(&folder, &id) else {
            continue;
        };
        if within(entry.used_at, request.now, request.max_unused_age) {
            continue;
        }
        let Locking::Taken(lock) = store::lock_exclusive(&folder)? else {
            continue;
        };
        let size = store::bytes(&folder)?;
        if !request.dry_run {
            store::delete(root, &folder)?;
        }
        drop(lock);
        bytes.insert(id.clone(), size);
        deleted.push(id);
    }
    Ok(CleanUpReport {
        deleted,
        bytes,
        not_cleaned: None,
    })
}

/// The automatic clean-up after a download: [`run`] at most once a day, and
/// never while `max_unused_age` is `null`. `None` when it did not run.
pub(super) fn run_daily(
    root: &Path,
    kept: BTreeSet<ArtifactId>,
    now: jiff::Timestamp,
    max_unused_age: MaxUnusedAge,
) -> io::Result<Option<CleanUpReport>> {
    const DAY: Duration = Duration::from_hours(24);
    let Some(age) = max_unused_age.automatic() else {
        return Ok(None);
    };
    if store::last_clean_up(root).is_some_and(|last| within(last, now, DAY)) {
        return Ok(None);
    }
    let report = run(
        root,
        &CleanUp {
            kept,
            now,
            max_unused_age: age,
            dry_run: false,
        },
    )?;
    if report.not_cleaned.is_none() {
        store::record_clean_up(root, now)?;
    }
    Ok(Some(report))
}
