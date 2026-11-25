//! `xtask release digests`: each Product's crate carries its executable's
//! archive per target, at its URL on the GitHub Release of the crate's
//! version and with its SHA-256 from the release's digest table, so the
//! crate can fetch the executable of its exact version pinned by digest
//! (ADR 0051). The release writes the table into the manifest before
//! `cargo package`; `xtask release plan` publishes no crate whose table, or
//! whose `[package.metadata.binstall]`, does not point at the version's
//! archives (ADR 0094).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use toml_edit::{DocumentMut, InlineTable, Item, Table, Value};

use super::{ProductVersion, Tree, assets, toml};
use crate::check::{self, Finding};
use crate::output;
use crate::products::Product;

/// The `[package.metadata]` key a Product's crate holds its table under.
const KEY: &str = "goodfetch-executables";
/// Where `cargo binstall` finds a Product's archive: on the GitHub Release
/// of the crate's version, named after the executable, which is the
/// crate's name (ADR 0090).
const BINSTALL_URL: &str = "{ repo }/releases/download/v{ version }/{ name }-{ target }.tar.xz";
const BINSTALL_FORMAT: &str = "txz";

/// Each target's archive: its URL, then its SHA-256.
type Executables = BTreeMap<String, (String, String)>;
/// Each Product's [`Executables`].
type Tables = Vec<(Product, Executables)>;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The folder of the GitHub Release's files.
    #[arg(long)]
    assets: PathBuf,
    #[arg(long)]
    version: ProductVersion,
    /// The repository whose GitHub Release holds the archives, as
    /// `owner/name`.
    #[arg(long, default_value = "goodfetch/goodfetch")]
    repository: String,
}

/// Writes each Product crate's table of its archives from the assets
/// folder's digest table, or only an `error[missing]` for each archive the
/// table lacks.
pub(super) fn run(root: &Path, arguments: &Arguments) -> Result<ExitCode> {
    let semver = arguments.version.semver();
    let tables = match tables(&arguments.assets, &arguments.repository, &semver)? {
        Ok(tables) => tables,
        Err(missing) => return Ok(check::report(&missing)),
    };
    for (product, executables) in tables {
        let path = manifest(product);
        let mut document = toml(Tree::Workspace(root), &path)?;
        write(&mut document, &executables, &path)?;
        std::fs::write(root.join(&path), document.to_string())
            .with_context(|| format!("writing {path}"))?;
        output::line(&path);
    }
    Ok(ExitCode::SUCCESS)
}

/// An `error[undigested]` for each Product crate whose table is not its
/// archives in `folder`'s digest table, an `error[binstall]` for each whose
/// `[package.metadata.binstall]` does not point at its archive of the same
/// version, and an `error[missing]` for each archive the table lacks.
pub(super) fn check(
    root: &Path,
    folder: &Path,
    repository: &str,
    semver: &str,
) -> Result<Vec<Finding>> {
    let tables = match tables(folder, repository, semver)? {
        Ok(tables) => tables,
        Err(missing) => return Ok(missing),
    };
    let mut findings = Vec::new();
    for (product, expected) in tables {
        let path = manifest(product);
        let document = toml(Tree::Workspace(root), &path)?;
        let metadata = document
            .get("package")
            .and_then(|package| package.get("metadata"));
        if read(metadata.and_then(|metadata| metadata.get(KEY))) != expected {
            findings.push(Finding::new(
                "undigested",
                &path,
                format!("its [package.metadata.{KEY}] is not v{semver}'s archives in {}'s digest table: run `xtask release digests --assets {} --version {semver}` first", folder.display(), folder.display()),
            ));
        }
        let binstall = metadata.and_then(|metadata| metadata.get("binstall"));
        let field = |key: &str| {
            binstall
                .and_then(|binstall| binstall.get(key))
                .and_then(Item::as_str)
        };
        if field("pkg-url") != Some(BINSTALL_URL) || field("pkg-fmt") != Some(BINSTALL_FORMAT) {
            findings.push(Finding::new(
                "binstall",
                &path,
                format!("its [package.metadata.binstall] must set pkg-url = \"{BINSTALL_URL}\" and pkg-fmt = \"{BINSTALL_FORMAT}\", the archive of the crate's version"),
            ));
        }
    }
    Ok(findings)
}

fn manifest(product: Product) -> String {
    format!("{}/Cargo.toml", product.executable().folder)
}

/// Each Product's archives at `semver` from `folder`'s digest table, or an
/// `error[missing]` for each archive it lacks, or for the table itself.
fn tables(folder: &Path, repository: &str, semver: &str) -> Result<Result<Tables, Vec<Finding>>> {
    let missing = |file: &str| {
        Finding::new(
            "missing",
            file,
            format!(
                "not in {}'s digest table: run `xtask release assets --folder {}` on the built files first",
                folder.display(),
                folder.display()
            ),
        )
    };
    let Some(listed) = assets::listed(folder)? else {
        return Ok(Err(vec![missing(assets::TABLE)]));
    };
    let mut tables = Vec::new();
    let mut findings = Vec::new();
    for product in Product::ALL {
        let mut executables = Executables::new();
        for target in assets::TARGETS {
            let archive = assets::archive(product, target);
            match listed.get(&archive) {
                Some(digest) => {
                    let url = format!(
                        "https://github.com/{repository}/releases/download/v{semver}/{archive}"
                    );
                    executables.insert(target.to_owned(), (url, digest.clone()));
                }
                None => findings.push(missing(&archive)),
            }
        }
        tables.push((product, executables));
    }
    Ok(if findings.is_empty() {
        Ok(tables)
    } else {
        Err(findings)
    })
}

/// The table a manifest holds, each entry's `url` and `sha256`; empty when
/// it holds none.
fn read(table: Option<&Item>) -> Executables {
    table
        .and_then(Item::as_table_like)
        .into_iter()
        .flat_map(|table| table.iter())
        .filter_map(|(target, entry)| {
            let field = |key: &str| entry.get(key).and_then(Item::as_str).map(str::to_owned);
            Some((target.to_owned(), (field("url")?, field("sha256")?)))
        })
        .collect()
}

/// Sets the manifest's table, in place of any earlier one.
fn write(document: &mut DocumentMut, executables: &Executables, path: &str) -> Result<()> {
    let package = document
        .get_mut("package")
        .and_then(Item::as_table_mut)
        .with_context(|| format!("{path} has no [package] table"))?;
    let metadata = package
        .entry("metadata")
        .or_insert_with(|| {
            let mut metadata = Table::new();
            metadata.set_implicit(true);
            Item::Table(metadata)
        })
        .as_table_mut()
        .with_context(|| format!("{path}'s [package.metadata] is not a table"))?;
    let mut table = Table::new();
    table.decor_mut().set_prefix(
        "\n# Each target's executable: its archive on the GitHub Release of this\n# version and its SHA-256, which `xtask release digests` writes before\n# `cargo package` (ADRs 0051 and 0094).\n",
    );
    for (target, (url, sha256)) in executables {
        let mut entry = InlineTable::new();
        entry.insert("url", Value::from(url.as_str()));
        entry.insert("sha256", Value::from(sha256.as_str()));
        table.insert(target, Item::Value(Value::InlineTable(entry)));
    }
    metadata.insert(KEY, Item::Table(table));
    Ok(())
}
