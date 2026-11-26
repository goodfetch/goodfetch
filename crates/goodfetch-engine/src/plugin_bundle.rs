//! Unpacking a Plugin's bundle (ADR 0033, issue 889): the tar archive of
//! its OCI artifact's one layer, unpacked into a new, empty folder that the
//! Plugin store then moves into place, without running anything.
//!
//! The archive is input Goodfetch did not write (ADR 0067), so the unpack
//! makes only regular files, folders and links, each under the folder:
//!
//! - every entry's path is relative, UTF-8 and safe on every OS
//!   ([`check_path`]), and no entry is written twice or below a file or a
//!   link of an earlier entry;
//! - a symbolic link's target, read from its own folder, stays inside the
//!   bundle and passes through no other link, checked once every entry is
//!   known and before any link is made; a hard link names a file an
//!   earlier entry made;
//! - a device, a FIFO, a sparse file or any other kind of entry is refused;
//! - a file is created new, never through what is there, with the
//!   permissions `0644`, or `0755` when the archive marks it executable:
//!   no setuid, setgid or sticky bit, owner, time or extended attribute
//!   comes from the archive;
//! - at most [`ENTRY_LIMIT`] entries and [`BYTE_LIMIT`] bytes, read
//!   through a reader that stops there, so no archive fills the disk.
//!
//! Nothing it makes runs: an install script, a `node_modules` or a
//! virtualenv in the bundle are files like the others.
//!
//! It writes the file system, so it blocks: call it from the tasks module's
//! blocking policy, never on an async worker.
#![expect(
    clippy::disallowed_methods,
    reason = "the bundle is unpacked on the tasks module's blocking policy, never on an async worker"
)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use goodfetch_protocol::hostile_input::PathHazard;
use tar::EntryType;

use crate::hostile_input::{PathShape, check_path};
use crate::plugin_oci::Compression;

/// The most entries a bundle may hold.
pub const ENTRY_LIMIT: u64 = 65_536;

/// The most bytes a bundle's archive may unpack to, its headers included.
pub const BYTE_LIMIT: u64 = 1024 * 1024 * 1024;

/// Why a bundle was not unpacked.
#[derive(Debug, thiserror::Error)]
pub enum BundleError {
    /// The archive does not read as a tar archive, or its gzip stream
    /// breaks.
    #[error("the Plugin's bundle is no tar archive the engine reads: {0}")]
    Archive(io::Error),
    /// An entry the unpack refuses.
    #[error("the Plugin's bundle holds the entry `{path}`, which is refused: {refusal}")]
    Entry {
        /// The entry's path, as the archive gives it.
        path: String,
        /// Why it is refused.
        refusal: Refusal,
    },
    /// The archive holds more than [`ENTRY_LIMIT`] entries.
    #[error("the Plugin's bundle holds more than {ENTRY_LIMIT} entries")]
    TooManyEntries,
    /// The archive unpacks to more than [`BYTE_LIMIT`] bytes.
    #[error("the Plugin's bundle unpacks to more than {BYTE_LIMIT} bytes")]
    TooLarge,
    /// The folder it unpacks into could not be written.
    #[error("the Plugin's bundle could not be written: {0}")]
    Write(io::Error),
}

/// Why an entry is refused.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Refusal {
    /// Its path or its link's target is no UTF-8, or holds a backslash,
    /// which names a folder on Windows only.
    #[error("its path is no portable `/`-separated UTF-8 text")]
    Unportable,
    /// Its path is unsafe on some OS: it climbs out, is absolute, or holds
    /// a reserved name or a control character.
    #[error("its path is unsafe ({0:?})")]
    Unsafe(PathHazard),
    /// It is a kind of entry the unpack does not make.
    #[error("it is a {0}, where a bundle holds files, folders and links")]
    Kind(String),
    /// An earlier entry made the same path.
    #[error("an earlier entry made the same path")]
    Duplicate,
    /// It is below a file or a link an earlier entry made.
    #[error("it is below a file or a link of the bundle")]
    BelowFileOrLink,
    /// Its link's target leaves the bundle.
    #[error("its link's target `{0}` leaves the bundle")]
    LinkLeaves(String),
    /// Its link's target passes through another link.
    #[error("its link's target `{0}` passes through another link")]
    LinkThroughLink(String),
    /// Its hard link names no file an earlier entry made.
    #[error("its hard link names `{0}`, no file of an earlier entry")]
    HardLinkTarget(String),
    /// It is a symbolic link, which the unpack makes on Unix only.
    #[error("it is a symbolic link, which a bundle holds on Unix only")]
    LinkUnsupported,
}

