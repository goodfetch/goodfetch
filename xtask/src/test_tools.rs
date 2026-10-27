//! `xtask test-tools` (ADRs 0062 and 0094): builds the commit's executables
//! that tests run, each one the workspace has, with the tests' own Cargo
//! profile and in one invocation of their own, so both Products' executables
//! share one resolution of features; then writes the test tool manifest of
//! their absolute paths, which `goodfetch-test-support`'s `tools()` and
//! `goodfetch-test-env` read. It is not the test build's invocation: there,
//! the dev-dependencies' features (test support's Tokio `test-util` among
//! them) would reach the executables, which tests then would not run as they
//! ship, so the crates whose features differ compile twice per suite (ADR
//! 0094's note).
//! Each Product's executable embeds its Console build, which it builds
//! first (`xtask console build`, ADR 0057). `just test` runs it before any
//! runner starts.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::console;
use crate::environment::Environment;
use crate::metadata;

/// Each executable a test may run: its binary's name and its manifest entry.
const TOOLS: [(&str, &str); 7] = [
    ("goodfetch-collie", "collie"),
    ("goodfetch-foxhound", "foxhound"),
    ("goodfetch-proving-ground", "proving_ground"),
    ("goodfetch-test-env", "test_env"),
    ("goodfetch-scripted-engine", "scripted_engine"),
    ("goodfetch-conformance", "conformance"),
    ("goodfetch-stub-gateway", "stub_gateway"),
];

/// `xtask test-tools`'s arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The Cargo profile the tests build with.
    #[arg(long)]
    cargo_profile: String,
    /// The Rust target the tests build for, when they build for one.
    #[arg(long)]
    target: Option<String>,
    /// Where to write the manifest.
    #[arg(long)]
    out: PathBuf,
}

/// The one message of Cargo's JSON output this command reads.
#[derive(Deserialize)]
struct Message {
    reason: String,
    target: Option<Target>,
    executable: Option<PathBuf>,
}

#[derive(Deserialize)]
struct Target {
    name: String,
    kind: Vec<String>,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let (cargo, root) = (environment.cargo.as_os_str(), environment.root.as_path());
    let mut binaries = Vec::new();
    let mut features = std::collections::BTreeSet::new();
    for package in metadata::packages(cargo, root)? {
        for target in package.targets {
            if !target.kind.iter().any(|kind| kind == "bin")
                || !TOOLS.iter().any(|(binary, _)| *binary == target.name)
            {
                continue;
            }
            // A Product's binary target requires its `executable` feature
            // (ADR 0094). Named alone, a feature applies to each selected
            // package that has it; `<package>/<feature>` would name the
            // package's dev-dependency on itself, which a build leaves off.
            features.extend(target.required_features);
            binaries.push(target.name);
        }
    }
    // Each Product's executable embeds its Console build, which this
    // workspace's Console packages make first (ADR 0057).
    if binaries
        .iter()
        .any(|binary| binary == "goodfetch-collie" || binary == "goodfetch-foxhound")
        && root.join(console::PACKAGE).join("package.json").is_file()
    {
        console::build(root)?;
    }
    let mut manifest = BTreeMap::new();
    if !binaries.is_empty() {
        let mut command = Command::new(cargo);
        command
            .args(["build", "--message-format", "json-render-diagnostics"])
            .args(["--profile", &arguments.cargo_profile])
            .args(
                arguments
                    .target
                    .iter()
                    .flat_map(|target| ["--target", target]),
            )
            .args(binaries.iter().flat_map(|binary| ["--bin", binary]))
            .args(
                (!features.is_empty())
                    .then(|| {
                        let names: Vec<String> = features.into_iter().collect();
                        ["--features".to_owned(), names.join(",")]
                    })
                    .into_iter()
                    .flatten(),
            )
            .current_dir(root)
            .stderr(Stdio::inherit());
        let output = command.output().context("running cargo build")?;
        if !output.status.success() {
            bail!("cargo build of the test tools failed");
        }
        for line in String::from_utf8_lossy(&output.stdout).lines() {
            let Ok(message) = serde_json::from_str::<Message>(line) else {
                continue;
            };
            let (Some(target), Some(executable)) = (message.target, message.executable) else {
                continue;
            };
            if message.reason != "compiler-artifact"
                || !target.kind.iter().any(|kind| kind == "bin")
            {
                continue;
            }
            if let Some((_, entry)) = TOOLS.iter().find(|(binary, _)| *binary == target.name) {
                manifest.insert(*entry, executable);
            }
        }
    }
    for binary in &binaries {
        let entry = TOOLS
            .iter()
            .find(|(name, _)| name == binary)
            .map(|(_, entry)| *entry)
            .expect("invariant: every binary is one of TOOLS");
        if !manifest.contains_key(entry) {
            bail!("cargo build reported no executable for {binary}");
        }
    }
    let out = environment.path(&arguments.out);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    let mut text = serde_json::to_string_pretty(&manifest)?;
    text.push('\n');
    std::fs::write(&out, text).with_context(|| format!("writing {}", out.display()))?;
    Ok(ExitCode::SUCCESS)
}
