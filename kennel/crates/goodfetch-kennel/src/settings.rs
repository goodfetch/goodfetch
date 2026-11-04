//! The Kennel's own settings (ADR 0141), in a TOML file of their own in the
//! Goodfetch home, `kennel.toml`, never in the engine's configuration. They
//! are read at launch and again on each change of the file, which the
//! platform's file notifications report. A missing file gives the
//! defaults; a file that cannot be read gives the defaults and says why.
//! Each later feature adds its switch here, with a free off state.
//!
//! The window's Kennel settings view changes them through [`edit`], which
//! rewrites only the key it changes, keeping the file's other keys and its
//! comments as written; the watch then reports the change like any other.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;
use tokio::sync::mpsc;

use crate::home::{self, Home, canonical, read_if_present};
use crate::tasks::Tasks;

/// The Kennel's settings.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each feature is a switch of its own with a free off state, as `kennel.toml` spells it (ADR 0141)"
)]
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
    /// Whether the menu offers "Open details", the details window; off
    /// gives a menu-only Kennel.
    pub window: bool,
    /// Whether the Kennel starts when the person logs in, through a
    /// launchd user agent that opens the installed `.app` (macOS).
    pub start_at_login: bool,
    /// Whether the Kennel checks once at launch that its icon shows in the
    /// menu bar, and opens the details window with a note when it is
    /// hidden behind the notch or in the overflow (macOS).
    pub hidden_icon_check: bool,
    /// The desktop notices, the `[notifications]` table.
    pub notifications: Notifications,
    /// The details window's views, each a switch of its own: `[views]` in
    /// the file.
    pub views: Views,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            controls: true,
            confirm_stop: true,
            keep_daemon_alive: false,
            window: true,
            start_at_login: false,
            hidden_icon_check: true,
            notifications: Notifications::default(),
            views: Views::default(),
        }
    }
}

/// The desktop notices (ADR 0141): a switch for them all, one per type,
/// and the period over which a burst coalesces.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each is a switch a person writes in the settings file as a TOML boolean"
)]
pub struct Notifications {
    /// Whether the Kennel sends desktop notices at all: off, no notice
    /// code runs, and macOS is never asked for the permission.
    pub enabled: bool,
    /// A new Needs you item.
    pub needs_you: bool,
    /// A Run failed.
    pub run_failed: bool,
    /// A Run was stopped by a Limit.
    pub run_stopped_by_limit: bool,
    /// The Daemon was lost while a Run was live.
    pub daemon_lost: bool,
    /// A Run ended well.
    pub run_finished: bool,
    /// The minutes over which a burst coalesces: the first notice goes
    /// out at once, then one summary per period; 0 sends each at once.
    pub period_minutes: u32,
}

impl Notifications {
    /// The period over which a burst coalesces.
    pub fn period(&self) -> Duration {
        Duration::from_secs(u64::from(self.period_minutes) * 60)
    }
}

impl Default for Notifications {
    fn default() -> Self {
        Self {
            enabled: true,
            needs_you: true,
            run_failed: true,
            run_stopped_by_limit: true,
            daemon_lost: true,
            run_finished: false,
            period_minutes: 20,
        }
    }
}

/// The details window's views beside its Runs table, which `window`
/// switches: each one off is neither shown nor read.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "each view is a switch of its own with a free off state, as `kennel.toml` spells it (ADR 0141)"
)]
pub struct Views {
    /// A Run's Records, through `dataset.query`, on a Foxhound Daemon.
    pub records: bool,
    /// The Daemon's settings in force and where each comes from, through
    /// `config.explain`.
    pub daemon_settings: bool,
    /// The Resource governor's readings, `engine.system_info`.
    pub governor: bool,
    /// The Kennel's own settings, which the view edits.
    pub kennel_settings: bool,
}

impl Default for Views {
    fn default() -> Self {
        Self {
            records: true,
            daemon_settings: true,
            governor: true,
            kennel_settings: true,
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
    /// The edited file could not be written; the text says why.
    #[error("{0}")]
    Unwritable(String),
}

/// A switch of the settings, which the Kennel settings view turns on or
/// off.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Switch {
    /// `controls`.
    Controls,
    /// `confirm_stop`.
    ConfirmStop,
    /// `window`.
    Window,
    /// `keep_daemon_alive`.
    KeepDaemonAlive,
    /// `start_at_login`.
    StartAtLogin,
    /// `hidden_icon_check`.
    HiddenIconCheck,
    /// `notifications.enabled`.
    Notices,
    /// `notifications.needs_you`.
    NeedsYouNotice,
    /// `notifications.run_failed`.
    RunFailedNotice,
    /// `notifications.run_stopped_by_limit`.
    RunStoppedByLimitNotice,
    /// `notifications.daemon_lost`.
    DaemonLostNotice,
    /// `notifications.run_finished`.
    RunFinishedNotice,
    /// `views.records`.
    RecordsView,
    /// `views.daemon_settings`.
    DaemonSettingsView,
    /// `views.governor`.
    GovernorView,
    /// `views.kennel_settings`.
    KennelSettingsView,
}

impl Switch {
    /// Every switch, in the order the Kennel settings view lists them.
    pub const ALL: [Self; 16] = [
        Self::Controls,
        Self::ConfirmStop,
        Self::Window,
        Self::KeepDaemonAlive,
        Self::StartAtLogin,
        Self::HiddenIconCheck,
        Self::Notices,
        Self::NeedsYouNotice,
        Self::RunFailedNotice,
        Self::RunStoppedByLimitNotice,
        Self::DaemonLostNotice,
        Self::RunFinishedNotice,
        Self::RecordsView,
        Self::DaemonSettingsView,
        Self::GovernorView,
        Self::KennelSettingsView,
    ];