/// What an earlier entry made at a path.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Made {
    Folder,
    File,
    Link,
}

/// Unpacks the bundle `archive`, a tar archive stored with `compression`,
/// into the folder `into`, which exists and is empty.
///
/// # Errors
///
/// [`BundleError`] when the archive does not read, holds an entry the
/// unpack refuses or more than its limits, or `into` cannot be written;
/// `into` then holds part of the bundle, for its caller to remove.
pub fn unpack(
    archive: impl Read,
    compression: Compression,
    into: &Path,
) -> Result<(), BundleError> {
    let decoded: Box<dyn Read> = match compression {
        Compression::None => Box::new(archive),
        Compression::Gzip => Box::new(flate2::read::GzDecoder::new(archive)),
    };
    let mut tar = tar::Archive::new(decoded.take(BYTE_LIMIT));
    let mut unpacking = Unpacking {
        into,
        made: BTreeMap::new(),
        links: Vec::new(),
        bytes: 0,
    };
    let entries = tar.entries().map_err(BundleError::Archive)?;
    for (count, entry) in (1..).zip(entries) {
        if count > ENTRY_LIMIT {
            return Err(BundleError::TooManyEntries);
        }
        unpacking.entry(&mut entry.map_err(BundleError::Archive)?)?;
    }
    unpacking.make_links()
}

/// An unpack in progress.
struct Unpacking<'a> {
    into: &'a Path,
    /// Each path an entry made, or a folder made to hold one, as its
    /// `/`-separated parts.
    made: BTreeMap<Vec<String>, Made>,
    /// The symbolic links to make once every entry is known: each one's
    /// path and its target, as the archive gives it.
    links: Vec<(Vec<String>, String)>,
    bytes: u64,
}

