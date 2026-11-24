//! The check that keeps the protocol's `foxhound` module Foxhound's (ADR
//! 0099): outside `crates/foxhound/`, only `goodfetch-protocol`, which
//! defines the module, and `xtask` name it. Every Rust file of every other
//! crate is read, its tests, benches and examples included, as tokens, so a
//! path inside a macro invocation counts and a word in a comment does not.
//! A name the crate gives the protocol's crate, in its manifest or in any of
//! its files, counts as the crate's in all of them, and so does a glob of
//! the crate's root, which brings the module in without its name. The check
//! does not resolve names: a module of the crate's own that carries such a
//! name and holds a `foxhound` of its own would be found too.
//! The generated `foxhound_names` are another module (ADR 0087), which any
//! crate may name.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use proc_macro2::{Delimiter, TokenStream, TokenTree};

use crate::check::Finding;
use crate::metadata::Package;
use crate::modules::{is_ident, is_path_separator};

/// The package that defines the module.
const PROTOCOL_PACKAGE: &str = "goodfetch-protocol";
/// That package's crate, as Rust code names it.
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
    let mut paths = Vec::new();
    rust_files(folder, &mut paths)?;
    paths.sort();
    let mut files = Vec::new();
    for path in paths {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let tokens: TokenStream = text
            .parse()
            .map_err(|error| anyhow::anyhow!("reading {} as Rust: {error}", path.display()))?;
        files.push((path, tokens));
    }
    let mut names = BTreeSet::from([PROTOCOL.to_owned()]);
    names.extend(
        package
            .dependencies
            .iter()
            .filter(|dependency| dependency.name == PROTOCOL_PACKAGE)
            .filter_map(|dependency| dependency.rename.as_deref())
            .map(|name| name.replace('-', "_")),
    );
    for (_, tokens) in &files {
        other_names(tokens.clone(), &mut names);
    }
    for (path, tokens) in files {
        if names_the_module(tokens, &names) {
            let relative = path.strip_prefix(root).unwrap_or(&path);
            findings.push(Finding::new(
                "names-foxhound-module",
                relative.to_string_lossy().replace('\\', "/"),
                format!(
                    "`{}` names `{PROTOCOL}::{MODULE}`, or globs the root of `{PROTOCOL}`, whose `{MODULE}` module only `goodfetch-protocol`, `xtask` and the crates under `{FOXHOUND_CRATES}/` name (ADR 0099)",
                    package.name
                ),
            ));
        }
    }
    Ok(())
}

/// Every `.rs` file under `folder`, but those under a folder of
/// [`SKIPPED_FOLDERS`].
pub(crate) fn rust_files(folder: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
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

/// Adds each name a file gives the protocol's crate: `goodfetch_protocol as
/// <name>`, in a `use` or an `extern crate`, and `goodfetch_protocol::{self
/// as <name>}`.
fn other_names(stream: TokenStream, names: &mut BTreeSet<String>) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    for (index, token) in tokens.iter().enumerate() {
        if let TokenTree::Group(group) = token {
            other_names(group.stream(), names);
        }
        if !is_ident(&tokens, index, PROTOCOL) {
            continue;
        }
        if is_ident(&tokens, index + 1, "as")
            && let Some(TokenTree::Ident(name)) = tokens.get(index + 2)
        {
            names.insert(name.to_string());
        }
        if is_path_separator(&tokens, index + 1)
            && let Some(TokenTree::Group(group)) = tokens.get(index + 3)
            && group.delimiter() == Delimiter::Brace
        {
            let inner: Vec<TokenTree> = group.stream().into_iter().collect();
            for start in segment_starts(&inner) {
                if is_ident(&inner, start, "self")
                    && is_ident(&inner, start + 1, "as")
                    && let Some(TokenTree::Ident(name)) = inner.get(start + 2)
                {
                    names.insert(name.to_string());
                }
            }
        }
    }
}

/// Whether a path of `stream` goes from one of the crate's `names` to the
/// module: `<name>::foxhound` or `<name>::*`, alone or in a `{…}` group.
fn names_the_module(stream: TokenStream, names: &BTreeSet<String>) -> bool {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    tokens.iter().enumerate().any(|(index, token)| match token {
        TokenTree::Group(group) => names_the_module(group.stream(), names),
        TokenTree::Ident(ident) => {
            names.contains(&ident.to_string())
                && is_path_separator(&tokens, index + 1)
                && reaches_the_module(&tokens, index + 3)
        }
        _ => false,
    })
}

/// Whether what follows the crate's name and `::`, at `index`, is the
/// module, a glob, or a `{…}` group one of whose paths starts with either.
fn reaches_the_module(tokens: &[TokenTree], index: usize) -> bool {
    match tokens.get(index) {
        Some(TokenTree::Ident(ident)) => ident == MODULE,
        Some(TokenTree::Punct(punct)) => punct.as_char() == '*',
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
            let inner: Vec<TokenTree> = group.stream().into_iter().collect();
            segment_starts(&inner).any(|start| reaches_the_module(&inner, start))
        }
        _ => false,
    }
}

/// Where each comma-separated path of a `{…}` group's tokens starts.
fn segment_starts(tokens: &[TokenTree]) -> impl Iterator<Item = usize> + '_ {
    std::iter::once(0).chain(tokens.iter().enumerate().filter_map(|(index, token)| {
        matches!(token, TokenTree::Punct(punct) if punct.as_char() == ',').then_some(index + 1)
    }))
}
