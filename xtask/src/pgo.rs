//! `xtask pgo`: the profile-guided release build of the Linux musl
//! executables (ADR 0057's note of 2026-11-14). ci/musl/cargo builds a
//! release musl executable with the merged profile and the order file
//! committed under `ci/pgo/<target>/`; `refresh` trains and rewrites them,
//! `check` holds a release build's `.text` to the order file, and
//! `validate` times the profile-guided build against a plain one on the
//! trained workloads and the held-out ones.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use clap::{Args, Subcommand};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::test_tools::{self, Tool, ToolManifest};

#[cfg(target_os = "linux")]
mod guided;
mod order;
mod refresh;
#[cfg(target_os = "linux")]
mod touch;
mod validate;

/// Where the committed profile and order file of a target are.
const COMMITTED: &str = "ci/pgo";
/// The merged profile's name in its folder.
const PROFILE: &str = "merged.profdata";
/// The order file's name in its folder.
const ORDER: &str = "order.txt";
/// The share of the order file's symbols, in percent, below which a build
/// is not the one the profile was trained on: halfway between the 90% a
/// build held of the 2026-11-14 profile's symbols in the crates whose names
/// held, and the 69% it held in all, once Cargo's metadata had renamed
/// seven of the Products' crates (#2203).
const LOWEST_MATCH: usize = 80;

#[derive(Subcommand)]
pub(crate) enum Command {
    /// Fails when a Product's release executable for the target does not
    /// start `.text` with the committed order file's first functions, in
    /// its order, as the link map beside the executable's build places
    /// them (Zig's linker strips the symbol table from the executable);
    /// and when the order file names a crate under an identity the build
    /// links it under no more, or the builds hold less than 80% of its
    /// symbols, a share it notes.
    Check(CheckArguments),
    /// Trains the profile and rewrites the committed profile and order
    /// file: an instrumented build runs the training workloads, its
    /// profiles merged; a build guided by them gives the touch map of the
    /// training crawls, and the order lists the code they touched first,
    /// the profile's hottest first, then the profile's other hot functions,
    /// then the data they touched. Linux on x86-64 alone, with `sudo -n` for
    /// the touch map. `--order-only` makes only the touch map and the order
    /// file, on the guided build the last refresh recorded.
    Refresh(refresh::Arguments),
    /// Runs the training workloads on the executables of the manifest,
    /// each instrumented one writing its profile where
    /// `LLVM_PROFILE_FILE` says: `refresh` runs it.
    #[command(hide = true)]
    Train(refresh::TrainArguments),
    /// Times a plain build and the profile-guided one, trial by trial in
    /// turn, on scenario 1 and on each Product's held-out workload, and
    /// prints each one's median wall time.
    Validate(validate::Arguments),
}

#[derive(Args)]
pub(crate) struct CheckArguments {
    /// The test tool manifest that names each Product's release executable.
    #[arg(long)]
    tools: PathBuf,
    /// The Rust target the executables were built for: the shipped one by
    /// default.
    #[arg(long)]
    target: Option<String>,
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Check(arguments) => {
            let target = arguments.target.clone().or_else(test_tools::shipped_target);
            Ok(check::report(&check_order(
                &environment.root,
                &environment.path(&arguments.tools),
                target.as_deref(),
            )?))
        }
        Command::Refresh(arguments) => refresh::refresh(&arguments, environment),
        Command::Train(arguments) => refresh::train(&arguments, environment),
        Command::Validate(arguments) => validate::validate(&arguments, environment),
    }
}

/// The folder of `target`'s committed profile and order file.
fn committed(root: &Path, target: &str) -> PathBuf {
    root.join(COMMITTED).join(target)
}

/// The text of a committed file, stored compressed with zstd as
/// `<path>.zst`, as a refresh writes it, or as is; none when neither is
/// there.
fn read_committed(path: &Path) -> Result<Option<String>> {
    let mut compressed = path.as_os_str().to_owned();
    compressed.push(".zst");
    let compressed = PathBuf::from(compressed);
    if compressed.is_file() {
        let output = std::process::Command::new("zstd")
            .args(["-q", "-d", "-c"])
            .arg(&compressed)
            .output()
            .with_context(|| format!("running zstd on {}", compressed.display()))?;
        if !output.status.success() {
            anyhow::bail!(
                "zstd could not read {}: {}",
                compressed.display(),
                String::from_utf8_lossy(&output.stderr).trim()
            );
        }
        return String::from_utf8(output.stdout)
            .map(Some)
            .with_context(|| format!("reading {}", compressed.display()));
    }
    if path.is_file() {
        return std::fs::read_to_string(path)
            .map(Some)
            .with_context(|| format!("reading {}", path.display()));
    }
    Ok(None)
}

