//! The uv workspace's third-party distributions that ship: the
//! dependencies and extras of every workspace package, followed through
//! `uv.lock`, never its dependency groups. Their licenses come from the
//! metadata `uv sync` left in `.venv`.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::entry::{Ecosystem, Entry, Usage};

const LOCKFILE: &str = "uv.lock";

const ENVIRONMENT: &str = ".venv";

#[derive(Deserialize)]
struct Lockfile {
    #[serde(default)]
    package: Vec<Package>,
}

#[derive(Deserialize)]
struct Package {
    name: String,
    /// Absent for a workspace package whose version is dynamic.
    #[serde(default)]
    version: String,
    source: Source,
    #[serde(default)]
    dependencies: Vec<Dependency>,
    #[serde(default, rename = "optional-dependencies")]
    optional_dependencies: BTreeMap<String, Vec<Dependency>>,
}

/// Where uv got a package; a workspace package, or another of Goodfetch's
/// folders, has none of these.
#[derive(Deserialize)]
struct Source {
    registry: Option<String>,
    git: Option<String>,
    url: Option<String>,
}

impl Source {
    fn location(&self) -> Option<&str> {
        self.registry
            .as_deref()
            .or(self.git.as_deref())
            .or(self.url.as_deref())
    }
}

#[derive(Deserialize)]
struct Dependency {
    name: String,
    /// Given when the lockfile holds more than one version of it.
    version: Option<String>,
    /// The environments it is installed in, when it is not in all.
    marker: Option<String>,
    #[serde(default)]
    extra: Vec<String>,
}

/// A dependency to follow: whether every path to it so far has a marker,
/// and the extras it is taken with.
struct Edge<'a> {
    dependency: &'a Dependency,
    conditional: bool,
}

/// Every third-party distribution a workspace package depends on, directly
/// or not; nothing when the workspace has no lockfile.
pub(super) fn entries(root: &Path) -> Result<Vec<Entry>> {
    let path = root.join(LOCKFILE);
    if !path.is_file() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let lockfile: Lockfile =
        toml::from_str(&text).with_context(|| format!("parsing {LOCKFILE}"))?;
    let installed = installed(root)?;
    let mut entries = Vec::new();
    for (package, conditional) in shipped(&lockfile) {
        let Some(location) = package.source.location() else {
            continue;
        };
        let identity = (normalized(&package.name), package.version.clone());
        let Some(license) = installed.get(&identity) else {
            // A distribution for other environments is not installed in this one.
            if conditional {
                continue;
            }
            bail!(
                "{}@{} is not installed in {ENVIRONMENT}: run `uv sync --all-extras`",
                package.name,
                package.version
            );
        };
        let source = if package.source.registry.as_deref() == Some("https://pypi.org/simple") {
            format!(
                "https://pypi.org/project/{}/{}/",
                package.name, package.version
            )
        } else {
            location.to_owned()
        };
        entries.push(Entry {
            ecosystem: Ecosystem::Python,
            name: package.name.clone(),
            version: package.version.clone(),
            license: license.clone(),
            source: Some(source),
            usage: Usage::Unmodified,
        });
    }
    Ok(entries)
}

