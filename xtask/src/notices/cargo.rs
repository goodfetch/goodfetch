//! The third-party crates the workspace ships, every feature on, read from
//! `cargo metadata`: what its shipped members (every member but `xtask` and
//! the test-only crates) reach through normal dependencies, never through a
//! dev or build dependency, as npm and Python list only shipped packages.
//! cargo-deny still checks the whole graph.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::entry::{Ecosystem, Entry, Usage};

/// crates.io, by either of its index protocols.
const CRATES_IO: [&str; 2] = [
    "registry+https://github.com/rust-lang/crates.io-index",
    "sparse+https://index.crates.io/",
];

/// The members no Product ships: the build tool and the test-only crates
/// (STYLE.md, Workspace and crates; TESTING.md, Shared helpers).
const NEVER_SHIPPED: [&str; 2] = ["xtask", "goodfetch-test-support"];

#[derive(Deserialize)]
struct Metadata {
    packages: Vec<Package>,
    workspace_members: BTreeSet<String>,
    resolve: Resolve,
}

#[derive(Deserialize)]
struct Resolve {
    nodes: Vec<Node>,
}

#[derive(Deserialize)]
struct Node {
    id: String,
    deps: Vec<NodeDependency>,
}

#[derive(Deserialize)]
struct NodeDependency {
    pkg: String,
    dep_kinds: Vec<DependencyKind>,
}

#[derive(Deserialize)]
struct DependencyKind {
    /// `None` for a normal dependency, else `dev` or `build`.
    kind: Option<String>,
}

#[derive(Deserialize)]
struct Package {
    id: String,
    name: String,
    version: String,
    license: Option<String>,
    /// Where Cargo got it; `None` for a folder.
    source: Option<String>,
    manifest_path: PathBuf,
}

/// Every crate a shipped member reaches through normal dependencies, the
/// members themselves aside.
pub(super) fn entries(cargo: &OsStr, root: &Path) -> Result<Vec<Entry>> {
    let output = crate::child_cargo::command(cargo)
        .args([
            "metadata",
            "--format-version",
            "1",
            "--all-features",
            "--locked",
        ])
        .current_dir(root)
        .output()
        .context("running cargo metadata")?;
    if !output.status.success() {
        bail!(
            "cargo metadata failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let metadata: Metadata =
        serde_json::from_slice(&output.stdout).context("reading cargo metadata's output")?;
    let shipped = shipped(&metadata);
    Ok(metadata
        .packages
        .into_iter()
        .filter(|package| {
            shipped.contains(&package.id) && !metadata.workspace_members.contains(&package.id)
        })
        .map(|package| Entry {
            ecosystem: Ecosystem::Cargo,
            source: Some(source(&package, root)),
            usage: usage(&package),
            name: package.name,
            version: package.version,
            license: package.license,
        })
        .collect())
}

/// The IDs of every package a shipped member reaches through normal
/// dependencies, the members included.
fn shipped(metadata: &Metadata) -> BTreeSet<String> {
    let names: std::collections::BTreeMap<&str, &str> = metadata
        .packages
        .iter()
        .map(|package| (package.id.as_str(), package.name.as_str()))
        .collect();
    let mut pending: Vec<&str> = metadata
        .workspace_members
        .iter()
        .map(String::as_str)
        .filter(|id| {
            !names
                .get(id)
                .is_some_and(|name| NEVER_SHIPPED.contains(name))
        })
        .collect();
    let mut reached = BTreeSet::new();
    while let Some(id) = pending.pop() {
        if !reached.insert(id.to_owned()) {
            continue;
        }
        let Some(node) = metadata.resolve.nodes.iter().find(|node| node.id == id) else {
            continue;
        };
        pending.extend(
            node.deps
                .iter()
                .filter(|dependency| dependency.dep_kinds.iter().any(|kind| kind.kind.is_none()))
                .map(|dependency| dependency.pkg.as_str()),
        );
    }
    reached
}

/// How it is shipped: as published when it comes from a registry, and with
/// Goodfetch's changes otherwise, as a fork the repository carries in place
/// of its registry release (the workspace's `[patch.crates-io]`).
fn usage(package: &Package) -> Usage {
    match package.source.as_deref() {
        Some(source) if source.starts_with("registry+") || source.starts_with("sparse+") => {
            Usage::Unmodified
        }
        _ => Usage::Modified,
    }
}

/// Its crates.io page, its other registry or repository, or its folder.
fn source(package: &Package, root: &Path) -> String {
    match package.source.as_deref() {
        Some(source) if CRATES_IO.contains(&source) => format!(
            "https://crates.io/crates/{}/{}",
            package.name, package.version
        ),
        Some(source) => source
            .split_once('+')
            .map_or(source, |(_, location)| location)
            .to_owned(),
        None => {
            // Cargo's paths are canonical; the root may not be.
            let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_owned());
            let folder = package.manifest_path.parent().unwrap_or(&root);
            folder
                .strip_prefix(&root)
                .unwrap_or(folder)
                .to_string_lossy()
                .replace('\\', "/")
        }
    }
}
