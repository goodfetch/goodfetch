//! The watch on the Daemons' runtime files (ADR 0141): the platform's file
//! notifications, never a poll, report each change that can change which
//! Daemon runs. A Daemon writes its runtime file into its per-major folder
//! by rename when it starts, and removes it when it stops, so the watch
//! covers the folders, not the files: the home, where per-major folders
//! appear, and each per-major folder, without their subfolders.
//!
//! A home that cannot be watched, as one no Product has made yet, is
//! watched once it can be, at the Client's next look.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use tokio::sync::mpsc;

use crate::home::{Home, canonical, major_folders, names_a_daemons_file};
use crate::tasks::Tasks;

/// The watch, and the changes it reported that the Client has not looked
/// at yet.
#[derive(Debug)]
pub(super) struct RuntimeFiles {
    watch: Option<Watch>,
    changed: mpsc::Sender<()>,
    /// One pending change is enough: the look that follows sees them all.
    pending: mpsc::Receiver<()>,
}

impl RuntimeFiles {
    /// No watch yet: [`RuntimeFiles::rewatch`] starts it.
    pub(super) fn new() -> Self {
        let (changed, pending) = mpsc::channel(1);
        Self {
            watch: None,
            changed,
            pending,
        }
    }

    /// Forgets the changes reported so far and watches every per-major
    /// folder of `home`, before the Client looks: the look sees every
    /// change made before it, and the watch reports every change after.
    ///
    /// # Cancel safety
    /// Not cancel safe: dropping the future loses the watch, which the
    /// next call starts again.
    pub(super) async fn rewatch(&mut self, tasks: &Tasks, home: &Home) {
        while self.pending.try_recv().is_ok() {}
        let watch = self.watch.take();
        let home = home.path().to_owned();
        let changed = self.changed.clone();
        self.watch = tasks
            .blocking(move || Watch::of_every_major(watch, &home, changed))
            .await;
    }

    /// Waits for the next change.
    ///
    /// # Cancel safety
    /// Cancel safe: a change is kept until a call takes it.
    pub(super) async fn changed(&mut self) {
        // This holds a sender, so the channel never closes.
        let _ = self.pending.recv().await;
    }
}

/// The platform's watch and the folders it covers.
#[derive(Debug)]
struct Watch {
    watcher: RecommendedWatcher,
    /// The per-major folders watched, by their canonical paths.
    folders: BTreeSet<PathBuf>,
}

impl Watch {
    /// `watch`, or a new one of `home`, extended to its per-major folders
    /// and rid of those gone; none when the platform cannot watch the
    /// home. Blocks.
    fn of_every_major(watch: Option<Self>, home: &Path, changed: mpsc::Sender<()>) -> Option<Self> {
        // FSEvents names changed files by their canonical path (`/private/var`
        // for `/var` on macOS), and a non-recursive watch keeps only the
        // events of a folder as named, so every folder is watched by its
        // canonical path.
        let home = canonical(home);
        let mut watch = match watch {
            Some(watch) => watch,
            None => match Self::of_home(&home, changed) {
                Ok(watch) => watch,
                Err(error) => {
                    tracing::warn!(%error, "the Daemon's runtime file is not watched, so a Daemon that starts is reached only after \"Start Daemon\"");
                    return None;
                }
            },
        };
        let present: BTreeSet<PathBuf> = major_folders(&home)
            .into_iter()
            .map(|(_, folder)| folder)
            .collect();
        for gone in watch.folders.difference(&present) {
            // The platform may already have dropped the watch of a folder
            // that is gone.
            let _ = watch.watcher.unwatch(gone);
        }
        watch.folders.retain(|folder| present.contains(folder));
        for folder in present {
            if watch.folders.contains(&folder) {
                continue;
            }
            match watch.watcher.watch(&folder, RecursiveMode::NonRecursive) {
                Ok(()) => {
                    watch.folders.insert(folder);
                }
                Err(error) => {
                    tracing::warn!(%error, folder = %folder.display(), "a per-major folder is not watched");
                }
            }
        }
        Some(watch)
    }

    /// A watch of `home` alone, which reports to `changed`.
    fn of_home(home: &Path, changed: mpsc::Sender<()>) -> notify::Result<Self> {
        let mut watcher =
            notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
                // A failed or overflowed watch may have missed a change, so it
                // counts as one.
                let concerns_a_daemon = event.map_or(true, |event| {
                    event.need_rescan()
                        || event
                            .paths
                            .iter()
                            .any(|path| path.file_name().is_some_and(names_a_daemons_file))
                });
                if concerns_a_daemon {
                    // Full: a change is already pending.
                    let _ = changed.try_send(());
                }
            })?;
        watcher.watch(home, RecursiveMode::NonRecursive)?;
        Ok(Self {
            watcher,
            folders: BTreeSet::new(),
        })
    }
}
