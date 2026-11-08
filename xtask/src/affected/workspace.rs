//! What the rules need to know of the repository's three workspaces (ADR
//! 0032): each Cargo crate, pnpm package and uv package, its folder and the
//! members of its own workspace it depends on, and where the visual tests are
//! (ADR 0073). Cargo's members come from `cargo metadata`; pnpm's from
//! `pnpm-workspace.yaml` and uv's from the root `pyproject.toml`, whose
//! patterns use `*` for one folder name. The Kennel's workspace, no member
//! of the root one, is read for what of the root's its build and tests take
//! (ADR 0141).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use super::{RepoPath, imports};
use crate::{console, kennel, metadata, package_dependencies, tlc, tree};

pub(crate) struct Workspace {
    pub(crate) crates: Vec<Crate>,
    pub(crate) packages: Vec<Package>,
    /// Every visual test file, where Playwright's and Vitest's `visual`
    /// projects look for them.
    pub(crate) visual_tests: Vec<RepoPath>,
    pub(crate) models: Vec<Model>,
    /// The packages whose build a crate embeds: a change to one changes
    /// that crate's executable.
    pub(crate) embedded: Vec<Embedded>,
    pub(crate) kennel: Kennel,
}

/// What the Kennel's workspace (ADR 0141), which `cargo metadata` of the
/// root leaves out, takes from the root's crates: its job runs when a
/// change reaches one as it would reach a crate's build or the executables
/// its tests run. Empty when the repository has no Kennel.
#[derive(Default)]
pub(crate) struct Kennel {
    /// The root's crates its crates depend on, of any kind:
    /// `goodfetch-protocol`.
    pub(crate) dependencies: BTreeSet<String>,
    /// The root's crates whose executables its tests run, from its crates'
    /// `[package.metadata.goodfetch] runs`.
    pub(crate) runs: BTreeSet<String>,
}

/// A package whose build a crate embeds, as Collie's executable embeds the
/// Console's (ADR 0057).
pub(crate) struct Embedded {
    /// The package's folder.
    pub(crate) package: RepoPath,
    /// The crate that embeds it.
    pub(crate) krate: String,
}

/// A TLA+ model (ADR 0060).
pub(crate) struct Model {
    pub(crate) name: String,
    pub(crate) folder: RepoPath,
    /// The files and folders of the modules it traces.
    pub(crate) traces: Vec<RepoPath>,
}

/// A member of the Cargo workspace.
pub(crate) struct Crate {
    pub(crate) name: String,
    pub(crate) folder: RepoPath,
    /// The workspace members it depends on, of any kind (normal, build, dev).
    pub(crate) dependencies: BTreeSet<String>,
    /// The workspace members its build depends on (normal and build
    /// dependencies, not dev): a change to one changes its executable.
    pub(crate) built_from: BTreeSet<String>,
    /// Its bench targets' names.
    pub(crate) benches: BTreeSet<String>,
    /// Whether it builds an executable (a `bin` target).
    pub(crate) executable: bool,
    /// The crates whose executables its tests run through the test tool
    /// manifest, from `[package.metadata.goodfetch] runs`: a change to one
    /// reaches it as a dependency's would.
    pub(crate) runs: BTreeSet<String>,
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
    /// The members of its own workspace it depends on, or its files
    /// import.
    pub(crate) dependencies: BTreeSet<String>,
    /// The members its build and its code at run time depend on, not its
    /// tests alone: a pnpm package's `dependencies`, a uv project's
    /// `dependencies`.
    pub(crate) runtime_dependencies: BTreeSet<String>,
    /// The crates whose executables its tests run through the test tool
    /// manifest, from `goodfetch.runs` in `package.json` or
    /// `[tool.goodfetch] runs` in `pyproject.toml`: a change to one reaches
    /// it as a dependency's would.
    pub(crate) runs: BTreeSet<String>,
    /// Whether every run of its runner loads it, whatever the tested
    /// package declares: a package the root Vitest or Playwright config
    /// imports, or a pytest plugin. A change to it reaches every package
    /// of its ecosystem.
    pub(crate) loaded_by_every_run: bool,
}

