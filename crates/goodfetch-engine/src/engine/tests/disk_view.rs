//! A test's view of filesystems, the governor's filesystem source in the
//! engine's tests (ADR 0122): each filesystem a folder and everything under
//! it, reading what the test sets, counting how often it is read.
#![expect(
    clippy::disallowed_types,
    reason = "the governor reads a source from its blocking threads: one Mutex guards the view, with nothing else under it, so there is no lock order for loom to test (ADR 0060)"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use crate::governor::{FilesystemReading, FilesystemSource, System};

/// The filesystems a test sets. Clones share them.
#[derive(Clone, Debug, Default)]
pub(super) struct DiskView(Arc<Mutex<Vec<ViewFilesystem>>>);

#[derive(Debug)]
struct ViewFilesystem {
    root: PathBuf,
    reading: FilesystemReading,
    reads: u64,
}

impl DiskView {
    /// The OS the engine runs on, its filesystems read through this view.
    pub(super) fn system(&self) -> System {
        System::os().with_filesystems(Arc::new(self.clone()))
    }

    /// A filesystem of `size` bytes with `free` left, holding `root` and
    /// everything under it; answers its index.
    pub(super) fn mount(&self, root: &Path, size: u64, free: u64) -> usize {
        let mut filesystems = self.filesystems();
        filesystems.push(ViewFilesystem {
            root: root.to_owned(),
            reading: FilesystemReading {
                size,
                free,
                inodes: None,
            },
            reads: 0,
        });
        filesystems.len() - 1
    }

    /// The filesystem `index` has `free` left from now on.
    pub(super) fn set_free(&self, index: usize, free: u64) {
        self.filesystems()[index].reading.free = free;
    }

    /// How often the filesystem `index` was read.
    pub(super) fn reads(&self, index: usize) -> u64 {
        self.filesystems()[index].reads
    }

    fn filesystems(&self) -> MutexGuard<'_, Vec<ViewFilesystem>> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The filesystem with the longest root above `folder`.
    fn holding(&self, folder: &Path) -> Option<usize> {
        self.filesystems()
            .iter()
            .enumerate()
            .filter(|(_, filesystem)| folder.starts_with(&filesystem.root))
            .max_by_key(|(_, filesystem)| filesystem.root.components().count())
            .map(|(index, _)| index)
    }
}

impl FilesystemSource for DiskView {
    fn identify(&self, folder: &Path) -> Option<u64> {
        self.holding(folder)
            .map(|index| u64::try_from(index).unwrap())
    }

    fn read(&self, folder: &Path) -> Option<FilesystemReading> {
        let index = self.holding(folder)?;
        let mut filesystems = self.filesystems();
        let filesystem = filesystems.get_mut(index)?;
        filesystem.reads += 1;
        Some(filesystem.reading)
    }
}
