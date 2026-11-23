//! The declared-dependency check of the npm and PyPI packages under `sdks/`,
//! `adapters/` and `integrations/` (ADR 0095, STYLE.md's package dependency
//! rule): Foxhound's packages depend on no Collie package but the Collie
//! client package, so never on one that carries Collie's executable; Collie's
//! depend on no Foxhound package; the client package depends on neither
//! Product's package.
//!
//! A package's Product is its folder's: `sdks/<language>/collie-client/`,
//! `sdks/<language>/<product>/`, `adapters/<product>/` and
//! `integrations/<product>/`. Only what installs with the package is read:
//! npm's `dependencies`, `peerDependencies` and `optionalDependencies`, and
//! PyPI's `project.dependencies` and `project.optional-dependencies`, never
//! development dependencies. `sdks/<language>/smoke/` holds ADR 0069's smoke
//! scripts, a package of no Product that is never published: the rule
//! passes it.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::tree;

/// The folders whose packages the rule covers.
const ROOTS: [&str; 3] = ["sdks", "adapters", "integrations"];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Side {
    /// The Collie client package both Products' SDKs build on.
    Client,
    Collie,
    Foxhound,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Registry {
    Npm,
    PyPi,
}

/// A package name as its registry compares it: npm's as written, PyPI's
/// normalized (PEP 503).
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Name {
    registry: Registry,
    text: String,
}

impl Name {
    fn npm(text: &str) -> Self {
        Self {
            registry: Registry::Npm,
            text: text.to_owned(),
        }
    }

    fn pypi(text: &str) -> Self {
        Self {
            registry: Registry::PyPi,
            text: pep503(text),
        }
    }
}

/// A Python distribution's name normalized by PEP 503: lowercase, each run
/// of `-`, `_` and `.` one `-`, which `xtask affected` reads names by too.
pub(crate) fn pep503(text: &str) -> String {
    let mut normalized = String::with_capacity(text.len());
    let mut separator = false;
    for character in text.chars() {
        if matches!(character, '-' | '_' | '.') {
            separator = true;
        } else {
            if separator && !normalized.is_empty() {
                normalized.push('-');
            }
            separator = false;
            normalized.push(character.to_ascii_lowercase());
        }
    }
    normalized
}

/// What a manifest declares: the package's name and its dependencies.
struct Declared {
    name: Name,
    dependencies: Vec<Name>,
}

/// A package with the side its folder gives it.
struct Package {
    manifest: String,
    side: Side,
    declared: Declared,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct PackageJson {
    name: String,
    #[serde(default)]
    dependencies: BTreeMap<String, String>,
    #[serde(default)]
    peer_dependencies: BTreeMap<String, String>,
    #[serde(default)]
    optional_dependencies: BTreeMap<String, String>,
}

#[derive(Deserialize)]
struct PyProject {
    project: Project,
}

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Project {
    name: String,
    #[serde(default)]
    dependencies: Vec<String>,
    #[serde(default)]
    optional_dependencies: BTreeMap<String, Vec<String>>,
}

pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let mut findings = Vec::new();
    let mut packages = Vec::new();
    for folder in ROOTS {
        for manifest in tree::files(root, folder)? {
            if smoke_scripts(&manifest) {
                continue;
            }
            let Some(declared) = read(root, &manifest)? else {
                continue;
            };
            match side(&manifest) {
                Some(side) => packages.push(Package {
                    manifest,
                    side,
                    declared,
                }),
                None => findings.push(Finding::new(
                    "package-of-no-product",
                    manifest,
                    "a package under sdks/, adapters/ or integrations/ sits in a folder of the Collie client package, Collie or Foxhound",
                )),
            }
        }
    }
    let sides: BTreeMap<&Name, Side> = packages
        .iter()
        .map(|package| (&package.declared.name, package.side))
        .collect();
    for package in &packages {
        for dependency in &package.declared.dependencies {
            let Some(&target) = sides.get(dependency) else {
                continue;
            };
            let kind = match (package.side, target) {
                (Side::Foxhound, Side::Collie) => "foxhound-depends-on-collie",
                (Side::Collie, Side::Foxhound) => "collie-depends-on-foxhound",
                (Side::Client, Side::Collie | Side::Foxhound) => "client-depends-on-product",
                _ => continue,
            };
            findings.push(Finding::new(
                kind,
                &package.declared.name.text,
                format!(
                    "{} depends on `{}`, against ADR 0095's package rule",
                    package.manifest, dependency.text
                ),
            ));
        }
    }
    Ok(findings)
}

/// What a manifest declares, or `None` when the file is no manifest.
fn read(root: &Path, manifest: &str) -> Result<Option<Declared>> {
    let file_name = manifest.rsplit('/').next().unwrap_or(manifest);
    let read_text = || {
        std::fs::read_to_string(root.join(manifest)).with_context(|| format!("reading {manifest}"))
    };
    let declared = match file_name {
        "package.json" => {
            let json: PackageJson = serde_json::from_str(&read_text()?)
                .with_context(|| format!("reading {manifest}"))?;
            Declared {
                name: Name::npm(&json.name),
                dependencies: json
                    .dependencies
                    .keys()
                    .chain(json.peer_dependencies.keys())
                    .chain(json.optional_dependencies.keys())
                    .map(|name| Name::npm(name))
                    .collect(),
            }
        }
        "pyproject.toml" => {
            let toml: PyProject =
                toml::from_str(&read_text()?).with_context(|| format!("reading {manifest}"))?;
            let project = toml.project;
            Declared {
                name: Name::pypi(&project.name),
                dependencies: project
                    .dependencies
                    .iter()
                    .chain(project.optional_dependencies.values().flatten())
                    .map(|requirement| Name::pypi(requirement_name(requirement)))
                    .collect(),
            }
        }
        _ => return Ok(None),
    };
    Ok(Some(declared))
}

/// The distribution a PEP 508 requirement names: what comes before its
/// extras, version, marker or URL.
fn requirement_name(requirement: &str) -> &str {
    let end = requirement
        .find(|character: char| !(character.is_ascii_alphanumeric() || "-_.".contains(character)))
        .unwrap_or(requirement.len());
    requirement[..end].trim()
}

/// The Product a manifest's folder gives it.
/// Whether `manifest` is in `sdks/<language>/smoke/`, ADR 0069's smoke
/// scripts: a package of no Product, never published, that runs the
/// installed packages, so nothing installs with it.
fn smoke_scripts(manifest: &str) -> bool {
    matches!(
        manifest.split('/').collect::<Vec<_>>().as_slice(),
        ["sdks", _language, "smoke", ..]
    )
}

fn side(manifest: &str) -> Option<Side> {
    let parts: Vec<&str> = manifest.split('/').collect();
    let product = match parts.as_slice() {
        ["sdks", _language, product, _, ..] => *product,
        ["adapters" | "integrations", product, _, ..] => *product,
        _ => return None,
    };
    match product {
        "collie-client" if parts[0] == "sdks" => Some(Side::Client),
        "collie" => Some(Side::Collie),
        "foxhound" => Some(Side::Foxhound),
        _ => None,
    }
}
