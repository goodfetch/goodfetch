//! The members a Rust SDK crate exports, read from its source with `syn`:
//! the module tree is walked from `src/lib.rs`, following `#[path]`, with
//! what only a test build compiles left out, and its names are resolved as
//! the compiler resolves them, through `use`s, globs and `super`. A public
//! item is what a dependent crate reaches by name from the crate's root:
//!
//! - each item a module it reaches declares `pub`, on the module's
//!   receiver: `(crate)` at the root, else the path it reaches the module
//!   by, as `blocking`;
//! - each item of the crate such a module re-exports, by name or by a glob,
//!   under the name it re-exports it as; a re-exported module is reached
//!   like a declared one. Another crate's item the crate re-exports is that
//!   crate's to list, so it is left out;
//! - each `pub` function and constant of an inherent `impl` of such a type,
//!   wherever the `impl` sits, on the path the type is reached by, as
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
use syn::punctuated::Punctuated;

use super::Exported;

/// The receiver of an item at the crate's root.
const CRATE: &str = "(crate)";
/// The module the generator writes into, at any depth.
const GENERATED: &str = "generated";
/// How many `use`s and globs a name may go through to what it names.
const HOPS: usize = 16;

/// What a name names in a module.
#[derive(Clone, PartialEq, Eq)]
enum Named {
    Module(usize),
    /// The module declaring the item, and its name there.
    Item(usize, String),
}

/// A path a `use` names, as written.
struct Use {
    segments: Vec<String>,
    /// A `pub use`: it re-exports what it names.
    public: bool,
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
    /// The path it is declared at.
    path: Vec<String>,
    parent: Option<usize>,
    /// The generator writes it.
    generated: bool,
    /// Its child modules, and whether each is `pub`.
    children: BTreeMap<String, (usize, bool)>,
    uses: BTreeMap<String, Use>,
    globs: Vec<Use>,
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
    /// The path of the module it is reached at.
    at: Vec<String>,
    /// The name it is reached by there.
    name: String,
}

/// Where a module's source sits.
struct Source<'a> {
    /// The file holding it.
    file: &'a Path,
    /// The folder its child modules' files sit in.
    children: PathBuf,
    /// The folder a `#[path]` on one of its child modules is relative to.
    path_base: PathBuf,
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
    let at = Source {
        file: &lib,
        children: source.clone(),
        path_base: source,
    };
    read.module(Vec::new(), None, false, &file.items, &at)?;
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

/// Whether `attributes` hold `#[doc(hidden)]`.
fn hidden(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("doc")
            && attribute
                .parse_args::<syn::Path>()
                .is_ok_and(|path| path.is_ident("hidden"))
    })
}

