//! What a Product version pins (ADRs 0024 and 0098): each Pinned artifact's
//! identity, the URL it downloads from, the digest of its archive and the
//! digest of its files, which every launch checks.
//!
//! The digest of a tree of files is SHA-256 over, for each file and symbolic
//! link in the order of their `/`-separated relative paths, the path, a NUL,
//! `f` for a file or `l` for a link, a NUL, then the SHA-256 of the file's
//! bytes or of the link's target, and a newline. Folders count only through
//! what they hold, and permissions not at all, since Windows keeps no
//! executable bit and one pin's digest must hold wherever its files land.
#![expect(
    clippy::disallowed_methods,
    reason = "the digest reads files on the tasks module's blocking policy, never on an async worker"
)]

use std::fmt;
use std::fs;
use std::io::{self, Read as _};
use std::path::Path;
use std::str::FromStr;

use goodfetch_protocol::browser_cache::{ArtifactId, ArtifactKind, PinnedBy};
use sha2::{Digest as _, Sha256};

/// A SHA-256 digest, written `sha256:<hex>`.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Digest([u8; 32]);

impl Digest {
    /// The digest of `bytes`.
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }

    pub(super) fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    fn from_hasher(hasher: Sha256) -> Self {
        Self(hasher.finalize().into())
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("sha256:")?;
        for byte in self.0 {
            write!(formatter, "{byte:02x}")?;
        }
        Ok(())
    }
}

impl fmt::Debug for Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

/// Text that is no `sha256:<hex>` digest.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("`{0}` is no digest `sha256:<64 lowercase hex digits>`")]
pub struct MalformedDigest(String);

impl FromStr for Digest {
    type Err = MalformedDigest;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let malformed = || MalformedDigest(text.to_owned());
        let hex = text.strip_prefix("sha256:").ok_or_else(malformed)?;
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
        {
            return Err(malformed());
        }
        let mut bytes = [0; 32];
        for (index, byte) in bytes.iter_mut().enumerate() {
            *byte =
                u8::from_str_radix(&hex[index * 2..index * 2 + 2], 16).map_err(|_| malformed())?;
        }
        Ok(Self(bytes))
    }
}

/// The platform a build runs on, where it is built for one.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum BuildPlatform {
    /// Linux on x86-64.
    LinuxX64,
    /// Linux on arm64.
    LinuxArm64,
    /// macOS on Apple silicon.
    MacArm64,
    /// macOS on x86-64.
    MacX64,
    /// Windows on x86-64, which Windows on arm64 also runs under emulation.
    WindowsX64,
}

/// One download a Product version pins: by URL, the digest of its archive,
/// and the digest of the files the archive holds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PinnedArtifact {
    /// Which artifact.
    pub id: ArtifactId,
    /// The platform it is built for, or `None` for every platform.
    pub platform: Option<BuildPlatform>,
    /// Where it downloads from, an `https` URL with no query.
    pub url: String,
    /// The digest of the ZIP archive the URL serves.
    pub archive_digest: Digest,
    /// The digest of the files the archive holds, which every launch checks.
    pub digest: Digest,
}

/// Every Pinned artifact a Product version pins (ADR 0098: each Product asks
/// for exactly these).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pins {
    /// The Product version that pins them.
    pub pinned_by: PinnedBy,
    /// The artifacts.
    pub artifacts: Vec<PinnedArtifact>,
}

impl Pins {
    /// The artifacts pinned for `platform`, and those for every platform;
    /// only the latter for a platform no build is made for (`None`).
    pub fn for_platform(
        &self,
        platform: Option<BuildPlatform>,
    ) -> impl Iterator<Item = &PinnedArtifact> {
        self.artifacts
            .iter()
            .filter(move |artifact| artifact.platform.is_none() || artifact.platform == platform)
    }

    /// The browser build named `name` pinned for `platform`.
    pub fn browser(&self, name: &str, platform: BuildPlatform) -> Option<&PinnedArtifact> {
        self.for_platform(Some(platform))
            .find(|artifact| artifact.id.kind == ArtifactKind::Browser && artifact.id.name == name)
    }
}

/// The digest of the tree of files under `root`, by the rule this module's
/// header states. Runs on a blocking thread.
pub fn content_digest(root: &Path) -> io::Result<Digest> {
    let mut files = Vec::new();
    collect(root, "", &mut files)?;
    files.sort_by(|left, right| left.0.cmp(&right.0));
    let mut tree = Sha256::new();
    for (relative, kind, digest) in files {
        tree.update(relative.as_bytes());
        tree.update([0, kind, 0]);
        tree.update(digest.0);
        tree.update(b"\n");
    }
    Ok(Digest::from_hasher(tree))
}

fn collect(folder: &Path, prefix: &str, files: &mut Vec<(String, u8, Digest)>) -> io::Result<()> {
    for entry in fs::read_dir(folder)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidData, "a file name is not UTF-8")
        })?;
        let relative = if prefix.is_empty() {
            name.to_owned()
        } else {
            format!("{prefix}/{name}")
        };
        let path = entry.path();
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            let target = fs::read_link(&path)?;
            let target = target.to_str().ok_or_else(|| {
                io::Error::new(io::ErrorKind::InvalidData, "a link's target is not UTF-8")
            })?;
            files.push((relative, b'l', Digest::of(target.as_bytes())));
        } else if file_type.is_dir() {
            collect(&path, &relative, files)?;
        } else {
            files.push((relative, b'f', file_digest(&path)?));
        }
    }
    Ok(())
}

fn file_digest(path: &Path) -> io::Result<Digest> {
    let mut file = fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            return Ok(Digest::from_hasher(hasher));
        }
        hasher.update(&buffer[..read]);
    }
}
