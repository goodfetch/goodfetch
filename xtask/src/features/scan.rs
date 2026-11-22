//! What a declaration is, and the Rust declarations of Features (ADRs 0072
//! and 0075): a `/// Covers:` declaration on a test function, or a `//! Covers:`
//! one on a test module for every test in it, read with `syn` from each
//! Cargo target's module tree and named as nextest names the test, an rstest
//! case included. Each declaration's level comes from the path of the file
//! that holds the test. The sibling modules `listed` and `covers` read the
//! other idioms. `tests` lists every Rust test the same way, declaring or
//! not, which `xtask clients check` reads too.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::journey::FeatureId;
use quote::ToTokens as _;

use super::rstest_names;
use crate::metadata;

/// Where a test sits, read from its path (ADR 0075).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Level {
    /// Under `src/`, or `tests/unit/`.
    Unit,
    /// Under `tests/` otherwise: `tests/it`, `<package>/tests/` or
    /// `tests/integration/`.
    Integration,
    /// Under the top-level `conformance/` folder, `conformance/clients/`
    /// included.
    Conformance,
    /// Under a `journeys` folder of `tests/`, such as `tests/it/journeys/`.
    Journey,
}

impl Level {
    /// The level of a test held in `path`, relative to the workspace's root.
    pub(crate) fn of(path: &Path) -> Self {
        let folders: Vec<&OsStr> = path
            .parent()
            .into_iter()
            .flat_map(Path::components)
            .filter_map(|component| match component {
                Component::Normal(name) => Some(name),
                _ => None,
            })
            .collect();
        let tests = folders.iter().position(|folder| *folder == "tests");
        let below_tests = |name: &str| {
            tests.is_some_and(|index| folders[index + 1..].iter().any(|folder| *folder == name))
        };
        // A module's own `tests/` folder below `src/` holds unit tests.
        let below_source = folders
            .iter()
            .position(|folder| *folder == "src")
            .is_some_and(|source| tests.is_none_or(|tests| source < tests));
        if below_tests("journeys") {
            Self::Journey
        } else if folders
            .first()
            .is_some_and(|folder| *folder == "conformance")
        {
            Self::Conformance
        } else if below_source
            || tests.is_some_and(|index| folders.get(index + 1) == Some(&OsStr::new("unit")))
        {
            Self::Unit
        } else if tests.is_some() {
            Self::Integration
        } else {
            Self::Unit
        }
    }

    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Unit => "unit",
            Self::Integration => "integration",
            Self::Conformance => "conformance",
            Self::Journey => "journey",
        }
    }
}

impl fmt::Display for Level {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// What declares a Feature: a runner's test, a Journey or a conformance case.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Source {
    Nextest,
    Vitest,
    Playwright,
    Pytest,
    /// A Journey's `covers` list.
    Journey,
    /// A conformance case's `covers` list, a Client conformance case's
    /// included.
    Conformance,
}

impl Source {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Nextest => "nextest",
            Self::Vitest => "vitest",
            Self::Playwright => "playwright",
            Self::Pytest => "pytest",
            Self::Journey => "journey",
            Self::Conformance => "conformance",
        }
    }
}

impl fmt::Display for Source {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

/// One test's claim that it proves one Feature.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Declaration {
    /// The Feature's ID as written.
    pub(crate) feature: String,
    /// Where the test sits.
    pub(crate) level: Level,
    /// What declares it.
    pub(crate) source: Source,
    /// The test as its runner names it (the quarantine list's test IDs): for
    /// nextest `<binary id> <test name>`; for a Journey its ID; for a
    /// conformance case `<suite> <case id>`.
    pub(crate) test: String,
    /// The package that holds the test: the crate, or the nearest
    /// `package.json`'s or `pyproject.toml`'s name; none for a Journey or a
    /// conformance case.
    pub(crate) package: Option<String>,
    /// The file that holds the test, relative to the workspace's root.
    pub(crate) path: String,
}

/// Every Rust declaration of the workspace at `root`, sorted.
pub(crate) fn rust(cargo: &OsStr, root: &Path) -> Result<Vec<Declaration>> {
    let mut declarations: Vec<Declaration> = tests(cargo, root, None)?
        .iter()
        .flat_map(|test| {
            test.features.iter().map(|feature| Declaration {
                feature: feature.clone(),
                level: Level::of(Path::new(&test.path)),
                source: Source::Nextest,
                test: format!("{} {}", test.binary, test.name),
                package: Some(test.package.clone()),
                path: test.path.clone(),
            })
        })
        .collect();
    declarations.sort();
    declarations.dedup();
    Ok(declarations)
}

