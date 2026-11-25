//! How `xtask` reads a crate's Rust source the way `rustc` does: the file a
//! `mod x;` names (`x.rs`, `x/mod.rs` or the file of its `#[path]`), the
//! read-and-parse of a source file, and which items only a test build
//! compiles. The module check, the SDK manifests' exports reader and the
//! code generator's index read the crates through it.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use syn::punctuated::Punctuated;

/// Where the modules a file or an inline module declares find their files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Folders {
    /// The folder `x.rs` and `x/mod.rs` sit in.
    pub(crate) children: PathBuf,
    /// The folder a `#[path]` is relative to.
    pub(crate) path_base: PathBuf,
}

impl Folders {
    /// A crate root's: `lib.rs` and `main.rs` hold their modules' files
    /// beside them.
    pub(crate) fn root(file: &Path) -> Self {
        let beside = beside(file);
        Self {
            children: beside.clone(),
            path_base: beside,
        }
    }

    /// The inline module `name`'s, declared where these are.
    pub(crate) fn inline(&self, name: &str) -> Self {
        let folder = self.children.join(name);
        Self {
            children: folder.clone(),
            path_base: folder,
        }
    }
}

/// The file a `mod x;` names, and where the modules it declares find theirs.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ModuleFile {
    pub(crate) path: PathBuf,
    pub(crate) folders: Folders,
}

/// The file of `module`, declared without a body in the file `declaring`,
/// whose modules find their files in `at`. A `#[path]` is relative to `at`'s
/// path base, and the file it names holds its own modules' files beside it,
/// as a `mod.rs` does; a flat `x.rs` holds them in the folder `x`.
pub(crate) fn module_file(
    module: &syn::ItemMod,
    declaring: &Path,
    at: &Folders,
) -> Result<ModuleFile> {
    let name = module.ident.to_string();
    let beside_it = |path: PathBuf| {
        let folder = beside(&path);
        ModuleFile {
            path,
            folders: Folders {
                children: folder.clone(),
                path_base: folder,
            },
        }
    };
    if let Some(named) = path_attribute(&module.attrs) {
        let path = at.path_base.join(named);
        if !path.is_file() {
            bail!(
                "{}: module `{name}`'s #[path] names no file: {}",
                declaring.display(),
                path.display()
            );
        }
        return Ok(beside_it(path));
    }
    let flat = at.children.join(format!("{name}.rs"));
    if flat.is_file() {
        return Ok(ModuleFile {
            folders: Folders {
                children: at.children.join(&name),
                path_base: at.children.clone(),
            },
            path: flat,
        });
    }
    let nested = at.children.join(&name).join("mod.rs");
    if nested.is_file() {
        return Ok(beside_it(nested));
    }
    bail!(
        "{}: module `{name}` has no file at {}",
        declaring.display(),
        flat.display()
    )
}

/// The source file at `path`, parsed.
pub(crate) fn parse(path: &Path) -> Result<syn::File> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Whether `attributes` hold a `#[cfg]` only a test build meets, as
/// `cfg(test)` or `cfg(all(test, …))`.
pub(crate) fn test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .parse_args::<syn::Meta>()
                .is_ok_and(|predicate| needs_test(&predicate))
    })
}

/// Whether the `cfg` predicate holds only when `test` does.
fn needs_test(predicate: &syn::Meta) -> bool {
    match predicate {
        syn::Meta::Path(path) => path.is_ident("test"),
        syn::Meta::List(list) => {
            let Ok(nested) =
                list.parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
            else {
                return false;
            };
            if list.path.is_ident("all") {
                nested.iter().any(needs_test)
            } else if list.path.is_ident("any") {
                !nested.is_empty() && nested.iter().all(needs_test)
            } else {
                false
            }
        }
        syn::Meta::NameValue(_) => false,
    }
}

/// The path a `#[path = "…"]` attribute names.
fn path_attribute(attributes: &[syn::Attribute]) -> Option<String> {
    attributes.iter().find_map(|attribute| {
        let syn::Meta::NameValue(pair) = &attribute.meta else {
            return None;
        };
        if !pair.path.is_ident("path") {
            return None;
        }
        match &pair.value {
            syn::Expr::Lit(syn::ExprLit {
                lit: syn::Lit::Str(path),
                ..
            }) => Some(path.value()),
            _ => None,
        }
    })
}

/// The folder `file` sits in.
fn beside(file: &Path) -> PathBuf {
    file.parent().map(Path::to_path_buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A crate whose root declares `mod route;` through a `#[path]` to a
    /// file outside `src`, which itself declares `mod hop;`.
    fn crate_with_a_path_attribute() -> tempfile::TempDir {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("src");
        let shared = folder.path().join("shared");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(
            source.join("lib.rs"),
            "#[path = \"../shared/route.rs\"]\nmod route;\n",
        )
        .unwrap();
        std::fs::write(shared.join("route.rs"), "mod hop;\n").unwrap();
        std::fs::write(shared.join("hop.rs"), "").unwrap();
        folder
    }

    fn first_module(file: &Path) -> syn::ItemMod {
        parse(file)
            .unwrap()
            .items
            .into_iter()
            .find_map(|item| match item {
                syn::Item::Mod(module) => Some(module),
                _ => None,
            })
            .unwrap()
    }

    #[test]
    fn module_with_a_path_attribute_is_the_file_it_names_holding_its_modules_beside_it() {
        let folder = crate_with_a_path_attribute();
        let lib = folder.path().join("src/lib.rs");
        let shared = folder.path().join("src/../shared");

        let found = module_file(&first_module(&lib), &lib, &Folders::root(&lib)).unwrap();

        assert_eq!(
            found,
            ModuleFile {
                path: shared.join("route.rs"),
                folders: Folders {
                    children: shared.clone(),
                    path_base: shared,
                },
            }
        );
    }

    #[test]
    fn module_a_path_attributes_file_declares_is_found_beside_that_file() {
        let folder = crate_with_a_path_attribute();
        let lib = folder.path().join("src/lib.rs");
        let route = module_file(&first_module(&lib), &lib, &Folders::root(&lib)).unwrap();

        let hop = module_file(&first_module(&route.path), &route.path, &route.folders).unwrap();

        assert_eq!(hop.path, folder.path().join("src/../shared/hop.rs"));
    }

    #[test]
    fn module_without_a_path_attribute_is_its_flat_file_holding_its_modules_in_its_folder() {
        let folder = tempfile::tempdir().unwrap();
        let source = folder.path().join("src");
        std::fs::create_dir_all(&source).unwrap();
        std::fs::write(source.join("lib.rs"), "mod route;\n").unwrap();
        std::fs::write(source.join("route.rs"), "").unwrap();
        let lib = source.join("lib.rs");

        let found = module_file(&first_module(&lib), &lib, &Folders::root(&lib)).unwrap();

        assert_eq!(
            found,
            ModuleFile {
                path: source.join("route.rs"),
                folders: Folders {
                    children: source.join("route"),
                    path_base: source,
                },
            }
        );
    }
}
