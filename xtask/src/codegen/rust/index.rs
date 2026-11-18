//! Where `goodfetch-protocol` defines each type of the bundle, read from
//! its source with `syn`, so the Rust handles name the crate's own types:
//! the public path a dependent crate spells, through a `pub use` when the
//! defining module is private, and each params struct's fields with their
//! wire names and types.
//!
//! The crate's module tree is walked from `src/lib.rs`, tests left out. A
//! type is found by its schema name, which is its `schemars` or `serde`
//! rename or its ident; a name two items share, or a field type the index
//! cannot spell from outside the crate, fails `xtask codegen` with the type
//! named.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use syn::punctuated::Punctuated;

/// The crate whose types the handles take and answer.
const CRATE: &str = "goodfetch_protocol";

/// Its root file, from the workspace's root.
const ROOT: &str = "crates/goodfetch-protocol/src/lib.rs";

/// The names a Rust file reaches without importing them.
const PRELUDE: [&str; 19] = [
    "Option", "Vec", "String", "Box", "bool", "char", "u8", "u16", "u32", "u64", "u128", "usize",
    "i8", "i16", "i32", "i64", "i128", "isize", "f64",
];

/// Where a name a module imports points.
#[derive(Clone, Debug)]
enum Target {
    /// A path inside the crate, from its root.
    Crate(Vec<String>),
    /// A path in another crate, its name first.
    External(Vec<String>),
}

struct Module {
    /// Its names below the crate root.
    path: Vec<String>,
    /// Whether a dependent crate reaches it: it and each module around it
    /// is `pub`.
    public: bool,
    /// What each name it imports points at.
    uses: BTreeMap<String, Target>,
    /// The names it re-exports with `pub use`.
    reexports: BTreeMap<String, Target>,
    /// The types it defines, by ident.
    items: BTreeMap<String, Item>,
}

struct Item {
    public: bool,
    /// The name its schema takes in the bundle.
    schema_name: String,
    /// A struct's named fields, or why the index cannot read them.
    fields: Result<Vec<RawField>, String>,
}

struct RawField {
    /// Whether a dependent crate sets it: it is `pub`.
    public: bool,
    ident: String,
    wire: String,
    /// Its type's tokens, parsed again when the index spells it.
    ty: String,
}

/// A field of a struct, as the handles set it.
pub(crate) struct Field {
    /// The field's ident, raw if it is a keyword.
    pub(crate) ident: String,
    /// The name it takes on the wire.
    pub(crate) wire: String,
    /// Its type, spelled from outside the crate.
    pub(crate) ty: String,
}

/// The types of `goodfetch-protocol`.
pub(crate) struct Index {
    modules: Vec<Module>,
}

impl Index {
    /// Reads the crate under `root`; `None` when the workspace has none.
    pub(crate) fn read(root: &Path) -> Result<Option<Self>> {
        let lib = root.join(ROOT);
        if !lib.is_file() {
            return Ok(None);
        }
        let mut index = Self {
            modules: Vec::new(),
        };
        let children = lib.parent().map(Path::to_path_buf).unwrap_or_default();
        let file = parse(&lib)?;
        index.module(Vec::new(), true, &file.items, &children, &lib)?;
        Ok(Some(index))
    }