/// The findings on the Products' executables the manifest at `tools` names:
/// none to make for a target with no committed order file, whose build is
/// not ordered.
fn check_order(root: &Path, tools: &Path, target: Option<&str>) -> Result<Vec<Finding>> {
    let Some(order) = target
        .map(|target| read_committed(&committed(root, target).join(ORDER)))
        .transpose()?
        .flatten()
    else {
        check::note(&[Finding::new(
            "not-ordered",
            target.unwrap_or("host"),
            "the target has no committed order file, so its release build is not ordered",
        )]);
        return Ok(Vec::new());
    };
    let order: Vec<String> = order.lines().map(str::to_owned).collect();
    let manifest = ToolManifest::read(tools)?;
    let mut findings = Vec::new();
    // Every symbol the Products' maps place, while each one has a map.
    let mut built = Some(HashSet::new());
    for tool in [Tool::Collie, Tool::Foxhound] {
        let executable = manifest.path(tool)?;
        let subject = executable.display().to_string();
        let Some(map) = link_map(executable)? else {
            findings.push(Finding::new(
                "no-link-map",
                subject,
                "its build wrote no link map beside it, so ci/musl/link-cc linked it without the order file",
            ));
            built = None;
            continue;
        };
        let text =
            std::fs::read_to_string(&map).with_context(|| format!("reading {}", map.display()))?;
        let placed = order::placed(&text);
        if let Some(built) = &mut built {
            built.extend(
                placed
                    .iter()
                    .flat_map(|section| section.symbols.iter().map(|(_, name)| name.clone())),
            );
        }
        let functions = order::text_functions(&placed);
        let Some(departure) = order::departure(&order, &functions) else {
            continue;
        };
        findings.push(match departure {
            order::Departure::NoneLinked => Finding::new(
                "none-linked",
                subject,
                "`.text` holds none of the order file's functions: the file is another build's",
            ),
            order::Departure::NotFirst { found, expected } => Finding::new(
                "not-first",
                subject,
                format!("`.text` starts with `{found}`, before the order file's first function `{expected}`"),
            ),
            order::Departure::OutOfOrder {
                index,
                function,
                before,
            } => Finding::new(
                "out-of-order",
                subject,
                format!("the order file's function {index}, `{function}`, lies before `{before}`, which the file lists ahead of it"),
            ),
        });
    }
    if let Some(built) = built {
        let built: HashSet<&str> = built.iter().map(String::as_str).collect();
        findings.extend(matched(&order, &built, target.unwrap_or("host")));
    }
    Ok(findings)
}

/// The findings on the order file against `built`, every symbol the
/// Products' builds hold: a crate the file names under an identity the
/// build links it under no more, and too low a share of the file's
/// symbols, which a note prints.
fn matched(order: &[String], built: &HashSet<&str>, target: &str) -> Vec<Finding> {
    let mut findings: Vec<Finding> = order::stale_crates(order, built)
        .into_iter()
        .map(|name| {
            Finding::new(
                "stale-crate",
                name,
                "the order file names the crate under an identity the build links it under no more, so the profile misses its code: its version changed, or the build did not compile through ci/musl/stable-metadata/rustc; run `just pgo-refresh`",
            )
        })
        .collect();
    let order::Share { matched, total } = order::match_share(order, built);
    let share = if total == 0 {
        100.0
    } else {
        #[expect(
            clippy::cast_precision_loss,
            reason = "a share of symbol counts, printed to a tenth of a percent"
        )]
        let share = matched as f64 * 100.0 / total as f64;
        share
    };
    let message =
        format!("{matched} of the order file's {total} symbols ({share:.1}%) are in the build");
    check::note(&[Finding::new("order-match", target, message.clone())]);
    if matched * 100 < total * LOWEST_MATCH {
        findings.push(Finding::new(
            "low-match",
            target,
            format!("{message}, below {LOWEST_MATCH}%: the profile and order file are another build's; run `just pgo-refresh`"),
        ));
    }
    findings
}

/// The link map of `executable`, a Cargo build's: the map ci/musl/link-cc
/// wrote beside the file the linker made in `deps/`, named after the
/// crate, which Cargo then
/// linked to `executable`'s path (one file, so its device and inode are
/// the same); `None` when there is none.
#[cfg(unix)]
fn link_map(executable: &Path) -> Result<Option<PathBuf>> {
    use std::os::unix::fs::MetadataExt as _;

    let metadata = std::fs::metadata(executable)
        .with_context(|| format!("reading {}", executable.display()))?;
    let (Some(name), Some(folder)) = (
        executable.file_name().and_then(|name| name.to_str()),
        executable.parent(),
    ) else {
        return Ok(None);
    };
    let deps = folder.join("deps");
    // Cargo names the file after the crate, whose `-` are `_`.
    let prefix = format!("{}-", name.replace('-', "_"));
    let entries = match std::fs::read_dir(&deps) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error).with_context(|| format!("reading {}", deps.display())),
    };
    for entry in entries {
        let entry = entry.with_context(|| format!("reading {}", deps.display()))?;
        let file_name = entry.file_name();
        let Some(file_name) = file_name.to_str() else {
            continue;
        };
        if !file_name.starts_with(&prefix) || file_name.contains('.') {
            continue;
        }
        let built = entry
            .metadata()
            .with_context(|| format!("reading {}", entry.path().display()))?;
        if built.dev() == metadata.dev() && built.ino() == metadata.ino() {
            let map = deps.join(format!("{file_name}.map"));
            return Ok(map.is_file().then_some(map));
        }
    }
    Ok(None)
}

/// Elsewhere no build is ordered, and no map is read.
#[cfg(not(unix))]
fn link_map(_executable: &Path) -> Result<Option<PathBuf>> {
    Ok(None)
}
