//! `xtask chromium protocol check` (ADR 0074): every CDP method, event and
//! parameter the driver uses is in the committed protocol JSON of the
//! Goodfetch Chromium, `chromium/protocol.json`, so a rebase that drops one
//! fails before a test reaches it.
//!
//! The driver's use is read from its source, `crates/goodfetch-page/src`,
//! leaving out test code (`tests.rs`, a `tests` folder, a `#[cfg(test)]`
//! item):
//!
//! - a binding of `goodfetch-cdp`'s generated layer it names, through a
//!   `use` of `generated::<module>` or of one of its types: a command's
//!   parameters it sets, other than to `None`, and its required ones, the
//!   required parameters of its answer, an event's required parameters;
//! - a binding's method name written as text, as `send_raw` takes it: the
//!   method alone.
//!
//! Without a committed protocol JSON the check has nothing to check against
//! and passes.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use anyhow::{Context as _, Result};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens as _;
use serde::Deserialize;

use crate::check::Finding;

/// The protocol JSON of the Goodfetch Chromium's build.
const PROTOCOL: &str = "chromium/protocol.json";

/// The generated bindings, one file per CDP domain.
const BINDINGS: &str = "crates/goodfetch-cdp/src/generated";

/// The driver's source.
const DRIVER: &str = "crates/goodfetch-page/src";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a CDP method, event or parameter the driver uses that
    /// `chromium/protocol.json` does not have.
    Check,
}

#[derive(Deserialize)]
struct Protocol {
    domains: Vec<Domain>,
}

#[derive(Deserialize)]
struct Domain {
    #[serde(rename = "domain")]
    name: String,
    #[serde(default)]
    commands: Vec<Member>,
    #[serde(default)]
    events: Vec<Member>,
}

/// A command or an event.
#[derive(Deserialize)]
struct Member {
    name: String,
    #[serde(default)]
    parameters: Vec<Parameter>,
    #[serde(default)]
    returns: Vec<Parameter>,
}

#[derive(Deserialize)]
struct Parameter {
    name: String,
}

/// A command's or an event's parameters, by their wire names.
struct Wire {
    parameters: BTreeSet<String>,
    returns: BTreeSet<String>,
}

/// A type of the generated layer: `(module, type)`.
type Key = (String, String);

/// A field of a generated struct.
struct Field {
    ident: String,
    wire: String,
    required: bool,
}

enum Binding {
    Command {
        method: String,
        returns: Option<Key>,
    },
    Event {
        method: String,
    },
}

impl Binding {
    fn method(&self) -> &str {
        match self {
            Self::Command { method, .. } | Self::Event { method } => method,
        }
    }
}

/// The generated layer: its structs' fields and its commands and events.
#[derive(Default)]
struct Bindings {
    structs: BTreeMap<Key, Vec<Field>>,
    bindings: BTreeMap<Key, Binding>,
}

/// What the driver uses of one binding.
#[derive(Default)]
struct Use {
    /// The driver files that use it.
    files: BTreeSet<String>,
    /// The fields it sets in a struct literal, other than to `None`.
    set: BTreeSet<String>,
    /// Only its method's name, written as text.
    by_name: bool,
    /// Its type, named.
    by_type: bool,
}

pub(crate) fn run(command: &Command, root: &Path) -> Result<Vec<Finding>> {
    let Command::Check = command;
    let path = root.join(PROTOCOL);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
    let protocol: Protocol =
        serde_json::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
    let bindings = Bindings::read(&root.join(BINDINGS))?;
    let uses = driver_uses(root, &bindings)?;
    Ok(findings(&protocol, &bindings, &uses))
}

