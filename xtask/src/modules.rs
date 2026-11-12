//! The module check (ADR 0126): every crate's `modules.toml` lists each of
//! its top-level modules with the siblings it may import, and the check fails
//! on any other import, on a cycle among the allowed imports, on a name that
//! is no module and on a module the file does not list. An item the crate's
//! root re-exports from a module is that module's: importing it through the
//! root (`crate::Item`) imports the module, and since a glob names nothing
//! the check can read, a root may not re-export a module's items with one.
//! A module's
//! `[submodules.<module>]` table holds its direct submodules to the same
//! rules among themselves. A file past 1,000
//! lines must state why it is still one concern in a `Size:` line of its `//!`
//! header. Source is parsed with `syn`, so nothing is built; a module's file
//! is found as `rustc` finds it, through its `#[path]` when it has one, and
//! `#[cfg(test)]` code, the crate root and generated code are not checked.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use proc_macro2::{Delimiter, TokenStream, TokenTree};
use quote::ToTokens as _;
use serde::Deserialize;

use crate::check::Finding;
use crate::metadata;

const MODULES_FILE: &str = "modules.toml";

/// ADR 0126's size alarm.
const SIZE_ALARM_LINES: usize = 1000;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ModulesFile {
    #[serde(default)]
    modules: BTreeMap<String, Vec<String>>,
    /// A module's own submodules' rules, by the module's name.
    #[serde(default)]
    submodules: BTreeMap<String, BTreeMap<String, Vec<String>>>,
}

/// One source file of a module, parsed.
struct Source {
    path: PathBuf,
    syntax: syn::File,
    /// Folder holding the files of the modules this file declares.
    children: PathBuf,
}

pub(crate) fn check(cargo: &OsStr, root: &Path) -> Result<Vec<Finding>> {
    // cargo metadata reports canonical paths; findings name them from the root.
    let root = &root
        .canonicalize()
        .with_context(|| format!("reading {}", root.display()))?;
    let mut findings = Vec::new();
    for package in metadata::packages(cargo, root)? {
        let crate_folder = package
            .manifest_path
            .parent()
            .context("a manifest sits in a folder")?
            .to_path_buf();
        check_crate(root, &package, &crate_folder, &mut findings)?;
    }
    Ok(findings)
}

fn check_crate(
    root: &Path,
    package: &metadata::Package,
    crate_folder: &Path,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let name = &package.name;
    let modules_path = crate_folder.join(MODULES_FILE);
    if !modules_path.is_file() {
        findings.push(Finding::new(
            "missing-modules-file",
            name,
            format!("the crate has no {MODULES_FILE} (ADR 0126)"),
        ));
        return Ok(());
    }
    let text = std::fs::read_to_string(&modules_path)
        .with_context(|| format!("reading {}", modules_path.display()))?;
    let rules: ModulesFile =
        toml::from_str(&text).with_context(|| format!("parsing {}", modules_path.display()))?;

    let top_level = top_level(root, package, crate_folder, findings)?;
    let modules = &top_level.modules;
    for module in top_level
        .globs
        .iter()
        .filter(|module| modules.contains_key(*module))
    {
        findings.push(Finding::new(
            "glob-re-export",
            format!("{name}/{module}"),
            format!("the root re-exports `{module}`'s items with a glob, which hides who imports them: name each item"),
        ));
    }

    for (source, _) in modules.values().flatten() {
        oversized(root, source, findings);
    }
    for (module, files) in modules {
        let Some(allowed) = rules.modules.get(module) else {
            findings.push(Finding::new(
                "module-not-listed",
                format!("{name}/{module}"),
                format!("the module is missing from {MODULES_FILE}"),
            ));
            continue;
        };
        let mut imported = BTreeSet::new();
        for (source, depth) in files {
            for item in non_test_items(&source.syntax.items) {
                scan_item(item, *depth, Scope::Crate, &mut imported);
            }
        }
        let siblings = top_level.modules_of(imported);
        for sibling in siblings {
            if sibling != *module && modules.contains_key(&sibling) && !allowed.contains(&sibling) {
                findings.push(Finding::new(
                    "forbidden-import",
                    format!("{name}/{module}->{sibling}"),
                    format!("{MODULES_FILE} does not allow `{module}` to import `{sibling}`"),
                ));
            }
        }
    }

    let named: BTreeSet<&String> = rules
        .modules
        .iter()
        .flat_map(|(module, allowed)| std::iter::once(module).chain(allowed))
        .collect();
    for unknown in named
        .into_iter()
        .filter(|name| !modules.contains_key(*name))
    {
        findings.push(Finding::new(
            "unknown-module",
            format!("{name}/{unknown}"),
            format!("{MODULES_FILE} names a module the crate does not have"),
        ));
    }

    if has_cycle(&rules.modules) {
        findings.push(Finding::new(
            "import-cycle",
            name,
            format!("the imports {MODULES_FILE} allows form a cycle"),
        ));
    }

    for (module, table) in &rules.submodules {
        let Some((source, _)) = modules.get(module).and_then(|files| files.first()) else {
            findings.push(Finding::new(
                "unknown-module",
                format!("{name}/{module}"),
                format!(
                    "{MODULES_FILE} has a submodule table for a module the crate does not have"
                ),
            ));
            continue;
        };
        check_submodules(name, module, source, table, findings)?;
    }
    Ok(())
}

