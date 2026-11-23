//! `xtask release plan`: both Products' publish, as one step per registry
//! and package in the order ADR 0094 sets, each step's state read through
//! its registry's public interface (no trait in front of registries). A
//! version already out is skipped, so a re-run after a failed step plans
//! only what is left; a name not on its registry yet is flagged, since
//! trusted publishing takes a package only after a first publish by hand.
//!
//! The order:
//! 1. the GitHub Release's draft, which takes the archives each crate's
//!    `[package.metadata.binstall]` points at, before any crate (ADR 0094);
//! 2. the crates, each after the workspace crates it depends on: the
//!    internal ones at `0.0.x`, the Collie client crate, then each Product's
//!    (none for a nightly, ADR 0030);
//! 3. npm: the Collie client package, then each Product's platform packages
//!    before its root package (ADR 0090);
//! 4. the tag, only after both Products' npm packages (ADR 0094);
//! 5. PyPI: the client's and each Product's wheels (none for a nightly,
//!    ADR 0057), with a warning when a Product's project passes 70% of the
//!    storage limit `ci/pypi-quota.toml` records for it (ADRs 0057, 0094);
//! 6. the GitHub Release's publishing, which makes it immutable, once both
//!    Products' files are on it (ADR 0094).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::Path;
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use toml_edit::{DocumentMut, Item, Table};

use super::{CRATES, ProductVersion, Stage, name, read, toml};
use crate::check::{self, Finding};
use crate::github::{self, List};
use crate::output;
use crate::products::Product;

/// crates.io's sparse index.
const CRATES_IO_INDEX: &str = "https://index.crates.io";
const NPM_REGISTRY: &str = "https://registry.npmjs.org";
const PYPI: &str = "https://pypi.org";

/// The Node platforms each Product's npm platform packages are named after,
/// `process.platform` and `process.arch` (ADR 0090).
const PLATFORMS: [&str; 6] = [
    "darwin-arm64",
    "darwin-x64",
    "linux-arm64",
    "linux-x64",
    "win32-arm64",
    "win32-x64",
];

/// The SDK packages' folder of the Collie client, under `sdks/typescript/`
/// and `sdks/python/`; each Product's is its key.
const CLIENT_PACKAGE: &str = "collie-client";

/// cargo-hakari's crate, which every member depends on and which never
/// publishes: `cargo hakari publish` takes its line out of a crate's
/// manifest for the publish (ADR 0094's note of 2026-11-06, #2089).
const WORKSPACE_HACK: &str = "goodfetch-workspace-hack";

/// Each Product's PyPI project's storage limit, in GiB.
const PYPI_QUOTA: &str = "ci/pypi-quota.toml";
/// The share of a project's limit past which the plan warns (ADR 0057).
const QUOTA_WARNING_PERCENT: u64 = 70;
const GIB: u64 = 1 << 30;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The Product version `xtask release prepare` set.
    #[arg(long)]
    version: ProductVersion,
    /// A sparse Cargo index: crates.io's, or a stand-in's address.
    #[arg(long, default_value = CRATES_IO_INDEX)]
    cargo_index: String,
    /// An npm registry: npm's, or a stand-in's address.
    #[arg(long, default_value = NPM_REGISTRY)]
    npm_registry: String,
    /// PyPI, whose JSON interface the plan reads, or a stand-in's address.
    #[arg(long, default_value = PYPI)]
    pypi: String,
    #[command(flatten)]
    github: github::Arguments,
}

/// One step of the publish: what it publishes, and what is left of it.
enum Step {
    /// A version of a package on its registry: `crates.io goodfetch-collie
    /// 1.4.0`.
    Package {
        registry: &'static str,
        name: String,
        version: String,
        state: PackageState,
    },
    /// The GitHub Release or its tag: `github tag v1.4.0`.
    GitHub { name: String, state: GitHubState },
}

impl fmt::Display for Step {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Package {
                registry,
                name,
                version,
                state,
            } => write!(formatter, "{registry} {name} {version}: {state}"),
            Self::GitHub { name, state } => write!(formatter, "github {name}: {state}"),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PackageState {
    /// The registry has the name, not this version.
    Publish,
    Published,
    /// The registry has never had the name.
    NewName,
}

impl fmt::Display for PackageState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Publish => "publish",
            Self::Published => "published",
            Self::NewName => "new name",
        })
    }
}

