//! What the checks of a Rust SDK crate's public items read of its sources
//! (ADRs 0081, 0095 and 0099): the paths a file re-exports, the Rust files
//! of a folder, the public types with a lifetime parameter and the public
//! methods of a type. Collie's and Foxhound's crates each check their own.
#![expect(
    clippy::disallowed_methods,
    reason = "the checks read the crates' sources on the test's own thread, never an async worker"
)]

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use syn::{Generics, ImplItem, Item, Type, UseTree, Visibility};

/// The Rust file at `path`, parsed.
fn parsed(path: &Path) -> syn::File {
    let text = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", path.display()));
    syn::parse_file(&text)
        .unwrap_or_else(|error| panic!("{} is no Rust file: {error}", path.display()))
}

/// What a `use` tree brings in: each leaf's path, a glob ending in `*`.
fn leaves(tree: &UseTree, prefix: &str, into: &mut Vec<String>) {
    match tree {
        UseTree::Path(path) => leaves(&path.tree, &format!("{prefix}{}::", path.ident), into),
        UseTree::Name(name) => into.push(format!("{prefix}{}", name.ident)),
        UseTree::Rename(rename) => into.push(format!("{prefix}{}", rename.ident)),
        UseTree::Glob(_) => into.push(format!("{prefix}*")),
        UseTree::Group(group) => {
            for tree in &group.items {
                leaves(tree, prefix, into);
            }
        }
    }
}

/// The paths the `pub use` items of the file at `path` re-export, a glob
/// ending in `*`.
///
/// # Panics
///
/// When the file is unreadable or no Rust file.
#[must_use]
pub fn re_exports(path: &Path) -> Vec<String> {
    let mut paths = Vec::new();
    for item in parsed(path).items {
        if let Item::Use(used) = item
            && matches!(used.vis, Visibility::Public(_))
        {
            leaves(&used.tree, "", &mut paths);
        }
    }
    paths
}

/// The Rust files under `folder`, but the executable's (`executable.rs` and
/// the `executable` folder), which is no part of the SDK.
///
/// # Panics
///
/// When a folder is unreadable.
#[must_use]
pub fn sdk_sources(folder: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    collect_sources(folder, &mut files);
    files
}

/// Pushes the Rust files under `folder`, but the executable's, onto `into`.
fn collect_sources(folder: &Path, into: &mut Vec<PathBuf>) {
    let entries = std::fs::read_dir(folder)
        .unwrap_or_else(|error| panic!("{} is unreadable: {error}", folder.display()));
    for entry in entries {
        let path = entry
            .unwrap_or_else(|error| panic!("{} is unreadable: {error}", folder.display()))
            .path();
        if path.ends_with("executable") || path.ends_with("executable.rs") {
            continue;
        }
        if path.is_dir() {
            collect_sources(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            into.push(path);
        }
    }
}

/// The public types of `files` with a lifetime parameter, by name.
///
/// # Panics
///
/// When a file is unreadable or no Rust file.
#[must_use]
pub fn public_types_with_lifetimes(files: &[PathBuf]) -> Vec<String> {
    let mut found = Vec::new();
    for file in files {
        with_lifetimes(&parsed(file).items, &mut found);
    }
    found
}

/// Pushes the public types of `items` with a lifetime parameter onto `into`.
fn with_lifetimes(items: &[Item], into: &mut Vec<String>) {
    let lifetimes = |generics: &Generics| generics.lifetimes().next().is_some();
    for item in items {
        let (vis, name, generics) = match item {
            Item::Struct(item) => (&item.vis, &item.ident, &item.generics),
            Item::Enum(item) => (&item.vis, &item.ident, &item.generics),
            Item::Union(item) => (&item.vis, &item.ident, &item.generics),
            Item::Trait(item) => (&item.vis, &item.ident, &item.generics),
            Item::Type(item) => (&item.vis, &item.ident, &item.generics),
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    with_lifetimes(items, into);
                }
                continue;
            }
            _ => continue,
        };
        if matches!(vis, Visibility::Public(_)) && lifetimes(generics) {
            into.push(name.to_string());
        }
    }
}

/// The names of the public methods the inherent `impl` blocks of the file
/// at `path` give the type named `receiver`.
///
/// # Panics
///
/// When the file is unreadable or no Rust file.
#[must_use]
pub fn public_methods(path: &Path, receiver: &str) -> BTreeSet<String> {
    let mut methods = BTreeSet::new();
    for item in parsed(path).items {
        let Item::Impl(block) = item else { continue };
        let Type::Path(named) = block.self_ty.as_ref() else {
            continue;
        };
        if block.trait_.is_some() || !named.path.is_ident(receiver) {
            continue;
        }
        for member in block.items {
            if let ImplItem::Fn(method) = member
                && matches!(method.vis, Visibility::Public(_))
            {
                methods.insert(method.sig.ident.to_string());
            }
        }
    }
    methods
}