/// What a crate's roots hold: its `lib.rs`'s and its `main.rs`'s together.
#[derive(Default)]
struct TopLevel {
    /// Each module the roots declare, with the files that make it up.
    modules: BTreeMap<String, Vec<(Source, usize)>>,
    /// Each name a root's `use` brings to it, with the first name of every
    /// path it comes from: a module's, when the item is the crate's own.
    re_exports: BTreeMap<String, BTreeSet<String>>,
    /// The first name of each path a root's `use` takes everything from.
    globs: BTreeSet<String>,
}

impl TopLevel {
    /// The modules the names `imported` reach: a name that is a module, and
    /// the module a root re-exports a name from, through as many `use`
    /// items as the root chains.
    fn modules_of(&self, imported: BTreeSet<String>) -> BTreeSet<String> {
        let mut reached = BTreeSet::new();
        let mut seen = BTreeSet::new();
        let mut pending: Vec<String> = imported.into_iter().collect();
        while let Some(named) = pending.pop() {
            if !seen.insert(named.clone()) {
                continue;
            }
            pending.extend(self.re_exports.get(&named).into_iter().flatten().cloned());
            if self.modules.contains_key(&named) {
                reached.insert(named);
            }
        }
        reached
    }
}

fn top_level(
    root: &Path,
    package: &metadata::Package,
    crate_folder: &Path,
    findings: &mut Vec<Finding>,
) -> Result<TopLevel> {
    let mut top_level = TopLevel::default();
    for crate_root in crate_roots(package, crate_folder) {
        let source = parse(
            &crate_root,
            crate_root.parent().context("a root sits in a folder")?,
        )?;
        let roots_children = source.children.clone();
        oversized(root, &source, findings);
        for item in non_test_items(&source.syntax.items) {
            match item {
                syn::Item::Mod(module) => {
                    let files = module_files(module, &roots_children, &roots_children, 1)?;
                    top_level
                        .modules
                        .entry(module.ident.to_string())
                        .or_default()
                        .extend(files);
                }
                // `::name` is another crate, whatever the modules are called.
                syn::Item::Use(re_export) if re_export.leading_colon.is_none() => {
                    record_re_exports(&re_export.tree, None, &mut top_level);
                }
                _ => {}
            }
        }
    }
    Ok(top_level)
}

