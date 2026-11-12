//! The set of built files `xtask size` measures, read from a TOML manifest
//! that names each file's Product, target, kind and path, and measured: a
//! file by its length, an OCI image by the compressed sizes its manifest
//! lists.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::{Deserialize, Serialize};

use goodfetch_protocol::catalog::Product;

use super::budget::{Bytes, Kind};

/// The Cargo profile the files were built with.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(transparent)]
pub(crate) struct Profile(pub(crate) String);

impl Profile {
    pub(crate) fn is_release(&self) -> bool {
        self.0 == "release"
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    profile: Profile,
    #[serde(rename = "file", default)]
    files: Vec<Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    product: Product,
    kind: Kind,
    /// Required for every kind but a crate tarball, which has none.
    target: Option<String>,
    /// What identifies the file across builds; its file name by default.
    name: Option<String>,
    /// From the workspace root.
    path: PathBuf,
}

impl Entry {
    /// What identifies the file across builds.
    fn name(&self) -> Result<String> {
        match &self.name {
            Some(name) => Ok(name.clone()),
            None => Ok(self
                .path
                .file_name()
                .with_context(|| format!("{} has no file name", self.path.display()))?
                .to_string_lossy()
                .into_owned()),
        }
    }
}

/// One measured file. Its identifier, `<product>/<target>/<name>` (or
/// `<product>/<name>` for a crate tarball), matches it with `main`'s figure
/// and with a "Size increase" section's line.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct Measured {
    pub(crate) product: Product,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(crate) target: Option<String>,
    pub(crate) kind: Kind,
    pub(crate) name: String,
    pub(crate) bytes: Bytes,
}

impl Measured {
    pub(crate) fn id(&self) -> String {
        match &self.target {
            Some(target) => format!("{}/{target}/{}", self.product.as_str(), self.name),
            None => format!("{}/{}", self.product.as_str(), self.name),
        }
    }
}

/// The OCI image manifest's descriptors, whose sizes are the compressed
/// blobs a registry stores and a pull downloads.
#[derive(Deserialize)]
struct ImageManifest {
    config: Descriptor,
    layers: Vec<Descriptor>,
}

#[derive(Deserialize)]
struct Descriptor {
    size: u64,
}

/// One executable the manifest names, for the checks that read the file
/// itself.
pub(crate) struct Executable {
    /// `<product>/<target>/<name>`, as [`Measured::id`] writes it.
    pub(crate) id: String,
    pub(crate) target: String,
    pub(crate) path: PathBuf,
}

fn read(root: &Path, path: &Path) -> Result<Manifest> {
    let path = root.join(path);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Reads the manifest at `path` and measures every file it names.
pub(crate) fn measure(root: &Path, path: &Path) -> Result<(Profile, Vec<Measured>)> {
    let manifest = read(root, path)?;
    let measured = manifest
        .files
        .into_iter()
        .map(|entry| measure_entry(root, entry))
        .collect::<Result<Vec<_>>>()?;
    Ok((manifest.profile, measured))
}

/// The executables the manifest at `path` names, each with its path under
/// `root`.
pub(crate) fn executables(root: &Path, path: &Path) -> Result<Vec<Executable>> {
    read(root, path)?
        .files
        .into_iter()
        .filter(|entry| entry.kind == Kind::Executable)
        .map(|entry| {
            let name = entry.name()?;
            let target = entry
                .target
                .with_context(|| format!("{}: the file needs a target", entry.path.display()))?;
            Ok(Executable {
                id: format!("{}/{target}/{name}", entry.product.as_str()),
                target,
                path: root.join(&entry.path),
            })
        })
        .collect()
}

fn measure_entry(root: &Path, entry: Entry) -> Result<Measured> {
    match (entry.kind, &entry.target) {
        (Kind::Crate, Some(_)) => bail!("{}: a crate tarball has no target", entry.path.display()),
        (Kind::Crate, None) | (_, Some(_)) => {}
        (_, None) => bail!("{}: the file needs a target", entry.path.display()),
    }
    let path = root.join(&entry.path);
    let bytes = match entry.kind {
        Kind::OciImage => {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let image: ImageManifest = serde_json::from_str(&text)
                .with_context(|| format!("parsing the OCI image manifest {}", path.display()))?;
            image.config.size + image.layers.iter().map(|layer| layer.size).sum::<u64>()
        }
        Kind::NpmPackage | Kind::Wheel | Kind::ReleaseArchive | Kind::Executable | Kind::Crate => {
            let metadata =
                std::fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
            if !metadata.is_file() {
                bail!("{} is not a file", path.display());
            }
            metadata.len()
        }
    };
    let name = entry.name()?;
    Ok(Measured {
        product: entry.product,
        target: entry.target,
        kind: entry.kind,
        name,
        bytes: Bytes(bytes),
    })
}
