//! `xtask chromium surfaces check` (ADR 0074): no patch of the Goodfetch
//! Chromium goes untested. Every patch file under `chromium/patches/`, a
//! `.patch` file at any depth, has a row in `chromium/surfaces.toml`: the
//! surface it changes, or an infrastructure row of its own (build fixes,
//! background traffic, the variations seed), with its WPT directories and
//! its Goodfetch tests. The check fails on a patch without a row, a row
//! naming a patch that is gone, a row without a test, and a test its
//! runner does not list. It lists tests only when a row names one, so it
//! builds nothing while `chromium/` is empty.

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::OsStr;
use std::path::Path;

use anyhow::{Context as _, Result};
use serde::Deserialize;

use crate::check::Finding;
use crate::runners::{Runner, TestId, listing};

/// The surface table.
const TABLE: &str = "chromium/surfaces.toml";

/// The folder of the patch series: fingerprint-chromium's and Goodfetch's
/// own patches.
const PATCHES: &str = "chromium/patches";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Fails on a patch file under `chromium/patches/` without a row in
    /// `chromium/surfaces.toml`, a row naming a patch file that is gone, a
    /// row naming no test, and a test its runner does not list.
    Check {
        /// The Cargo profile nextest builds its listing with.
        #[arg(long)]
        cargo_profile: Option<String>,
    },
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Table {
    #[serde(default, rename = "surface")]
    surfaces: Vec<Surface>,
}

/// One row: a surface the series changes, or an infrastructure row.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Surface {
    name: String,
    /// Its patch files, by their path under `chromium/patches/`.
    patches: Vec<String>,
    /// The WPT directories the gate runs for it; an infrastructure row may
    /// name none.
    #[serde(default)]
    #[expect(
        dead_code,
        reason = "parsed so that a misspelt key fails; the gate's WPT run reads it"
    )]
    wpt: Vec<String>,
    tests: Vec<Test>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Test {
    runner: Runner,
    id: String,
}

pub(crate) fn run(command: &Command, root: &Path, cargo: &OsStr) -> Result<Vec<Finding>> {
    let Command::Check { cargo_profile } = command;
    let patches = patch_files(&root.join(PATCHES))?;
    let path = root.join(TABLE);
    let table: Table = if path.exists() {
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?
    } else {
        Table {
            surfaces: Vec::new(),
        }
    };

    let mut findings = Vec::new();
    let rowed: BTreeSet<&str> = table
        .surfaces
        .iter()
        .flat_map(|surface| surface.patches.iter().map(String::as_str))
        .collect();
    for patch in patches
        .iter()
        .filter(|patch| !rowed.contains(patch.as_str()))
    {
        findings.push(Finding::new(
            "patch-without-row",
            patch.clone(),
            format!("no row of `{TABLE}` names it: give it its surface's row, or an infrastructure row with its tests"),
        ));
    }

    let mut named: BTreeMap<Runner, Vec<(&Surface, TestId)>> = BTreeMap::new();
    for surface in &table.surfaces {
        for patch in surface
            .patches
            .iter()
            .filter(|patch| !patches.contains(*patch))
        {
            findings.push(Finding::new(
                "patch-missing",
                format!("{} {patch}", surface.name),
                format!("the row names it, but `{PATCHES}/` holds no such patch file"),
            ));
        }
        if surface.tests.is_empty() {
            findings.push(Finding::new(
                "row-without-test",
                surface.name.clone(),
                "the row names no test: a patch is never untested",
            ));
        }
        for test in &surface.tests {
            let id = TestId::parse(test.runner, &test.id)
                .with_context(|| format!("reading {TABLE}'s row `{}`", surface.name))?;
            named.entry(test.runner).or_default().push((surface, id));
        }
    }

    for (runner, tests) in named {
        let listed = listing::tests(runner, root, cargo, cargo_profile.as_deref())?;
        for (surface, id) in tests.into_iter().filter(|(_, id)| !listed.contains(id)) {
            findings.push(Finding::new(
                "test-missing",
                format!("{} {id}", surface.name),
                format!("{} lists no such test", runner.name()),
            ));
        }
    }
    Ok(findings)
}

/// Every `.patch` file under `folder`, by its `/`-separated path from it;
/// none when it is absent.
fn patch_files(folder: &Path) -> Result<BTreeSet<String>> {
    let mut patches = BTreeSet::new();
    if folder.exists() {
        under(folder, "", &mut patches)?;
    }
    Ok(patches)
}

fn under(folder: &Path, prefix: &str, patches: &mut BTreeSet<String>) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("{} is not named in UTF-8", path.display()))?;
        let relative = format!("{prefix}{name}");
        if path.is_dir() {
            under(&path, &format!("{relative}/"), patches)?;
        } else if path.extension().is_some_and(|ending| ending == "patch") {
            patches.insert(relative);
        }
    }
    Ok(())
}
