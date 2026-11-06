//! Where the governor reads filesystems (ADR 0122), a value: the OS, or a
//! [`FilesystemSource`] the engine was given in its place; the folders it samples, each held by whoever
//! writes there; and the storage sample, which reads each filesystem
//! holding one of those folders once.
#![expect(
    clippy::disallowed_types,
    reason = "a held folder leaves the sample as it drops, which cannot wait: one Mutex guards the folders' counts, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use goodfetch_protocol::settings::SettingLimit;
use goodfetch_protocol::storage::StorageLevel;

#[cfg(unix)]
use super::FilesystemInodes;
use super::{FilesystemReading, Settled};

#[cfg(unix)]
mod unix;
#[cfg(windows)]
mod windows;

/// One filesystem as a storage sample found it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filesystem {
    /// The sampled folders it holds, in order.
    pub folders: Vec<PathBuf>,
    /// Its level.
    pub level: StorageLevel,
    /// The space left on it to the engine's user, in bytes.
    pub free: u64,
    /// The inodes left on it to the engine's user, where it reports them.
    pub free_inodes: Option<u64>,
    /// The setting that set its level, with the size in force; none at
    /// `normal`.
    pub limit: Option<SettingLimit>,
}

impl Filesystem {
    /// Whether it is at `critical`, where every Run with an essential write
    /// on it stops and no Run starts (ADR 0122).
    pub fn is_critical(&self) -> bool {
        self.level == StorageLevel::Critical
    }
}

/// What a storage sample found: each filesystem holding a sampled folder,
/// once; a folder whose filesystem cannot be read is in none.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StorageSample {
    /// The sample's number, from 1 for the first: the one the governor's
    /// count of storage samples begun read as it began.
    pub number: u64,
    /// The filesystems, in the order of their first folder.
    pub filesystems: Vec<Filesystem>,
}

impl StorageSample {
    /// The filesystem holding the sampled `folder`, if it was read.
    pub fn of(&self, folder: &Path) -> Option<&Filesystem> {
        self.filesystems
            .iter()
            .find(|filesystem| filesystem.folders.iter().any(|held| held == folder))
    }
}

/// What tells two filesystems apart.
#[derive(Clone, Debug, PartialEq, Eq)]
enum FilesystemId {
    /// A Unix device.
    #[cfg(unix)]
    Device(u64),
    /// A Windows volume's mount point.
    #[cfg(windows)]
    Volume(String),
    /// One a chosen source tells apart by this number.
    Chosen(u64),
}

/// Where the governor reads filesystems in place of the OS's own calls,
/// such as an embedder whose filesystems the OS does not report as the
/// engine writes to them. Both calls block, and run under the governor's
/// blocking policy.
pub trait FilesystemSource: Send + Sync + std::fmt::Debug {
    /// The filesystem holding `folder`, by a number that tells it apart
    /// from the source's other filesystems; none where none holds it.
    fn identify(&self, folder: &Path) -> Option<u64>;

    /// What the filesystem holding `folder` reads now; none where it
    /// cannot be read.
    fn read(&self, folder: &Path) -> Option<FilesystemReading>;
}

/// Where the governor reads filesystems.
#[derive(Clone, Debug, Default)]
pub(crate) enum Disks {
    /// The OS the engine runs on.
    #[default]
    Os,
    /// A source the engine was given in place of the OS's calls.
    Chosen(Arc<dyn FilesystemSource>),
}

impl Disks {
    /// The filesystem holding `folder`, through its nearest folder that
    /// exists. Blocking.
    fn identify(&self, folder: &Path) -> Option<FilesystemId> {
        match self {
            Self::Os => os_identify(&existing(folder)?),
            Self::Chosen(source) => source.identify(folder).map(FilesystemId::Chosen),
        }
    }

    /// What the filesystem holding `folder` reads now, through its nearest
    /// folder that exists. Blocking.
    fn read(&self, folder: &Path) -> Option<FilesystemReading> {
        match self {
            Self::Os => os_read(&existing(folder)?),
            Self::Chosen(source) => source.read(folder),
        }
    }