/// The rules of one module's `[submodules.<module>]` table over its direct
/// submodules, the way the top-level rules hold the crate's modules.
fn check_submodules(
    name: &str,
    module: &str,
    source: &Source,
    table: &BTreeMap<String, Vec<String>>,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let mut children: BTreeMap<String, Vec<(Source, usize)>> = BTreeMap::new();
    for item in non_test_items(&source.syntax.items) {
        if let syn::Item::Mod(child) = item {
            children.insert(
                child.ident.to_string(),
                module_files(
                    child,
                    &source.children,
                    source
                        .path
                        .parent()
                        .context("a module file sits in a folder")?,
                    2,
                )?,
            );
        }
    }
    for (child, files) in &children {
        let Some(allowed) = table.get(child) else {
            findings.push(Finding::new(
                "module-not-listed",
                format!("{name}/{module}::{child}"),
                format!("the submodule is missing from {MODULES_FILE}'s [submodules.{module}]"),
            ));
            continue;
        };
        let mut imported = BTreeSet::new();
        for (file, depth) in files {
            for item in non_test_items(&file.syntax.items) {
                scan_item(item, *depth, Scope::Within(module), &mut imported);
            }
        }
        for sibling in imported {
            if sibling != *child && children.contains_key(&sibling) && !allowed.contains(&sibling) {
                findings.push(Finding::new(
                    "forbidden-import",
                    format!("{name}/{module}::{child}->{sibling}"),
                    format!("{MODULES_FILE}'s [submodules.{module}] does not allow `{child}` to import `{sibling}`"),
                ));
            }
        }
    }
    let named: BTreeSet<&String> = table
        .iter()
        .flat_map(|(child, allowed)| std::iter::once(child).chain(allowed))
        .collect();
    for unknown in named
        .into_iter()
        .filter(|child| !children.contains_key(*child))
    {
        findings.push(Finding::new(
            "unknown-module",
            format!("{name}/{module}::{unknown}"),
            format!(
                "{MODULES_FILE}'s [submodules.{module}] names a submodule `{module}` does not have"
            ),
        ));
    }
    if has_cycle(table) {
        findings.push(Finding::new(
            "import-cycle",
            format!("{name}/{module}"),
            format!("the imports {MODULES_FILE}'s [submodules.{module}] allows form a cycle"),
        ));
    }
    Ok(())
}

/// Records each name a root's `use` brings to the root, with the first name
/// of the path it comes from after any `crate::` or `self::`: `from` once the
/// walk is past that name, and the name itself when the root only renames it.
fn record_re_exports(tree: &syn::UseTree, from: Option<&syn::Ident>, top_level: &mut TopLevel) {
    let mut record = |named: &syn::Ident, from: &syn::Ident| {
        top_level
            .re_exports
            .entry(named.to_string())
            .or_default()
            .insert(from.to_string());
    };
    match tree {
        syn::UseTree::Path(path) => {
            let from = match from {
                None if path.ident == "crate" || path.ident == "self" => None,
                None => Some(&path.ident),
                from => from,
            };
            record_re_exports(&path.tree, from, top_level);
        }
        syn::UseTree::Group(group) => {
            for tree in &group.items {
                record_re_exports(tree, from, top_level);
            }
        }
        syn::UseTree::Name(syn::UseName { ident }) => {
            if let Some(from) = from {
                record(ident, from);
            }
        }
        syn::UseTree::Rename(syn::UseRename { ident, rename, .. }) => {
            record(rename, from.unwrap_or(ident));
        }
        syn::UseTree::Glob(_) => {
            if let Some(from) = from {
                top_level.globs.insert(from.to_string());
            }
        }
    }
}

/// The crate's `lib.rs` and `main.rs`, which only declare modules (STYLE.md).
fn crate_roots(package: &metadata::Package, crate_folder: &Path) -> Vec<PathBuf> {
    let source = crate_folder.join("src");
    package
        .targets
        .iter()
        .filter(|target| {
            target
                .kind
                .iter()
                .any(|kind| kind == "lib" || kind == "bin" || kind == "proc-macro")
        })
        .map(|target| target.src_path.clone())
        .filter(|path| *path == source.join("lib.rs") || *path == source.join("main.rs"))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn parse(path: &Path, folder: &Path) -> Result<Source> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
    let syntax = syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))?;
    let stem = path.file_stem().and_then(OsStr::to_str).unwrap_or_default();
    let children = if matches!(stem, "lib" | "main" | "mod") {
        folder.to_path_buf()
    } else {
        folder.join(stem)
    };
    Ok(Source {
        path: path.to_path_buf(),
        syntax,
        children,
    })
}