    fn module(
        &mut self,
        path: Vec<String>,
        public: bool,
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
        let mut module = Module {
            path: path.clone(),
            public,
            uses: BTreeMap::new(),
            reexports: BTreeMap::new(),
            items: BTreeMap::new(),
        };
        let mut nested = Vec::new();
        for item in items {
            match item {
                syn::Item::Use(used) => {
                    let mut found = Vec::new();
                    use_tree(&used.tree, &mut Vec::new(), &mut found);
                    for (name, segments) in found {
                        let target = resolve(&path, &child_modules, segments);
                        if matches!(used.vis, syn::Visibility::Public(_)) {
                            module.reexports.insert(name.clone(), target.clone());
                        }
                        module.uses.insert(name, target);
                    }
                }
                syn::Item::Struct(defined) => {
                    let fields = match &defined.fields {
                        syn::Fields::Named(named) => raw_fields(&defined.attrs, named),
                        _ => Err("it has no named fields".to_owned()),
                    };
                    module.items.insert(
                        defined.ident.to_string(),
                        Item {
                            public: matches!(defined.vis, syn::Visibility::Public(_)),
                            schema_name: schema_name(&defined.attrs, &defined.ident)?,
                            fields,
                        },
                    );
                }
                syn::Item::Enum(defined) => {
                    module.items.insert(
                        defined.ident.to_string(),
                        Item {
                            public: matches!(defined.vis, syn::Visibility::Public(_)),
                            schema_name: schema_name(&defined.attrs, &defined.ident)?,
                            fields: Err("it is an enum".to_owned()),
                        },
                    );
                }
                syn::Item::Type(defined) => {
                    module.items.insert(
                        defined.ident.to_string(),
                        Item {
                            public: matches!(defined.vis, syn::Visibility::Public(_)),
                            schema_name: defined.ident.to_string(),
                            fields: Err("it is a type alias".to_owned()),
                        },
                    );
                }
                syn::Item::Mod(child) if !is_test(child) => nested.push(child),
                _ => {}
            }
        }
        self.modules.push(module);
        for child in nested {
            let name = child.ident.to_string();
            let mut child_path = path.clone();
            child_path.push(name.clone());
            let child_public = public && matches!(child.vis, syn::Visibility::Public(_));
            if let Some((_, items)) = &child.content {
                self.module(child_path, child_public, items, &children.join(&name), file)?;
                continue;
            }
            let candidates = [
                children.join(format!("{name}.rs")),
                children.join(&name).join("mod.rs"),
            ];
            let Some(found) = candidates.iter().find(|candidate| candidate.is_file()) else {
                bail!(
                    "{}: module `{name}` has no file at {}",
                    file.display(),
                    candidates[0].display()
                );
            };
            let grandchildren: PathBuf = if found.ends_with("mod.rs") {
                found.parent().map(Path::to_path_buf).unwrap_or_default()
            } else {
                children.join(&name)
            };
            let parsed = parse(found)?;
            self.module(
                child_path,
                child_public,
                &parsed.items,
                &grandchildren,
                found,
            )?;
        }
        Ok(())
    }

    /// The module and ident of the type the bundle names `name`.
    fn find(&self, name: &str) -> Result<(usize, &str)> {
        let mut found = self.modules.iter().enumerate().flat_map(|(index, module)| {
            module
                .items
                .iter()
                .filter(|(_, item)| item.schema_name == name)
                .map(move |(ident, _)| (index, ident.as_str()))
        });
        let Some(first) = found.next() else {
            bail!("`{CRATE}` defines no type the bundle names `{name}`");
        };
        if let Some(second) = found.next() {
            bail!(
                "`{CRATE}` defines two types the bundle names `{name}`, in `{}` and `{}`, so the Rust handles cannot tell which one they take",
                self.modules[first.0].path.join("::"),
                self.modules[second.0].path.join("::")
            );
        }
        Ok(first)
    }

    /// The path a dependent crate spells the type the bundle names `name`
    /// with.
    pub(crate) fn type_path(&self, name: &str) -> Result<String> {
        let (module, ident) = self.find(name)?;
        self.public_path(module, ident)
    }

    /// The fields of the struct the bundle names `name`.
    pub(crate) fn fields(&self, name: &str) -> Result<Vec<Field>> {
        let (module, ident) = self.find(name)?;
        let item = &self.modules[module].items[ident];
        let raw = item.fields.as_ref().map_err(|why| {
            anyhow::anyhow!("the params type `{name}` has no fields to set: {why}")
        })?;
        raw.iter()
            .map(|field| {
                if !field.public {
                    bail!("the field `{}` of `{name}` is not `pub`, so the Rust handles cannot set it", field.ident);
                }
                Ok(Field {
                    ident: field.ident.clone(),
                    wire: field.wire.clone(),
                    ty: syn::parse_str::<syn::Type>(&field.ty)
                        .map_err(anyhow::Error::from)
                        .and_then(|ty| self.render(module, &ty))
                        .with_context(|| format!("the field `{}` of `{name}`", field.ident))?,
                })
            })
            .collect()
    }

    fn public_path(&self, module: usize, ident: &str) -> Result<String> {
        let defined = &self.modules[module];
        if defined.public && defined.items[ident].public {
            return Ok(spelled(&defined.path, ident));
        }
        let mut target = defined.path.clone();
        target.push(ident.to_owned());
        let reexported = self
            .modules
            .iter()
            .filter(|module| module.public)
            .find_map(|module| {
                module
                    .reexports
                    .iter()
                    .find_map(|(name, pointed)| match pointed {
                        Target::Crate(segments) if *segments == target => {
                            Some(spelled(&module.path, name))
                        }
                        _ => None,
                    })
            });
        reexported.with_context(|| {
            format!(
                "`{CRATE}::{}` is no public type, nor re-exported by a public module, so the Rust handles cannot name it",
                target.join("::")
            )
        })
    }

