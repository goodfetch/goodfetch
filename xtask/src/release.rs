//! `xtask release`: a release's preparation (spec #130). `prepare` sets the
//! one Product version (ADR 0030) in every released manifest of both
//! Products, in each registry's form, and in the lockfiles' entries for
//! them: the Rust crates each Product publishes and the Collie client crate
//! they build on, which the workspace's manifest pins at exactly that
//! version for both SDK crates (ADR 0095), and the TypeScript and Python packages of each Product and
//! of the Collie client (ADRs 0090 and 0095). The internal `goodfetch-*`
//! crates keep the workspace's `0.0.x` version, and the Zapier and Make apps,
//! which their marketplaces version, are not among them. Every crate the
//! release publishes, the internal ones the released crates depend on
//! included, is marked publishable, and each path dependency on one carries
//! the version `cargo publish` needs.
//!
//! The release line (ADR 0032): `cut` gives a minor its `release/x.y`
//! branch, `backports` lists the fixes it lacks, `range` gives a release's
//! git-cliff range, `changelog` writes its entries in `CHANGELOG.md` and
//! `check` holds a release, or a promotion to `x.y.0`, at its gates;
//! `prepare` holds a patch that changes a format (ADR 0066).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod changelog;
mod gates;
mod line;

use std::collections::{BTreeMap, BTreeSet};
use std::num::NonZeroU64;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use toml_edit::{DocumentMut, InlineTable, Item, Table, TableLike, Value};

use crate::environment::Environment;
use crate::git;
use crate::github;
use crate::output;
use crate::products;

mod publish;

/// The Collie client crate's folder.
const CLIENT_CRATE: &str = "crates/goodfetch-collie-client";
/// Each released Rust crate's folder: each Product's executable crate,
/// published as its Rust SDK (ADR 0090), and the Collie client crate both
/// take (ADR 0095).
const CRATES: [&str; 3] = [
    products::COLLIE.folder,
    products::FOXHOUND.folder,
    CLIENT_CRATE,
];
/// The workspace's manifest, whose `[workspace.dependencies]` entries pin
/// each published crate: the client crate at exactly the Product version
/// for both SDK crates (ADR 0095), an internal crate at its `0.0.x`.
const CARGO_TOML: &str = "Cargo.toml";
/// Each released package's folder under `sdks/typescript/` and
/// `sdks/python/`.
const PACKAGES: [&str; 3] = ["collie", "collie-client", "foxhound"];
const CARGO_LOCK: &str = "Cargo.lock";

/// The suffix of a minor's cut point tag on `main`, `x.y-branch` (ADR 0032).
const CUT_POINT: &str = "-branch";
const UV_LOCK: &str = "uv.lock";
/// Each library outside the workspace that takes a released crate by a
/// range of the Product's line, as its author would publish it (story
/// 11-180): repinned to the new version's, or the workspace no longer
/// resolves its dev-dependency on it.
const LIBRARIES: [&str; 1] =
    ["crates/foxhound/goodfetch-foxhound/tests/libraries/connection-library"];