/// The files of a module declared in a file, with their depth below the crate
/// root; an inline module's items stay in its declaring file. `folder` holds
/// the declared modules' files, and `declaring` is the declaring file's own
/// folder, which a `#[path]` is relative to, as `rustc` reads it.
fn module_files(
    module: &syn::ItemMod,
    folder: &Path,
    declaring: &Path,
    depth: usize,
) -> Result<Vec<(Source, usize)>> {
    if module.content.is_some() {
        // Inline: its items are scanned with its declaring file's items.
        return Ok(Vec::new());
    }
    let name = module.ident.to_string();
    let path = if let Some(path) = path_attribute(&module.attrs) {
        let path = declaring.join(path);
        if !path.is_file() {
            bail!(
                "module `{name}`'s #[path] names no file: {}",
                path.display()
            );
        }
        path
    } else {
        [
            folder.join(format!("{name}.rs")),
            folder.join(&name).join("mod.rs"),
        ]
        .into_iter()
        .find(|path| path.is_file())
        .with_context(|| format!("module `{name}` has no file in {}", folder.display()))?
    };
    if path
        .components()
        .any(|component| component.as_os_str() == "generated")
    {
        return Ok(Vec::new());
    }
    let declaring = path.parent().context("a module file sits in a folder")?;
    let mut source = parse(&path, declaring)?;
    // `rustc` reads a `#[path]` file as it reads a `mod.rs`: the modules it
    // declares sit beside it, not in a folder named after it.
    if path_attribute(&module.attrs).is_some() {
        source.children = declaring.to_path_buf();
    }
    let mut files = Vec::new();
    for item in non_test_items(&source.syntax.items) {
        if let syn::Item::Mod(child) = item {
            files.extend(module_files(child, &source.children, declaring, depth + 1)?);
        }
    }
    files.insert(0, (source, depth));
    Ok(files)
}

/// The file a `#[path = "…"]` attribute names, if the module has one.
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

fn is_test_only(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute.path().is_ident("cfg")
            && attribute
                .meta
                .require_list()
                .is_ok_and(|list| list.tokens.to_string() == "test")
    })
}