/// One Rust test, as nextest names it, with the Features it declares.
#[derive(Debug)]
pub(crate) struct Test {
    /// nextest's binary ID of the target that holds it.
    pub(crate) binary: String,
    /// Its name in that binary, an rstest case's included:
    /// `store::keeps_a_value::case_1_empty`.
    pub(crate) name: String,
    /// The crate that holds it.
    pub(crate) package: String,
    /// The file that holds it, relative to the workspace's root.
    pub(crate) path: String,
    /// The Features its `Covers:` lines and its modules' declare.
    pub(crate) features: Vec<String>,
    /// What may keep it from running, on it or an enclosing module: each
    /// `cfg` predicate but `test`, as `cfg(unix)`, a `cfg_attr`, and
    /// `ignore` for an `#[ignore]`d test.
    pub(crate) gates: Vec<String>,
    /// Its function's body, which `xtask clients check` reads for the
    /// `ClientCaseRun::start` call that carries a case on the `sdk-rust`
    /// surface.
    pub(crate) body: proc_macro2::TokenStream,
}

/// Every Rust test of the workspace at `root`, read from each Cargo target's
/// module tree; only those of `packages` when given.
pub(crate) fn tests(cargo: &OsStr, root: &Path, packages: Option<&[&str]>) -> Result<Vec<Test>> {
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut tests = Vec::new();
    for package in metadata::packages(cargo, root)? {
        if packages.is_some_and(|packages| !packages.contains(&package.name.as_str())) {
            continue;
        }
        for target in &package.targets {
            let Some(binary) = binary_id(&package.name, target) else {
                continue;
            };
            let context = Target {
                root: &canonical_root,
                package: &package.name,
                manifest_dir: package
                    .manifest_path
                    .parent()
                    .context("a manifest sits in a folder")?,
                binary: &binary,
            };
            let source = &target.src_path;
            let folder = source.parent().context("a crate root sits in a folder")?;
            context.file(source, folder, &[], &Inherited::default(), &mut tests)?;
        }
    }
    Ok(tests)
}

/// nextest's binary ID of a target that holds tests: the package's name for
/// its library, `<package>::bin/<name>` for an executable and
/// `<package>::<name>` for an integration-test target.
fn binary_id(package: &str, target: &metadata::Target) -> Option<String> {
    let kinds = &target.kind;
    if kinds
        .iter()
        .any(|kind| kind == "lib" || kind == "proc-macro")
    {
        Some(package.to_owned())
    } else if kinds.iter().any(|kind| kind == "bin") {
        Some(format!("{package}::bin/{}", target.name))
    } else if kinds.iter().any(|kind| kind == "test") {
        Some(format!("{package}::{}", target.name))
    } else {
        None
    }
}

struct Target<'a> {
    root: &'a Path,
    package: &'a str,
    /// The package's folder, which an rstest `#[files]` glob is relative to.
    manifest_dir: &'a Path,
    binary: &'a str,
}

/// What a module gives every test in it.
#[derive(Clone, Default)]
struct Inherited {
    /// The Features its `Covers:` lines declare.
    features: Vec<String>,
    /// Its gates, as [`Test::gates`] holds them.
    gates: Vec<String>,
}

impl Inherited {
    /// What a module or function with these attributes gives within it.
    fn within(&self, attributes: &[syn::Attribute]) -> Self {
        let mut within = self.clone();
        within.features.extend(covers(attributes));
        within.gates.extend(gates(attributes));
        within
    }
}