fn findings(protocol: &Protocol, bindings: &Bindings, uses: &BTreeMap<Key, Use>) -> Vec<Finding> {
    let mut commands = BTreeMap::new();
    let mut events = BTreeMap::new();
    for domain in &protocol.domains {
        for (members, into) in [
            (&domain.commands, &mut commands),
            (&domain.events, &mut events),
        ] {
            for member in members {
                into.insert(
                    format!("{}.{}", domain.name, member.name),
                    Wire {
                        parameters: names(&member.parameters),
                        returns: names(&member.returns),
                    },
                );
            }
        }
    }

    let mut findings = BTreeMap::new();
    for (key, used) in uses {
        let Some(binding) = bindings.bindings.get(key) else {
            continue;
        };
        let method = binding.method();
        let files = used
            .files
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join(", ");
        let (kind, wire) = match binding {
            Binding::Command { .. } => ("method-missing", commands.get(method)),
            Binding::Event { .. } => ("event-missing", events.get(method)),
        };
        let Some(wire) = wire else {
            findings.insert(
                (kind, method.to_owned()),
                format!("the driver uses it ({files}), but `{PROTOCOL}` has no such method"),
            );
            continue;
        };
        if !used.by_type {
            continue;
        }
        let mut needed: Vec<(&str, &BTreeSet<String>)> = Vec::new();
        let fields = bindings.structs.get(key).map_or(&[][..], Vec::as_slice);
        for field in fields {
            let sent = field.required || used.set.contains(&field.ident);
            if sent {
                needed.push((&field.wire, &wire.parameters));
            }
        }
        if let Binding::Command {
            returns: Some(returns),
            ..
        } = binding
        {
            let fields = bindings.structs.get(returns).map_or(&[][..], Vec::as_slice);
            for field in fields.iter().filter(|field| field.required) {
                needed.push((&field.wire, &wire.returns));
            }
        }
        for (name, known) in needed {
            if !known.contains(name) {
                findings.insert(
                    ("parameter-missing", format!("{method} {name}")),
                    format!("the driver needs it ({files}), but `{PROTOCOL}` does not list it"),
                );
            }
        }
    }
    findings
        .into_iter()
        .map(|((kind, subject), message)| Finding::new(kind, subject, message))
        .collect()
}

fn names(parameters: &[Parameter]) -> BTreeSet<String> {
    parameters
        .iter()
        .map(|parameter| parameter.name.clone())
        .collect()
}

impl Bindings {
    fn read(folder: &Path) -> Result<Self> {
        let mut bindings = Self::default();
        if !folder.exists() {
            return Ok(bindings);
        }
        for path in rust_files(folder)? {
            let module = path
                .file_stem()
                .and_then(|stem| stem.to_str())
                .with_context(|| format!("{} is not named in UTF-8", path.display()))?
                .to_owned();
            let file = parse(&path)?;
            for item in &file.items {
                bindings
                    .record(&module, item)
                    .with_context(|| format!("reading {}", path.display()))?;
            }
        }
        Ok(bindings)
    }

    fn record(&mut self, module: &str, item: &syn::Item) -> syn::Result<()> {
        match item {
            syn::Item::Struct(item) => {
                let fields = match &item.fields {
                    syn::Fields::Named(fields) => {
                        fields.named.iter().map(field).collect::<syn::Result<_>>()?
                    }
                    syn::Fields::Unnamed(_) | syn::Fields::Unit => Vec::new(),
                };
                self.structs
                    .insert((module.to_owned(), item.ident.to_string()), fields);
            }
            syn::Item::Impl(item) => {
                let Some((trait_path, _)) = &item.trait_ else {
                    return Ok(());
                };
                let Some(type_name) = last_ident(&item.self_ty) else {
                    return Ok(());
                };
                let Some(method) = item.items.iter().find_map(method_of) else {
                    return Ok(());
                };
                let binding = if path_is(trait_path, "Command") {
                    let returns = item.items.iter().find_map(|item| match item {
                        syn::ImplItem::Type(returns) if returns.ident == "Returns" => {
                            last_ident(&returns.ty).map(|name| (module.to_owned(), name))
                        }
                        _ => None,
                    });
                    Binding::Command { method, returns }
                } else if path_is(trait_path, "Event") {
                    Binding::Event { method }
                } else {
                    return Ok(());
                };
                self.bindings
                    .insert((module.to_owned(), type_name), binding);
            }
            _ => {}
        }
        Ok(())
    }
}

/// `crate::Command` or `crate::Event`.
fn path_is(path: &syn::Path, name: &str) -> bool {
    path.segments.len() == 2 && path.segments[0].ident == "crate" && path.segments[1].ident == name
}

/// The string of a `const METHOD: &str = "…";`.
fn method_of(item: &syn::ImplItem) -> Option<String> {
    let syn::ImplItem::Const(constant) = item else {
        return None;
    };
    if constant.ident != "METHOD" {
        return None;
    }
    match &constant.expr {
        syn::Expr::Lit(syn::ExprLit {
            lit: syn::Lit::Str(method),
            ..
        }) => Some(method.value()),
        _ => None,
    }
}