/// Every package the workspace's own packages reach through their
/// dependencies and extras, each with whether only a marker's paths reach it.
fn shipped(lockfile: &Lockfile) -> Vec<(&Package, bool)> {
    let own = lockfile
        .package
        .iter()
        .filter(|package| package.source.location().is_none());
    let mut pending: Vec<Edge<'_>> = own
        .flat_map(|package| {
            package
                .dependencies
                .iter()
                .chain(package.optional_dependencies.values().flatten())
        })
        .map(|dependency| Edge {
            dependency,
            conditional: false,
        })
        .collect();
    // Each package's dependencies, then each extra's, are followed once per
    // kind of path: unconditional, or only through markers.
    let mut followed: BTreeSet<(&str, &str, Option<&str>, bool)> = BTreeSet::new();
    let mut reached: BTreeMap<(&str, &str), bool> = BTreeMap::new();
    while let Some(edge) = pending.pop() {
        let conditional = edge.conditional || edge.dependency.marker.is_some();
        for package in lockfile.package.iter().filter(|package| {
            package.name == edge.dependency.name
                && edge
                    .dependency
                    .version
                    .as_ref()
                    .is_none_or(|version| *version == package.version)
        }) {
            let identity = (package.name.as_str(), package.version.as_str());
            let only_conditional = reached.entry(identity).or_insert(true);
            *only_conditional &= conditional;
            let groups = std::iter::once((None, &package.dependencies)).chain(
                edge.dependency.extra.iter().filter_map(|extra| {
                    Some((
                        Some(extra.as_str()),
                        package.optional_dependencies.get(extra)?,
                    ))
                }),
            );
            for (extra, dependencies) in groups {
                if followed.insert((identity.0, identity.1, extra, conditional)) {
                    pending.extend(dependencies.iter().map(|dependency| Edge {
                        dependency,
                        conditional,
                    }));
                }
            }
        }
    }
    lockfile
        .package
        .iter()
        .filter_map(|package| {
            let conditional = *reached.get(&(package.name.as_str(), package.version.as_str()))?;
            Some((package, conditional))
        })
        .collect()
}

/// A distribution name as PEP 503 compares it.
fn normalized(name: &str) -> String {
    let mut normalized = String::new();
    for character in name.chars() {
        if matches!(character, '-' | '_' | '.') {
            if !normalized.ends_with('-') {
                normalized.push('-');
            }
        } else {
            normalized.push(character.to_ascii_lowercase());
        }
    }
    normalized
}

/// The license each installed distribution states, by normalized name and
/// version.
fn installed(root: &Path) -> Result<BTreeMap<(String, String), Option<String>>> {
    let mut installed = BTreeMap::new();
    for site_packages in site_packages(&root.join(ENVIRONMENT)) {
        let entries = std::fs::read_dir(&site_packages)
            .with_context(|| format!("reading {}", site_packages.display()))?;
        for entry in entries {
            let entry = entry.with_context(|| format!("reading {}", site_packages.display()))?;
            if !entry.file_name().to_string_lossy().ends_with(".dist-info") {
                continue;
            }
            let path = entry.path().join("METADATA");
            let Ok(text) = std::fs::read_to_string(&path) else {
                continue;
            };
            let metadata = Metadata::parse(&text);
            if let (Some(name), Some(version)) = (metadata.name, metadata.version) {
                installed.insert((normalized(&name), version), metadata.license);
            }
        }
    }
    Ok(installed)
}

/// The environment's `site-packages` folders: `lib/python3.x/` on Unix,
/// `Lib/` on Windows.
fn site_packages(environment: &Path) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let windows = environment.join("Lib/site-packages");
    if windows.is_dir() {
        folders.push(windows);
    }
    if let Ok(entries) = std::fs::read_dir(environment.join("lib")) {
        for entry in entries.flatten() {
            let folder = entry.path().join("site-packages");
            if folder.is_dir() {
                folders.push(folder);
            }
        }
    }
    folders.sort();
    folders
}

/// The fields of a distribution's core metadata the inventory reads.
struct Metadata {
    name: Option<String>,
    version: Option<String>,
    /// `License-Expression` (PEP 639), or else the older `License` field.
    license: Option<String>,
}

impl Metadata {
    fn parse(text: &str) -> Self {
        let mut fields: BTreeMap<String, String> = BTreeMap::new();
        // The headers end at the first empty line, where the description begins.
        for line in text.lines().take_while(|line| !line.is_empty()) {
            if let Some((field, value)) = line.split_once(':')
                && !line.starts_with(char::is_whitespace)
            {
                fields
                    .entry(field.to_ascii_lowercase())
                    .or_insert_with(|| value.trim().to_owned());
            }
        }
        let license = fields
            .get("license-expression")
            .or_else(|| fields.get("license"))
            .filter(|license| !license.is_empty() && *license != "UNKNOWN")
            .cloned();
        Self {
            name: fields.remove("name"),
            version: fields.remove("version"),
            license,
        }
    }
}