impl Unpacking<'_> {
    /// Unpacks `entry`, or records it when it is a symbolic link.
    fn entry<R: Read>(&mut self, entry: &mut tar::Entry<'_, R>) -> Result<(), BundleError> {
        let shown = String::from_utf8_lossy(&entry.path_bytes()).into_owned();
        let refused = |refusal| BundleError::Entry {
            path: shown.clone(),
            refusal,
        };
        let kind = entry.header().entry_type();
        if kind == EntryType::XGlobalHeader {
            return Ok(());
        }
        let Some(parts) = parts_of(&entry.path_bytes()).map_err(refused)? else {
            return if kind == EntryType::Directory {
                Ok(())
            } else {
                Err(refused(Refusal::Unsafe(PathHazard::DotName)))
            };
        };
        self.below_folders(&parts).map_err(refused)?;
        self.make_folders(&parts).map_err(BundleError::Write)?;
        let previous = self.made.get(&parts).copied();
        match (previous, kind) {
            (Some(Made::Folder), EntryType::Directory) => return Ok(()),
            (Some(_), _) => return Err(refused(Refusal::Duplicate)),
            (None, _) => {}
        }
        let path = self.path(&parts);
        match kind {
            EntryType::Directory => {
                fs::create_dir(&path).map_err(BundleError::Write)?;
                self.made.insert(parts, Made::Folder);
            }
            EntryType::Regular | EntryType::Continuous => {
                self.bytes = self.bytes.saturating_add(entry.size());
                if self.bytes > BYTE_LIMIT {
                    return Err(BundleError::TooLarge);
                }
                let executable = entry.header().mode().is_ok_and(|mode| mode & 0o111 != 0);
                let mut file = create_new(&path, executable).map_err(BundleError::Write)?;
                io::copy(entry, &mut file).map_err(BundleError::Archive)?;
                self.made.insert(parts, Made::File);
            }
            EntryType::Link => {
                let target = entry
                    .link_name_bytes()
                    .ok_or_else(|| refused(Refusal::HardLinkTarget(String::new())))?;
                let shown_target = String::from_utf8_lossy(&target).into_owned();
                let target_parts = parts_of(&target)
                    .map_err(refused)?
                    .filter(|target_parts| self.made.get(target_parts) == Some(&Made::File))
                    .ok_or_else(|| refused(Refusal::HardLinkTarget(shown_target)))?;
                fs::hard_link(self.path(&target_parts), &path).map_err(BundleError::Write)?;
                self.made.insert(parts, Made::File);
            }
            EntryType::Symlink if cfg!(unix) => {
                let target = entry
                    .link_name_bytes()
                    .map(|target| String::from_utf8(target.into_owned()))
                    .and_then(Result::ok)
                    .filter(|target| !target.contains('\\'))
                    .ok_or_else(|| refused(Refusal::Unportable))?;
                self.made.insert(parts.clone(), Made::Link);
                self.links.push((parts, target));
            }
            EntryType::Symlink => return Err(refused(Refusal::LinkUnsupported)),
            other => return Err(refused(Refusal::Kind(kind_name(other)))),
        }
        Ok(())
    }

    /// Refuses `parts` below a file or a link an earlier entry made.
    fn below_folders(&self, parts: &[String]) -> Result<(), Refusal> {
        let below_file_or_link = (1..parts.len()).any(|depth| {
            matches!(
                self.made.get(&parts[..depth]),
                Some(Made::File | Made::Link)
            )
        });
        if below_file_or_link {
            return Err(Refusal::BelowFileOrLink);
        }
        Ok(())
    }

    /// Makes the folders that hold `parts` and no entry made yet.
    fn make_folders(&mut self, parts: &[String]) -> io::Result<()> {
        for depth in 1..parts.len() {
            let folder = &parts[..depth];
            if !self.made.contains_key(folder) {
                fs::create_dir(self.path(folder))?;
                self.made.insert(folder.to_vec(), Made::Folder);
            }
        }
        Ok(())
    }

    /// Checks every symbolic link's target, then makes them all.
    fn make_links(self) -> Result<(), BundleError> {
        for (parts, target) in &self.links {
            resolve_link(parts, target, &self.made).map_err(|refusal| BundleError::Entry {
                path: parts.join("/"),
                refusal,
            })?;
        }
        for (parts, target) in &self.links {
            symlink(target, &self.path(parts)).map_err(BundleError::Write)?;
        }
        Ok(())
    }

    /// The path of `parts` under the folder.
    fn path(&self, parts: &[String]) -> PathBuf {
        parts
            .iter()
            .fold(self.into.to_owned(), |path, part| path.join(part))
    }
}

/// The parts of `raw`, an entry's path: `None` for the bundle's own
/// folder, `.` or `./`. A leading `./` and a trailing `/` are dropped.
fn parts_of(raw: &[u8]) -> Result<Option<Vec<String>>, Refusal> {
    let text = std::str::from_utf8(raw).map_err(|_| Refusal::Unportable)?;
    if text.contains('\\') {
        return Err(Refusal::Unportable);
    }
    let mut trimmed = text;
    while let Some(rest) = trimmed.strip_prefix("./") {
        trimmed = rest;
    }
    let trimmed = trimmed.strip_suffix('/').unwrap_or(trimmed);
    if trimmed.is_empty() || trimmed == "." {
        return Ok(None);
    }
    check_path(trimmed, PathShape::RelativePath)
        .map_err(|unsafe_path| Refusal::Unsafe(unsafe_path.hazard))?;
    Ok(Some(trimmed.split('/').map(str::to_owned).collect()))
}