    fn module_at(&self, path: &[String]) -> Option<usize> {
        self.modules.iter().position(|module| module.path == path)
    }

    /// The type a crate path names, as a dependent crate spells it.
    fn crate_path(&self, segments: &[String]) -> Result<String> {
        let Some((ident, path)) = segments.split_last() else {
            bail!("an empty path");
        };
        let Some(module) = self.module_at(path) else {
            bail!("`{CRATE}` has no module `{}`", path.join("::"));
        };
        if self.modules[module].items.contains_key(ident) {
            return self.public_path(module, ident);
        }
        match self.modules[module].uses.get(ident) {
            Some(Target::Crate(next)) if next != segments => self.crate_path(next),
            Some(Target::External(next)) => Ok(format!("::{}", next.join("::"))),
            _ => bail!("`{CRATE}::{}` names no type", segments.join("::")),
        }
    }

    /// `ty`, written in `module`, as a dependent crate spells it.
    fn render(&self, module: usize, ty: &syn::Type) -> Result<String> {
        match ty {
            syn::Type::Tuple(tuple) if tuple.elems.is_empty() => Ok("()".to_owned()),
            syn::Type::Path(path) if path.qself.is_none() => self.render_path(module, &path.path),
            _ => bail!(
                "its type `{}` is no path the Rust handles spell",
                quote::ToTokens::to_token_stream(ty)
            ),
        }
    }

    fn render_path(&self, module: usize, path: &syn::Path) -> Result<String> {
        let segments: Vec<String> = path
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        for segment in path.segments.iter().rev().skip(1) {
            if !matches!(segment.arguments, syn::PathArguments::None) {
                bail!("a path with generic arguments before its last segment");
            }
        }
        let arguments = match path.segments.last().map(|segment| &segment.arguments) {
            None | Some(syn::PathArguments::None) => String::new(),
            Some(syn::PathArguments::AngleBracketed(arguments)) => {
                let mut rendered = Vec::new();
                for argument in &arguments.args {
                    let syn::GenericArgument::Type(ty) = argument else {
                        bail!("a generic argument that is no type");
                    };
                    rendered.push(self.render(module, ty)?);
                }
                format!("<{}>", rendered.join(", "))
            }
            Some(syn::PathArguments::Parenthesized(_)) => bail!("a function type"),
        };
        let here = &self.modules[module];
        let base = if path.leading_colon.is_some() {
            format!("::{}", segments.join("::"))
        } else if let [name] = segments.as_slice() {
            if here.items.contains_key(name) {
                self.public_path(module, name)?
            } else if let Some(target) = here.uses.get(name) {
                match target {
                    Target::Crate(pointed) => self.crate_path(pointed)?,
                    Target::External(pointed) => format!("::{}", pointed.join("::")),
                }
            } else if PRELUDE.contains(&name.as_str()) {
                name.clone()
            } else {
                bail!("the type `{name}` resolves to nothing the index read");
            }
        } else {
            let children: Vec<String> = self
                .modules
                .iter()
                .filter(|module| {
                    module.path.len() == here.path.len() + 1 && module.path.starts_with(&here.path)
                })
                .filter_map(|module| module.path.last().cloned())
                .collect();
            match resolve(&here.path, &children, segments.clone()) {
                Target::Crate(pointed) => self.crate_path(&pointed)?,
                Target::External(pointed) => match here.uses.get(&pointed[0]) {
                    Some(Target::Crate(prefix)) => {
                        let mut joined = prefix.clone();
                        joined.extend(pointed[1..].iter().cloned());
                        self.crate_path(&joined)?
                    }
                    Some(Target::External(prefix)) => {
                        let mut joined = prefix.clone();
                        joined.extend(pointed[1..].iter().cloned());
                        format!("::{}", joined.join("::"))
                    }
                    None => format!("::{}", pointed.join("::")),
                },
            }
        };
        Ok(format!("{base}{arguments}"))
    }
}

fn spelled(path: &[String], ident: &str) -> String {
    let mut spelled = format!("::{CRATE}");
    for segment in path {
        spelled.push_str("::");
        spelled.push_str(segment);
    }
    spelled.push_str("::");
    spelled.push_str(ident);
    spelled
}

fn parse(path: &Path) -> Result<syn::File> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Whether a module holds tests: `mod tests` or one behind `#[cfg(test)]`.
fn is_test(module: &syn::ItemMod) -> bool {
    module.ident == "tests" || module.attrs.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && matches!(&attribute.meta, syn::Meta::List(list) if list.tokens.to_string() == "test")
    })
}

