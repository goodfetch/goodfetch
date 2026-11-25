//! The dependency-direction check over the Cargo workspace's manifests: the
//! directions STYLE.md's crate rules make the compiler enforce.
//! - `goodfetch-test-support` depends on no Goodfetch crate, and every other
//!   crate but `goodfetch-test-client` takes it only as a dev-dependency
//!   (ADR 0062).
//! - `goodfetch-test-client` depends on no Goodfetch crate but
//!   `goodfetch-protocol` and `goodfetch-test-support`, every other crate
//!   takes it only as a dev-dependency, and `goodfetch-protocol` never takes
//!   it, so its tests never see two copies of its types (ADR 0062's note of
//!   2026-11-07).
//! - A test adapter crate, one of [`TEST_ADAPTERS`] (ADR 0125), has
//!   `publish = false`, every crate takes it only as a dev-dependency, and
//!   no crate among its dependencies, direct or through other workspace
//!   crates, names it in a file under its `src/`, where its unit tests
//!   live: such a test would link two copies of that crate. Their
//!   integration tests may take it, and so may any other crate's unit
//!   tests. `goodfetch-simulation`, turmoil's adapter of `transport`, is
//!   one: only tests run on the Simulation (ADR 0060).
//! - The Proving ground depends on no Goodfetch crate but
//!   `goodfetch-test-support`, so on no engine crate, nor on
//!   `goodfetch-conformance`, `goodfetch-protocol` or `goodfetch-test-client`
//!   (ADR 0032): the oracle stays independent of what it judges.
//! - `goodfetch-conformance` depends on no Goodfetch crate but
//!   `goodfetch-protocol` (and `goodfetch-test-support` for its tests): the
//!   harness tests only through the interface.
//! - `xtask` builds no Goodfetch crate but `goodfetch-protocol`: its
//!   dependencies and build-dependencies take no engine crate, so building
//!   it builds no engine, and `xtask codegen` reads the Products'
//!   executables through what they print (ADR 0072). Its tests may take
//!   any crate as a dev-dependency.
//! - `goodfetch-runtime` depends on no Goodfetch crate: every crate that
//!   builds a Tokio runtime takes it, the test helpers and the Proving ground
//!   included (ADR 0081).
//!
//! Beside them, a rule the manifests cannot hold: outside `crates/foxhound/`,
//! only `goodfetch-protocol` and `xtask` name the protocol's `foxhound`
//! module in their source (ADR 0099, [`crate::foxhound_module`]).
//!
//! `goodfetch-workspace-hack`, which every member takes (ADR 0094), holds no
//! code, and `goodfetch-runtime` builds a runtime and nothing else, so no
//! other rule counts either as a Goodfetch crate.
//!
//! The workspace-wide direction of ADR 0087 is cargo-deny's (`deny.toml`).

#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result};
use proc_macro2::{TokenStream, TokenTree};

use crate::check::Finding;
use crate::foxhound_module;
use crate::metadata::{self, Dependency, Package};

const TEST_SUPPORT: &str = "goodfetch-test-support";
const TEST_CLIENT: &str = "goodfetch-test-client";
/// The test adapter crates (ADR 0125): each implements a Goodfetch
/// interface for tests. The pull request that adds one adds it here.
const TEST_ADAPTERS: [&str; 5] = [
    "goodfetch-simulation",
    "goodfetch-scripted-model",
    "goodfetch-recording-gate",
    "goodfetch-stub-gateway",
    "goodfetch-memory-site",
];
const CONFORMANCE: &str = "goodfetch-conformance";
const PROTOCOL: &str = "goodfetch-protocol";
const WORKSPACE_HACK: &str = "goodfetch-workspace-hack";
const RUNTIME: &str = "goodfetch-runtime";
const XTASK: &str = "xtask";
/// The Proving ground's manifest, from the workspace's root (ADR 0032).
const PROVING_GROUND_MANIFEST: &str = "proving-ground/Cargo.toml";