#[derive(Clone, Copy)]
enum GitHubState {
    /// No GitHub Release of the version yet.
    Create,
    /// A draft of it, which a re-run uploads the missing files to.
    Exists,
    Push,
    Pushed,
    Publish,
    Published,
}

impl fmt::Display for GitHubState {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Create => "create",
            Self::Exists => "exists",
            Self::Push => "push",
            Self::Pushed => "pushed",
            Self::Publish => "publish",
            Self::Published => "published",
        })
    }
}

/// Writes the plan's steps in order, a `note[pypi-quota]` for each Product
/// project past its warning, and an `error[new-name]` for each name not on
/// its registry, or only an `error[unprepared]` for each released crate
/// whose manifest does not hold the version.
pub(crate) fn run(root: &Path, arguments: &Arguments, token: Option<&str>) -> Result<ExitCode> {
    let version = &arguments.version;
    let nightly = matches!(version.stage, Stage::Nightly(_));
    let semver = version.semver();
    let crates = publish_order(root)?;
    let unprepared = unprepared(&crates, &semver);
    if !unprepared.is_empty() {
        return Ok(check::report(&unprepared));
    }

    let api = arguments.github.authenticated(token)?;
    let repository = &arguments.github.repository;
    let tag = format!("v{semver}");
    let releases: Vec<Release> =
        api.get_all(&format!("/repos/{repository}/releases"), List::Array)?;
    let release = releases.iter().find(|release| release.tag_name == tag);

    let mut steps = vec![Step::GitHub {
        name: format!("draft {tag}"),
        state: match release {
            None => GitHubState::Create,
            Some(release) if release.draft => GitHubState::Exists,
            Some(_) => GitHubState::Published,
        },
    }];
    if !nightly {
        for krate in &crates {
            let published = crate_versions(&arguments.cargo_index, &krate.name)?;
            steps.push(package_step(
                "crates.io",
                &krate.name,
                &krate.version,
                published,
            ));
        }
    }
    for name in npm_packages(root)? {
        let published = npm_versions(&arguments.npm_registry, &name)?;
        steps.push(package_step("npm", &name, &semver, published));
    }
    let tagged = api
        .find::<serde_json::Value>(&format!("/repos/{repository}/git/ref/tags/{tag}"))?
        .is_some();
    steps.push(Step::GitHub {
        name: format!("tag {tag}"),
        state: if tagged {
            GitHubState::Pushed
        } else {
            GitHubState::Push
        },
    });
    let mut notes = Vec::new();
    if !nightly {
        let (pypi, warnings) = pypi_steps(root, &arguments.pypi, &version.pep440())?;
        steps.extend(pypi);
        notes = warnings;
    }
    steps.push(Step::GitHub {
        name: format!("release {tag}"),
        state: match release {
            Some(release) if !release.draft => GitHubState::Published,
            _ => GitHubState::Publish,
        },
    });

    for step in &steps {
        output::line(&step.to_string());
    }
    check::note(&notes);
    let new_names: Vec<Finding> = steps
        .iter()
        .filter_map(|step| match step {
            Step::Package {
                registry,
                name,
                state: PackageState::NewName,
                ..
            } => Some(Finding::new(
                "new-name",
                format!("{registry} {name}"),
                "not on its registry yet: publish it once by hand, so trusted publishing can take it",
            )),
            _ => None,
        })
        .collect();
    Ok(check::report(&new_names))
}

/// An `error[unprepared]` for each released crate whose manifest does not
/// hold `semver`.
fn unprepared(crates: &[Crate], semver: &str) -> Vec<Finding> {
    crates
        .iter()
        .filter(|krate| krate.released && krate.version != semver)
        .map(|krate| {
            Finding::new(
                "unprepared",
                &krate.manifest,
                format!(
                    "holds {}, not {semver}: run `xtask release prepare --version {semver}` first",
                    krate.version
                ),
            )
        })
        .collect()
}

