//! The Browser cache's folder on disk (ADR 0098), read and written only on
//! the tasks module's blocking policy:
//!
//! ```text
//! <root>/<kind>/<name>/<build>/   one entry: entry.json, lock, files/
//! <root>/.partial/<unique>/       an install in progress
//! <root>/.deleting/<unique>/      a deletion in progress, finished by the next clean-up
//! <root>/.clean-up                when the automatic clean-up last ran
//! ```
//!
//! A launched browser holds a shared lock on its entry's `lock` file; a
//! deletion takes it exclusively without waiting, renames the entry into
//! `.deleting/`, then deletes it.
#![expect(
    clippy::disallowed_methods,
    reason = "the cache's files are read and written on the tasks module's blocking policy, never on an async worker"
)]

use std::fs::{self, File, TryLockError};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, BrowserCacheEntry};
use goodfetch_protocol::formats::{BROWSER_CACHE, Compatibility};

const ENTRY: &str = "entry.json";
const LOCK: &str = "lock";
const FILES: &str = "files";
const PARTIAL: &str = ".partial";
const DELETING: &str = ".deleting";
const CLEAN_UP_MARKER: &str = ".clean-up";

/// Every kind this engine knows, whose folders it reads.
pub(super) const KINDS: [ArtifactKind; 6] = [
    ArtifactKind::Browser,
    ArtifactKind::Driver,
    ArtifactKind::ExecutableCopy,
    ArtifactKind::EngineExecutable,
    ArtifactKind::CleanUpRules,
    ArtifactKind::MouseMovements,
];

/// Whether `segment` may name a folder of the cache: a name or a build.
pub(super) fn is_segment(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.starts_with('.')
        && segment
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b'+'))
}

/// Writes `files`, each a relative path with its bytes, under `folder`.
///
/// # Errors
///
/// When a path leaves `folder`, or a file cannot be written.
pub(super) fn write_files(folder: &Path, files: &[(String, Vec<u8>)]) -> io::Result<()> {
    for (path, bytes) in files {
        if !path.split('/').all(is_segment) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("`{path}` is no path inside an artifact"),
            ));
        }
        let file = folder.join(path);
        if let Some(parent) = file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(file, bytes)?;
    }
    Ok(())
}

/// The folder of `id`'s entry.
pub(super) fn entry_folder(root: &Path, id: &ArtifactId) -> PathBuf {
    root.join(id.kind.as_str()).join(&id.name).join(&id.build)
}

/// The folder of an entry's files.
pub(super) fn files(folder: &Path) -> PathBuf {
    folder.join(FILES)
}

/// What an entry's folder holds, as this engine reads it.
pub(super) enum Read {
    /// An entry it can read, of the artifact its path names.
    Entry(BrowserCacheEntry),
    /// Nothing at that path.
    Absent,
    /// An entry it cannot read: in a newer format, malformed, or naming
    /// another artifact than its path. It is never deleted.
    Unreadable,
}

/// The entry in `folder`, which should hold `id`.
pub(super) fn read_entry(folder: &Path, id: &ArtifactId) -> Read {
    let bytes = match fs::read(folder.join(ENTRY)) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == io::ErrorKind::NotFound && !folder.exists() => {
            return Read::Absent;
        }
        Err(_) => return Read::Unreadable,
    };
    let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return Read::Unreadable;
    };
    // The version decides first, so an entry of a newer minor with fields
    // this engine does not know is unreadable, not malformed.
    let readable = value
        .get("format_version")
        .and_then(|version| serde_json::from_value(version.clone()).ok())
        .is_some_and(|found| {
            BROWSER_CACHE
                .rule
                .compatibility(BROWSER_CACHE.version, found)
                == Compatibility::Compatible
        });
    if !readable {
        return Read::Unreadable;
    }
    match serde_json::from_value::<BrowserCacheEntry>(value) {
        Ok(entry) if entry.artifact == *id => Read::Entry(entry),
        _ => Read::Unreadable,
    }
}

/// Writes `entry` into `folder`, replacing what was there in one rename.
pub(super) fn write_entry(folder: &Path, entry: &BrowserCacheEntry) -> io::Result<()> {
    let mut bytes = serde_json::to_vec_pretty(entry).map_err(io::Error::other)?;
    bytes.push(b'\n');
    let staged = folder.join(".entry.json.new");
    {
        let mut file = File::create(&staged)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
    }
    fs::rename(staged, folder.join(ENTRY))
}

/// A lock on an entry, released when it drops.
pub(super) struct Lock(
    #[expect(dead_code, reason = "held for its lock, which drops with it")] File,
);

/// How taking a lock went.
pub(super) enum Locking {
    /// Taken.
    Taken(Lock),
    /// Another process holds it in a way that excludes this one.
    Held,
    /// The entry has no lock file, such as when it is gone.
    Missing,
}

fn try_lock(folder: &Path, exclusive: bool) -> io::Result<Locking> {
    let file = match File::open(folder.join(LOCK)) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Locking::Missing),
        Err(error) => return Err(error),
    };
    let taken = if exclusive {
        file.try_lock()
    } else {
        file.try_lock_shared()
    };
    match taken {
        Ok(()) => Ok(Locking::Taken(Lock(file))),
        Err(TryLockError::WouldBlock) => Ok(Locking::Held),
        Err(TryLockError::Error(error)) => Err(error),
    }
}