pub(crate) fn read(cargo: &OsStr, root: &Path) -> Result<Workspace> {
    // cargo metadata reports canonical paths; folders are named from the root.
    let root = &root
        .canonicalize()
        .with_context(|| format!("reading {}", root.display()))?;
    let mut packages = typescript_packages(root)?;
    packages.extend(python_packages(root)?);
    let mut visual_tests = Vec::new();
    find_visual_tests(root, root, &mut visual_tests)?;
    visual_tests.sort();
    let crates = crates(cargo, root)?;
    let embedded = embedded(&crates, &packages);
    let kennel = kennel(cargo, root, &crates)?;
    Ok(Workspace {
        crates,
        packages,
        visual_tests,
        models: tlc::models(root)?
            .into_iter()
            .map(|model| Model {
                name: model.name,
                folder: RepoPath::new(model.folder),
                traces: model.traces.into_iter().map(RepoPath::new).collect(),
            })
            .collect(),
        embedded,
        kennel,
    })
}

/// What the Kennel's workspace takes from `crates`, the root's, read from
/// its own `cargo metadata`.
fn kennel(cargo: &OsStr, root: &Path, crates: &[Crate]) -> Result<Kennel> {
    let folder = root.join(kennel::FOLDER);
    if !folder.join("Cargo.toml").is_file() {
        return Ok(Kennel::default());
    }
    let root_crate = |name: &String| crates.iter().any(|krate| &krate.name == name);
    let mut found = Kennel::default();
    for package in metadata::packages(cargo, &folder)? {
        found.dependencies.extend(
            package
                .dependencies
                .iter()
                .map(|dependency| &dependency.name)
                .filter(|name| root_crate(name))
                .cloned(),
        );
        found.runs.extend(package.runs());
    }
    Ok(found)
}

/// Each Product's Console, whose build its executable embeds (ADRs 0057
/// and 0087), when both exist: the crate is the one whose folder holds the
/// build's.
pub(super) fn embedded(crates: &[Crate], packages: &[Package]) -> Vec<Embedded> {
    let mut embedded = Vec::new();
    for product_console in console::CONSOLES {
        let package = RepoPath::new(product_console.package);
        let build = RepoPath::new(product_console.build);
        let krate = crates
            .iter()
            .filter(|krate| build.is_in(krate.folder.as_str()))
            .max_by_key(|krate| krate.folder.as_str().len());
        if let Some(krate) = krate
            && packages.iter().any(|candidate| candidate.folder == package)
        {
            embedded.push(Embedded {
                package,
                krate: krate.name.clone(),
            });
        }
    }
    embedded
}

/// The root files whose imports every run of a pnpm runner loads: the
/// Vitest and Playwright configs.
fn runner_config_imports(root: &Path, names: &BTreeSet<String>) -> Result<BTreeSet<String>> {
    let mut loaded = BTreeSet::new();
    for entry in std::fs::read_dir(root).with_context(|| format!("reading {}", root.display()))? {
        let entry = entry.with_context(|| format!("reading {}", root.display()))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        let config = ["vitest.config.", "playwright.config."]
            .iter()
            .any(|prefix| name.starts_with(prefix));
        if config && entry.file_type()?.is_file() {
            let text = std::fs::read_to_string(entry.path())
                .with_context(|| format!("reading {}", entry.path().display()))?;
            loaded.extend(imports::typescript(&text, names));
        }
    }
    Ok(loaded)
}

/// Whether the file name is a visual test's: `*.visual.spec.ts` for
/// Playwright, `*.visual.test.ts[x]` for Vitest (TESTING.md).
pub(crate) fn is_visual_test(name: &str) -> bool {
    [".visual.spec.ts", ".visual.test.ts", ".visual.test.tsx"]
        .iter()
        .any(|suffix| name.ends_with(suffix))
}