/// The PyPI steps, the client's project first, and a `pypi-quota` note for
/// each Product's project past its warning.
fn pypi_steps(root: &Path, pypi: &str, pep440: &str) -> Result<(Vec<Step>, Vec<Finding>)> {
    let quotas = pypi_quotas(root)?;
    let (client, products) = pypi_projects(root)?;
    let mut steps = Vec::new();
    let mut notes = Vec::new();
    for (project, product) in [(client, false)]
        .into_iter()
        .chain(products.into_iter().map(|project| (project, true)))
    {
        let releases = pypi_releases(pypi, &project)?;
        if product {
            let quota = *quotas
                .get(&project)
                .with_context(|| format!("{PYPI_QUOTA} records no limit for {project}"))?;
            if let Some(releases) = &releases {
                notes.extend(quota_warning(&project, releases, quota));
            }
        }
        let published = releases.map(|releases| releases.into_keys().collect());
        steps.push(package_step("pypi", &project, pep440, published));
    }
    Ok((steps, notes))
}

fn package_step(
    registry: &'static str,
    name: &str,
    version: &str,
    published: Option<BTreeSet<String>>,
) -> Step {
    Step::Package {
        registry,
        name: name.to_owned(),
        version: version.to_owned(),
        state: match published {
            None => PackageState::NewName,
            Some(versions) if versions.contains(version) => PackageState::Published,
            Some(_) => PackageState::Publish,
        },
    }
}

#[derive(Deserialize)]
struct Release {
    tag_name: String,
    draft: bool,
}

/// A crate the release publishes.
struct Crate {
    name: String,
    version: String,
    /// Its manifest's path, from the workspace's.
    manifest: String,
    /// Whether it is a released crate, at the Product version; an internal
    /// crate keeps the workspace's `0.0.x`.
    released: bool,
}

/// The released crates and every workspace crate they depend on outside
/// their dev-dependencies, which `cargo publish` strips: each after its
/// dependencies, which crates.io needs published first, and in name order
/// otherwise.
fn publish_order(root: &Path) -> Result<Vec<Crate>> {
    let workspace = toml(root, "Cargo.toml")?;
    let empty = Table::new();
    let workspace_table = workspace
        .get("workspace")
        .and_then(Item::as_table)
        .unwrap_or(&empty);
    let shared_dependencies = workspace_table
        .get("dependencies")
        .and_then(Item::as_table)
        .unwrap_or(&empty);
    let workspace_version = workspace_table
        .get("package")
        .and_then(|package| package.get("version"))
        .and_then(Item::as_str);

    let mut manifests = BTreeMap::new();
    let mut pending: Vec<String> = CRATES.iter().map(|folder| (*folder).to_owned()).collect();
    while let Some(folder) = pending.pop() {
        if manifests.contains_key(&folder) {
            continue;
        }
        let path = format!("{folder}/Cargo.toml");
        let manifest = toml(root, &path)?;
        let dependencies = path_dependencies(&manifest, &folder, shared_dependencies);
        pending.extend(dependencies.iter().cloned());
        manifests.insert(folder, (manifest, dependencies));
    }

    let mut by_folder = BTreeMap::new();
    for (folder, (manifest, dependencies)) in &manifests {
        let path = format!("{folder}/Cargo.toml");
        let package = manifest
            .get("package")
            .and_then(Item::as_table)
            .with_context(|| format!("{path} has no [package] table"))?;
        let version = match package.get("version") {
            Some(item) if item.as_str().is_some() => item.as_str(),
            Some(_) => workspace_version,
            None => None,
        }
        .with_context(|| format!("{path} sets no version"))?;
        by_folder.insert(
            folder.clone(),
            (
                Crate {
                    name: name(package, &path)?,
                    version: version.to_owned(),
                    manifest: path,
                    released: CRATES.contains(&folder.as_str()),
                },
                dependencies.clone(),
            ),
        );
    }

    // Dependencies first; among the crates whose dependencies are all
    // placed, the first by name.
    let mut placed = BTreeSet::new();
    let mut ordered = Vec::new();
    while !by_folder.is_empty() {
        let next = by_folder
            .iter()
            .filter(|(_, (_, dependencies))| {
                dependencies
                    .iter()
                    .all(|dependency| placed.contains(dependency))
            })
            .min_by(|(_, (left, _)), (_, (right, _))| left.name.cmp(&right.name))
            .map(|(folder, _)| folder.clone());
        let Some((folder, (krate, _))) = next.and_then(|folder| by_folder.remove_entry(&folder))
        else {
            bail!("the released crates' path dependencies form a cycle");
        };
        placed.insert(folder);
        ordered.push(krate);
    }
    Ok(ordered)
}