/// `xtask release`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Sets the Product version in every released manifest of both
    /// Products and in the lockfiles' entries for them, in each registry's
    /// form; writes nothing when one cannot take it, or, for a patch, when
    /// a format's version or schema at `HEAD` differs from `x.y.0`'s (ADR
    /// 0066).
    Prepare {
        /// `x.y.z`; a release candidate `x.y.0-rc.N`, `N` from 1; or a
        /// nightly `x.y.z-dev.N`, published under npm's `nightly` tag.
        #[arg(long)]
        version: ProductVersion,
    },
    /// Plans both Products' publish: one step per registry and package in
    /// publish order, each marked published, to publish, or a new name to
    /// publish by hand first, as its registry's public interface shows it.
    Plan(publish::Arguments),
    /// Cuts minor `x.y` from `origin`'s `main`, once its release gates
    /// pass: pushes the `release/x.y` branch and the `x.y-branch` tag at
    /// `main`'s commit and the first candidate's version commit on
    /// `release-pr/x.y.0-rc.1`, then opens that candidate's release pull
    /// request against the branch. `main` is left as it was.
    Cut {
        #[arg(long)]
        minor: Minor,
        /// The Feature report of the milestone the minor ships
        /// (`xtask features report`'s `features.json`).
        #[arg(long)]
        feature_report: PathBuf,
        #[command(flatten)]
        github: github::Arguments,
    },
    /// Lists the commits on `main` since minor `x.y` was cut whose pull
    /// requests carry the `backport` label and that `release/x.y` lacks.
    Backports {
        #[arg(long)]
        minor: Minor,
        #[command(flatten)]
        github: github::Arguments,
    },
    /// Prints the git-cliff range of `version`'s changelog, ending at
    /// `to`: from the previous minor's cut point for a first candidate and
    /// a stable minor, the whole history before the first cut; from the
    /// previous release tag for a later candidate and a patch.
    Range {
        #[arg(long)]
        version: ProductVersion,
        #[arg(long, default_value = "HEAD")]
        to: String,
    },
    /// Writes `version`'s entries above the earlier releases' in
    /// `CHANGELOG.md`, with git-cliff, from the commits of its range up to
    /// `to`: under Foxhound only a commit whose every changed path is on
    /// Foxhound's side, under both Products every other (ADR 0094).
    Changelog {
        #[arg(long)]
        version: ProductVersion,
        #[arg(long, default_value = "HEAD")]
        to: String,
        /// The release's date; today when not given.
        #[arg(long, hide = true)]
        at: Option<jiff::Timestamp>,
    },
    /// Holds a release of `version`, built from `commit`, at its gates: an
    /// empty quarantine list and no untested Feature for every release; for
    /// a promotion to `x.y.0`, no unexplained Live-check failure and no open
    /// fuzz crash, on a version commit over the last candidate; for a patch,
    /// every format's version and schema as `x.y.0` released them.
    Check {
        #[arg(long)]
        version: ProductVersion,
        /// The Feature report of the milestone the release ships.
        #[arg(long)]
        feature_report: PathBuf,
        #[arg(long, default_value = "HEAD")]
        commit: String,
        /// The private repository whose open `Crash in …` issues are the
        /// fuzz crashes (ADR 0067).
        #[arg(long, default_value = "goodfetch/goodfetch-fuzzing")]
        fuzz_repository: String,
        /// The time the two business days are counted to; now when not
        /// given.
        #[arg(long, hide = true)]
        at: Option<jiff::Timestamp>,
        #[command(flatten)]
        github: github::Arguments,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    let root = &environment.root;
    match command {
        Command::Plan(arguments) => publish::run(
            &environment.root,
            &arguments,
            environment.github_token.as_deref(),
        ),
        Command::Prepare { version } => {
            let findings = gates::patch(root, "HEAD", &version)?;
            if !findings.is_empty() {
                return Ok(crate::check::report(&findings));
            }
            for (path, text) in prepare(Tree::Workspace(root), &version)? {
                std::fs::write(root.join(&path), text)
                    .with_context(|| format!("writing {path}"))?;
                output::line(&path);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Cut {
            minor,
            feature_report,
            github,
        } => {
            let findings = gates::release(root, &feature_report)?;
            if !findings.is_empty() {
                return Ok(crate::check::report(&findings));
            }
            let api = github.authenticated(environment.token())?;
            line::cut(root, minor, &github.repository, api)?;
            Ok(ExitCode::SUCCESS)
        }
        Command::Backports { minor, github } => {
            for backport in line::backports(
                root,
                minor,
                &github.repository,
                github.api(environment.token()),
            )? {
                output::line(&backport);
            }
            Ok(ExitCode::SUCCESS)
        }
        Command::Range { version, to } => {
            output::line(&line::range(root, &version, &to)?);
            Ok(ExitCode::SUCCESS)
        }
        Command::Changelog { version, to, at } => {
            changelog::write(root, &version, &to, at.unwrap_or_else(now))?;
            output::line(changelog::CHANGELOG);
            Ok(ExitCode::SUCCESS)
        }
        Command::Check {
            version,
            feature_report,
            commit,
            fuzz_repository,
            at,
            github,
        } => {
            let mut findings = gates::release(root, &feature_report)?;
            let at = at.unwrap_or_else(now);
            let api = github.api(environment.token());
            findings.extend(gates::version(
                root,
                &version,
                &commit,
                &gates::Issues {
                    api,
                    repository: &github.repository,
                    fuzz_repository: &fuzz_repository,
                    at,
                },
            )?);
            Ok(crate::check::report(&findings))
        }
    }
}

/// The time a promotion is checked at, or a release's changelog dated,
/// when `--at` does not give it.
#[expect(
    clippy::disallowed_methods,
    reason = "ADR 0059: the two business days of ADR 0064 count to the wall clock's now, and a release is dated today; tests pass --at"
)]
fn now() -> jiff::Timestamp {
    jiff::Timestamp::now()
}

