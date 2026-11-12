//! The check that keeps the protocol's `foxhound` module Foxhound's (ADR
//! 0099): outside `crates/foxhound/`, only `goodfetch-protocol`, which
//! defines the module, and `xtask` name it. Every Rust file of every other
//! crate is read, its tests, benches and examples included, as tokens, so a
//! path inside a macro invocation counts and a word in a comment does not.
//! The generated `foxhound_names` are another module (ADR 0087), which any
//! crate may name.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use proc_macro2::{TokenStream, TokenTree};

use crate::check::Finding;
use crate::metadata::Package;
use crate::modules::{is_ident, is_path_separator, record_targets};

/// The crate that defines the module, as Rust code names it.
const PROTOCOL: &str = "goodfetch_protocol";
const MODULE: &str = "foxhound";
/// The folder of the crates only Foxhound links, from the workspace's root
/// (ADR 0087).
const FOXHOUND_CRATES: &str = "crates/foxhound";
/// Folders of a crate that hold no code of it: build output, and the trees
/// its tests copy.
const SKIPPED_FOLDERS: [&str; 2] = ["target", "fixtures"];

/// A finding for each Rust file of `package` that names the module. `root`
/// is the workspace's, canonical.
pub(crate) fn check(root: &Path, package: &Package, findings: &mut Vec<Finding>) -> Result<()> {
    let folder = package
        .manifest_path
        .parent()
        .context("a manifest sits in a folder")?;
    if folder.starts_with(root.join(FOXHOUND_CRATES)) {
        return Ok(());
    }
    let mut files = Vec::new();
    rust_files(folder, &mut files)?;
    files.sort();
    for file in files {
        let text = std::fs::read_to_string(&file)
            .with_context(|| format!("reading {}", file.display()))?;
        let tokens: TokenStream = text
            .parse()
            .map_err(|error| anyhow::anyhow!("reading {} as Rust: {error}", file.display()))?;
        let mut names = BTreeSet::from([PROTOCOL.to_owned()]);
        other_names(tokens.clone(), &mut names);
        if names_the_module(tokens, &names) {
            let relative = file.strip_prefix(root).unwrap_or(&file);
            findings.push(Finding::new(
                "names-foxhound-module",
                relative.to_string_lossy().replace('\\', "/"),
                format!(
                    "`{}` names `{PROTOCOL}::{MODULE}`, which only `goodfetch-protocol`, `xtask` and the crates under `{FOXHOUND_CRATES}/` name (ADR 0099)",
                    package.name
                ),
            ));
        }
    }
    Ok(())
}

/// Every `.rs` file under `folder`.
fn rust_files(folder: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        let path = entry.path();
        if path.is_dir() {
            let skipped = path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| SKIPPED_FOLDERS.contains(&name));
            if !skipped {
                rust_files(&path, files)?;
            }
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    Ok(())
}

/// Adds each name the file gives the protocol's crate: `goodfetch_protocol
/// as <name>`, in a `use` or an `extern crate`.
fn other_names(stream: TokenStream, names: &mut BTreeSet<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            other_names(group.stream(), names);
        }
        if is_ident(&tokens, index, PROTOCOL)
            && is_ident(&tokens, index + 1, "as")
            && let Some(TokenTree::Ident(name)) = tokens.get(index + 2)
        {
            names.insert(name.to_string());
        }
    }
}

/// Whether a path of `stream` goes from one of the crate's `names` to the
/// module: `<name>::foxhound`, alone or in a `{…}` group.
fn names_the_module(stream: TokenStream, names: &BTreeSet<String>) -> bool {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    tokens.iter().enumerate().any(|(index, token)| match token {
        TokenTree::Group(group) => names_the_module(group.stream(), names),
        TokenTree::Ident(ident)
            if names.contains(&ident.to_string()) && is_path_separator(&tokens, index + 1) =>
        {
            let mut reached = BTreeSet::new();
            record_targets(&tokens, index + 3, &mut reached);
            reached.contains(MODULE)
        }
        _ => false,
    })
}
