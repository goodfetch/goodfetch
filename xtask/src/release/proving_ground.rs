//! `xtask release proving-ground-digests`: fixes the Proving ground binary's
//! SHA-256 per target before the executables build, since `plugins test`
//! pins the Proving ground's release asset by digest (ADRs 0033, 0039 and
//! 0094; issue 141).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result};
use serde::Serialize;

use super::assets;
use crate::check::{self, Finding};
use crate::output;

/// The Proving ground executable's base name.
const EXECUTABLE: &str = "goodfetch-proving-ground";

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// Folder holding `<target>/goodfetch-proving-ground[.exe]` for each target.
    #[arg(long)]
    binaries: PathBuf,
    /// Where to write the JSON digest table read by the executables' build.
    #[arg(long)]
    out: PathBuf,
    /// Targets to require; defaults to all eight release targets.
    #[arg(long, value_delimiter = ',')]
    targets: Vec<String>,
}

#[derive(Serialize)]
struct DigestTable {
    targets: BTreeMap<String, TargetDigest>,
}

#[derive(Serialize)]
struct TargetDigest {
    binary: String,
    sha256: String,
}

fn binary_name(target: &str) -> String {
    if target.contains("-windows-") {
        format!("{EXECUTABLE}.exe")
    } else {
        EXECUTABLE.to_owned()
    }
}

/// Computes and writes the Proving ground's digest table for every requested
/// target, or reports `error[missing]` for each absent target binary without
/// writing the output file.
pub(super) fn run(root: &Path, arguments: &Arguments) -> Result<ExitCode> {
    let targets: Vec<String> = if arguments.targets.is_empty() {
        assets::TARGETS.iter().map(|t| (*t).to_owned()).collect()
    } else {
        arguments.targets.clone()
    };
    let binaries_root = root.join(&arguments.binaries);
    let mut missing = Vec::new();
    let mut entries = BTreeMap::new();

    for target in &targets {
        let name = binary_name(target);
        let path = binaries_root.join(target).join(&name);
        if !path.is_file() {
            missing.push(Finding::new(
                "missing",
                target,
                format!(
                    "{} is not built yet: build and sign the Proving ground before the executables",
                    path.display()
                ),
            ));
            continue;
        }
        let digest = assets::sha256_file(&path)?;
        entries.insert(
            target.clone(),
            TargetDigest {
                binary: name,
                sha256: digest,
            },
        );
    }

    if !missing.is_empty() {
        return Ok(check::report(&missing));
    }

    let out_path = root.join(&arguments.out);
    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let body = serde_json::to_string_pretty(&DigestTable { targets: entries })
        .context("serializing proving-ground digest table")?;
    std::fs::write(&out_path, format!("{body}\n"))
        .with_context(|| format!("writing {}", out_path.display()))?;
    output::line(&arguments.out.display().to_string());
    Ok(ExitCode::SUCCESS)
}