/// Whether the repository at `root` has tag `tag`.
fn tagged(root: &Path, tag: &str) -> Result<bool> {
    Ok(git::succeeds(
        root,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("refs/tags/{tag}"),
        ],
    )?)
}

/// A minor, `x.y`, whose release line `release/x.y` carries.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Minor {
    major: u64,
    minor: u64,
}

impl Minor {
    /// Its release branch.
    fn branch(self) -> String {
        format!("release/{self}")
    }

    /// The tag of its cut point on `main`.
    fn cut_point(self) -> String {
        format!("{self}{CUT_POINT}")
    }
}

impl std::fmt::Display for Minor {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}.{}", self.major, self.minor)
    }
}

impl FromStr for Minor {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let numbers: Vec<Option<u64>> = text.split('.').map(number).collect();
        let [Some(major), Some(minor)] = numbers[..] else {
            return Err(format!("`{text}` is not a minor: x.y"));
        };
        Ok(Self { major, minor })
    }
}

/// A Product version a release takes (ADR 0030): a stable or patch
/// `x.y.z`, a release candidate of a minor, or a nightly.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ProductVersion {
    major: u64,
    minor: u64,
    patch: u64,
    stage: Stage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stage {
    Stable,
    /// `-rc.N`, of an `x.y.0` only.
    Candidate(NonZeroU64),
    /// `-dev.N`.
    Nightly(u64),
}

impl ProductVersion {
    /// The form crates.io and npm take: `1.4.0`, `1.4.0-rc.2`,
    /// `1.5.0-dev.7`.
    fn semver(&self) -> String {
        let core = format!("{}.{}.{}", self.major, self.minor, self.patch);
        match self.stage {
            Stage::Stable => core,
            Stage::Candidate(number) => format!("{core}-rc.{number}"),
            Stage::Nightly(number) => format!("{core}-dev.{number}"),
        }
    }

    /// The form PyPI takes (PEP 440): `1.4.0`, `1.4.0rc2`, `1.5.0.dev7`.
    fn pep440(&self) -> String {
        let core = format!("{}.{}.{}", self.major, self.minor, self.patch);
        match self.stage {
            Stage::Stable => core,
            Stage::Candidate(number) => format!("{core}rc{number}"),
            Stage::Nightly(number) => format!("{core}.dev{number}"),
        }
    }

    fn minor(&self) -> Minor {
        Minor {
            major: self.major,
            minor: self.minor,
        }
    }

    /// Its release tag: `v1.4.0`, `v1.4.0-rc.2`.
    fn tag(&self) -> String {
        format!("v{}", self.semver())
    }
}

