//! What a declaration is, and the Rust declarations of Features (ADRs 0072
//! and 0075): a `/// Covers:` line on a test function, or a `//! Covers:`
//! line on a test module for every test in it, read with `syn` from each
//! Cargo target's module tree and named as nextest names the test, an rstest
//! case included. Each declaration's level comes from the path of the file
//! that holds the test. The sibling modules `listed` and `covers` read the
//! other idioms.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::ffi::OsStr;
use std::fmt;
use std::path::{Component, Path, PathBuf};

use anyhow::{Context as _, Result};

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
    let canonical_root = root.canonicalize().unwrap_or_else(|_| root.to_path_buf());
    let mut declarations = Vec::new();
    for package in metadata::packages(cargo, root)? {
        for target in &package.targets {
            let Some(binary) = binary_id(&package.name, target) else {
                continue;
            };
            let context = Target {
                root: &canonical_root,
                package: &package.name,
                binary: &binary,
            };
            let source = &target.src_path;
            let folder = source.parent().context("a crate root sits in a folder")?;
            context.file(source, folder, &[], &[], &mut declarations)?;
        }
    }
    declarations.sort();
    declarations.dedup();
    Ok(declarations)
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
    binary: &'a str,
}

impl Target<'_> {
    /// Reads the module in `path`, whose child modules' files sit in
    /// `children`, at `module_path` below the crate root, with the Features
    /// its enclosing modules declare for every test in them.
    fn file(
        &self,
        path: &Path,
        children: &Path,
        module_path: &[String],
        inherited: &[String],
        declarations: &mut Vec<Declaration>,
    ) -> Result<()> {
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let syntax =
            syn::parse_file(&text).with_context(|| format!("parsing {}", path.display()))?;
        let mut features = inherited.to_vec();
        features.extend(covers(&syntax.attrs));
        let module = Module {
            path,
            children,
            names: module_path,
        };
        self.items(&module, &syntax.items, &features, declarations)
    }

    fn items(
        &self,
        module: &Module<'_>,
        items: &[syn::Item],
        features: &[String],
        declarations: &mut Vec<Declaration>,
    ) -> Result<()> {
        for item in items {
            match item {
                syn::Item::Fn(function) => {
                    self.function(module, function, features, declarations);
                }
                syn::Item::Mod(child) => self.module(module, child, features, declarations)?,
                _ => {}
            }
        }
        Ok(())
    }

    fn module(
        &self,
        parent: &Module<'_>,
        child: &syn::ItemMod,
        features: &[String],
        declarations: &mut Vec<Declaration>,
    ) -> Result<()> {
        let name = child.ident.to_string();
        let mut module_path = parent.names.to_vec();
        module_path.push(name.clone());
        let mut inherited = features.to_vec();
        inherited.extend(covers(&child.attrs));
        if let Some((_, items)) = &child.content {
            let module = Module {
                path: parent.path,
                children: &parent.children.join(&name),
                names: &module_path,
            };
            return self.items(&module, items, &inherited, declarations);
        }
        let file = [
            parent.children.join(format!("{name}.rs")),
            parent.children.join(&name).join("mod.rs"),
        ]
        .into_iter()
        .find(|path| path.is_file());
        // A module behind a `cfg` the platform lacks, or generated code
        // `include!`d elsewhere, may have no file here; it declares nothing.
        let Some(file) = file else {
            return Ok(());
        };
        let children = if file.file_name() == Some(OsStr::new("mod.rs")) {
            file.parent().map(Path::to_path_buf)
        } else {
            Some(parent.children.join(&name))
        }
        .unwrap_or_default();
        self.file(&file, &children, &module_path, &inherited, declarations)
    }

    fn function(
        &self,
        module: &Module<'_>,
        function: &syn::ItemFn,
        features: &[String],
        declarations: &mut Vec<Declaration>,
    ) {
        let attributes = &function.attrs;
        if !is_test(attributes) {
            return;
        }
        let mut all = features.to_vec();
        all.extend(covers(attributes));
        if all.is_empty() {
            return;
        }
        let mut name = module.names.to_vec();
        name.push(function.sig.ident.to_string());
        let base = name.join("::");
        let cases = cases(attributes);
        let tests: Vec<String> = if cases.is_empty() {
            vec![base]
        } else {
            cases.iter().map(|case| format!("{base}::{case}")).collect()
        };
        let path = relative(self.root, module.path);
        for test in &tests {
            for feature in &all {
                declarations.push(Declaration {
                    feature: feature.clone(),
                    level: Level::of(Path::new(&path)),
                    source: Source::Nextest,
                    test: format!("{} {test}", self.binary),
                    package: Some(self.package.to_owned()),
                    path: path.clone(),
                });
            }
        }
    }
}

struct Module<'a> {
    /// The file that holds the module's items.
    path: &'a Path,
    /// Where its child modules' files sit.
    children: &'a Path,
    /// Its path of module names below the crate root.
    names: &'a [String],
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

/// The Feature IDs of the `Covers:` lines among doc attributes.
fn covers(attributes: &[syn::Attribute]) -> Vec<String> {
    attributes
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
            }) => Some(text.value()),
            _ => None,
        })
        .filter_map(|line| line.trim().strip_prefix("Covers:").map(ToOwned::to_owned))
        .flat_map(|rest| {
            rest.split(',')
                .map(str::trim)
                .filter(|feature| !feature.is_empty())
                .map(str::to_owned)
                .collect::<Vec<_>>()
        })
        .collect()
}

/// The names nextest gives an rstest test's cases: `case_<n>` or
/// `case_<n>_<description>`, `n` from 1, padded to the count's digits.
fn cases(attributes: &[syn::Attribute]) -> Vec<String> {
    let descriptions: Vec<Option<String>> = attributes
        .iter()
        .filter_map(|attribute| {
            let segments: Vec<String> = attribute
                .path()
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            match segments.as_slice() {
                [case] if case == "case" => Some(None),
                [case, description] if case == "case" => Some(Some(description.clone())),
                _ => None,
            }
        })
        .collect();
    let width = descriptions.len().to_string().len();
    descriptions
        .iter()
        .enumerate()
        .map(|(index, description)| {
            let number = format!("{:0width$}", index + 1);
            match description {
                Some(description) => format!("case_{number}_{description}"),
                None => format!("case_{number}"),
            }
        })
        .collect()
}
