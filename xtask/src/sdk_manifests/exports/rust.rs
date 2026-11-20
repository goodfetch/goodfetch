//! The members a Rust SDK crate exports, read from its source with `syn`:
//! the module tree is walked from `src/lib.rs`, its `#[cfg(test)]` modules
//! left out, and a public item is what a dependent crate reaches by name.
//!
//! - each item a public module declares `pub`, on the module's receiver:
//!   `(crate)` at the root, else the module's path, as `blocking`;
//! - each item of the crate a public module re-exports with `pub use`,
//!   under the name it re-exports it as. Another crate's item the crate
//!   re-exports is that crate's to list, so it is left out;
//! - each `pub` function and constant of an inherent `impl` of such a type,
//!   wherever the `impl` sits, on the type's public path, as
//!   `blocking::Foxhound`; a trait's are the trait's, so a trait `impl`
//!   adds none;
//! - each function of such a trait, on the trait's path.
//!
//! An item or member under `#[doc(hidden)]` is no one's. One in a module
//! named `generated`, or read from a file under a `generated` folder, is the
//! generator's. A member a macro expands to is not read.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};

use super::Exported;

/// The receiver of an item at the crate's root.
const CRATE: &str = "(crate)";
/// The module the generator writes into, at any depth.
const GENERATED: &str = "generated";
/// How many `use` hops a re-export may take to its item.
const HOPS: usize = 16;

/// Where a name a module imports points.
#[derive(Clone)]
enum Target {
    /// A path inside the crate, from its root, ending in the name.
    Crate(Vec<String>),
    /// Another crate's item.
    External,
}

/// How a module is reached and who writes it.
#[derive(Clone, Copy)]
struct Place {
    /// It and each module around it is `pub`.
    public: bool,
    /// The generator writes it.
    generated: bool,
}

/// What an item declares.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    /// A struct, enum or union, whose inherent `impl`s add members.
    Type,
    /// A trait, whose functions are its members.
    Trait,
    /// A function, constant, static or type alias.
    Value,
}

struct Item {
    public: bool,
    kind: Kind,
    /// A trait's functions, `pub` by the trait's.
    functions: Vec<String>,
}

struct Module {
    path: Vec<String>,
    /// It and each module around it is `pub`.
    public: bool,
    /// The generator writes it.
    generated: bool,
    uses: BTreeMap<String, Target>,
    reexports: BTreeMap<String, Target>,
    /// Globs it re-exports, by the module they name.
    glob_reexports: Vec<Target>,
    items: BTreeMap<String, Item>,
}

/// An inherent `impl`'s public members.
struct Impl {
    /// The module it sits in.
    module: usize,
    /// Its self type's path, as written.
    self_path: Vec<String>,
    members: Vec<String>,
}

/// An item a dependent crate reaches.
struct Reached {
    /// The module that declares it, and its name there.
    item: (usize, String),
    /// The module it is reached at.
    at: Vec<String>,
    /// The name it is reached by there.
    name: String,
}

struct Crate {
    /// Its `src` folder.
    source: PathBuf,
    modules: Vec<Module>,
    impls: Vec<Impl>,
}

/// The members the crate in `folder`, named `name`, exports.
pub(super) fn exports(folder: &Path, name: &str) -> Result<Vec<Exported>> {
    let source = folder.join("src");
    let lib = source.join("lib.rs");
    let file = parse(&lib)?;
    let mut read = Crate {
        source: source.clone(),
        modules: Vec::new(),
        impls: Vec::new(),
    };
    let root = Place {
        public: true,
        generated: false,
    };
    read.module(&[], root, &file.items, &source, &lib)?;
    Ok(read.exported(name))
}

fn parse(path: &Path) -> Result<syn::File> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))
}

fn is_public(visibility: &syn::Visibility) -> bool {
    matches!(visibility, syn::Visibility::Public(_))
}

/// Whether `attributes` hold the attribute `words`, as `cfg(test)`.
fn holds(attributes: &[syn::Attribute], words: &str) -> bool {
    attributes.iter().any(|attribute| {
        quote::ToTokens::to_token_stream(&attribute.meta)
            .to_string()
            .replace(' ', "")
            == words
    })
}

