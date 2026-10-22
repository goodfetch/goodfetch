//! The third-party notice inventory (ADRs 0018 and 0047; spec #130's `xtask
//! notices`): every dependency of each ecosystem and every declared input
//! with the license it is used under and where its source is, checked
//! against ADR 0018's allowlist. A license the allowlist refuses for its use,
//! or a declared input that states no source, is a finding naming the
//! dependency and its ecosystem; so is anything named or sourced from a
//! watchlist (story 61), from which nothing but vendor names is taken.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod cargo;
mod entry;
mod inputs;
mod license;
mod npm;
mod python;

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Serialize;

use self::entry::{Ecosystem, Entry, Usage};
use self::license::{Allowlist, Verdict};
use crate::check::Finding;
use crate::environment::Environment;
use crate::watchlist::Watchlist;

/// What the inventory is built from, beyond the workspace's own files.
pub(crate) struct Sources<'a> {
    pub(crate) cargo: &'a OsStr,
    /// Files of inputs beside the workspace's `third-party.toml` files.
    pub(crate) inputs: &'a [PathBuf],
}

/// One line of the inventory as it is written.
#[derive(Serialize)]
struct Notice<'a> {
    ecosystem: Ecosystem,
    name: &'a str,
    version: &'a str,
    /// The license it is used under: the stated one, or for a compiled
    /// Filter list the license of the adaptation.
    license: String,
    source: Option<&'a str>,
    #[serde(rename = "use")]
    usage: Usage,
}

#[derive(Serialize)]
struct Inventory<'a> {
    notices: Vec<Notice<'a>>,
}

/// Builds the inventory of the workspace at `root`, writes it to `out` when
/// given, and returns what the allowlist refuses.
pub(crate) fn check(
    root: &Path,
    sources: &Sources<'_>,
    out: Option<&Path>,
) -> Result<Vec<Finding>> {
    let allowlist = Allowlist::read(root)?;
    let mut entries = cargo::entries(sources.cargo, root)?;
    entries.extend(npm::entries(root)?);
    entries.extend(python::entries(root)?);
    entries.extend(inputs::entries(root, sources.inputs)?);
    entries.sort_by(|left, right| identity(left).cmp(&identity(right)));
    // A package in two of a lockfile's documents is listed once.
    entries.dedup_by(|left, right| identity(left) == identity(right));
    let mut findings = Vec::new();
    let mut notices = Vec::new();
    for entry in &entries {
        let license = match allowlist.judge(entry) {
            Verdict::Allowed(license) => license,
            Verdict::Refused(refusal) => {
                findings.push(Finding::new(
                    refusal.kind(),
                    subject(entry),
                    refusal.message(entry.license.as_deref()),
                ));
                entry.license.clone().unwrap_or_default()
            }
        };
        if entry
            .source
            .as_deref()
            .is_none_or(|source| source.trim().is_empty())
        {
            findings.push(Finding::new(
                "source-unknown",
                subject(entry),
                "it states no place its source is; state one",
            ));
        }
        if let Some(watchlist) = std::iter::once(entry.name.as_str())
            .chain(entry.source.as_deref())
            .find_map(Watchlist::named_in)
        {
            findings.push(Finding::new(
                "watchlist-source",
                subject(entry),
                format!(
                    "it is {} ({}), a watchlist Goodfetch reads vendor names from and \
                     takes nothing else (story 61); write the rule from the vendor's \
                     documentation and Goodfetch's own captures",
                    watchlist.name(),
                    watchlist.license()
                ),
            ));
        }
        notices.push(Notice {
            ecosystem: entry.ecosystem,
            name: &entry.name,
            version: &entry.version,
            license,
            source: entry.source.as_deref(),
            usage: entry.usage,
        });
    }
    if let Some(out) = out {
        let mut text = serde_json::to_string_pretty(&Inventory { notices })?;
        text.push('\n');
        if let Some(folder) = out.parent() {
            std::fs::create_dir_all(folder)
                .with_context(|| format!("creating {}", folder.display()))?;
        }
        std::fs::write(out, text).with_context(|| format!("writing {}", out.display()))?;
    }
    Ok(findings)
}

fn identity(entry: &Entry) -> (Ecosystem, &str, &str) {
    (entry.ecosystem, &entry.name, &entry.version)
}

/// `<ecosystem> <name>@<version>`.
fn subject(entry: &Entry) -> String {
    format!("{} {}@{}", entry.ecosystem, entry.name, entry.version)
}

/// `xtask notices`'s arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// A further file of inputs in `third-party.toml`'s format, such as
    /// the list a Goodfetch Chromium build writes.
    #[arg(long)]
    inputs: Vec<PathBuf>,
    /// Writes the inventory, as JSON, to this file.
    #[arg(long)]
    out: Option<PathBuf>,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let inputs: Vec<PathBuf> = arguments
        .inputs
        .iter()
        .map(|path| environment.path(path))
        .collect();
    let sources = Sources {
        cargo: &environment.cargo,
        inputs: &inputs,
    };
    let out = arguments.out.as_deref().map(|path| environment.path(path));
    Ok(crate::check::report(&check(
        &environment.root,
        &sources,
        out.as_deref(),
    )?))
}
