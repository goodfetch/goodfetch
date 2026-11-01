//! The names nextest gives an rstest test's cases, derived as rstest 0.27's
//! macro derives them (ADR 0072, "What counts as tested", rule 2): one
//! module per `#[case]`, then one module per `#[values]` argument in the
//! signature's order, then one per `#[files]` argument, the last one the
//! test function itself. A form whose names this module cannot derive is an
//! error, so a declaration never names a test nextest does not run. As
//! rstest does, it gives a `#[case]` the attributes written between the case
//! before it, or `#[rstest]`, and its own line.
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use quote::ToTokens as _;
use regex::Regex;
use syn::parse::ParseStream;
use syn::punctuated::Punctuated;

/// An rstest test's cases, and the attributes all of them share.
pub(crate) struct Cases<'a> {
    /// Empty for a test that is not parametrized.
    pub(crate) suffixes: Vec<Suffix<'a>>,
    /// The function's attributes rstest gives every case: those above
    /// `#[rstest]` and below the last `#[case]`; all of them without a
    /// `#[case]`.
    pub(crate) shared: Vec<&'a syn::Attribute>,
}

/// The name suffix of one case below the function's name, such as
/// `case_1_empty` or `key_1___a__::layer_2_Run`, with the attributes rstest
/// gives its `#[case]` alone.
pub(crate) struct Suffix<'a> {
    pub(crate) name: String,
    pub(crate) own: Vec<&'a syn::Attribute>,
}

/// The cases of an rstest test. `manifest_dir` is the test's package
/// folder, which a `#[files]` glob is relative to.
pub(crate) fn suffixes<'a>(function: &'a syn::ItemFn, manifest_dir: &Path) -> Result<Cases<'a>> {
    if let Some(attribute) = function
        .attrs
        .iter()
        .find(|attribute| is_named(attribute, "rstest"))
        && !matches!(attribute.meta, syn::Meta::Path(_))
    {
        bail!(
            "arguments in the `#[rstest(...)]` attribute are not read; use `#[case]` and `#[values]`"
        );
    }
    let (cases, shared) = cases(&function.attrs);
    let mut values = Vec::new();
    let mut files = Vec::new();
    for input in &function.sig.inputs {
        let syn::FnArg::Typed(argument) = input else {
            continue;
        };
        let has_values = argument
            .attrs
            .iter()
            .any(|attribute| is_named(attribute, "values"));
        let has_files = argument
            .attrs
            .iter()
            .any(|attribute| is_named(attribute, "files"));
        if !has_values && !has_files {
            continue;
        }
        let syn::Pat::Ident(pattern) = argument.pat.as_ref() else {
            bail!(
                "a `#[values]` or `#[files]` argument must be a plain name, which rstest names its cases by"
            );
        };
        let name = pattern.ident.to_string();
        if has_values {
            values.push(List {
                argument: name,
                descriptions: value_descriptions(&argument.attrs)?,
            });
        } else {
            files.push(List {
                argument: name,
                descriptions: file_descriptions(&argument.attrs, manifest_dir)?,
            });
        }
    }
    let mut names: Vec<Suffix<'a>> = cases;
    for list in values.iter().chain(&files) {
        let width = list.descriptions.len().to_string().len();
        let segments: Vec<String> = list
            .descriptions
            .iter()
            .enumerate()
            .map(|(index, description)| {
                let description: String = sanitize(description).chars().take(64).collect();
                format!("{}_{:0width$}_{description}", list.argument, index + 1)
            })
            .collect();
        names = if names.is_empty() {
            segments
                .into_iter()
                .map(|name| Suffix {
                    name,
                    own: Vec::new(),
                })
                .collect()
        } else {
            names
                .iter()
                .flat_map(|suffix| {
                    segments.iter().map(move |segment| Suffix {
                        name: format!("{}::{segment}", suffix.name),
                        own: suffix.own.clone(),
                    })
                })
                .collect()
        };
    }
    Ok(Cases {
        suffixes: names,
        shared,
    })
}