/// Whether `attributes` hold a `#[cfg]` only a test build meets, as
/// `cfg(test)` or `cfg(all(test, …))`.
fn test_only(attributes: &[syn::Attribute]) -> bool {
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

/// Whether what `attributes` mark is no one's: test-only or hidden.
fn left_out(attributes: &[syn::Attribute]) -> bool {
    test_only(attributes) || hidden(attributes)
}

/// Each name a `use` tree brings in, with the path it names; a glob's name
/// is `*`.
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

impl Crate {
    /// Reads the module at `path`, holding `items`, and its child modules;
    /// returns its index.
    fn module(
        &mut self,
        path: Vec<String>,
        parent: Option<usize>,
        generated: bool,
        items: &[syn::Item],
        source: &Source<'_>,
    ) -> Result<usize> {
        let index = self.modules.len();
        self.modules.push(Module {
            path,
            parent,
            generated,
            children: BTreeMap::new(),
            uses: BTreeMap::new(),
            globs: Vec::new(),
            items: BTreeMap::new(),
        });
        for item in items {
            match item {
                syn::Item::Use(used) if !test_only(&used.attrs) => {
                    let public = is_public(&used.vis) && !hidden(&used.attrs);
                    let mut found = Vec::new();
                    use_tree(&used.tree, &mut Vec::new(), &mut found);
                    let module = &mut self.modules[index];
                    for (name, segments) in found {
                        let used = Use { segments, public };
                        match name.as_str() {
                            "*" => module.globs.push(used),
                            "_" => {}
                            _ => {
                                module.uses.insert(name, used);
                            }
                        }
                    }
                }
                syn::Item::Mod(child) if !test_only(&child.attrs) => {
                    let public = is_public(&child.vis) && !hidden(&child.attrs);
                    let child_index = self.child(index, child, source)?;
                    self.modules[index]
                        .children
                        .insert(child.ident.to_string(), (child_index, public));
                }
                syn::Item::Impl(inherent) => self.impls.extend(inherent_impl(index, inherent)),
                other if !test_only(item_attributes(other)) => {
                    if let Some((name, declared)) = declared(other) {
                        self.modules[index].items.insert(name, declared);
                    }
                }
                _ => {}
            }
        }
        Ok(index)
    }

    /// Reads the child module `child` of the module `parent`, whose source
    /// is `source`; returns its index.
    fn child(&mut self, parent: usize, child: &syn::ItemMod, source: &Source<'_>) -> Result<usize> {
        let name = child.ident.to_string();
        let mut path = self.modules[parent].path.clone();
        path.push(name.clone());
        let generated = self.modules[parent].generated || name == GENERATED;
        if let Some((_, items)) = &child.content {
            let folder = source.children.join(&name);
            let inline = Source {
                file: source.file,
                children: folder.clone(),
                path_base: folder,
            };
            return self.module(path, Some(parent), generated, items, &inline);
        }
        let (file, children) = module_file(child, source)?;
        let generated = generated
            || file
                .strip_prefix(&self.source)
                .unwrap_or(&file)
                .components()
                .any(|component| component.as_os_str() == GENERATED);
        let parsed = parse(&file)?;
        let own = Source {
            file: &file,
            path_base: file.parent().map(Path::to_path_buf).unwrap_or_default(),
            children,
        };
        self.module(path, Some(parent), generated, &parsed.items, &own)
    }

    /// What `name` names in `module`: a child module, an item it declares,
    /// or what a `use` or glob of it brings in; `None` for another crate's
    /// or a name it lacks.
    fn lookup(&self, module: usize, name: &str, hops: usize) -> Option<Named> {
        let at = &self.modules[module];
        if let Some(&(child, _)) = at.children.get(name) {
            return Some(Named::Module(child));
        }
        if at.items.contains_key(name) {
            return Some(Named::Item(module, name.to_owned()));
        }
        let hops = hops.checked_sub(1)?;
        if let Some(used) = at.uses.get(name) {
            return self.resolve(module, &used.segments, hops);
        }
        at.globs
            .iter()
            .find_map(|glob| match self.resolve(module, &glob.segments, hops)? {
                Named::Module(globbed) => self.lookup(globbed, name, hops),
                Named::Item(..) => None,
            })
    }

    /// What the path `segments`, written in `module`, names.
    fn resolve(&self, module: usize, segments: &[String], hops: usize) -> Option<Named> {
        let (first, rest) = segments.split_first()?;
        let mut at = match first.as_str() {
            "crate" => 0,
            "self" => module,
            "super" => self.modules[module].parent?,
            name => match self.lookup(module, name, hops)? {
                Named::Module(found) => found,
                item @ Named::Item(..) => return rest.is_empty().then_some(item),
            },
        };
        for (index, segment) in rest.iter().enumerate() {
            if segment == "super" {
                at = self.modules[at].parent?;
                continue;
            }
            match self.lookup(at, segment, hops)? {
                Named::Module(found) => at = found,
                item @ Named::Item(..) => return (index + 1 == rest.len()).then_some(item),
            }
        }
        Some(Named::Module(at))
    }

    /// The names `module` makes public, by `pub` or `pub use`, with what
    /// each names; a name it gives itself hides one a glob brings in.
    fn public_names(&self, module: usize, hops: usize) -> BTreeMap<String, Named> {
        let at = &self.modules[module];
        let mut names = BTreeMap::new();
        if let Some(hops) = hops.checked_sub(1) {
            for glob in at.globs.iter().filter(|glob| glob.public) {
                if let Some(Named::Module(globbed)) = self.resolve(module, &glob.segments, hops) {
                    names.extend(self.public_names(globbed, hops));
                }
            }
        }
        for (name, used) in at.uses.iter().filter(|(_, used)| used.public) {
            if let Some(named) = self.resolve(module, &used.segments, hops) {
                names.insert(name.clone(), named);
            }
        }
        for (name, &(child, public)) in &at.children {
            if public {
                names.insert(name.clone(), Named::Module(child));
            }
        }
        for (name, item) in &at.items {
            if item.public {
                names.insert(name.clone(), Named::Item(module, name.clone()));
            }
        }
        names
    }

    /// Each item a dependent crate reaches through `module`, which it
    /// reaches at `at`; `walked` holds the modules on the way, so a module
    /// re-exporting its own parent ends.
    fn reach(
        &self,
        module: usize,
        at: &[String],
        walked: &mut Vec<usize>,
        reached: &mut Vec<Reached>,
    ) {
        for (name, named) in self.public_names(module, HOPS) {
            match named {
                Named::Item(declaring, item) => reached.push(Reached {
                    item: (declaring, item),
                    at: at.to_vec(),
                    name,
                }),
                Named::Module(child) if !walked.contains(&child) => {
                    let mut path = at.to_vec();
                    path.push(name);
                    walked.push(child);
                    self.reach(child, &path, walked, reached);
                    walked.pop();
                }
                Named::Module(_) => {}
            }
        }
    }

    fn exported(&self, krate: &str) -> Vec<Exported> {
        let mut reached = Vec::new();
        self.reach(0, &[], &mut vec![0], &mut reached);
        let targets: Vec<Option<Named>> = self
            .impls
            .iter()
            .map(|inherent| self.resolve(inherent.module, &inherent.self_path, HOPS))
            .collect();
        let generated = |module: usize| self.modules[module].generated;
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
        } in reached
        {
            let item = &self.modules[module].items[&name];
            let mut declared_at = self.modules[module].path.clone();
            declared_at.push(name.clone());
            let item_id = declared_at.join("::");
            let receiver = if at.is_empty() {
                CRATE.to_owned()
            } else {
                at.join("::")
            };
            exported.push(export(
                receiver,
                as_name.clone(),
                item_id.clone(),
                generated(module),
            ));
            let mut owner = at;
            owner.push(as_name);
            let owner = owner.join("::");
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
                    let this = Named::Item(module, name.clone());
                    for (inherent, target) in self.impls.iter().zip(&targets) {
                        if target.as_ref() != Some(&this) {
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
/// for a trait's `impl`, one only a test build compiles, a hidden one or
/// one of a type with no path.
fn inherent_impl(module: usize, inherent: &syn::ItemImpl) -> Option<Impl> {
    if inherent.trait_.is_some() || left_out(&inherent.attrs) {
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

/// The file of the module `child` declared without a body in `source`, and
/// the folder its own modules' files sit in. A `#[path]` is relative to
/// `source`'s path base, and the file it names holds its modules' files
/// beside it, as `mod.rs` does; otherwise the file sits in `source`'s
/// children folder.
fn module_file(child: &syn::ItemMod, source: &Source<'_>) -> Result<(PathBuf, PathBuf)> {
    let name = child.ident.to_string();
    let beside = |found: &Path| found.parent().map(Path::to_path_buf).unwrap_or_default();
    let missing = |at: &Path| {
        format!(
            "{}: module `{name}` has no file at {}",
            source.file.display(),
            at.display()
        )
    };
    if let Some(named) = path_attribute(&child.attrs) {
        let found = source.path_base.join(named);
        if !found.is_file() {
            bail!(missing(&found));
        }
        let folder = beside(&found);
        return Ok((found, folder));
    }
    let flat = source.children.join(format!("{name}.rs"));
    if flat.is_file() {
        return Ok((flat, source.children.join(&name)));
    }
    let nested = source.children.join(&name).join("mod.rs");
    if nested.is_file() {
        let folder = beside(&nested);
        return Ok((nested, folder));
    }
    bail!(missing(&flat))
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

/// The attributes of an item that may declare a name.
fn item_attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        _ => &[],
    }
}

/// The name and item an item of a module declares, `None` for one that
/// declares no name a dependent crate could reach.
fn declared(item: &syn::Item) -> Option<(String, Item)> {
    let (ident, visibility, kind) = match item {
        syn::Item::Struct(item) => (&item.ident, &item.vis, Kind::Type),
        syn::Item::Enum(item) => (&item.ident, &item.vis, Kind::Type),
        syn::Item::Union(item) => (&item.ident, &item.vis, Kind::Type),
        syn::Item::Trait(item) => (&item.ident, &item.vis, Kind::Trait),
        syn::Item::Fn(item) => (&item.sig.ident, &item.vis, Kind::Value),
        syn::Item::Const(item) => (&item.ident, &item.vis, Kind::Value),
        syn::Item::Static(item) => (&item.ident, &item.vis, Kind::Value),
        syn::Item::Type(item) => (&item.ident, &item.vis, Kind::Value),
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
            public: is_public(visibility) && !hidden(item_attributes(item)),
            kind,
            functions,
        },
    ))
}