impl Target<'_> {
    /// Reads the module in `path`, whose child modules' files sit in
    /// `children`, at `module_path` below the crate root, with the Features
    /// and gates its enclosing modules give every test in them.
    fn file(
        &self,
        path: &Path,
        children: &Path,
        module_path: &[String],
        inherited: &Inherited,
        tests: &mut Vec<Test>,
    ) -> Result<()> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let syntax =
            syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))?;
        let within = inherited.within(&syntax.attrs);
        let module = Module {
            path,
            children,
            names: module_path,
            inline: false,
        };
        self.items(&module, &syntax.items, &within, tests)
    }

    fn items(
        &self,
        module: &Module<'_>,
        items: &[syn::Item],
        inherited: &Inherited,
        tests: &mut Vec<Test>,
    ) -> Result<()> {
        for item in items {
            match item {
                syn::Item::Fn(function) => self.function(module, function, inherited, tests)?,
                syn::Item::Mod(child) => self.module(module, child, inherited, tests)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn module(
        &self,
        parent: &Module<'_>,
        child: &syn::ItemMod,
        inherited: &Inherited,
        tests: &mut Vec<Test>,
    ) -> Result<()> {
        let name = child.ident.to_string();
        let mut module_path = parent.names.to_vec();
        module_path.push(name.clone());
        let inherited = inherited.within(&child.attrs);
        if let Some((_, items)) = &child.content {
            let module = Module {
                path: parent.path,
                children: &parent.children.join(&name),
                names: &module_path,
                inline: true,
            };
            return self.items(&module, items, &inherited, tests);
        }
        let (file, children) = if let Some(named) = path_attribute(&child.attrs)? {
            // rustc reads a `#[path]` outside an inline module from the
            // declaring file's folder, and the named file's children from
            // its own folder, as a `mod.rs` file's.
            if parent.inline {
                bail!(
                    "{}: module `{name}` has a `#[path]` inside an inline module, which the scan does not read",
                    parent.path.display()
                );
            }
            let file = parent.path.parent().unwrap_or(Path::new("")).join(named);
            let children = file.parent().map(Path::to_path_buf).unwrap_or_default();
            (file, children)
        } else {
            let candidates = [
                parent.children.join(format!("{name}.rs")),
                parent.children.join(&name).join("mod.rs"),
            ];
            let file = candidates
                .iter()
                .find(|path| path.is_file())
                .unwrap_or(&candidates[0])
                .clone();
            let children = if file.file_name() == Some(OsStr::new("mod.rs")) {
                file.parent().map(Path::to_path_buf).unwrap_or_default()
            } else {
                parent.children.join(&name)
            };
            (file, children)
        };
        if !file.is_file() {
            // A module behind a `cfg` the platform lacks may have no file
            // here; it declares nothing. Any other would leave its tests'
            // declarations unread.
            if child
                .attrs
                .iter()
                .any(|attribute| attribute.path().is_ident("cfg"))
            {
                return Ok(());
            }
            bail!(
                "{}: module `{name}` has no file at {}",
                parent.path.display(),
                file.display()
            );
        }
        self.file(&file, &children, &module_path, &inherited, tests)
    }

    fn function(
        &self,
        module: &Module<'_>,
        function: &syn::ItemFn,
        inherited: &Inherited,
        tests: &mut Vec<Test>,
    ) -> Result<()> {
        let attributes = &function.attrs;
        if !is_test(attributes) {
            return Ok(());
        }
        let mut name = module.names.to_vec();
        name.push(function.sig.ident.to_string());
        let base = name.join("::");
        let cases = rstest_names::suffixes(function, self.manifest_dir).with_context(|| {
            format!(
                "naming the cases of `{}` in {} as nextest does",
                function.sig.ident,
                module.path.display()
            )
        })?;
        // A `Covers:` line rstest gives one case declares for that case only.
        let shared: Vec<String> = inherited
            .features
            .iter()
            .cloned()
            .chain(covers(cases.shared))
            .collect();
        let gates = inherited.within(attributes).gates;
        let named: Vec<(String, Vec<String>)> = if cases.suffixes.is_empty() {
            vec![(base, Vec::new())]
        } else {
            cases
                .suffixes
                .into_iter()
                .map(|case| (format!("{base}::{}", case.name), covers(case.own)))
                .collect()
        };
        let path = relative(self.root, module.path);
        let body = function.block.to_token_stream();
        tests.extend(named.into_iter().map(|(name, own)| Test {
            binary: self.binary.to_owned(),
            name,
            package: self.package.to_owned(),
            path: path.clone(),
            features: shared.iter().cloned().chain(own).collect(),
            gates: gates.clone(),
            body: body.clone(),
        }));
        Ok(())
    }
}

struct Module<'a> {
    /// The file that holds the module's items.
    path: &'a Path,
    /// Where its child modules' files sit.
    children: &'a Path,
    /// Its path of module names below the crate root.
    names: &'a [String],
    /// Whether it is a `mod name { … }` block inside its file.
    inline: bool,
}

/// The file a `#[path = "…"]` attribute names, if the module has one.
fn path_attribute(attributes: &[syn::Attribute]) -> Result<Option<String>> {
    let Some(attribute) = attributes
        .iter()
        .find(|attribute| attribute.path().is_ident("path"))
    else {
        return Ok(None);
    };
    match &attribute.meta {
        syn::Meta::NameValue(syn::MetaNameValue {
            value:
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(named),
                    ..
                }),
            ..
        }) => Ok(Some(named.value())),
        _ => bail!("a `#[path]` attribute that names no file"),
    }
}