impl FromStr for ProductVersion {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let invalid = || {
            format!(
                "`{text}` is not a release version: x.y.z, x.y.0-rc.N with N from 1, or x.y.z-dev.N"
            )
        };
        let (core, pre_release) = match text.split_once('-') {
            Some((core, pre_release)) => (core, Some(pre_release)),
            None => (text, None),
        };
        let numbers: Vec<Option<u64>> = core.split('.').map(number).collect();
        let [Some(major), Some(minor), Some(patch)] = numbers[..] else {
            return Err(invalid());
        };
        let stage = match pre_release {
            None => Stage::Stable,
            Some(pre_release) => {
                if let Some(rest) = pre_release.strip_prefix("rc.") {
                    match number(rest).and_then(NonZeroU64::new) {
                        Some(candidate) if patch == 0 => Stage::Candidate(candidate),
                        _ => return Err(invalid()),
                    }
                } else if let Some(rest) = pre_release.strip_prefix("dev.") {
                    Stage::Nightly(number(rest).ok_or_else(invalid)?)
                } else {
                    return Err(invalid());
                }
            }
        };
        Ok(Self {
            major,
            minor,
            patch,
            stage,
        })
    }
}

/// A version's number: digits, without a leading zero.
fn number(text: &str) -> Option<u64> {
    let digits = !text.is_empty() && text.bytes().all(|byte| byte.is_ascii_digit());
    if !digits || (text.len() > 1 && text.starts_with('0')) {
        return None;
    }
    text.parse().ok()
}

/// Each released manifest's and lockfile's path, from the workspace's, and
/// its text with `version` set; an error, before anything is written, when
/// one is missing or cannot take it.
fn prepare(tree: Tree<'_>, version: &ProductVersion) -> Result<Vec<(String, String)>> {
    let semver = version.semver();
    let pep440 = version.pep440();
    let mut written = Vec::new();

    let published = publish::publish_order(tree)?;
    let requirements: BTreeMap<&str, String> = published
        .iter()
        .map(|krate| {
            let requirement = if krate.released {
                format!("={semver}")
            } else {
                krate.version.clone()
            };
            (krate.folder.as_str(), requirement)
        })
        .collect();
    let mut workspace = toml(tree, CARGO_TOML)?;
    let shared = published_entries(&workspace, &requirements);
    let mut crates = BTreeSet::new();
    for krate in &published {
        let path = &krate.manifest;
        let mut manifest = toml(tree, path)?;
        let package = table(manifest.as_table_mut(), "package", path)?;
        if krate.released {
            crates.insert(krate.name.clone());
            set_string(package, "version", &semver);
        }
        set(package, "publish", Value::from(true));
        for dependencies in dependency_tables(&mut manifest, &publish::KEPT) {
            require_versions(dependencies, &krate.folder, &requirements);
        }
        for dependencies in dependency_tables(&mut manifest, &["dev-dependencies"]) {
            unversion(dependencies, &krate.folder, &shared);
        }
        written.push((path.clone(), manifest.to_string()));
    }
    if let Some(dependencies) = workspace_dependencies(&mut workspace) {
        require_versions(dependencies, "", &requirements);
    }
    written.push((CARGO_TOML.to_owned(), workspace.to_string()));
    let line: BTreeMap<&str, String> = published
        .iter()
        .filter(|krate| krate.released)
        .map(|krate| (krate.folder.as_str(), format!("^{semver}")))
        .collect();
    for library in LIBRARIES {
        let path = format!("{library}/Cargo.toml");
        let mut manifest = toml(tree, &path)?;
        for dependencies in dependency_tables(&mut manifest, &publish::KEPT) {
            require_versions(dependencies, library, &line);
        }
        written.push((path, manifest.to_string()));
    }
    written.push(lock(tree, CARGO_LOCK, &crates, &semver, |entry| {
        !entry.contains_key("source")
    })?);

    for package in PACKAGES {
        let path = format!("sdks/typescript/{package}/package.json");
        written.push((
            path.clone(),
            package_json(&tree.read(&path)?, &semver, &path)?,
        ));
    }

    let mut projects = Vec::new();
    let mut python = BTreeSet::new();
    for package in PACKAGES {
        let path = format!("sdks/python/{package}/pyproject.toml");
        let mut manifest = toml(tree, &path)?;
        python.insert(name(
            table(manifest.as_table_mut(), "project", &path)?,
            &path,
        )?);
        projects.push((path, manifest));
    }
    for (path, mut manifest) in projects {
        let project = table(manifest.as_table_mut(), "project", &path)?;
        set_string(project, "version", &pep440);
        if let Some(dependencies) = project.get_mut("dependencies").and_then(Item::as_array_mut) {
            for dependency in dependencies.iter_mut() {
                let pinned = dependency
                    .as_str()
                    .and_then(|requirement| repin(requirement, &python, &pep440));
                if let Some(pinned) = pinned {
                    replace_string(dependency, &pinned);
                }
            }
        }
        written.push((path, manifest.to_string()));
    }
    written.push(lock(tree, UV_LOCK, &python, &pep440, |entry| {
        entry
            .get("source")
            .and_then(Item::as_inline_table)
            .is_some_and(|source| source.contains_key("editable"))
    })?);

    Ok(written)
}