fn last_ident(ty: &syn::Type) -> Option<String> {
    let syn::Type::Path(path) = ty else {
        return None;
    };
    path.path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

/// A named field: its wire name is its serde `rename`, else its name; it is
/// required unless it is an `Option` or has a serde `default`.
fn field(field: &syn::Field) -> syn::Result<Field> {
    let ident = field
        .ident
        .as_ref()
        .map(ToString::to_string)
        .unwrap_or_default();
    let ident = ident.strip_prefix("r#").unwrap_or(&ident).to_owned();
    let mut wire = ident.clone();
    let mut default = false;
    for attribute in field
        .attrs
        .iter()
        .filter(|attribute| attribute.path().is_ident("serde"))
    {
        attribute.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") {
                wire = meta.value()?.parse::<syn::LitStr>()?.value();
            } else if meta.path.is_ident("default") {
                default = true;
            } else if meta.input.peek(syn::Token![=]) {
                meta.value()?.parse::<syn::Expr>()?;
            }
            Ok(())
        })?;
    }
    let optional = last_ident(&field.ty).is_some_and(|name| name == "Option");
    Ok(Field {
        required: !optional && !default,
        ident,
        wire,
    })
}

/// The names one driver file gives the generated layer.
#[derive(Default)]
struct Names {
    /// A name of a module: `page`, or `page as cdp_page`.
    modules: BTreeMap<String, String>,
    /// A name of a type: `FrameNavigatedEvent as Navigated`.
    types: BTreeMap<String, Key>,
    /// The modules imported whole: `page::*`.
    globs: BTreeSet<String>,
}

fn driver_uses(root: &Path, bindings: &Bindings) -> Result<BTreeMap<Key, Use>> {
    let mut uses: BTreeMap<Key, Use> = BTreeMap::new();
    let folder = root.join(DRIVER);
    if !folder.exists() {
        return Ok(uses);
    }
    let by_method: BTreeMap<&str, &Key> = bindings
        .bindings
        .iter()
        .map(|(key, binding)| (binding.method(), key))
        .collect();
    for path in rust_files(&folder)? {
        let relative = path.strip_prefix(root).unwrap_or(&path);
        if relative
            .components()
            .any(|component| component.as_os_str() == "tests")
            || path.file_name().is_some_and(|name| name == "tests.rs")
        {
            continue;
        }
        let file_name = relative.to_string_lossy().replace('\\', "/");
        let file = parse(&path)?;
        let items = driver_items(&file.items);
        let mut names = Names::default();
        for item in &items {
            if let syn::Item::Use(item) = item {
                import(&item.tree, &mut Vec::new(), &mut names);
            }
        }
        let mut scan = Scan {
            names: &names,
            bindings,
            by_method: &by_method,
            file: &file_name,
            uses: &mut uses,
        };
        for item in items
            .iter()
            .filter(|item| !matches!(item, syn::Item::Use(_)))
        {
            scan.tokens(item.to_token_stream());
        }
    }
    Ok(uses)
}

/// The file's items that are not test code, inline modules' items included.
fn driver_items(items: &[syn::Item]) -> Vec<&syn::Item> {
    let mut kept = Vec::new();
    for item in items {
        if crate::modules::is_test_only(crate::modules::item_attributes(item)) {
            continue;
        }
        match item {
            syn::Item::Mod(syn::ItemMod {
                content: Some((_, inner)),
                ..
            }) => kept.extend(driver_items(inner)),
            item => kept.push(item),
        }
    }
    kept
}

/// Records the names a `use` tree gives under a `generated` segment.
fn import(tree: &syn::UseTree, prefix: &mut Vec<String>, names: &mut Names) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            import(&path.tree, prefix, names);
            prefix.pop();
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                import(tree, prefix, names);
            }
        }
        syn::UseTree::Name(name) => {
            let ident = name.ident.to_string();
            if ident == "self" {
                if let Some(last) = prefix.last().cloned() {
                    prefix.pop();
                    name_of(prefix, &last, last.clone(), names);
                    prefix.push(last);
                }
            } else {
                name_of(prefix, &ident, ident.clone(), names);
            }
        }
        syn::UseTree::Rename(rename) => {
            name_of(
                prefix,
                &rename.ident.to_string(),
                rename.rename.to_string(),
                names,
            );
        }
        syn::UseTree::Glob(_) => {
            if let [.., generated, module] = prefix.as_slice()
                && generated == "generated"
            {
                names.globs.insert(module.clone());
            }
        }
    }
}

fn name_of(prefix: &[String], ident: &str, local: String, names: &mut Names) {
    match prefix {
        [.., generated] if generated == "generated" => {
            names.modules.insert(local, ident.to_owned());
        }
        [.., generated, module] if generated == "generated" => {
            names
                .types
                .insert(local, (module.clone(), ident.to_owned()));
        }
        _ => {}
    }
}