/// The path of `path` from the workspace's root, with `/`.
fn relative(root: &Path, path: &Path) -> String {
    let canonical: PathBuf = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    canonical
        .strip_prefix(root)
        .unwrap_or(&canonical)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Whether a function is a test: `#[test]`, an async runtime's `test`
/// attribute such as `#[tokio::test]`, or `#[rstest]`.
fn is_test(attributes: &[syn::Attribute]) -> bool {
    attributes.iter().any(|attribute| {
        attribute
            .path()
            .segments
            .last()
            .is_some_and(|segment| segment.ident == "test" || segment.ident == "rstest")
    })
}

/// What among `attributes` may keep a test from running: each `cfg`
/// predicate but `test`, which holds wherever tests run, as `cfg(<predicate>)`;
/// a `cfg_attr`, whose attribute may be `ignore`; and `ignore`.
fn gates(attributes: &[syn::Attribute]) -> Vec<String> {
    attributes
        .iter()
        .filter_map(|attribute| {
            let path = attribute.path();
            if path.is_ident("ignore") {
                return Some("ignore".to_owned());
            }
            if !path.is_ident("cfg") && !path.is_ident("cfg_attr") {
                return None;
            }
            let predicate = match &attribute.meta {
                syn::Meta::List(list) => list.tokens.to_string(),
                _ => String::new(),
            };
            (predicate != "test").then(|| format!("{}({predicate})", path.segments[0].ident))
        })
        .collect()
}

/// The Feature IDs of the `Covers:` declarations among doc attributes. A
/// declaration wrapped onto the doc lines below it reads on while each line
/// holds Feature IDs only: prose, a blank doc line or another attribute ends
/// it.
fn covers<'a>(attributes: impl IntoIterator<Item = &'a syn::Attribute>) -> Vec<String> {
    let mut features = Vec::new();
    let mut declaring = false;
    for attribute in attributes {
        let Some(text) = doc_text(attribute) else {
            declaring = false;
            continue;
        };
        // `split`, not `lines`: a blank `///` is one empty line, which ends
        // a declaration.
        for line in text.split('\n').map(str::trim) {
            if let Some(rest) = line.strip_prefix("Covers:") {
                features.extend(feature_ids(rest));
                declaring = true;
            } else if declaring && let Some(continued) = continuation(line) {
                features.extend(continued);
            } else {
                declaring = false;
            }
        }
    }
    features
}

/// A doc attribute's text, `None` for any other attribute.
fn doc_text(attribute: &syn::Attribute) -> Option<String> {
    if !attribute.path().is_ident("doc") {
        return None;
    }
    match &attribute.meta {
        syn::Meta::NameValue(syn::MetaNameValue {
            value:
                syn::Expr::Lit(syn::ExprLit {
                    lit: syn::Lit::Str(text),
                    ..
                }),
            ..
        }) => Some(text.value()),
        _ => None,
    }
}

/// The comma-separated Feature IDs of a declaration's line.
fn feature_ids(line: &str) -> impl Iterator<Item = String> + '_ {
    line.split(',')
        .map(str::trim)
        .filter(|feature| !feature.is_empty())
        .map(str::to_owned)
}

/// The Feature IDs of a doc line below a `Covers:` line when it carries on
/// the declaration, holding Feature IDs and nothing else; `None` otherwise.
fn continuation(line: &str) -> Option<Vec<String>> {
    let features: Vec<String> = feature_ids(line).collect();
    let carries_on = !features.is_empty()
        && features
            .iter()
            .all(|feature| FeatureId::try_from(feature.clone()).is_ok());
    carries_on.then_some(features)
}
