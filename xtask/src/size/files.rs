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

use super::budget::{Bytes, Kind, Product};

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
            Some(target) => format!("{}/{target}/{}", self.product.key(), self.name),
            None => format!("{}/{}", self.product.key(), self.name),
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

/// Reads the manifest at `path` and measures every file it names.
pub(crate) fn measure(root: &Path, path: &Path) -> Result<(Profile, Vec<Measured>)> {
    let path = root.join(path);
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let manifest: Manifest =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let measured = manifest
        .files
        .into_iter()
        .map(|entry| measure_entry(root, entry))
        .collect::<Result<Vec<_>>>()?;
    Ok((manifest.profile, measured))
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
    let name = match entry.name {
        Some(name) => name,
        None => entry
            .path
            .file_name()
            .with_context(|| format!("{} has no file name", entry.path.display()))?
            .to_string_lossy()
            .into_owned(),
    };
    Ok(Measured {
        product: entry.product,
        target: entry.target,
        kind: entry.kind,
        name,
        bytes: Bytes(bytes),
    })
}