fn item_attributes(item: &syn::Item) -> &[syn::Attribute] {
    match item {
        syn::Item::Const(item) => &item.attrs,
        syn::Item::Enum(item) => &item.attrs,
        syn::Item::ExternCrate(item) => &item.attrs,
        syn::Item::Fn(item) => &item.attrs,
        syn::Item::ForeignMod(item) => &item.attrs,
        syn::Item::Impl(item) => &item.attrs,
        syn::Item::Macro(item) => &item.attrs,
        syn::Item::Mod(item) => &item.attrs,
        syn::Item::Static(item) => &item.attrs,
        syn::Item::Struct(item) => &item.attrs,
        syn::Item::Trait(item) => &item.attrs,
        syn::Item::TraitAlias(item) => &item.attrs,
        syn::Item::Type(item) => &item.attrs,
        syn::Item::Union(item) => &item.attrs,
        syn::Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn non_test_items(items: &[syn::Item]) -> impl Iterator<Item = &syn::Item> {
    items
        .iter()
        .filter(|item| !is_test_only(item_attributes(item)))
}

/// Whose siblings an import reaches: the crate's top-level modules, or one
/// top-level module's direct submodules.
#[derive(Clone, Copy)]
enum Scope<'a> {
    Crate,
    Within(&'a str),
}

impl Scope<'_> {
    /// How many `super::` climb from a file at `depth` to the siblings' parent.
    fn climb(self, depth: usize) -> usize {
        match self {
            Self::Crate => depth,
            Self::Within(_) => depth.saturating_sub(1),
        }
    }
}

/// Records the siblings in `scope` an item's paths reach. `depth` is the
/// item's module depth below the crate root (1 for a top-level module's file).
fn scan_item(item: &syn::Item, depth: usize, scope: Scope<'_>, imported: &mut BTreeSet<String>) {
    if let syn::Item::Mod(syn::ItemMod {
        content: Some((_, items)),
        ..
    }) = item
    {
        for inner in non_test_items(items) {
            scan_item(inner, depth + 1, scope, imported);
        }
        return;
    }
    scan_tokens(item.to_token_stream(), depth, scope, imported);
}

fn is_path_separator(tokens: &[TokenTree], index: usize) -> bool {
    matches!(
        (tokens.get(index), tokens.get(index + 1)),
        (Some(TokenTree::Punct(first)), Some(TokenTree::Punct(second)))
            if first.as_char() == ':' && second.as_char() == ':'
    )
}

fn is_ident(tokens: &[TokenTree], index: usize, word: &str) -> bool {
    matches!(tokens.get(index), Some(TokenTree::Ident(ident)) if ident == word)
}

/// Scans tokens, macro invocations' included, for `crate::<sibling>` (or
/// `crate::<module>::<sibling>` within a module) and for `super::` chains
/// that climb to the siblings' parent.
fn scan_tokens(
    stream: TokenStream,
    depth: usize,
    scope: Scope<'_>,
    imported: &mut BTreeSet<String>,
) {
    let tokens: Vec<TokenTree> = stream.into_iter().collect();
    let mut index = 0;
    while index < tokens.len() {
        if let TokenTree::Group(group) = &tokens[index] {
            scan_tokens(group.stream(), depth, scope, imported);
        }
        let after_separator = index > 1 && is_path_separator(&tokens, index - 2);
        if !after_separator
            && is_ident(&tokens, index, "crate")
            && is_path_separator(&tokens, index + 1)
        {
            match scope {
                Scope::Crate => record_targets(&tokens, index + 3, imported),
                Scope::Within(module) => {
                    if is_ident(&tokens, index + 3, module) && is_path_separator(&tokens, index + 4)
                    {
                        record_targets(&tokens, index + 6, imported);
                    }
                }
            }
        } else if !after_separator && is_ident(&tokens, index, "super") {
            let mut climbed = 0;
            let mut cursor = index;
            while is_ident(&tokens, cursor, "super") && is_path_separator(&tokens, cursor + 1) {
                climbed += 1;
                cursor += 3;
            }
            if climbed == scope.climb(depth) {
                record_targets(&tokens, cursor, imported);
            }
            index = cursor.max(index + 1);
            continue;
        }
        index += 1;
    }
}

/// Records the module a path names after `crate::`: one name, or each first
/// name of a `{…}` group.
fn record_targets(tokens: &[TokenTree], index: usize, imported: &mut BTreeSet<String>) {
    match tokens.get(index) {
        Some(TokenTree::Ident(ident)) if ident != "self" => {
            imported.insert(ident.to_string());
        }
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Brace => {
            let inner: Vec<TokenTree> = group.stream().into_iter().collect();
            let mut at_segment_start = true;
            for (position, token) in inner.iter().enumerate() {
                if at_segment_start {
                    record_targets(&inner, position, imported);
                }
                at_segment_start =
                    matches!(token, TokenTree::Punct(punct) if punct.as_char() == ',');
            }
        }
        _ => {}
    }
}

/// ADR 0126's size alarm: the file's lines, less its `#[cfg(test)]` items'.
fn oversized(root: &Path, source: &Source, findings: &mut Vec<Finding>) {
    let Ok(text) = std::fs::read_to_string(&source.path) else {
        return;
    };
    let total = text.lines().count();
    let test_lines: usize = source
        .syntax
        .items
        .iter()
        .filter(|item| is_test_only(item_attributes(item)))
        .map(item_lines)
        .sum();
    let counted = total.saturating_sub(test_lines);
    let states_why = source.syntax.attrs.iter().any(|attribute| {
        attribute.path().is_ident("doc")
            && attribute.meta.require_name_value().is_ok_and(|pair| {
                pair.value
                    .to_token_stream()
                    .to_string()
                    .trim_matches('"')
                    .trim_start()
                    .starts_with("Size:")
            })
    });
    if counted > SIZE_ALARM_LINES && !states_why {
        let relative = source.path.strip_prefix(root).unwrap_or(&source.path);
        findings.push(Finding::new(
            "oversized-file",
            relative.to_string_lossy().replace('\\', "/"),
            format!("{counted} lines past 1,000 need a `Size:` line in the `//!` header saying why it is one concern"),
        ));
    }
}

/// The lines an item spans in its file, its attributes included.
fn item_lines(item: &syn::Item) -> usize {
    let tokens: Vec<TokenTree> = item.to_token_stream().into_iter().collect();
    match (tokens.first(), tokens.last()) {
        (Some(first), Some(last)) => last.span().end().line + 1 - first.span().start().line,
        _ => 0,
    }
}

fn has_cycle(graph: &BTreeMap<String, Vec<String>>) -> bool {
    fn visit<'a>(
        node: &'a str,
        graph: &'a BTreeMap<String, Vec<String>>,
        visiting: &mut BTreeSet<&'a str>,
        done: &mut BTreeSet<&'a str>,
    ) -> bool {
        if done.contains(node) {
            return false;
        }
        if !visiting.insert(node) {
            return true;
        }
        let cyclic = graph
            .get(node)
            .is_some_and(|next| next.iter().any(|next| visit(next, graph, visiting, done)));
        visiting.remove(node);
        done.insert(node);
        cyclic
    }
    let mut done = BTreeSet::new();
    graph
        .keys()
        .any(|node| visit(node, graph, &mut BTreeSet::new(), &mut done))
}