pub(crate) fn check(cargo: &OsStr, root: &Path) -> Result<Vec<Finding>> {
    let packages = metadata::packages(cargo, root)?;
    let root = root
        .canonicalize()
        .with_context(|| format!("resolving {}", root.display()))?;
    let is_member = |dependency: &Dependency| {
        dependency.name != WORKSPACE_HACK
            && dependency.name != RUNTIME
            && packages
                .iter()
                .any(|package| package.name == dependency.name)
    };
    let mut findings = Vec::new();
    for package in &packages {
        let members = || package.dependencies.iter().filter(|d| is_member(d));
        takes_none(package, members().next(), &mut findings);
        takes_test_crates_as_dev_only(package, &mut findings);
        if TEST_ADAPTERS.contains(&package.name.as_str())
            && !package.publish.as_ref().is_some_and(Vec::is_empty)
        {
            findings.push(Finding::new(
                "test-adapter-published",
                &package.name,
                "a test adapter has `publish = false` (ADR 0125)",
            ));
        }
        if package.name == TEST_CLIENT
            && let Some(dependency) =
                members().find(|d| d.name != PROTOCOL && d.name != TEST_SUPPORT)
        {
            findings.push(Finding::new(
                "test-client-depends-on-more-than-protocol",
                &package.name,
                format!(
                    "it depends on `{}`; it takes no Goodfetch crate but `{PROTOCOL}` and `{TEST_SUPPORT}`",
                    dependency.name
                ),
            ));
        } else if package.name == PROTOCOL
            && package.dependencies.iter().any(|d| d.name == TEST_CLIENT)
        {
            findings.push(Finding::new(
                "protocol-takes-test-client",
                &package.name,
                format!(
                    "it takes `{TEST_CLIENT}`, which would give its tests two copies of its types"
                ),
            ));
        } else if takes_outside_dev(package, TEST_CLIENT) {
            findings.push(Finding::new(
                "test-client-not-dev-only",
                &package.name,
                format!("it takes `{TEST_CLIENT}` outside `[dev-dependencies]`"),
            ));
        }
        if is_proving_ground(package, &root)
            && let Some(dependency) = members().find(|d| d.name != TEST_SUPPORT)
        {
            findings.push(Finding::new(
                "proving-ground-depends-on-goodfetch",
                &package.name,
                format!(
                    "the Proving ground depends on `{}`; it takes no Goodfetch crate but `{TEST_SUPPORT}`",
                    dependency.name
                ),
            ));
        }
        if package.name == CONFORMANCE
            && let Some(dependency) =
                members().find(|d| d.name != PROTOCOL && d.name != TEST_SUPPORT)
        {
            findings.push(Finding::new(
                "conformance-depends-on-more-than-protocol",
                &package.name,
                format!(
                    "it depends on `{}`; it takes no Goodfetch crate but `{PROTOCOL}`",
                    dependency.name
                ),
            ));
        }
        if package.name != PROTOCOL && package.name != XTASK {
            foxhound_module::check(&root, package, &mut findings)?;
        }
        if package.name == XTASK
            && let Some(dependency) =
                members().find(|d| d.name != PROTOCOL && d.kind.as_deref() != Some("dev"))
        {
            findings.push(Finding::new(
                "xtask-builds-a-goodfetch-crate",
                &package.name,
                format!(
                    "it builds `{}`; outside `[dev-dependencies]` it takes no Goodfetch crate but `{PROTOCOL}`",
                    dependency.name
                ),
            ));
        }
    }
    for adapter in TEST_ADAPTERS {
        in_unit_tests_of_its_dependencies(adapter, &packages, &root, &mut findings)?;
    }
    Ok(findings)
}

