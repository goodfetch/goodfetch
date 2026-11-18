//! The check of the SDK's public items (ADRs 0081, 0095 and 0117): the
//! crate re-exports only items of Foxhound's catalog from
//! `goodfetch-protocol`, every value a connection hands out from
//! `goodfetch-collie-client`, never a glob, and no public type of its own
//! has a lifetime parameter.

#![expect(
    clippy::disallowed_methods,
    reason = "the check reads the crate's sources on the test's own thread, never an async worker"
)]

use std::path::{Path, PathBuf};

use syn::{Generics, Item, UseTree, Visibility};

/// The crate's own folder.
fn crate_folder() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
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

/// The paths `file`'s `pub use` items re-export.
fn re_exports(file: &Path) -> Vec<String> {
    let parsed = syn::parse_file(&std::fs::read_to_string(file).unwrap()).unwrap();
    let mut paths = Vec::new();
    for item in parsed.items {
        if let Item::Use(used) = item
            && matches!(used.vis, Visibility::Public(_))
        {
            leaves(&used.tree, "", &mut paths);
        }
    }
    paths
}

/// The names Foxhound's catalog defines.
fn foxhound_definitions() -> serde_json::Map<String, serde_json::Value> {
    let catalog = crate_folder().join("../../goodfetch-protocol/generated/foxhound.json");
    let catalog: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(catalog).unwrap()).unwrap();
    catalog["$defs"].as_object().unwrap().clone()
}

/// Covers: story:11-1004
#[test]
fn the_sdk_re_exports_the_client_crates_values_and_foxhounds_protocol_items() {
    let definitions = foxhound_definitions();
    let paths = re_exports(&crate_folder().join("src/lib.rs"));

    for path in &paths {
        assert!(!path.ends_with('*'), "{path} is a glob re-export");
        if let Some(name) = path.strip_prefix("goodfetch_protocol::run::") {
            assert!(
                definitions.contains_key(name),
                "{path} is not in Foxhound's catalog"
            );
        }
    }
    for value in ["Connection", "Error", "Locator", "Page", "Run", "Session"] {
        let path = format!("goodfetch_collie_client::{value}");
        assert!(
            paths.contains(&path),
            "{path} is not re-exported: {paths:?}"
        );
    }
}

/// The Rust files under `folder`, but the executable's.
fn sources(folder: &Path, into: &mut Vec<PathBuf>) {
    for entry in std::fs::read_dir(folder).unwrap() {
        let path = entry.unwrap().path();
        if path.ends_with("executable") || path.ends_with("executable.rs") {
            continue;
        }
        if path.is_dir() {
            sources(&path, into);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            into.push(path);
        }
    }
}

/// The public types of `items` with a lifetime parameter, by name.
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

/// Collie's crate checks its own types and the client crate's.
#[test]
fn no_public_type_of_the_sdk_has_a_lifetime_parameter() {
    let mut files = Vec::new();
    sources(&crate_folder().join("src"), &mut files);
    assert!(
        files.iter().any(|file| file.ends_with("foxhound.rs")),
        "{files:?}"
    );

    let mut found = Vec::new();
    for file in &files {
        let parsed = syn::parse_file(&std::fs::read_to_string(file).unwrap()).unwrap();
        with_lifetimes(&parsed.items, &mut found);
    }

    assert_eq!(found, Vec::<String>::new());
}