/// `manifest`'s dependency tables of the `kinds` given, each target's
/// included.
fn dependency_tables<'manifest>(
    manifest: &'manifest mut DocumentMut,
    kinds: &[&str],
) -> Vec<&'manifest mut dyn TableLike> {
    let mut tables = Vec::new();
    for (key, item) in manifest.as_table_mut().iter_mut() {
        if kinds.contains(&key.get()) {
            tables.extend(item.as_table_like_mut());
        } else if key.get() == "target"
            && let Some(targets) = item.as_table_like_mut()
        {
            for (_, target) in targets.iter_mut() {
                let Some(target) = target.as_table_like_mut() else {
                    continue;
                };
                for (key, item) in target.iter_mut() {
                    if kinds.contains(&key.get()) {
                        tables.extend(item.as_table_like_mut());
                    }
                }
            }
        }
    }
    tables
}

/// The workspace manifest's `[workspace.dependencies]`.
fn workspace_dependencies(workspace: &mut DocumentMut) -> Option<&mut dyn TableLike> {
    workspace
        .get_mut("workspace")
        .and_then(Item::as_table_like_mut)
        .and_then(|workspace| workspace.get_mut("dependencies"))
        .and_then(Item::as_table_like_mut)
}

/// Each `[workspace.dependencies]` entry on a published crate, by its key,
/// with its `path` from the workspace's folder: the entries
/// `require_versions` gives a version.
fn published_entries(
    workspace: &DocumentMut,
    requirements: &BTreeMap<&str, String>,
) -> BTreeMap<String, InlineTable> {
    let Some(dependencies) = workspace
        .get("workspace")
        .and_then(|workspace| workspace.get("dependencies"))
        .and_then(Item::as_table_like)
    else {
        return BTreeMap::new();
    };
    let mut entries = BTreeMap::new();
    for (key, dependency) in dependencies.iter() {
        let Some(dependency) = dependency.as_table_like() else {
            continue;
        };
        let Some(path) = dependency.get("path").and_then(Item::as_str) else {
            continue;
        };
        if !requirements.contains_key(publish::normalized(path).as_str()) {
            continue;
        }
        let mut entry = InlineTable::new();
        for (key, item) in dependency.iter() {
            if let Some(value) = item.as_value() {
                entry.insert(key, value.clone());
            }
        }
        entries.insert(key.to_owned(), entry);
    }
    entries
}