/// Whether `attributes` hold `#[cfg(test)]` or `#[doc(hidden)]`.
fn left_out(attributes: &[syn::Attribute]) -> bool {
    holds(attributes, "cfg(test)") || holds(attributes, "doc(hidden)")
}

/// Each name a `use` tree brings in, with the path it names.
fn use_tree(tree: &syn::UseTree, prefix: &mut Vec<String>, found: &mut Vec<(String, Vec<String>)>) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            use_tree(&path.tree, prefix, found);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let ident = name.ident.to_string();
            let mut segments = prefix.clone();
            if ident == "self" {
                if let Some(last) = prefix.last() {
                    found.push((last.clone(), segments));
                }
            } else {
                segments.push(ident.clone());
                found.push((ident, segments));
            }
        }
        syn::UseTree::Rename(rename) => {
            let mut segments = prefix.clone();
            segments.push(rename.ident.to_string());
            found.push((rename.rename.to_string(), segments));
        }
        syn::UseTree::Glob(_) => found.push(("*".to_owned(), prefix.clone())),
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                use_tree(tree, prefix, found);
            }
        }
    }
}

/// The path `segments`, written in the module at `path` among its child
/// modules `children`, from the crate's root.
fn resolve(path: &[String], children: &[String], segments: &[String]) -> Target {
    let Some(first) = segments.first() else {
        return Target::External;
    };
    let mut base = match first.as_str() {
        "crate" => Vec::new(),
        "self" => path.to_vec(),
        "super" => {
            let mut base = path.to_vec();
            let mut rest = segments;
            while rest.first().is_some_and(|segment| segment == "super") {
                base.pop();
                rest = &rest[1..];
            }
            base.extend(rest.iter().cloned());
            return Target::Crate(base);
        }
        name if children.iter().any(|child| child == name) => {
            let mut base = path.to_vec();
            base.push(name.to_owned());
            base.extend(segments[1..].iter().cloned());
            return Target::Crate(base);
        }
        _ => return Target::External,
    };
    base.extend(segments[1..].iter().cloned());
    Target::Crate(base)
}

impl Crate {
    fn module(
        &mut self,
        path: &[String],
        place: Place,
        items: &[syn::Item],
        children: &Path,
        file: &Path,
    ) -> Result<()> {
        let child_modules: Vec<String> = items
            .iter()
            .filter_map(|item| match item {
                syn::Item::Mod(child) => Some(child.ident.to_string()),
                _ => None,
            })
            .collect();
        let index = self.modules.len();
        let mut module = Module {
            path: path.to_vec(),
            public: place.public,
            generated: place.generated,
            uses: BTreeMap::new(),
            reexports: BTreeMap::new(),
            glob_reexports: Vec::new(),
            items: BTreeMap::new(),
        };
        let mut nested = Vec::new();
        for item in items {
            match item {
                syn::Item::Use(used) => {
                    let mut found = Vec::new();
                    use_tree(&used.tree, &mut Vec::new(), &mut found);
                    let reexported = is_public(&used.vis) && !left_out(&used.attrs);
                    for (name, segments) in found {
                        let target = resolve(path, &child_modules, &segments);
                        if name == "*" {
                            if reexported {
                                module.glob_reexports.push(target);
                            }
                            continue;
                        }
                        if reexported {
                            module.reexports.insert(name.clone(), target.clone());
                        }
                        module.uses.insert(name, target);
                    }
                }
                syn::Item::Mod(child) if !holds(&child.attrs, "cfg(test)") => nested.push(child),
                syn::Item::Impl(inherent) => self.impls.extend(inherent_impl(index, inherent)),
                other => {
                    if let Some((name, declared)) = declared(other) {
                        module.items.insert(name, declared);
                    }
                }
            }
        }
        self.modules.push(module);
        for child in nested {
            let name = child.ident.to_string();
            let mut child_path = path.to_vec();
            child_path.push(name.clone());
            let mut child_place = Place {
                public: place.public && is_public(&child.vis) && !left_out(&child.attrs),
                generated: place.generated || name == GENERATED,
            };
            if let Some((_, items)) = &child.content {
                self.module(&child_path, child_place, items, &children.join(&name), file)?;
                continue;
            }
            let (found, grandchildren) = module_file(child, children, file)?;
            child_place.generated |= found
                .strip_prefix(&self.source)
                .unwrap_or(&found)
                .components()
                .any(|component| component.as_os_str() == GENERATED);
            let parsed = parse(&found)?;
            self.module(
                &child_path,
                child_place,
                &parsed.items,
                &grandchildren,
                &found,
            )?;
        }
        Ok(())
    }

