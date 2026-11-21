//! The storage levels (ADR 0122): a filesystem's level, a pure function of
//! what it reads and the two `storage.*` settings, and the governor's
//! storage sample ([`disks`]), which reads every filesystem holding a folder
//! it samples once, however many of those folders it holds.

use goodfetch_protocol::catalog::Setting;
use goodfetch_protocol::settings::{SettingLimit, SettingsLayer};
use goodfetch_protocol::storage::{CRITICAL_FREE, LOW_FREE, StorageLevel};

use crate::settings::Resolved;

mod disks;

pub(crate) use disks::{Disks, Folders};
pub use disks::{Filesystem, FilesystemSource, Held, StorageSample};

/// What one filesystem reads: its size and the space left to the engine's
/// user, and its inodes where it reports them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesystemReading {
    /// Its size, in bytes.
    pub size: u64,
    /// The space left to the engine's user, in bytes.
    pub free: u64,
    /// Its inodes, where it reports them.
    pub inodes: Option<FilesystemInodes>,
}

/// A filesystem's inodes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct FilesystemInodes {
    /// How many it has.
    pub total: u64,
    /// How many are left to the engine's user.
    pub free: u64,
}

/// `storage.low_free` and `storage.critical_free`, none when unset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Thresholds {
    pub(crate) low_free: Option<u64>,
    pub(crate) critical_free: Option<u64>,
}

/// `low`'s built-in size: the smaller of 5 GiB and 10 % of the
/// filesystem.
const LOW: BuiltIn = BuiltIn {
    bytes: 5 << 30,
    percent: 10,
};

/// `critical`'s built-in size: the smaller of 1 GiB and 2 % of the
/// filesystem.
const CRITICAL: BuiltIn = BuiltIn {
    bytes: 1 << 30,
    percent: 2,
};

/// A level's built-in size.
struct BuiltIn {
    bytes: u64,
    percent: u64,
}

impl BuiltIn {
    /// Its size on a filesystem of `size` bytes.
    fn on(&self, size: u64) -> u64 {
        self.bytes.min(share(size, self.percent))
    }
}

/// `percent` of `total`, rounded down.
fn share(total: u64, percent: u64) -> u64 {
    u64::try_from(u128::from(total) * u128::from(percent) / 100).unwrap_or(u64::MAX)
}

impl Thresholds {
    /// The free space below which a filesystem of `size` bytes is at
    /// `level`, `low` or `critical`; none when the level is off, or for
    /// `normal`.
    pub(crate) fn below(&self, level: StorageLevel, size: u64) -> Option<u64> {
        let (set, default) = match level {
            StorageLevel::Normal => return None,
            StorageLevel::Low => (self.low_free, &LOW),
            StorageLevel::Critical => (self.critical_free, &CRITICAL),
        };
        match set {
            Some(0) => None,
            Some(bytes) => Some(bytes),
            None => Some(default.on(size)),
        }
    }

    /// Whether a filesystem that reads `reading` is at `level` or worse:
    /// its free space below the level's size, or its free inodes below
    /// the level's percentage where it reports them.
    fn reached(&self, level: StorageLevel, reading: &FilesystemReading) -> bool {
        let Some(bytes) = self.below(level, reading.size) else {
            return false;
        };
        let percent = match level {
            StorageLevel::Normal => return false,
            StorageLevel::Low => LOW.percent,
            StorageLevel::Critical => CRITICAL.percent,
        };
        reading.free < bytes
            || reading
                .inodes
                .is_some_and(|inodes| inodes.free < share(inodes.total, percent))
    }
}

/// The level of a filesystem that reads `reading` under `thresholds`: the
/// worst it reached, with no memory of an earlier one, so a level drops
/// back at the first reading that finds the space again.
pub(crate) fn level(reading: &FilesystemReading, thresholds: &Thresholds) -> StorageLevel {
    [StorageLevel::Critical, StorageLevel::Low]
        .into_iter()
        .find(|level| thresholds.reached(*level, reading))
        .unwrap_or(StorageLevel::Normal)
}

/// The two settings as the governor read them at a sample, with the layer
/// that set each.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Settled {
    pub(crate) thresholds: Thresholds,
    low_layer: SettingsLayer,
    critical_layer: SettingsLayer,
}

impl Settled {
    /// The two settings of `settings`.
    pub(crate) fn of(settings: &Resolved) -> Self {
        let layer = |setting: &Setting| {
            settings
                .effective(setting)
                .filter(|effective| !effective.value.is_null())
                .map_or(SettingsLayer::Default, |effective| effective.layer)
        };
        Self {
            thresholds: Thresholds {
                low_free: settings.get(&LOW_FREE),
                critical_free: settings.get(&CRITICAL_FREE),
            },
            low_layer: layer(&LOW_FREE),
            critical_layer: layer(&CRITICAL_FREE),
        }
    }

    /// Both built-in values, before the governor read any setting.
    pub(crate) fn built_in() -> Self {
        Self {
            thresholds: Thresholds {
                low_free: None,
                critical_free: None,
            },
            low_layer: SettingsLayer::Default,
            critical_layer: SettingsLayer::Default,
        }
    }

    /// The setting that sets `level` on a filesystem of `size` bytes, with
    /// the size it is in force at; none for `normal` or a level off.
    fn limit(&self, level: StorageLevel, size: u64) -> Option<SettingLimit> {
        let (key, layer) = match level {
            StorageLevel::Normal => return None,
            StorageLevel::Low => (LOW_FREE.name, self.low_layer),
            StorageLevel::Critical => (CRITICAL_FREE.name, self.critical_layer),
        };
        let bytes = self.thresholds.below(level, size)?;
        Some(SettingLimit {
            key: key.to_owned(),
            value: serde_json::json!(bytes),
            layer,
            origin: None,
        })
    }

    /// What the filesystem that reads `reading` and holds `folders` is at.
    fn filesystem(
        &self,
        folders: Vec<std::path::PathBuf>,
        reading: &FilesystemReading,
    ) -> Filesystem {
        let level = level(reading, &self.thresholds);
        Filesystem {
            folders,
            level,
            free: reading.free,
            free_inodes: reading.inodes.map(|inodes| inodes.free),
            limit: self.limit(level, reading.size),
        }
    }
}

#[cfg(test)]
mod tests;