/// Each name a `use` tree imports, with the path it imports, a glob left
/// out.
fn use_tree(tree: &syn::UseTree, prefix: &mut Vec<String>, found: &mut Vec<(String, Vec<String>)>) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            use_tree(&path.tree, prefix, found);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let ident = name.ident.to_string();
            let mut path = prefix.clone();
            if ident == "self" {
                if let Some(last) = path.last().cloned() {
                    found.push((last, path));
                }
            } else {
                path.push(ident.clone());
                found.push((ident, path));
            }
        }
        syn::UseTree::Rename(rename) => {
            let mut path = prefix.clone();
            let ident = rename.ident.to_string();
            if ident != "self" {
                path.push(ident);
            }
            found.push((rename.rename.to_string(), path));
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                use_tree(tree, prefix, found);
            }
        }
        syn::UseTree::Glob(_) => {}
    }
}

/// Where `segments`, written in the module at `here` whose child modules
/// are `children`, points.
fn resolve(here: &[String], children: &[String], segments: Vec<String>) -> Target {
    let Some(first) = segments.first() else {
        return Target::External(segments);
    };
    match first.as_str() {
        "crate" => Target::Crate(segments[1..].to_vec()),
        "self" => {
            let mut path = here.to_vec();
            path.extend(segments[1..].iter().cloned());
            Target::Crate(path)
        }
        "super" => {
            let mut path = here.to_vec();
            let mut rest = segments.as_slice();
            while rest.first().is_some_and(|segment| segment == "super") {
                path.pop();
                rest = &rest[1..];
            }
            path.extend(rest.iter().cloned());
            Target::Crate(path)
        }
        _ if children.contains(first) => {
            let mut path = here.to_vec();
            path.extend(segments);
            Target::Crate(path)
        }
        _ => Target::External(segments),
    }
}

/// The arguments of each `#[<name>(…)]` attribute.
fn arguments(attributes: &[syn::Attribute], name: &str) -> Result<Vec<syn::Meta>> {
    let mut found = Vec::new();
    for attribute in attributes
        .iter()
        .filter(|attribute| attribute.path().is_ident(name))
    {
        if !matches!(attribute.meta, syn::Meta::List(_)) {
            continue;
        }
        let parsed = attribute
            .parse_args_with(Punctuated::<syn::Meta, syn::Token![,]>::parse_terminated)
            .with_context(|| format!("reading a `#[{name}]` attribute"))?;
        found.extend(parsed);
    }
    Ok(found)
}

/// The string `key = "…"` gives among `arguments`.
fn string(arguments: &[syn::Meta], key: &str) -> Option<String> {
    arguments.iter().find_map(|argument| match argument {
        syn::Meta::NameValue(syn::MetaNameValue {
            path,
            value:
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                }),
            ..
        }) if path.is_ident(key) => Some(text.value()),
        _ => None,
    })
}

fn has(arguments: &[syn::Meta], key: &str) -> bool {
    arguments
        .iter()
        .any(|argument| argument.path().is_ident(key))
}

/// The name a type's schema takes: its `schemars` rename, its `serde`
/// rename, or its ident.
fn schema_name(attributes: &[syn::Attribute], ident: &syn::Ident) -> Result<String> {
    Ok(string(&arguments(attributes, "schemars")?, "rename")
        .or(string(&arguments(attributes, "serde")?, "rename"))
        .unwrap_or_else(|| ident.to_string()))
}

/// A struct's named fields with their wire names, or why the handles
/// cannot set them by name.
fn raw_fields(
    attributes: &[syn::Attribute],
    named: &syn::FieldsNamed,
) -> Result<Vec<RawField>, String> {
    let serde = arguments(attributes, "serde").map_err(|error| error.to_string())?;
    if let Some(rule) = string(&serde, "rename_all").filter(|rule| rule != "snake_case") {
        return Err(format!("it renames its fields `{rule}`"));
    }
    let mut fields = Vec::new();
    for field in &named.named {
        let Some(ident) = &field.ident else {
            return Err("a field has no name".to_owned());
        };
        let serde = arguments(&field.attrs, "serde").map_err(|error| error.to_string())?;
        if has(&serde, "flatten") {
            return Err(format!("its field `{ident}` is flattened"));
        }
        let spelled = ident.to_string();
        let wire =
            string(&serde, "rename").unwrap_or_else(|| spelled.trim_start_matches("r#").to_owned());
        fields.push(RawField {
            public: matches!(field.vis, syn::Visibility::Public(_)),
            ident: spelled,
            wire,
            ty: quote::ToTokens::to_token_stream(&field.ty).to_string(),
        });
    }
    Ok(fields)
}