/// One argument's list of values, by their unsanitized descriptions.
struct List {
    argument: String,
    descriptions: Vec<String>,
}

/// Whether an attribute's last path segment is `name`.
fn is_named(attribute: &syn::Attribute, name: &str) -> bool {
    attribute
        .path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == name)
}

/// The names nextest gives an rstest test's cases, `case_<n>` or
/// `case_<n>_<description>`, `n` from 1, padded to the count's digits, each
/// with the attributes between the case before it, or `#[rstest]`, and its
/// `#[case]`; and the attributes every case shares.
fn cases(attributes: &[syn::Attribute]) -> (Vec<Suffix<'_>>, Vec<&syn::Attribute>) {
    let mut found: Vec<(Option<String>, Vec<&syn::Attribute>)> = Vec::new();
    let mut shared = Vec::new();
    let mut own = Vec::new();
    let mut below_rstest = false;
    for attribute in attributes {
        let segments: Vec<String> = attribute
            .path()
            .segments
            .iter()
            .map(|segment| segment.ident.to_string())
            .collect();
        match segments.as_slice() {
            [case] if case == "case" => found.push((None, std::mem::take(&mut own))),
            [case, description] if case == "case" => {
                found.push((Some(description.clone()), std::mem::take(&mut own)));
            }
            _ if below_rstest => own.push(attribute),
            _ => {
                below_rstest = is_named(attribute, "rstest");
                shared.push(attribute);
            }
        }
    }
    shared.extend(own);
    let width = found.len().to_string().len();
    let suffixes = found
        .into_iter()
        .enumerate()
        .map(|(index, (description, own))| {
            let number = format!("{:0width$}", index + 1);
            let name = match description {
                Some(description) => format!("case_{number}_{description}"),
                None => format!("case_{number}"),
            };
            Suffix { name, own }
        })
        .collect();
    (suffixes, shared)
}

/// The descriptions of a `#[values(...)]` list: a value's doc comment, its
/// lines joined by `_`, or else its expression's tokens.
fn value_descriptions(attributes: &[syn::Attribute]) -> Result<Vec<String>> {
    let mut descriptions = Vec::new();
    for attribute in attributes
        .iter()
        .filter(|attribute| is_named(attribute, "values"))
    {
        let values = attribute
            .parse_args_with(|input: ParseStream<'_>| {
                Punctuated::<(Vec<syn::Attribute>, syn::Expr), syn::Token![,]>::parse_terminated_with(
                    input,
                    |input| Ok((input.call(syn::Attribute::parse_outer)?, input.parse()?)),
                )
            })
            .context("reading a `#[values(...)]` list")?;
        for (doc, expression) in values {
            let lines: Vec<String> = doc
                .iter()
                .filter(|attribute| attribute.path().is_ident("doc"))
                .filter_map(|attribute| match &attribute.meta {
                    syn::Meta::NameValue(syn::MetaNameValue {
                        value:
                            syn::Expr::Lit(syn::ExprLit {
                                lit: syn::Lit::Str(text),
                                ..
                            }),
                        ..
                    }) => Some(text.value().trim().to_owned()),
                    _ => None,
                })
                .filter(|line| !line.is_empty())
                .collect();
            descriptions.push(if lines.is_empty() {
                expression.to_token_stream().to_string()
            } else {
                sanitize(&lines.join(" ").replace(' ', "_"))
            });
        }
    }
    Ok(descriptions)
}

