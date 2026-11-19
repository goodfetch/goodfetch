//! The versioned copies of executables a Daemon runs from (ADRs 0024 and
//! 0098): a Product's executable, copied into the Browser cache under the
//! kind `executable_copy`, named by its Product and its Product version and
//! the start of its file's digest, so an update of the package never
//! replaces the file a running Daemon runs from, which Windows forbids. Its
//! entry in the list of installed executables pins it, and the Daemon holds
//! it under a shared lock while it runs, so the clean-up deletes it only
//! once nothing uses it. Read and written by blocking calls, which an
//! executable makes before its runtime starts.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, BrowserCacheEntry};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::connection::ProductVersion;
use goodfetch_protocol::formats::BROWSER_CACHE;

use super::HeldArtifact;
use super::artifact::content_digest;
use super::store::{self, Locking, Read};
use crate::pinned_download::Digest;

/// How many hex digits of the file's digest a copy's build ID carries.
const DIGEST_DIGITS: usize = 16;

/// A versioned copy in the Browser cache.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutableCopy {
    /// Its artifact.
    pub id: ArtifactId,
    /// The copied executable's path, which the Daemon runs.
    pub executable: PathBuf,
}

/// The versioned copy of the executable at `source`, `product`'s at
/// `version`, in the cache at `root`: the one there when its files are
/// whole, otherwise copied there now, installed and used at `now`.
///
/// # Errors
///
/// When the executable cannot be read, or the cache cannot be written.
pub fn place_executable_copy(
    root: &Path,
    source: &Path,
    product: Product,
    version: &ProductVersion,
    now: jiff::Timestamp,
) -> io::Result<ExecutableCopy> {
    let name = source
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "an executable is a file"))?
        .to_owned();
    #[expect(
        clippy::disallowed_methods,
        reason = "an executable places its copy before its runtime starts"
    )]
    let bytes = fs::read(source)?;
    let digest = Digest::of(&bytes).to_string();
    let hex = digest.trim_start_matches("sha256:");
    let id = ArtifactId {
        kind: ArtifactKind::ExecutableCopy,
        name: product.executable_name().to_owned(),
        build: format!("{version}-{}", &hex[..DIGEST_DIGITS.min(hex.len())]),
    };
    let folder = store::entry_folder(root, &id);
    let executable = store::files(&folder).join(&name);
    match store::read_entry(&folder, &id) {
        Read::Entry(entry)
            if content_digest(&store::files(&folder))
                .is_ok_and(|found| found.to_string() == entry.digest) =>
        {
            return Ok(ExecutableCopy { id, executable });
        }
        Read::Entry(_) => store::delete(root, &folder, None)?,
        Read::Absent => {}
        // An entry it cannot read is never touched: this engine cannot
        // make the copy there, so it says so.
        Read::Unreadable => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "{} holds an entry this engine cannot read",
                    folder.display()
                ),
            ));
        }
    }
    let partial = store::partial(root)?;
    let files = store::files(partial.path());
    #[expect(
        clippy::disallowed_methods,
        reason = "an executable places its copy before its runtime starts"
    )]
    fs::create_dir_all(&files)?;
    // `fs::copy` keeps the executable's permissions.
    #[expect(
        clippy::disallowed_methods,
        reason = "an executable places its copy before its runtime starts"
    )]
    fs::copy(source, files.join(&name))?;
    let entry = BrowserCacheEntry {
        format_version: BROWSER_CACHE.version,
        artifact: id.clone(),
        digest: content_digest(&files)?.to_string(),
        installed_at: now,
        used_at: now,
    };
    store::install(root, partial, &id, &entry)?;
    Ok(ExecutableCopy { id, executable })
}

/// Whether the cache at `root` holds `id` in an entry this engine reads.
pub fn entry_present(root: &Path, id: &ArtifactId) -> bool {
    let folder = store::entry_folder(root, id);
    matches!(store::read_entry(&folder, id), Read::Entry(_)) && store::files(&folder).is_dir()
}

/// The copy `id` in the cache at `root`, held under a shared lock while a
/// Daemon runs from it; `None` when it is not there or a deletion holds it.
///
/// # Errors
///
/// When its lock cannot be opened.
pub fn hold(root: &Path, id: &ArtifactId) -> io::Result<Option<HeldArtifact>> {
    let folder = store::entry_folder(root, id);
    Ok(match store::lock_shared(&folder)? {
        Locking::Taken(lock) => Some(HeldArtifact {
            folder,
            _lock: Some(lock),
        }),
        Locking::Held | Locking::Missing => None,
    })
}