/// A finding for each file under the `src/` of a crate among `adapter`'s
/// dependencies, direct or through other workspace crates, that names it.
/// `root` is the workspace's, canonical.
fn in_unit_tests_of_its_dependencies(
    adapter: &str,
    packages: &[Package],
    root: &Path,
    findings: &mut Vec<Finding>,
) -> Result<()> {
    let package = |name: &str| packages.iter().find(|package| package.name == name);
    let mut dependencies = BTreeSet::new();
    let mut next = vec![adapter];
    while let Some(name) = next.pop() {
        for dependency in package(name)
            .iter()
            .flat_map(|package| &package.dependencies)
        {
            if dependency.kind.as_deref() != Some("dev")
                && package(&dependency.name).is_some()
                && dependencies.insert(dependency.name.as_str())
            {
                next.push(&dependency.name);
            }
        }
    }
    for name in dependencies {
        let Some(dependency) = package(name) else {
            continue;
        };
        let names: BTreeSet<String> = dependency
            .dependencies
            .iter()
            .filter(|taken| taken.name == adapter)
            .map(|taken| taken.rename.as_deref().unwrap_or(adapter).replace('-', "_"))
            .collect();
        if names.is_empty() {
            continue;
        }
        let source = dependency
            .manifest_path
            .parent()
            .context("a manifest sits in a folder")?
            .join("src");
        let mut paths = Vec::new();
        if source.is_dir() {
            foxhound_module::rust_files(&source, &mut paths)?;
        }
        paths.sort();
        for path in paths {
            let text = std::fs::read_to_string(&path)
                .with_context(|| format!("reading {}", path.display()))?;
            let tokens: TokenStream = text
                .parse()
                .map_err(|error| anyhow::anyhow!("reading {} as Rust: {error}", path.display()))?;
            if names_any(tokens, &names) {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                findings.push(Finding::new(
                    "test-adapter-in-unit-tests-of-its-dependency",
                    relative.to_string_lossy().replace('\\', "/"),
                    format!(
                        "`{name}` names the test adapter `{adapter}` under its `src/`, which depends on `{name}`: a unit test there would link two copies of `{name}`; take it in `tests/` (ADR 0125)"
                    ),
                ));
            }
        }
    }
    Ok(())
}

/// Whether an identifier of `stream`, macro invocations' included, is one
/// of `names`.
fn names_any(stream: TokenStream, names: &BTreeSet<String>) -> bool {
    stream.into_iter().any(|token| match token {
        TokenTree::Group(group) => names_any(group.stream(), names),
        TokenTree::Ident(ident) => names.contains(&ident.to_string()),
        _ => false,
    })
}

/// The crates that take no Goodfetch crate, `goodfetch-runtime` and
/// `goodfetch-test-support`: `member` is the first one `package` takes.
fn takes_none(package: &Package, member: Option<&Dependency>, findings: &mut Vec<Finding>) {
    let code = match package.name.as_str() {
        RUNTIME => "runtime-depends-on-goodfetch",
        TEST_SUPPORT => "test-support-depends-on-goodfetch",
        _ => return,
    };
    if let Some(dependency) = member {
        findings.push(Finding::new(
            code,
            &package.name,
            format!("it depends on the workspace crate `{}`", dependency.name),
        ));
    }
}

/// `goodfetch-test-support`, which `goodfetch-test-client` alone takes as
/// a normal dependency, and the test adapters, which no crate does.
fn takes_test_crates_as_dev_only(package: &Package, findings: &mut Vec<Finding>) {
    if package.name != TEST_SUPPORT
        && package.name != TEST_CLIENT
        && takes_outside_dev(package, TEST_SUPPORT)
    {
        findings.push(Finding::new(
            "test-support-not-dev-only",
            &package.name,
            format!("it takes `{TEST_SUPPORT}` outside `[dev-dependencies]`"),
        ));
    }
    for adapter in TEST_ADAPTERS {
        if takes_outside_dev(package, adapter) {
            findings.push(Finding::new(
                "test-adapter-not-dev-only",
                &package.name,
                format!(
                    "it takes the test adapter `{adapter}` outside `[dev-dependencies]` (ADR 0125)"
                ),
            ));
        }
    }
}

/// Whether `package` takes the workspace crate `name` as a normal or build
/// dependency.
fn takes_outside_dev(package: &Package, name: &str) -> bool {
    package
        .dependencies
        .iter()
        .any(|dependency| dependency.name == name && dependency.kind.as_deref() != Some("dev"))
}

fn is_proving_ground(package: &Package, root: &Path) -> bool {
    package.manifest_path == root.join(PROVING_GROUND_MANIFEST)
}