fn find_visual_tests(root: &Path, folder: &Path, found: &mut Vec<RepoPath>) -> Result<()> {
    let entries =
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?;
    for entry in entries {
        let path = entry
            .with_context(|| format!("reading {}", folder.display()))?
            .path();
        let name = path.file_name().and_then(OsStr::to_str).unwrap_or_default();
        let metadata = std::fs::symlink_metadata(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        if metadata.is_dir() {
            // The visual projects look in no fixtures folder and not in the
            // proofs either (`playwright.config.ts`'s `testIgnore`).
            let folder = relative(root, &path)?;
            let skipped = tree::hidden(name)
                || tree::never_entered(folder.as_str())
                || name == tree::FIXTURES
                || folder.as_str() == tree::PROOFS;
            if !skipped {
                find_visual_tests(root, &path, found)?;
            }
        } else if metadata.is_file() && is_visual_test(name) {
            found.push(relative(root, &path)?);
        }
    }
    Ok(())
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
                built_from: package
                    .dependencies
                    .iter()
                    .filter(|dependency| {
                        members.contains(dependency.name.as_str())
                            && dependency.kind.as_deref() != Some("dev")
                    })
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
                runs: package.runs().into_iter().collect(),
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
    #[serde(default)]
    goodfetch: Option<GoodfetchField>,
}

/// `goodfetch` in a `package.json`, or `[tool.goodfetch]` in a
/// `pyproject.toml`.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GoodfetchField {
    /// The crates whose executables the package's tests run.
    #[serde(default)]
    runs: BTreeSet<String>,
}

fn runs(goodfetch: Option<GoodfetchField>) -> BTreeSet<String> {
    goodfetch
        .map(|goodfetch| goodfetch.runs)
        .unwrap_or_default()
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
    let loaded = runner_config_imports(root, &names)?;
    let mut imported = Vec::new();
    for (folder, manifest) in &manifests {
        let mut found = imports::of_folder(
            &root.join(folder.as_str()),
            &imports::SCRIPT_EXTENSIONS,
            |text| imports::typescript(text, &names),
        )?;
        found.remove(&manifest.name);
        imported.push(found);
    }
    Ok(manifests
        .into_iter()
        .zip(imported)
        .map(|((folder, manifest), imported)| Package {
            ecosystem: Ecosystem::TypeScript,
            loaded_by_every_run: loaded.contains(&manifest.name),
            runtime_dependencies: manifest
                .dependencies
                .keys()
                .filter(|name| names.contains(*name))
                .cloned()
                .collect(),
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
            .chain(imported)
            .collect(),
            name: manifest.name,
            runs: runs(manifest.goodfetch),
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
    /// `[project.entry-points]`: a `pytest11` group makes it a pytest
    /// plugin, which every pytest run of the workspace loads.
    #[serde(default, rename = "entry-points")]
    entry_points: BTreeMap<String, toml::Value>,
}

#[derive(Deserialize)]
struct Tool {
    uv: Option<Uv>,
    goodfetch: Option<GoodfetchField>,
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
        let runs = runs(pyproject.tool.and_then(|tool| tool.goodfetch));
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
        let plugin = project.entry_points.contains_key("pytest11");
        let runtime: Vec<String> = project.dependencies.clone();
        members.push((
            relative(root, &folder)?,
            python_name(&project.name),
            requirements,
            runs,
            (plugin, runtime),
        ));
    }
    let names: BTreeSet<String> = members
        .iter()
        .map(|(_, name, _, _, _)| name.clone())
        .collect();
    let mut packages = Vec::new();
    for (folder, name, requirements, runs, (plugin, runtime)) in members {
        let mut imported = imports::of_folder(&root.join(folder.as_str()), &["py"], |text| {
            imports::python(text, &names)
        })?;
        imported.remove(&name);
        packages.push(Package {
            ecosystem: Ecosystem::Python,
            dependencies: requirements
                .iter()
                .map(|requirement| python_name(requirement))
                .filter(|dependency| names.contains(dependency))
                .chain(imported)
                .collect(),
            runtime_dependencies: runtime
                .iter()
                .map(|requirement| python_name(requirement))
                .filter(|dependency| names.contains(dependency))
                .collect(),
            name,
            folder,
            runs,
            loaded_by_every_run: plugin,
        });
    }
    Ok(packages)
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