/// The folders, from the workspace's, of the workspace crates `manifest`
/// depends on outside its dev-dependencies and [`WORKSPACE_HACK`]: each
/// dependency with a `path`, or taken from a `[workspace.dependencies]`
/// entry with one.
fn path_dependencies(manifest: &DocumentMut, folder: &str, shared: &Table) -> BTreeSet<String> {
    let mut tables: Vec<&Table> = ["dependencies", "build-dependencies"]
        .iter()
        .filter_map(|key| manifest.get(key).and_then(Item::as_table))
        .collect();
    if let Some(targets) = manifest.get("target").and_then(Item::as_table) {
        for (_, target) in targets {
            tables.extend(
                ["dependencies", "build-dependencies"]
                    .iter()
                    .filter_map(|key| target.get(key).and_then(Item::as_table)),
            );
        }
    }
    let mut folders = BTreeSet::new();
    for (key, dependency) in tables
        .into_iter()
        .flat_map(Table::iter)
        .filter(|(key, _)| *key != WORKSPACE_HACK)
    {
        let inherited = dependency
            .get("workspace")
            .and_then(Item::as_bool)
            .unwrap_or(false);
        let path = if inherited {
            shared
                .get(key)
                .and_then(|entry| entry.get("path"))
                .and_then(Item::as_str)
                .map(str::to_owned)
        } else {
            dependency
                .get("path")
                .and_then(Item::as_str)
                .map(|path| format!("{folder}/{path}"))
        };
        if let Some(path) = path {
            folders.insert(normalized(&path));
        }
    }
    folders
}

/// `path` without its `.` and `..` parts.
fn normalized(path: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            part => parts.push(part),
        }
    }
    parts.join("/")
}

/// Each npm package of both Products, in publish order: the Collie
/// client's, then each Product's platform packages before its root
/// package.
fn npm_packages(root: &Path) -> Result<Vec<String>> {
    let mut names = vec![package_json_name(root, CLIENT_PACKAGE)?];
    for product in Product::ALL {
        let name = package_json_name(root, product.key())?;
        names.extend(
            PLATFORMS
                .iter()
                .map(|platform| format!("{name}-{platform}")),
        );
        names.push(name);
    }
    Ok(names)
}

fn package_json_name(root: &Path, folder: &str) -> Result<String> {
    let path = format!("sdks/typescript/{folder}/package.json");
    let package: Named =
        serde_json::from_str(&read(root, &path)?).with_context(|| format!("parsing {path}"))?;
    Ok(package.name)
}

#[derive(Deserialize)]
struct Named {
    name: String,
}

/// The Collie client's PyPI project, and each Product's.
fn pypi_projects(root: &Path) -> Result<(String, Vec<String>)> {
    let client = pyproject_name(root, CLIENT_PACKAGE)?;
    let products = Product::ALL
        .into_iter()
        .map(|product| pyproject_name(root, product.key()))
        .collect::<Result<_>>()?;
    Ok((client, products))
}

fn pyproject_name(root: &Path, folder: &str) -> Result<String> {
    let path = format!("sdks/python/{folder}/pyproject.toml");
    let manifest = toml(root, &path)?;
    let project = manifest
        .get("project")
        .and_then(Item::as_table)
        .with_context(|| format!("{path} has no [project] table"))?;
    name(project, &path)
}

/// Each Product project's storage limit in bytes, from [`PYPI_QUOTA`].
fn pypi_quotas(root: &Path) -> Result<BTreeMap<String, u64>> {
    let quotas = toml(root, PYPI_QUOTA)?;
    quotas
        .iter()
        .map(|(project, gib)| {
            let gib = gib
                .as_integer()
                .and_then(|gib| u64::try_from(gib).ok())
                .with_context(|| format!("{PYPI_QUOTA}: {project} is not a number of GiB"))?;
            Ok((project.to_owned(), gib * GIB))
        })
        .collect()
}

