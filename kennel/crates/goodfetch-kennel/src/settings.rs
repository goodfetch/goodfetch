//! The Kennel's own settings (ADR 0141), in a TOML file of their own in the
//! Goodfetch home, `kennel.toml`, never in the engine's configuration. They
//! are read at launch and again on each change of the file, which the
//! platform's file notifications report. A missing file gives the
//! defaults; a file that cannot be read gives the defaults and says why.
//! Each later feature adds its switch here, with a free off state.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::home::{Home, canonical, read_if_present};
use crate::tasks::Tasks;

/// The Kennel's settings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Settings {
    /// Whether the menu offers actions; off gives a read-only Kennel.
    pub controls: bool,
    /// Whether a Run's Stop asks for confirmation first; Abort and Retry
    /// failed always ask, Pause never does.
    pub confirm_stop: bool,
    /// Whether the Kennel, while connected, keeps the Daemon alive: off, it
    /// only observes, and the Daemon stops at its idle timeout and hands
    /// over to its major's pick as if no Kennel were connected (ADR 0141).
    pub keep_daemon_alive: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            controls: true,
            confirm_stop: true,
            keep_daemon_alive: false,
        }
    }
}

/// Why the settings file gave the defaults instead.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum SettingsError {
    /// The file could not be read or is not the settings' TOML; the text
    /// says where, for the person who edits it.
    #[error("{0}")]
    Unreadable(String),
}

/// The settings in the file at `path`, or the defaults when it does not
/// exist. Blocks.
pub fn read(path: &Path) -> Result<Settings, SettingsError> {
    let bytes =
        read_if_present(path).map_err(|error| SettingsError::Unreadable(error.to_string()))?;
    let Some(bytes) = bytes else {
        return Ok(Settings::default());
    };
    let text =
        String::from_utf8(bytes).map_err(|error| SettingsError::Unreadable(error.to_string()))?;
    toml::from_str(&text).map_err(|error| SettingsError::Unreadable(error.message().to_owned()))
}

/// Reports the home's settings at once, then again each time their file
/// changes, until the Kennel stops. When the platform cannot watch the
/// home, the launch's settings stay, and a warning says so.
///
/// # Cancel safety
/// Dropping the future stops the watch; nothing else is lost.
pub async fn follow(
    tasks: Tasks,
    home: Home,
    report: impl Fn(Result<Settings, SettingsError>) + Send + Sync + 'static,
) {
    let path = Arc::new(home.settings_file());
    let read_now = |path: Arc<PathBuf>| tasks.blocking(move || read(&path));
    // One pending change is enough: the read that follows sees them all.
    let (changed, mut pending) = mpsc::channel(1);
    // FSEvents names changed files by their canonical path (`/private/var`
    // for `/var` on macOS), and a non-recursive watch keeps only the events
    // of the folder as named, so the folder is watched by its canonical
    // path. The watch starts before the first read, so no change after
    // that read is missed.
    let folder = home.path().to_owned();
    let folder = tasks.blocking(move || canonical(&folder)).await;
    let watcher = watch(&folder, Arc::clone(&path), changed);
    report(read_now(Arc::clone(&path)).await);
    if let Err(error) = &watcher {
        tracing::warn!(%error, "the settings file is not watched, so a change applies at the next launch");
        return;
    }
    let stop = tasks.stopping();
    loop {
        tokio::select! {
            () = stop.cancelled() => return,
            change = pending.recv() => {
                if change.is_none() {
                    return;
                }
                report(read_now(Arc::clone(&path)).await);
            }
        }
    }
}

/// Watches `folder` for changes to the file at `path`: an editor may
/// replace the file by rename, so the folder is watched, not the file.
fn watch(
    folder: &Path,
    path: Arc<PathBuf>,
    changed: mpsc::Sender<()>,
) -> notify::Result<RecommendedWatcher> {
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let concerns_the_file = event.is_ok_and(|event| {
            event
                .paths
                .iter()
                .any(|changed| changed.file_name() == path.file_name())
        });
        if concerns_the_file {
            // Full: a change is already pending.
            let _ = changed.try_send(());
        }
    })?;
    watcher.watch(folder, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}