/// Writes out each dev-dependency in `dependencies`, of the crate in
/// `folder`, that takes its entry from `shared`, without the version that
/// entry carries: `cargo publish` keeps a dev-dependency that carries one,
/// so the crate would wait on its test dependencies being published first,
/// and strips one that carries none.
fn unversion(
    dependencies: &mut dyn TableLike,
    folder: &str,
    shared: &BTreeMap<String, InlineTable>,
) {
    for (key, dependency) in dependencies.iter_mut() {
        let Some(entry) = shared.get(key.get()) else {
            continue;
        };
        let Some(own) = dependency.as_table_like() else {
            continue;
        };
        if !own
            .get("workspace")
            .and_then(Item::as_bool)
            .unwrap_or(false)
        {
            continue;
        }
        let mut written = InlineTable::new();
        for (key, value) in entry {
            match (key, value.as_str()) {
                ("version", _) => {}
                ("path", Some(path)) => {
                    written.insert(
                        key,
                        Value::from(relative(folder, &publish::normalized(path))),
                    );
                }
                _ => {
                    written.insert(key, value.clone());
                }
            }
        }
        for (key, item) in own.iter() {
            let Some(value) = item.as_value() else {
                continue;
            };
            if key == "workspace" {
                continue;
            }
            if key == "features"
                && let (Some(added), Some(Value::Array(features))) =
                    (value.as_array(), written.get_mut("features"))
            {
                features.extend(added.iter().cloned());
                continue;
            }
            written.insert(key, value.clone());
        }
        written.fmt();
        *dependency = Item::Value(Value::InlineTable(written));
    }
}

/// The path from the folder `from` to the folder `to`, both from the
/// workspace's.
fn relative(from: &str, to: &str) -> String {
    let from: Vec<&str> = from.split('/').collect();
    let to: Vec<&str> = to.split('/').collect();
    let common = from
        .iter()
        .zip(&to)
        .take_while(|(from, to)| from == to)
        .count();
    let mut parts = vec![".."; from.len() - common];
    parts.extend(&to[common..]);
    if parts.is_empty() {
        ".".to_owned()
    } else {
        parts.join("/")
    }
}

/// Gives each dependency in `dependencies` whose `path`, from `folder`
/// (empty for the workspace's own manifest), is a published crate's the
/// version `cargo publish` needs, which `requirements` holds by folder:
/// exactly the Product version for a released crate (ADR 0095), an
/// internal crate's own `0.0.x` (ADR 0030). A dependency taken from the
/// workspace's entry is pinned there.
fn require_versions(
    dependencies: &mut dyn TableLike,
    folder: &str,
    requirements: &BTreeMap<&str, String>,
) {
    for (_, dependency) in dependencies.iter_mut() {
        let Some(dependency) = dependency.as_table_like_mut() else {
            continue;
        };
        let requirement = dependency
            .get("path")
            .and_then(Item::as_str)
            .and_then(|path| {
                requirements.get(publish::normalized(&format!("{folder}/{path}")).as_str())
            });
        let Some(requirement) = requirement else {
            continue;
        };
        if let Some(version) = dependency.get_mut("version").and_then(Item::as_value_mut) {
            replace_string(version, requirement);
        } else {
            dependency.insert("version", toml_edit::value(requirement.as_str()));
            // `{ path = "…" , version = "…" }` otherwise: the space after
            // the last value stays with it.
            dependency.fmt();
        }
    }
}

/// Where [`prepare`] reads the released manifests and lockfiles from.
#[derive(Clone, Copy)]
enum Tree<'a> {
    /// The files in the workspace's folder.
    Workspace(&'a Path),
    /// A commit's, in the repository at `root`: `main`'s commit, which `cut`
    /// builds the first candidate's version commit over, and the stable
    /// version commit's parent, which `check` prepares again to compare
    /// (ADR 0032).
    Commit { root: &'a Path, commit: &'a str },
}