    fn module_at(&self, path: &[String]) -> Option<usize> {
        self.modules.iter().position(|module| module.path == path)
    }

    /// The module and name of the item a crate path names, following its
    /// modules' `use`s; `None` for another crate's or a name it lacks.
    fn item_at(&self, segments: &[String], hops: usize) -> Option<(usize, String)> {
        let (name, path) = segments.split_last()?;
        let module = self.module_at(path)?;
        if self.modules[module].items.contains_key(name) {
            return Some((module, name.clone()));
        }
        if hops == 0 {
            return None;
        }
        match self.modules[module].uses.get(name)? {
            Target::Crate(next) if next.as_slice() != segments => self.item_at(next, hops - 1),
            _ => None,
        }
    }

    /// Each item a dependent crate reaches, with the module and name it
    /// reaches it at.
    fn reached(&self) -> Vec<Reached> {
        let mut reached = Vec::new();
        let mut reach = |item: (usize, String), at: &[String], name: &str| {
            reached.push(Reached {
                item,
                at: at.to_vec(),
                name: name.to_owned(),
            });
        };
        for (index, module) in self.modules.iter().enumerate() {
            if !module.public {
                continue;
            }
            for (name, item) in &module.items {
                if item.public {
                    reach((index, name.clone()), &module.path, name);
                }
            }
            for (name, target) in &module.reexports {
                if let Target::Crate(segments) = target
                    && let Some(found) = self.item_at(segments, HOPS)
                {
                    reach(found, &module.path, name);
                }
            }
            for target in &module.glob_reexports {
                let Target::Crate(segments) = target else {
                    continue;
                };
                let Some(globbed) = self.module_at(segments) else {
                    continue;
                };
                for (name, item) in &self.modules[globbed].items {
                    if item.public {
                        reach((globbed, name.clone()), &module.path, name);
                    }
                }
            }
        }
        reached
    }

    /// The type an `impl` in `module` names by `self_path`.
    fn impl_target(&self, module: usize, self_path: &[String]) -> Option<(usize, String)> {
        let path = &self.modules[module].path;
        if let [name] = self_path {
            if self.modules[module].items.contains_key(name) {
                return Some((module, name.clone()));
            }
            return match self.modules[module].uses.get(name)? {
                Target::Crate(segments) => self.item_at(segments, HOPS),
                Target::External => None,
            };
        }
        let children: Vec<String> = self
            .modules
            .iter()
            .filter(|child| child.path.len() == path.len() + 1 && child.path.starts_with(path))
            .filter_map(|child| child.path.last().cloned())
            .collect();
        match resolve(path, &children, self_path) {
            Target::Crate(segments) => self.item_at(&segments, HOPS),
            Target::External => None,
        }
    }

    fn exported(&self, krate: &str) -> Vec<Exported> {
        let generated = |module: usize| self.modules[module].generated;
        let id = |module: usize, name: &str| {
            let mut path = self.modules[module].path.clone();
            path.push(name.to_owned());
            path.join("::")
        };
        let receiver = |path: &[String]| {
            if path.is_empty() {
                CRATE.to_owned()
            } else {
                path.join("::")
            }
        };
        let export = |receiver: String, member: String, id: String, generated: bool| Exported {
            entry: krate.to_owned(),
            receiver,
            member,
            id,
            home: krate.to_owned(),
            package: Some(krate.to_owned()),
            generated,
            library: false,
        };
        let mut exported = Vec::new();
        for Reached {
            item: (module, name),
            at,
            name: as_name,
        } in self.reached()
        {
            let item = &self.modules[module].items[&name];
            let item_id = id(module, &name);
            exported.push(export(
                receiver(&at),
                as_name.clone(),
                item_id.clone(),
                generated(module),
            ));
            let mut path = at.clone();
            path.push(as_name);
            let owner = path.join("::");
            match item.kind {
                Kind::Value => {}
                Kind::Trait => {
                    for function in &item.functions {
                        exported.push(export(
                            owner.clone(),
                            function.clone(),
                            format!("{item_id}.{function}"),
                            generated(module),
                        ));
                    }
                }
                Kind::Type => {
                    for inherent in &self.impls {
                        if self.impl_target(inherent.module, &inherent.self_path)
                            != Some((module, name.clone()))
                        {
                            continue;
                        }
                        for member in &inherent.members {
                            exported.push(export(
                                owner.clone(),
                                member.clone(),
                                format!("{item_id}.{member}"),
                                generated(inherent.module),
                            ));
                        }
                    }
                }
            }
        }
        exported
    }
}