/// Resolves `target`, the target of the link at `parts`, from the link's
/// folder: it must stay inside the bundle and pass through no link that
/// `made` holds, though it may end at one.
fn resolve_link(
    parts: &[String],
    target: &str,
    made: &BTreeMap<Vec<String>, Made>,
) -> Result<(), Refusal> {
    if target.is_empty() || target.starts_with('/') {
        return Err(Refusal::LinkLeaves(target.to_owned()));
    }
    let mut cursor = parts[..parts.len() - 1].to_vec();
    let steps: Vec<&str> = target
        .split('/')
        .filter(|step| !step.is_empty() && *step != ".")
        .collect();
    for (index, step) in steps.iter().enumerate() {
        if *step == ".." {
            if cursor.pop().is_none() {
                return Err(Refusal::LinkLeaves(target.to_owned()));
            }
            continue;
        }
        check_path(step, PathShape::Name)
            .map_err(|unsafe_path| Refusal::Unsafe(unsafe_path.hazard))?;
        cursor.push((*step).to_owned());
        let last = index + 1 == steps.len();
        if !last && made.get(&cursor) == Some(&Made::Link) {
            return Err(Refusal::LinkThroughLink(target.to_owned()));
        }
    }
    if cursor.is_empty() {
        return Err(Refusal::LinkLeaves(target.to_owned()));
    }
    Ok(())
}

/// Creates the new file `path`, executable when `executable`.
fn create_new(path: &Path, executable: bool) -> io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt as _;
        options.mode(if executable { 0o755 } else { 0o644 });
    }
    #[cfg(not(unix))]
    let _ = executable;
    options.open(path)
}

/// Makes the symbolic link `path` to `target`.
#[cfg(unix)]
fn symlink(target: &str, path: &Path) -> io::Result<()> {
    std::os::unix::fs::symlink(target, path)
}

/// No symbolic link is recorded off Unix: [`Refusal::LinkUnsupported`].
#[cfg(not(unix))]
fn symlink(_target: &str, _path: &Path) -> io::Result<()> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}

/// What a refusal calls an entry of `kind`.
fn kind_name(kind: EntryType) -> String {
    match kind {
        EntryType::Char => "character device".to_owned(),
        EntryType::Block => "block device".to_owned(),
        EntryType::Fifo => "FIFO".to_owned(),
        EntryType::GNUSparse => "sparse file".to_owned(),
        other => format!(
            "tar entry of type `{}`",
            char::from(other.as_byte()).escape_default()
        ),
    }
}

/// The fuzz target's body for Plugin bundles (ADR 0067): unpacks `data`, a
/// gzip stream when it starts as one and a tar archive otherwise, into a
/// new folder; whatever it unpacks stays in that folder and every link it
/// makes resolves inside it. It may refuse `data`, never panic.
///
/// # Panics
///
/// When an unpack writes outside its folder or makes a link that leaves
/// it, or no temporary folder can be made.
pub fn fuzz_plugin_bundle(data: &[u8]) {
    let outer = tempfile::tempdir().expect("invariant: a temporary folder");
    let into = outer.path().join("bundle");
    fs::create_dir(&into).expect("invariant: a new folder");
    let compression = if data.starts_with(&[0x1f, 0x8b]) {
        Compression::Gzip
    } else {
        Compression::None
    };
    let unpacked = unpack(data, compression, &into);
    let beside: Vec<_> = fs::read_dir(outer.path())
        .expect("invariant: the temporary folder lists")
        .collect();
    assert_eq!(beside.len(), 1, "an unpack wrote beside its folder");
    if unpacked.is_ok() {
        let root = fs::canonicalize(&into).expect("invariant: the folder resolves");
        assert_inside(&into, &root);
    }
}

/// Asserts that every link below `folder` that resolves, resolves below
/// `root`.
fn assert_inside(folder: &Path, root: &Path) {
    for entry in fs::read_dir(folder).expect("invariant: an unpacked folder lists") {
        let path = entry.expect("invariant: an unpacked entry").path();
        let metadata = fs::symlink_metadata(&path).expect("invariant: an unpacked entry");
        if metadata.is_symlink() {
            if let Ok(resolved) = fs::canonicalize(&path) {
                assert!(
                    resolved.starts_with(root),
                    "a link leaves the bundle: {path:?}"
                );
            }
        } else if metadata.is_dir() {
            assert_inside(&path, root);
        }
    }
}

#[cfg(test)]
mod tests;