impl Tree<'_> {
    fn read(self, path: &str) -> Result<String> {
        match self {
            Self::Workspace(root) => {
                std::fs::read_to_string(root.join(path)).with_context(|| format!("reading {path}"))
            }
            Self::Commit { root, commit } => {
                git::text(root, &["show", &format!("{commit}:{path}")])
                    .with_context(|| format!("reading {path} at {commit}"))
            }
        }
    }
}

fn toml(tree: Tree<'_>, path: &str) -> Result<DocumentMut> {
    tree.read(path)?
        .parse()
        .with_context(|| format!("parsing {path}"))
}

fn table<'document>(
    parent: &'document mut Table,
    key: &str,
    path: &str,
) -> Result<&'document mut Table> {
    parent
        .get_mut(key)
        .and_then(Item::as_table_mut)
        .with_context(|| format!("{path} has no [{key}] table"))
}

fn name(table: &Table, path: &str) -> Result<String> {
    table
        .get("name")
        .and_then(Item::as_str)
        .map(str::to_owned)
        .with_context(|| format!("{path} names no package"))
}

/// Sets `key` to the string `text`, keeping the value's place and spacing;
/// `version.workspace = true` becomes `version = "…"`.
fn set_string(table: &mut Table, key: &str, text: &str) {
    set(table, key, Value::from(text));
}

/// Sets `key` to `new`, keeping the value's place and spacing;
/// `publish.workspace = true` becomes `publish = …`.
fn set(table: &mut Table, key: &str, new: Value) {
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(value) => replace(value, new),
        None => table[key] = Item::Value(new),
    }
}

fn replace_string(value: &mut Value, text: &str) {
    replace(value, Value::from(text));
}

fn replace(value: &mut Value, new: Value) {
    let decor = value.decor().clone();
    *value = new;
    *value.decor_mut() = decor;
}

/// The lockfile at `path` with the version of each entry named in `names`
/// that `local` accepts, the workspace's own, set to `version`; an error
/// when one of `names` has no such entry.
fn lock(
    tree: Tree<'_>,
    path: &str,
    names: &BTreeSet<String>,
    version: &str,
    local: impl Fn(&Table) -> bool,
) -> Result<(String, String)> {
    let mut lockfile = toml(tree, path)?;
    let entries = lockfile
        .get_mut("package")
        .and_then(Item::as_array_of_tables_mut)
        .with_context(|| format!("{path} has no [[package]] entries"))?;
    let mut missing = names.clone();
    for entry in entries.iter_mut() {
        let Some(name) = entry.get("name").and_then(Item::as_str) else {
            continue;
        };
        if names.contains(name) && local(entry) {
            missing.remove(name);
            set_string(entry, "version", version);
        }
    }
    if let Some(name) = missing.first() {
        bail!("{path} has no entry for the workspace's {name}");
    }
    Ok((path.to_owned(), lockfile.to_string()))
}

/// `text`, a `package.json`, with its `version` set; its formatting is
/// npm's, so the one `"version": "…"` member is replaced as written.
fn package_json(text: &str, version: &str, path: &str) -> Result<String> {
    let parsed: serde_json::Value =
        serde_json::from_str(text).with_context(|| format!("parsing {path}"))?;
    let current = parsed
        .get("version")
        .and_then(serde_json::Value::as_str)
        .with_context(|| format!("{path} has no version"))?;
    let member = |version: &str| format!("\"version\": {}", serde_json::Value::from(version));
    let written = member(current);
    if text.matches(&written).count() != 1 {
        bail!("{path} does not write its version once as `{written}`");
    }
    Ok(text.replacen(&written, &member(version), 1))
}

/// A requirement `name==…` that pins one of the workspace's own Python
/// packages exactly (ADR 0095), pinned to `version` instead.
fn repin(requirement: &str, names: &BTreeSet<String>, version: &str) -> Option<String> {
    let (name, _) = requirement.split_once("==")?;
    names.contains(name).then(|| format!("{name}=={version}"))
}
