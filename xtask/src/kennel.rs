//! The Kennel's workspace (ADR 0141), no member of the root one: its folder,
//! and the check that its manifest keeps the root's rules. Cargo passes no
//! `[workspace.lints]`, `[workspace.package]` or `[workspace.dependencies]`
//! to a nested workspace, so the Kennel's manifest copies the root's, and
//! the check fails when a copy drifts: its lints, the version of each
//! dependency both declare, and its `rust-version`, the root's, since the
//! Kennel builds `goodfetch-protocol`, whose `rust-version` is the root's.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::Path;

use anyhow::{Context as _, Result};
use toml::{Table, Value};

use crate::check::Finding;

/// The Kennel's workspace folder, from the repository's root.
pub(crate) const FOLDER: &str = "kennel";

const MANIFEST: &str = "Cargo.toml";

/// The findings of the Kennel's manifest against the root's, at `root`.
pub(crate) fn check(root: &Path) -> Result<Vec<Finding>> {
    let root_workspace = workspace_table(&root.join(MANIFEST))?;
    let kennel_workspace = workspace_table(&root.join(FOLDER).join(MANIFEST))?;
    let mut findings = Vec::new();
    if kennel_workspace.get("lints") != root_workspace.get("lints") {
        findings.push(Finding::new(
            "lints",
            "workspace.lints",
            "the Kennel's `[workspace.lints]` differ from the root's: copy the root's tables",
        ));
    }
    let (root_rust_version, kennel_rust_version) = (
        rust_version(&root_workspace),
        rust_version(&kennel_workspace),
    );
    if kennel_rust_version != root_rust_version {
        findings.push(Finding::new(
            "rust-version",
            "workspace.package.rust-version",
            format!(
                "the Kennel declares {}, the root {}: the Kennel builds goodfetch-protocol, so its MSRV is the root's (ADR 0030)",
                shown(kennel_rust_version),
                shown(root_rust_version),
            ),
        ));
    }
    let root_dependencies = dependencies(&root_workspace);
    for (name, kennel_dependency) in dependencies(&kennel_workspace).into_iter().flatten() {
        let Some(root_dependency) = root_dependencies.and_then(|table| table.get(name)) else {
            continue;
        };
        let (kennel_version, root_version) = (version(kennel_dependency), version(root_dependency));
        if kennel_version != root_version {
            findings.push(Finding::new(
                "dependency-version",
                format!("workspace.dependencies.{name}"),
                format!(
                    "the Kennel takes {}, the root {}: take the root's",
                    shown(kennel_version),
                    shown(root_version),
                ),
            ));
        }
    }
    Ok(findings)
}

/// The `[workspace]` table of the manifest at `path`.
fn workspace_table(path: &Path) -> Result<Table> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let mut manifest: Table =
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    Ok(match manifest.remove("workspace") {
        Some(Value::Table(workspace)) => workspace,
        _ => Table::new(),
    })
}

/// `[workspace.package] rust-version`.
fn rust_version(workspace: &Table) -> Option<&Value> {
    workspace
        .get("package")
        .and_then(|package| package.get("rust-version"))
}

/// `[workspace.dependencies]`.
fn dependencies(workspace: &Table) -> Option<&Table> {
    workspace.get("dependencies").and_then(Value::as_table)
}

/// A dependency's version requirement: the whole value when it is a
/// string, its `version` key when it is a table, none for a path
/// dependency.
fn version(dependency: &Value) -> Option<&Value> {
    match dependency {
        Value::String(_) => Some(dependency),
        Value::Table(table) => table.get("version"),
        _ => None,
    }
}

fn shown(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => format!("`{text}`"),
        Some(other) => format!("{other:?}"),
        None => "none".to_owned(),
    }
}