/// A shared lock on the entry in `folder`, which a launched browser holds.
pub(super) fn lock_shared(folder: &Path) -> io::Result<Locking> {
    try_lock(folder, false)
}

/// The exclusive lock a deletion takes, without waiting.
pub(super) fn lock_exclusive(folder: &Path) -> io::Result<Locking> {
    try_lock(folder, true)
}

/// Whether this engine can write the cache at `root`, which it creates if
/// it can.
pub(super) fn writable(root: &Path) -> bool {
    fs::create_dir_all(root).is_ok()
        && tempfile::Builder::new()
            .prefix(".write-check")
            .tempfile_in(root)
            .is_ok()
}

/// A folder for an install in progress, removed when it drops unless it
/// was moved into place.
pub(super) fn partial(root: &Path) -> io::Result<tempfile::TempDir> {
    let folder = root.join(PARTIAL);
    fs::create_dir_all(&folder)?;
    tempfile::Builder::new()
        .prefix("install")
        .tempdir_in(folder)
}

/// Puts the install in `partial`, whose `files/` holds the artifact's files,
/// in place as `id`'s entry, with its lock file and `entry`. `Ok(false)` when
/// another process put the same entry in place first.
pub(super) fn install(
    root: &Path,
    partial: tempfile::TempDir,
    id: &ArtifactId,
    entry: &BrowserCacheEntry,
) -> io::Result<bool> {
    File::create(partial.path().join(LOCK))?.sync_all()?;
    write_entry(partial.path(), entry)?;
    let folder = entry_folder(root, id);
    if let Some(parent) = folder.parent() {
        fs::create_dir_all(parent)?;
    }
    let staged = partial.keep();
    match fs::rename(&staged, &folder) {
        Ok(()) => Ok(true),
        Err(_) if folder.join(ENTRY).exists() => {
            fs::remove_dir_all(staged)?;
            Ok(false)
        }
        Err(error) => {
            let _ = fs::remove_dir_all(staged);
            Err(error)
        }
    }
}

/// The bytes the files under `folder` take, its subfolders' included;
/// links are counted as themselves, never followed.
pub(super) fn bytes(folder: &Path) -> io::Result<u64> {
    let mut total = 0;
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let metadata = fs::symlink_metadata(entry.path())?;
        total += if metadata.is_dir() {
            bytes(&entry.path())?
        } else {
            metadata.len()
        };
    }
    Ok(total)
}

/// Moves `folder` out of the cache into `.deleting/`, then deletes it. A
/// deletion cut off after the rename is finished by [`finish_deletions`].
pub(super) fn delete(root: &Path, folder: &Path) -> io::Result<()> {
    let deleting = root.join(DELETING);
    fs::create_dir_all(&deleting)?;
    let target = tempfile::Builder::new()
        .prefix("entry")
        .tempdir_in(&deleting)?
        .keep();
    fs::remove_dir(&target)?;
    fs::rename(folder, &target)?;
    fs::remove_dir_all(target)
}

/// Finishes every deletion an earlier clean-up was cut off in.
pub(super) fn finish_deletions(root: &Path) -> io::Result<()> {
    let deleting = root.join(DELETING);
    let entries = match fs::read_dir(&deleting) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            fs::remove_dir_all(path)?;
        } else {
            fs::remove_file(path)?;
        }
    }
    Ok(())
}

/// Every entry folder of the kinds this engine knows: `(kind, name, build,
/// folder)`. Folders of other kinds are never listed, so never deleted.
pub(super) fn entry_folders(root: &Path) -> io::Result<Vec<(ArtifactId, PathBuf)>> {
    let mut found = Vec::new();
    for kind in KINDS {
        let kind_folder = root.join(kind.as_str());
        for name in subfolders(&kind_folder)? {
            for build in subfolders(&kind_folder.join(&name))? {
                let folder = kind_folder.join(&name).join(&build);
                found.push((
                    ArtifactId {
                        kind,
                        name: name.clone(),
                        build,
                    },
                    folder,
                ));
            }
        }
    }
    found.sort();
    Ok(found)
}

fn subfolders(folder: &Path) -> io::Result<Vec<String>> {
    let entries = match fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error),
    };
    let mut names = Vec::new();
    for entry in entries {
        let entry = entry?;
        if entry.file_type()?.is_dir()
            && let Some(name) = entry.file_name().to_str()
            && is_segment(name)
        {
            names.push(name.to_owned());
        }
    }
    names.sort();
    Ok(names)
}

/// When the automatic clean-up last ran, if this engine can read it.
pub(super) fn last_clean_up(root: &Path) -> Option<jiff::Timestamp> {
    fs::read_to_string(root.join(CLEAN_UP_MARKER))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// Records that the automatic clean-up ran at `now`.
pub(super) fn record_clean_up(root: &Path, now: jiff::Timestamp) -> io::Result<()> {
    fs::write(root.join(CLEAN_UP_MARKER), format!("{now}\n"))
}
