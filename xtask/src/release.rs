//! `xtask release`: a release's preparation (spec #130). `prepare` sets the
//! one Product version (ADR 0030) in every released manifest of both
//! Products, in each registry's form, and in the lockfiles' entries for
//! them: the Rust crates each Product publishes and the Collie client crate
//! they build on, which the workspace's manifest pins at exactly that
//! version for both SDK crates (ADR 0095), and the TypeScript and Python packages of each Product and
//! of the Collie client (ADRs 0090 and 0095). The internal `goodfetch-*`
//! crates keep the workspace's `0.0.x` version, and the Zapier and Make apps,
//! which their marketplaces version, are not among them.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::num::NonZeroU64;
use std::path::Path;
use std::process::ExitCode;
use std::str::FromStr;

use anyhow::{Context as _, Result, bail};
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::environment::Environment;
use crate::output;
use crate::products;

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
/// The workspace's manifest, whose `[workspace.dependencies]` entry pins
/// the client crate at exactly the Product version for both SDK crates
/// (ADR 0095).
const CARGO_TOML: &str = "Cargo.toml";
/// Each released package's folder under `sdks/typescript/` and
/// `sdks/python/`.
const PACKAGES: [&str; 3] = ["collie", "collie-client", "foxhound"];
const CARGO_LOCK: &str = "Cargo.lock";
const UV_LOCK: &str = "uv.lock";

/// `xtask release`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Sets the Product version in every released manifest of both
    /// Products and in the lockfiles' entries for them, in each registry's
    /// form; writes nothing when one cannot take it.
    Prepare {
        /// `x.y.z`; a release candidate `x.y.0-rc.N`, `N` from 1; or a
        /// nightly `x.y.z-dev.N`, published under npm's `nightly` tag.
        #[arg(long)]
        version: ProductVersion,
    },
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Prepare { version } => {
            for (path, text) in prepare(&environment.root, &version)? {
                std::fs::write(environment.root.join(&path), text)
                    .with_context(|| format!("writing {path}"))?;
                output::line(&path);
            }
            Ok(ExitCode::SUCCESS)
        }
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
fn prepare(root: &Path, version: &ProductVersion) -> Result<Vec<(String, String)>> {
    let semver = version.semver();
    let pep440 = version.pep440();
    let mut written = Vec::new();

    let mut crates = BTreeSet::new();
    let mut client = String::new();
    for folder in CRATES {
        let path = format!("{folder}/Cargo.toml");
        let mut manifest = toml(root, &path)?;
        let package = table(manifest.as_table_mut(), "package", &path)?;
        let crate_name = name(package, &path)?;
        if folder == CLIENT_CRATE {
            client.clone_from(&crate_name);
        }
        crates.insert(crate_name);
        set_string(package, "version", &semver);
        written.push((path, manifest.to_string()));
    }
    written.push(pin_client(root, &client, &semver)?);
    written.push(lock(root, CARGO_LOCK, &crates, &semver, |entry| {
        !entry.contains_key("source")
    })?);

    for package in PACKAGES {
        let path = format!("sdks/typescript/{package}/package.json");
        written.push((
            path.clone(),
            package_json(&read(root, &path)?, &semver, &path)?,
        ));
    }

    let mut projects = Vec::new();
    let mut python = BTreeSet::new();
    for package in PACKAGES {
        let path = format!("sdks/python/{package}/pyproject.toml");
        let mut manifest = toml(root, &path)?;
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
    written.push(lock(root, UV_LOCK, &python, &pep440, |entry| {
        entry
            .get("source")
            .and_then(Item::as_inline_table)
            .is_some_and(|source| source.contains_key("editable"))
    })?);

    Ok(written)
}

/// The workspace's manifest with its dependency on the client crate pinned
/// at exactly `version`; an error when that dependency pins no version.
fn pin_client(root: &Path, client: &str, version: &str) -> Result<(String, String)> {
    let mut manifest = toml(root, CARGO_TOML)?;
    let pin = manifest
        .get_mut("workspace")
        .and_then(Item::as_table_like_mut)
        .and_then(|workspace| workspace.get_mut("dependencies"))
        .and_then(Item::as_table_like_mut)
        .and_then(|dependencies| dependencies.get_mut(client))
        .and_then(Item::as_table_like_mut)
        .and_then(|dependency| dependency.get_mut("version"))
        .and_then(Item::as_value_mut)
        .with_context(|| {
            format!(
                "{CARGO_TOML}'s [workspace.dependencies] pins no version of {client} (ADR 0095)"
            )
        })?;
    replace_string(pin, &format!("={version}"));
    Ok((CARGO_TOML.to_owned(), manifest.to_string()))
}

fn read(root: &Path, path: &str) -> Result<String> {
    std::fs::read_to_string(root.join(path)).with_context(|| format!("reading {path}"))
}

fn toml(root: &Path, path: &str) -> Result<DocumentMut> {
    read(root, path)?
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
    match table.get_mut(key).and_then(Item::as_value_mut) {
        Some(value) => replace_string(value, text),
        None => table[key] = toml_edit::value(text),
    }
}

fn replace_string(value: &mut Value, text: &str) {
    let decor = value.decor().clone();
    *value = Value::from(text);
    *value.decor_mut() = decor;
}

/// The lockfile at `path` with the version of each entry named in `names`
/// that `local` accepts, the workspace's own, set to `version`; an error
/// when one of `names` has no such entry.
fn lock(
    root: &Path,
    path: &str,
    names: &BTreeSet<String>,
    version: &str,
    local: impl Fn(&Table) -> bool,
) -> Result<(String, String)> {
    let mut lockfile = toml(root, path)?;
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