    /// Whether `settings` turn it on.
    pub fn of(self, settings: &Settings) -> bool {
        let notices = &settings.notifications;
        match self {
            Self::Controls => settings.controls,
            Self::ConfirmStop => settings.confirm_stop,
            Self::Window => settings.window,
            Self::KeepDaemonAlive => settings.keep_daemon_alive,
            Self::StartAtLogin => settings.start_at_login,
            Self::HiddenIconCheck => settings.hidden_icon_check,
            Self::Notices => notices.enabled,
            Self::NeedsYouNotice => notices.needs_you,
            Self::RunFailedNotice => notices.run_failed,
            Self::RunStoppedByLimitNotice => notices.run_stopped_by_limit,
            Self::DaemonLostNotice => notices.daemon_lost,
            Self::RunFinishedNotice => notices.run_finished,
            Self::RecordsView => settings.views.records,
            Self::DaemonSettingsView => settings.views.daemon_settings,
            Self::GovernorView => settings.views.governor,
            Self::KennelSettingsView => settings.views.kennel_settings,
        }
    }

    /// Its key in the file: its table, if any, and its name.
    fn key(self) -> (Option<&'static str>, &'static str) {
        match self {
            Self::Controls => (None, "controls"),
            Self::ConfirmStop => (None, "confirm_stop"),
            Self::Window => (None, "window"),
            Self::KeepDaemonAlive => (None, "keep_daemon_alive"),
            Self::StartAtLogin => (None, "start_at_login"),
            Self::HiddenIconCheck => (None, "hidden_icon_check"),
            Self::Notices => (Some(NOTIFICATIONS), "enabled"),
            Self::NeedsYouNotice => (Some(NOTIFICATIONS), "needs_you"),
            Self::RunFailedNotice => (Some(NOTIFICATIONS), "run_failed"),
            Self::RunStoppedByLimitNotice => (Some(NOTIFICATIONS), "run_stopped_by_limit"),
            Self::DaemonLostNotice => (Some(NOTIFICATIONS), "daemon_lost"),
            Self::RunFinishedNotice => (Some(NOTIFICATIONS), "run_finished"),
            Self::RecordsView => (Some(VIEWS), "records"),
            Self::DaemonSettingsView => (Some(VIEWS), "daemon_settings"),
            Self::GovernorView => (Some(VIEWS), "governor"),
            Self::KennelSettingsView => (Some(VIEWS), "kennel_settings"),
        }
    }
}

/// The `[notifications]` table's name.
const NOTIFICATIONS: &str = "notifications";
/// The `[views]` table's name.
const VIEWS: &str = "views";

/// A change the Kennel settings view makes to the settings file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    /// Turns the switch on or off.
    Switch(Switch, bool),
    /// Sets the minutes over which a burst of desktop notices coalesces.
    Period(u32),
}

/// Makes `edit` to the settings file at `path`, which it makes when
/// missing: only the key it changes is rewritten, so the file's other
/// keys, even those the Kennel does not know, and its comments stay as
/// written. The file is replaced by rename, never written in place. A
/// file that is not TOML is left as it is. Blocks.
pub fn edit(path: &Path, edit: Edit) -> Result<(), SettingsError> {
    let unreadable = |error: &dyn std::fmt::Display| SettingsError::Unreadable(error.to_string());
    let text = match read_if_present(path).map_err(|error| unreadable(&error))? {
        Some(bytes) => String::from_utf8(bytes).map_err(|error| unreadable(&error))?,
        None => String::new(),
    };
    let mut document: toml_edit::DocumentMut = text.parse().map_err(|error| unreadable(&error))?;
    let ((table, key), value) = match edit {
        Edit::Switch(switch, on) => (switch.key(), toml_edit::value(on)),
        Edit::Period(minutes) => (
            (Some(NOTIFICATIONS), "period_minutes"),
            toml_edit::value(i64::from(minutes)),
        ),
    };
    let keys = match table {
        None => document.as_table_mut() as &mut dyn toml_edit::TableLike,
        Some(table) => document
            .entry(table)
            .or_insert_with(toml_edit::table)
            .as_table_like_mut()
            .ok_or_else(|| SettingsError::Unreadable(format!("`{table}` is not a table")))?,
    };
    keys.insert(key, value);
    home::replace(path, document.to_string().as_bytes())
        .map_err(|error| SettingsError::Unwritable(error.to_string()))
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