/// A `pypi-quota` note when the sizes of every file of every version of
/// `project` pass [`QUOTA_WARNING_PERCENT`] of its `quota`.
fn quota_warning(
    project: &str,
    releases: &BTreeMap<String, Vec<PypiFile>>,
    quota: u64,
) -> Option<Finding> {
    let used: u64 = releases.values().flatten().map(|file| file.size).sum();
    (used * 100 > quota * QUOTA_WARNING_PERCENT).then(|| {
        Finding::new(
            "pypi-quota",
            project,
            format!(
                "uses {used} of its {quota} bytes, past {QUOTA_WARNING_PERCENT}%: ask pypi/support for a larger limit before the next publish fails"
            ),
        )
    })
}

/// The versions of `name` in the sparse Cargo index at `index`; `None` when
/// the index has never had it.
fn crate_versions(index: &str, name: &str) -> Result<Option<BTreeSet<String>>> {
    let name = name.to_lowercase();
    let path = match name.len() {
        1 => format!("1/{name}"),
        2 => format!("2/{name}"),
        3 => format!("3/{}/{name}", &name[..1]),
        _ => format!("{}/{}/{name}", &name[..2], &name[2..4]),
    };
    let url = format!("{}/{path}", index.trim_end_matches('/'));
    let Some(text) = fetch(&url, |response| response.body_mut().read_to_string())? else {
        return Ok(None);
    };
    text.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str::<IndexEntry>(line)
                .map(|entry| entry.vers)
                .with_context(|| format!("reading {url}"))
        })
        .collect::<Result<_>>()
        .map(Some)
}

#[derive(Deserialize)]
struct IndexEntry {
    vers: String,
}

/// The versions of `name` on the npm registry at `registry`, from its
/// packument; `None` when the registry has never had it.
fn npm_versions(registry: &str, name: &str) -> Result<Option<BTreeSet<String>>> {
    let url = format!(
        "{}/{}",
        registry.trim_end_matches('/'),
        name.replace('/', "%2f")
    );
    let packument: Option<Packument> = fetch_json(&url)?;
    Ok(packument.map(|packument| packument.versions.into_keys().collect()))
}

#[derive(Deserialize)]
struct Packument {
    #[serde(default)]
    versions: BTreeMap<String, serde_json::Value>,
}

/// Each version of `project` on PyPI at `pypi`, with its files, from its
/// JSON interface; `None` when PyPI has never had it.
fn pypi_releases(pypi: &str, project: &str) -> Result<Option<BTreeMap<String, Vec<PypiFile>>>> {
    let url = format!("{}/pypi/{project}/json", pypi.trim_end_matches('/'));
    let project: Option<PypiProject> = fetch_json(&url)?;
    Ok(project.map(|project| project.releases))
}

#[derive(Deserialize)]
struct PypiProject {
    #[serde(default)]
    releases: BTreeMap<String, Vec<PypiFile>>,
}

#[derive(Deserialize)]
struct PypiFile {
    size: u64,
}

fn fetch_json<T: DeserializeOwned>(url: &str) -> Result<Option<T>> {
    fetch(url, |response| response.body_mut().read_json())
}

/// GETs `url` and reads its body with `body`; `None` when the registry
/// answers 404.
fn fetch<T>(
    url: &str,
    body: impl FnOnce(&mut ureq::http::Response<ureq::Body>) -> Result<T, ureq::Error>,
) -> Result<Option<T>> {
    match ureq::get(url)
        .header(
            "User-Agent",
            "goodfetch-xtask (https://github.com/goodfetch/goodfetch)",
        )
        .call()
    {
        Ok(mut response) => body(&mut response)
            .map(Some)
            .with_context(|| format!("reading {url}")),
        Err(ureq::Error::StatusCode(404)) => Ok(None),
        Err(error) => Err(error).with_context(|| format!("reading {url}")),
    }
}
