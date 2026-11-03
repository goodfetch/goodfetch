//! The pnpm workspace's third-party packages that ship: the `dependencies`
//! and `optionalDependencies` of every workspace package, followed through
//! `pnpm-lock.yaml`, never its `devDependencies`. Their licenses come from
//! the manifests `pnpm install` left under `node_modules/.pnpm/`.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::entry::{Ecosystem, Entry, Usage};

const LOCKFILE: &str = "pnpm-lock.yaml";

/// pnpm's virtual store, one folder per package and set of peers.
const STORE: &str = "node_modules/.pnpm";

/// One document of the lockfile; pnpm writes its own installation's
/// packages in a document of their own, ahead of the workspace's.
#[derive(Deserialize)]
struct Lockfile {
    #[serde(default)]
    importers: BTreeMap<String, Dependencies>,
    #[serde(default)]
    packages: BTreeMap<String, Resolved>,
    #[serde(default)]
    snapshots: BTreeMap<String, Snapshot>,
}

#[derive(Deserialize, Default)]
struct Dependencies {
    #[serde(default)]
    dependencies: BTreeMap<String, Specified>,
    #[serde(default, rename = "optionalDependencies")]
    optional_dependencies: BTreeMap<String, Specified>,
}

#[derive(Deserialize)]
struct Specified {
    version: String,
}

#[derive(Deserialize)]
struct Resolved {
    resolution: Option<Resolution>,
    /// The platforms a package is for, when it is for some only.
    #[serde(default)]
    os: Vec<String>,
    #[serde(default)]
    cpu: Vec<String>,
    #[serde(default)]
    libc: Vec<String>,
}

#[derive(Deserialize)]
struct Resolution {
    tarball: Option<String>,
}

#[derive(Deserialize, Default)]
struct Snapshot {
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default, rename = "optionalDependencies")]
    optional_dependencies: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct Manifest {
    name: String,
    version: String,
    license: Option<LicenseField>,
    /// The legacy form, one entry per license offered.
    licenses: Option<Vec<LegacyLicense>>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum LicenseField {
    Expression(String),
    Legacy(LegacyLicense),
    /// An older form: the names of the licenses offered.
    Names(Vec<String>),
}

#[derive(Deserialize)]
struct LegacyLicense {
    #[serde(rename = "type")]
    kind: String,
}

impl Manifest {
    fn license(self) -> Option<String> {
        match (self.license, self.licenses) {
            (Some(LicenseField::Expression(expression)), _) => Some(expression),
            (Some(LicenseField::Legacy(legacy)), _) => Some(legacy.kind),
            (Some(LicenseField::Names(names)), _) if !names.is_empty() => Some(names.join(" OR ")),
            (None, Some(licenses)) if !licenses.is_empty() => Some(
                licenses
                    .into_iter()
                    .map(|license| license.kind)
                    .collect::<Vec<_>>()
                    .join(" OR "),
            ),
            (None | Some(LicenseField::Names(_)), _) => None,
        }
    }
}

/// Every third-party package a workspace package depends on, directly or
/// not; nothing when the workspace has no lockfile.
pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let path = root.join(LOCKFILE);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let documents: Vec<Lockfile> =
        serde_saphyr::from_multiple(&text).with_context(|| format!("parsing {LOCKFILE}"))?;
    let installed = installed(root)?;
    let mut entries = Vec::new();
    for lockfile in &documents {
        for key in shipped(lockfile) {
            let (name, version) = name_and_version(&key);
            let resolved = lockfile.packages.get(&format!("{name}@{version}"));
            let Some(license) = installed.get(&(name.to_owned(), version.to_owned())) else {
                // A package for other platforms is not installed on this one.
                if resolved.is_some_and(|resolved| {
                    !(resolved.os.is_empty() && resolved.cpu.is_empty() && resolved.libc.is_empty())
                }) {
                    continue;
                }
                bail!("{name}@{version} is not installed under {STORE}: run `pnpm install`");
            };
            let source = resolved
                .and_then(|resolved| resolved.resolution.as_ref()?.tarball.clone())
                .unwrap_or_else(|| format!("https://www.npmjs.com/package/{name}/v/{version}"));
            entries.push(Entry {
                ecosystem: Ecosystem::Npm,
                name: name.to_owned(),
                version: version.to_owned(),
                license: license.clone(),
                source: Some(source),
                usage: Usage::Unmodified,
            });
        }
    }
    Ok(entries)
}

/// The snapshot keys (`<name>@<version>` and any peers) of every package
/// that a workspace package's `dependencies` or `optionalDependencies`
/// reach, sorted.
fn shipped(lockfile: &Lockfile) -> BTreeSet<String> {
    let mut pending: Vec<String> = lockfile
        .importers
        .values()
        .flat_map(|importer| {
            importer
                .dependencies
                .iter()
                .chain(&importer.optional_dependencies)
                .filter_map(|(name, specified)| key(name, &specified.version))
        })
        .collect();
    let mut reached = BTreeSet::new();
    while let Some(key) = pending.pop() {
        if !reached.insert(key.clone()) {
            continue;
        }
        if let Some(snapshot) = lockfile.snapshots.get(&key) {
            pending.extend(
                snapshot
                    .dependencies
                    .iter()
                    .chain(&snapshot.optional_dependencies)
                    .filter_map(|(name, version)| self::key(name, version)),
            );
        }
    }
    reached
}

/// The snapshot key a dependency on `name` at `version` names; `None` for a
/// workspace package (`link:`).
fn key(name: &str, version: &str) -> Option<String> {
    if version.starts_with("link:") {
        return None;
    }
    // An alias names the real package: `version` is `<name>@<version>`.
    let without_peers = version.split('(').next().unwrap_or(version);
    if without_peers.rfind('@').is_some_and(|at| at > 0) {
        return Some(version.to_owned());
    }
    Some(format!("{name}@{version}"))
}

/// A snapshot key's package name and version, without its peers.
fn name_and_version(key: &str) -> (&str, &str) {
    let without_peers = key.split('(').next().unwrap_or(key);
    // A scoped name begins with `@`; the version follows the last one.
    match without_peers.rfind('@') {
        Some(at) if at > 0 => (&without_peers[..at], &without_peers[at + 1..]),
        _ => (without_peers, ""),
    }
}

/// The license each installed package states, by name and version.
fn installed(root: &Path) -> Result<BTreeMap<(String, String), Option<String>>> {
    let mut installed = BTreeMap::new();
    let store = root.join(STORE);
    let Ok(folders) = std::fs::read_dir(&store) else {
        return Ok(installed);
    };
    for folder in folders {
        let folder = folder.with_context(|| format!("reading {}", store.display()))?;
        let modules = folder.path().join("node_modules");
        for package in packages(&modules)? {
            let path = package.join("package.json");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let manifest: Manifest = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            let identity = (manifest.name.clone(), manifest.version.clone());
            installed.insert(identity, manifest.license());
        }
    }
    Ok(installed)
}

/// The package folders in one `node_modules` folder of the store, scoped
/// ones included, but not the links to the package's dependencies.
fn packages(modules: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut packages = Vec::new();
    let Ok(entries) = std::fs::read_dir(modules) else {
        return Ok(packages);
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", modules.display()))?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        if entry.file_name().to_string_lossy().starts_with('@') {
            packages.extend(self::packages(&entry.path())?);
        } else {
            packages.push(entry.path());
        }
    }
    Ok(packages)
}