struct Scan<'a> {
    names: &'a Names,
    bindings: &'a Bindings,
    by_method: &'a BTreeMap<&'a str, &'a Key>,
    file: &'a str,
    uses: &'a mut BTreeMap<Key, Use>,
}

impl Scan<'_> {
    fn tokens(&mut self, stream: TokenStream) {
        let tokens: Vec<TokenTree> = stream.into_iter().collect();
        for (index, token) in tokens.iter().enumerate() {
            match token {
                TokenTree::Group(group) => self.tokens(group.stream()),
                TokenTree::Literal(literal) => {
                    if let syn::Lit::Str(text) = syn::Lit::new(literal.clone())
                        && let Some(key) = self.by_method.get(text.value().as_str())
                    {
                        let key = (*key).clone();
                        self.entry(key).by_name = true;
                    }
                }
                TokenTree::Ident(ident) => {
                    if let Some((key, after)) = self.binding_at(&tokens, index, &ident.to_string())
                        && self.bindings.structs.contains_key(&key)
                    {
                        let set = match tokens.get(after) {
                            Some(TokenTree::Group(group))
                                if group.delimiter() == Delimiter::Brace =>
                            {
                                set_fields(group.stream())
                            }
                            _ => BTreeSet::new(),
                        };
                        let used = self.entry(key);
                        used.by_type = true;
                        used.set.extend(set);
                    }
                }
                TokenTree::Punct(_) => {}
            }
        }
    }

    /// The type of the generated layer named at `index`, and the index of
    /// the token after its name.
    fn binding_at(&self, tokens: &[TokenTree], index: usize, ident: &str) -> Option<(Key, usize)> {
        let module = self.names.modules.get(ident).cloned().or_else(|| {
            (index >= 3
                && crate::modules::is_ident(tokens, index - 3, "generated")
                && crate::modules::is_path_separator(tokens, index - 2))
            .then(|| ident.to_owned())
        });
        if let Some(module) = module
            && crate::modules::is_path_separator(tokens, index + 1)
            && let Some(TokenTree::Ident(name)) = tokens.get(index + 3)
        {
            return Some(((module, name.to_string()), index + 4));
        }
        if index >= 1
            && let Some(TokenTree::Punct(punct)) = tokens.get(index - 1)
            && punct.as_char() == ':'
        {
            // A path's later segment: named by the arm above or not a type
            // of the generated layer.
            return None;
        }
        if let Some(key) = self.names.types.get(ident) {
            return Some((key.clone(), index + 1));
        }
        self.names.globs.iter().find_map(|module| {
            let key = (module.clone(), ident.to_owned());
            self.bindings
                .structs
                .contains_key(&key)
                .then_some((key, index + 1))
        })
    }

    fn entry(&mut self, key: Key) -> &mut Use {
        let used = self.uses.entry(key).or_default();
        used.files.insert(self.file.to_owned());
        used
    }
}

/// The fields a struct literal's body sets, other than to `None`.
fn set_fields(body: TokenStream) -> BTreeSet<String> {
    let tokens: Vec<TokenTree> = body.into_iter().collect();
    tokens
        .split(|token| matches!(token, TokenTree::Punct(punct) if punct.as_char() == ','))
        .filter_map(|part| match part {
            [TokenTree::Ident(name)] => Some(name.to_string()),
            [TokenTree::Ident(name), TokenTree::Punct(colon), value @ ..]
                if colon.as_char() == ':'
                    && !matches!(value, [TokenTree::Punct(punct), ..] if punct.as_char() == ':')
                    && !matches!(value, [TokenTree::Ident(none)] if none == "None") =>
            {
                Some(name.to_string())
            }
            _ => None,
        })
        .map(|name| name.strip_prefix("r#").unwrap_or(&name).to_owned())
        .collect()
}

fn parse(path: &Path) -> Result<syn::File> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))
}

/// Every `.rs` file under `folder`, in a stable order.
fn rust_files(folder: &Path) -> Result<Vec<std::path::PathBuf>> {
    let mut files = Vec::new();
    let mut folders = vec![folder.to_path_buf()];
    while let Some(folder) = folders.pop() {
        for entry in
            std::fs::read_dir(&folder).with_context(|| format!("reading {}", folder.display()))?
        {
            let path = entry?.path();
            if path.is_dir() {
                folders.push(path);
            } else if path.extension().is_some_and(|ending| ending == "rs") {
                files.push(path);
            }
        }
    }
    files.sort();
    Ok(files)
}