/// The descriptions of a `#[files(...)]` argument: each matched file's path
/// relative to the base folder, `..` written `_UP`, sorted by absolute path,
/// without directories or dot files unless asked for, nor excluded paths.
fn file_descriptions(attributes: &[syn::Attribute], manifest_dir: &Path) -> Result<Vec<String>> {
    let string = |attribute: &syn::Attribute| -> Result<String> {
        let text = match &attribute.meta {
            syn::Meta::List(_) => attribute.parse_args::<syn::LitStr>()?.value(),
            syn::Meta::NameValue(syn::MetaNameValue {
                value:
                    syn::Expr::Lit(syn::ExprLit {
                        lit: syn::Lit::Str(text),
                        ..
                    }),
                ..
            }) => text.value(),
            _ => bail!("expected a string"),
        };
        if text.contains('$') {
            bail!("`{text}` names an environment variable, which the scanner does not read");
        }
        Ok(text)
    };
    let named = |name: &'static str| {
        attributes
            .iter()
            .filter(move |attribute| is_named(attribute, name))
    };
    let base = match named("base_dir").next() {
        Some(attribute) => manifest_dir.join(string(attribute)?),
        None => manifest_dir.to_path_buf(),
    };
    let base = base.canonicalize().unwrap_or(base);
    let excludes = named("exclude")
        .map(|attribute| Regex::new(&string(attribute)?).context("reading an `#[exclude]` pattern"))
        .collect::<Result<Vec<_>>>()?;
    let directories = named("dirs").next().is_some();
    let dot_files = named("include_dot_files").next().is_some();
    let mut paths = Vec::new();
    for attribute in named("files") {
        let pattern = base.join(string(attribute)?);
        let pattern = pattern.to_string_lossy();
        for path in glob::glob(&pattern).with_context(|| format!("reading the glob `{pattern}`"))? {
            let path = path.with_context(|| format!("reading the glob `{pattern}`"))?;
            paths.push(
                path.canonicalize()
                    .with_context(|| format!("resolving {}", path.display()))?,
            );
        }
    }
    paths.sort();
    paths.dedup();
    let mut descriptions = Vec::new();
    for path in paths {
        let relative = relative_to(&path, &base);
        let components: Vec<String> = relative
            .components()
            .filter_map(|component| match component {
                Component::ParentDir => Some("_UP".to_owned()),
                Component::Normal(name) => Some(name.to_string_lossy().into_owned()),
                _ => None,
            })
            .collect();
        let joined = relative.to_string_lossy().replace('\\', "/");
        if (!directories && path.is_dir())
            || (!dot_files && components.iter().any(|name| name.starts_with('.')))
            || excludes.iter().any(|exclude| exclude.is_match(&joined))
        {
            continue;
        }
        descriptions.push(components.join("/"));
    }
    if descriptions.is_empty() {
        bail!("a `#[files]` glob matches no file");
    }
    Ok(descriptions)
}

/// `path` relative to `base`, with `..` for each of `base`'s folders it is
/// not below, as `relative-path`'s `relative_to` gives it to rstest.
fn relative_to(path: &Path, base: &Path) -> PathBuf {
    let path: Vec<Component<'_>> = path.components().collect();
    let base: Vec<Component<'_>> = base.components().collect();
    let common = path
        .iter()
        .zip(&base)
        .take_while(|(left, right)| left == right)
        .count();
    let mut relative = PathBuf::new();
    for _ in common..base.len() {
        relative.push("..");
    }
    for component in &path[common..] {
        relative.push(component);
    }
    relative
}

/// rstest's `sanitize_ident`: whitespace dropped, quotes made `__`, the
/// punctuation that cannot sit in a name made `_`, and any other character
/// that cannot continue an identifier dropped.
fn sanitize(text: &str) -> String {
    text.chars()
        .filter(|character| !character.is_whitespace())
        .flat_map(|character| match character {
            '"' | '\'' => vec!['_', '_'],
            ':' | '(' | ')' | '{' | '}' | '[' | ']' | ',' | '.' | '*' | '+' | '/' | '-' | '%'
            | '^' | '!' | '&' | '|' => vec!['_'],
            other => vec![other],
        })
        .filter(|character| unicode_ident::is_xid_continue(*character))
        .collect()
}
