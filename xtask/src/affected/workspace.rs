//! What the rules need to know of the repository's three workspaces (ADR
//! 0032): each Cargo crate, pnpm package and uv package, its folder and the
//! members of its own workspace it depends on. Cargo's members come from
//! `cargo metadata`; pnpm's from `pnpm-workspace.yaml` and uv's from the root
//! `pyproject.toml`, whose patterns use `*` for one folder name.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::RepoPath;
use crate::{metadata, package_dependencies};

pub(crate) struct Workspace {
    pub(crate) crates: Vec<Crate>,
    pub(crate) packages: Vec<Package>,
}

/// A member of the Cargo workspace.
pub(crate) struct Crate {
    pub(crate) name: String,
    pub(crate) folder: RepoPath,
    /// The workspace members it depends on, of any kind (normal, build, dev).
    pub(crate) dependencies: BTreeSet<String>,
    /// Its bench targets' names.
    pub(crate) benches: BTreeSet<String>,
    /// Whether it builds an executable (a `bin` target).
    pub(crate) executable: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Ecosystem {
    /// A pnpm workspace member, tested by Vitest.
    TypeScript,
    /// A uv workspace member, tested by pytest.
    Python,
}

/// A member of the pnpm or uv workspace.
pub(crate) struct Package {
    pub(crate) ecosystem: Ecosystem,
    pub(crate) name: String,
    pub(crate) folder: RepoPath,
    /// The members of its own workspace it depends on.
    pub(crate) dependencies: BTreeSet<String>,
}

pub(crate) fn read(cargo: &OsStr, root: &Path) -> Result<Workspace> {
    // cargo metadata reports canonical paths; folders are named from the root.
    let root = &root
        .canonicalize()
        .with_context(|| format!("reading {}", root.display()))?;
    let mut packages = typescript_packages(root)?;
    packages.extend(python_packages(root)?);
    Ok(Workspace {
        crates: crates(cargo, root)?,
        packages,
    })
}

fn relative(root: &Path, folder: &Path) -> Result<RepoPath> {
    let relative = folder
        .strip_prefix(root)
        .with_context(|| format!("{} is outside {}", folder.display(), root.display()))?;
    let segments: Vec<&str> = relative
        .components()
        .map(|component| {
            component
                .as_os_str()
                .to_str()
                .with_context(|| format!("{} is not UTF-8", relative.display()))
        })
        .collect::<Result<_>>()?;
    Ok(RepoPath::new(segments.join("/")))
}

fn crates(cargo: &OsStr, root: &Path) -> Result<Vec<Crate>> {
    let packages = metadata::packages(cargo, root)?;
    let members: BTreeSet<&str> = packages
        .iter()
        .map(|package| package.name.as_str())
        .collect();
    packages
        .iter()
        .map(|package| {
            let folder = package
                .manifest_path
                .parent()
                .with_context(|| format!("{} has no folder", package.manifest_path.display()))?;
            Ok(Crate {
                name: package.name.clone(),
                folder: relative(root, folder)?,
                dependencies: package
                    .dependencies
                    .iter()
                    .filter(|dependency| members.contains(dependency.name.as_str()))
                    .map(|dependency| dependency.name.clone())
                    .collect(),
                benches: package
                    .targets
                    .iter()
                    .filter(|target| target.kind.iter().any(|kind| kind == "bench"))
                    .map(|target| target.name.clone())
                    .collect(),
                executable: package
                    .targets
                    .iter()
                    .any(|target| target.kind.iter().any(|kind| kind == "bin")),
            })
        })
        .collect()
}

#[derive(Deserialize)]
struct PnpmWorkspace {
    #[serde(default)]
    packages: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageJson {
    name: String,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default)]
    dev_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    peer_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    optional_dependencies: BTreeMap<String, String>,
}

fn typescript_packages(root: &Path) -> Result<Vec<Package>> {
    let Some(text) = read_if_present(&root.join("pnpm-workspace.yaml"))? else {
        return Ok(Vec::new());
    };
    let workspace: PnpmWorkspace =
        serde_saphyr::from_str(&text).context("parsing pnpm-workspace.yaml")?;
    let mut manifests = Vec::new();
    for folder in expand(root, &workspace.packages)? {
        let path = folder.join("package.json");
        if let Some(text) = read_if_present(&path)? {
            let manifest: PackageJson = serde_json::from_str(&text)
                .with_context(|| format!("parsing {}", path.display()))?;
            manifests.push((relative(root, &folder)?, manifest));
        }
    }
    let names: BTreeSet<String> = manifests
        .iter()
        .map(|(_, manifest)| manifest.name.clone())
        .collect();
    Ok(manifests
        .into_iter()
        .map(|(folder, manifest)| Package {
            ecosystem: Ecosystem::TypeScript,
            dependencies: [
                &manifest.dependencies,
                &manifest.dev_dependencies,
                &manifest.peer_dependencies,
                &manifest.optional_dependencies,
            ]
            .into_iter()
            .flat_map(BTreeMap::keys)
            .filter(|name| names.contains(*name))
            .cloned()
            .collect(),
            name: manifest.name,
            folder,
        })
        .collect())
}