    /// The level now of the filesystem holding `path`, a folder or a file
    /// to write, under `settled`; none when it cannot be read. Blocking.
    pub(crate) fn at(&self, path: &Path, settled: &Settled) -> Option<Filesystem> {
        let reading = self.read(path)?;
        Some(settled.filesystem(vec![path.to_owned()], &reading))
    }

    /// Samples each filesystem holding one of `folders` once, under
    /// `settled`, as the sample `number`. Blocking.
    pub(crate) fn sample(
        &self,
        number: u64,
        folders: &[PathBuf],
        settled: &Settled,
    ) -> StorageSample {
        let mut held: Vec<(FilesystemId, Vec<PathBuf>)> = Vec::new();
        for folder in folders {
            let Some(id) = self.identify(folder) else {
                continue;
            };
            match held.iter_mut().find(|(known, _)| *known == id) {
                Some((_, folders)) => folders.push(folder.clone()),
                None => held.push((id, vec![folder.clone()])),
            }
        }
        let filesystems = held
            .into_iter()
            .filter_map(|(_, folders)| {
                let reading = self.read(&folders[0])?;
                Some(settled.filesystem(folders, &reading))
            })
            .collect();
        StorageSample {
            number,
            filesystems,
        }
    }
}

/// `path`'s nearest folder, itself included, that exists.
fn existing(path: &Path) -> Option<PathBuf> {
    #[expect(
        clippy::disallowed_methods,
        reason = "every read here runs under the governor's blocking policy (STYLE.md)"
    )]
    path.ancestors()
        .find(|ancestor| std::fs::metadata(ancestor).is_ok_and(|metadata| metadata.is_dir()))
        .map(Path::to_owned)
}

#[cfg(unix)]
fn os_identify(folder: &Path) -> Option<FilesystemId> {
    unix::device(folder).map(FilesystemId::Device)
}

#[cfg(unix)]
fn os_read(folder: &Path) -> Option<FilesystemReading> {
    unix::read(folder)
}

#[cfg(windows)]
fn os_identify(folder: &Path) -> Option<FilesystemId> {
    windows::volume(folder).map(FilesystemId::Volume)
}

#[cfg(windows)]
fn os_read(folder: &Path) -> Option<FilesystemReading> {
    windows::read(folder)
}

#[cfg(not(any(unix, windows)))]
fn os_identify(_folder: &Path) -> Option<FilesystemId> {
    None
}

#[cfg(not(any(unix, windows)))]
fn os_read(_folder: &Path) -> Option<FilesystemReading> {
    None
}

/// The folders the governor samples, each held as long as a holder keeps
/// it.
#[derive(Clone, Debug, Default)]
pub(crate) struct Folders(Arc<Mutex<BTreeMap<PathBuf, usize>>>);

impl Folders {
    /// Samples `folder` until the answer drops.
    pub(crate) fn hold(&self, folder: PathBuf) -> Held {
        self.hold_together([folder])
            .pop()
            .expect("invariant: one folder held gives one answer")
    }

    /// Samples each of `folders` until its answer drops, all from the same
    /// sample on: no sample holds some of them without the others.
    pub(crate) fn hold_together(&self, folders: impl IntoIterator<Item = PathBuf>) -> Vec<Held> {
        let mut held = lock(&self.0);
        folders
            .into_iter()
            .map(|folder| {
                *held.entry(folder.clone()).or_default() += 1;
                Held {
                    folders: self.clone(),
                    folder,
                }
            })
            .collect()
    }

    /// Every folder held now, in order.
    pub(crate) fn now(&self) -> Vec<PathBuf> {
        lock(&self.0).keys().cloned().collect()
    }
}

/// A folder the governor samples while it is held.
#[derive(Debug)]
pub struct Held {
    folders: Folders,
    folder: PathBuf,
}

impl Held {
    /// The folder.
    pub fn folder(&self) -> &Path {
        &self.folder
    }
}

impl Drop for Held {
    fn drop(&mut self) {
        let mut folders = lock(&self.folders.0);
        if let Some(count) = folders.get_mut(&self.folder) {
            *count -= 1;
            if *count == 0 {
                folders.remove(&self.folder);
            }
        }
    }
}

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}