/// The public members of an inherent `impl` in the module `module`; `None`
/// for a trait's `impl` or one of a type with no path.
fn inherent_impl(module: usize, inherent: &syn::ItemImpl) -> Option<Impl> {
    if inherent.trait_.is_some() {
        return None;
    }
    let syn::Type::Path(self_type) = inherent.self_ty.as_ref() else {
        return None;
    };
    let members = inherent
        .items
        .iter()
        .filter_map(|member| match member {
            syn::ImplItem::Fn(function)
                if is_public(&function.vis) && !left_out(&function.attrs) =>
            {
                Some(function.sig.ident.to_string())
            }
            syn::ImplItem::Const(constant)
                if is_public(&constant.vis) && !left_out(&constant.attrs) =>
            {
                Some(constant.ident.to_string())
            }
            _ => None,
        })
        .collect();
    Some(Impl {
        module,
        self_path: self_type
            .path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect(),
        members,
    })
}

/// The file of the module `child` that `file` declares without a body,
/// and the folder its own modules' files sit in. A `#[path]` is relative to
/// `file`'s folder, and the file it names holds its modules' files beside
/// it, as `mod.rs` does; otherwise the file sits in `children`.
fn module_file(child: &syn::ItemMod, children: &Path, file: &Path) -> Result<(PathBuf, PathBuf)> {
    let name = child.ident.to_string();
    let beside = |found: &Path| found.parent().map(Path::to_path_buf).unwrap_or_default();
    if let Some(named) = path_attribute(&child.attrs) {
        let found = file.parent().unwrap_or(Path::new("")).join(named);
        if !found.is_file() {
            bail!(
                "{}: module `{name}` has no file at {}",
                file.display(),
                found.display()
            );
        }
        let folder = beside(&found);
        return Ok((found, folder));
    }
    let flat = children.join(format!("{name}.rs"));
    if flat.is_file() {
        return Ok((flat, children.join(&name)));
    }
    let nested = children.join(&name).join("mod.rs");
    if nested.is_file() {
        let folder = beside(&nested);
        return Ok((nested, folder));
    }
    bail!(
        "{}: module `{name}` has no file at {}",
        file.display(),
        flat.display()
    )
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

/// The name and item an item of a module declares, `None` for one that
/// declares no name a dependent crate could reach.
fn declared(item: &syn::Item) -> Option<(String, Item)> {
    let (ident, visibility, attributes, kind) = match item {
        syn::Item::Struct(item) => (&item.ident, &item.vis, &item.attrs, Kind::Type),
        syn::Item::Enum(item) => (&item.ident, &item.vis, &item.attrs, Kind::Type),
        syn::Item::Union(item) => (&item.ident, &item.vis, &item.attrs, Kind::Type),
        syn::Item::Trait(item) => (&item.ident, &item.vis, &item.attrs, Kind::Trait),
        syn::Item::Fn(item) => (&item.sig.ident, &item.vis, &item.attrs, Kind::Value),
        syn::Item::Const(item) => (&item.ident, &item.vis, &item.attrs, Kind::Value),
        syn::Item::Static(item) => (&item.ident, &item.vis, &item.attrs, Kind::Value),
        syn::Item::Type(item) => (&item.ident, &item.vis, &item.attrs, Kind::Value),
        _ => return None,
    };
    let functions = match item {
        syn::Item::Trait(item) => item
            .items
            .iter()
            .filter_map(|member| match member {
                syn::TraitItem::Fn(function) if !left_out(&function.attrs) => {
                    Some(function.sig.ident.to_string())
                }
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    };
    Some((
        ident.to_string(),
        Item {
            public: is_public(visibility) && !left_out(attributes),
            kind,
            functions,
        },
    ))
}