#[derive(Deserialize)]
struct PyProject {
    project: Option<Project>,
    #[serde(default, rename = "dependency-groups")]
    dependency_groups: BTreeMap<String, Vec<toml::Value>>,
    tool: Option<Tool>,
}

#[derive(Deserialize)]
struct Project {
    name: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default, rename = "optional-dependencies")]
    optional_dependencies: BTreeMap<String, Vec<String>>,
}

#[derive(Deserialize)]
struct Tool {
    uv: Option<Uv>,
}

#[derive(Deserialize)]
struct Uv {
    workspace: Option<UvWorkspace>,
}

#[derive(Deserialize)]
struct UvWorkspace {
    #[serde(default)]
    members: Vec<String>,
    #[serde(default)]
    exclude: Vec<String>,
}

fn read_pyproject(path: &Path) -> Result<Option<PyProject>> {
    read_if_present(path)?
        .map(|text| toml::from_str(&text).with_context(|| format!("parsing {}", path.display())))
        .transpose()
}

/// A requirement's or project's name, normalized by PEP 503.
fn python_name(requirement: &str) -> String {
    let name: String = requirement
        .trim()
        .chars()
        .take_while(|character| character.is_ascii_alphanumeric() || "-_.".contains(*character))
        .collect();
    package_dependencies::pep503(&name)
}

fn python_packages(root: &Path) -> Result<Vec<Package>> {
    let workspace = read_pyproject(&root.join("pyproject.toml"))?
        .and_then(|pyproject| pyproject.tool)
        .and_then(|tool| tool.uv)
        .and_then(|uv| uv.workspace);
    let Some(workspace) = workspace else {
        return Ok(Vec::new());
    };
    let excluded = expand(root, &workspace.exclude)?;
    let mut members = Vec::new();
    for folder in expand(root, &workspace.members)? {
        if excluded.contains(&folder) {
            continue;
        }
        let Some(pyproject) = read_pyproject(&folder.join("pyproject.toml"))? else {
            continue;
        };
        let Some(project) = pyproject.project else {
            bail!("{} has no [project] table", folder.display());
        };
        let requirements: Vec<String> = project
            .dependencies
            .iter()
            .chain(project.optional_dependencies.values().flatten())
            .cloned()
            .chain(
                pyproject
                    .dependency_groups
                    .values()
                    .flatten()
                    .filter_map(|entry| entry.as_str().map(str::to_owned)),
            )
            .collect();
        members.push((
            relative(root, &folder)?,
            python_name(&project.name),
            requirements,
        ));
    }
    let names: BTreeSet<String> = members.iter().map(|(_, name, _)| name.clone()).collect();
    Ok(members
        .into_iter()
        .map(|(folder, name, requirements)| Package {
            ecosystem: Ecosystem::Python,
            dependencies: requirements
                .iter()
                .map(|requirement| python_name(requirement))
                .filter(|dependency| names.contains(dependency))
                .collect(),
            name,
            folder,
        })
        .collect())
}

fn read_if_present(path: &Path) -> Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {}", path.display())),
    }
}

/// The folders under `root` that `patterns` name, where `*` stands for any
/// one folder name that does not start with a dot.
fn expand(root: &Path, patterns: &[String]) -> Result<BTreeSet<PathBuf>> {
    let mut folders = BTreeSet::new();
    for pattern in patterns {
        if pattern.starts_with('!') || pattern.contains("**") {
            bail!(
                "the workspace pattern `{pattern}` is not supported: use `*` for one folder name"
            );
        }
        let mut matched = vec![root.to_path_buf()];
        for segment in pattern.trim_end_matches('/').split('/') {
            let mut next = Vec::new();
            for folder in &matched {
                if segment == "*" {
                    next.extend(subfolders(folder)?);
                } else if segment.contains('*') {
                    bail!(
                        "the workspace pattern `{pattern}` is not supported: use `*` for one folder name"
                    );
                } else if folder.join(segment).is_dir() {
                    next.push(folder.join(segment));
                }
            }
            matched = next;
        }
        folders.extend(matched);
    }
    Ok(folders)
}

fn subfolders(folder: &Path) -> Result<Vec<PathBuf>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(error) => return Err(error).with_context(|| format!("reading {}", folder.display())),
    };
    let mut subfolders = Vec::new();
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let hidden = entry.file_name().to_string_lossy().starts_with('.');
        if !hidden && entry.file_type()?.is_dir() {
            subfolders.push(entry.path());
        }
    }
    Ok(subfolders)
}
