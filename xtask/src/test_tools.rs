//! `xtask test-tools` (ADRs 0062 and 0094): `build` builds the commit's
//! executables that tests run, each one the workspace has, with the tests'
//! own Cargo profile and in one invocation of their own, so both Products'
//! executables share one resolution of features; then writes the test tool
//! manifest of their absolute paths, which `goodfetch-test-support`'s
//! `tools()` and `goodfetch-test-env` read. It is not the test build's
//! invocation: there, the dev-dependencies' features (test support's Tokio `test-util` among
//! them) would reach the executables, which tests then would not run as they
//! ship, so the crates whose features differ compile twice per suite (ADR
//! 0094's note).
//! Each Product's executable embeds its Console build, which it builds
//! first (`xtask console build`, ADR 0057). `just test` runs it before any
//! runner starts. With `--shipped-target`, it builds for the Rust target
//! the Products ship for on this host, the static musl one on Linux (ADR
//! 0024), so `just targets` and `just rss` measure what users run. A
//! `*-linux-musl` build runs through [`MUSL_CARGO`], the static musl
//! toolchain's Cargo (ADR 0024's note of 2026-11-06), as the Release suite's
//! musl cells build. [`ToolManifest`] is the manifest's one reader in
//! `xtask`, and `path` reads it for a script.
//!
//! Beside the manifest, `build` writes its input record,
//! `<manifest>.inputs.json`: the workspace's folder and the size and
//! modification time of each source the tools were built from, which
//! `goodfetch-test-support` compares with the tree before a test runs a
//! tool, so tools built before a merge or from another tree are refused
//! (issue 1631). The sources are those Cargo's dep-info file of each
//! executable names inside the workspace (Rust sources, included files and
//! what build scripts watch), with `Cargo.lock`, the toolchain and Cargo
//! configuration files and the manifests of the packages they are in. Git's
//! own files and the target folder are left out: a commit or another
//! worktree's fetch changes no source.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::process::{ExitCode, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context as _, Result, bail};
use goodfetch_protocol::catalog::Product;
use serde::{Deserialize, Serialize};

use crate::console;
use crate::environment::Environment;
use crate::metadata;
use crate::output;
use crate::products;

/// An executable a test may run, named as its entry in the test tool
/// manifest.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, clap::ValueEnum,
)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "snake_case")]
pub(crate) enum Tool {
    Collie,
    Foxhound,
    ProvingGround,
    TestEnv,
    ScriptedEngine,
    Conformance,
    StubGateway,
}

impl Tool {
    const ALL: [Self; 7] = [
        Self::Collie,
        Self::Foxhound,
        Self::ProvingGround,
        Self::TestEnv,
        Self::ScriptedEngine,
        Self::Conformance,
        Self::StubGateway,
    ];

    /// Its binary's name.
    fn binary(self) -> &'static str {
        match self {
            Self::Collie => products::COLLIE.package,
            Self::Foxhound => products::FOXHOUND.package,
            Self::ProvingGround => "goodfetch-proving-ground",
            Self::TestEnv => "goodfetch-test-env",
            Self::ScriptedEngine => "goodfetch-scripted-engine",
            Self::Conformance => "goodfetch-conformance",
            Self::StubGateway => "goodfetch-stub-gateway",
        }
    }

    /// `product`'s executable.
    pub(crate) fn of_product(product: Product) -> Self {
        match product {
            Product::Collie => Self::Collie,
            Product::Foxhound => Self::Foxhound,
        }
    }

    /// The tool whose binary is `name`.
    fn of_binary(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|tool| tool.binary() == name)
    }

    /// Its entry's name in the manifest.
    fn entry(self) -> String {
        serde_json::to_value(self)
            .ok()
            .and_then(|value| value.as_str().map(str::to_owned))
            .expect("invariant: a tool serializes as its entry's name")
    }
}

/// The test tool manifest: each tool's absolute path. An entry no [`Tool`]
/// names fails to parse, so a renamed entry is an error at once, never a
/// tool the manifest seems to lack.
#[derive(Default, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct ToolManifest(BTreeMap<Tool, PathBuf>);

impl ToolManifest {
    /// The manifest at `path`.
    pub(crate) fn read(path: &Path) -> Result<Self> {
        let text = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
        serde_json::from_slice(&text).with_context(|| format!("parsing {}", path.display()))
    }

    /// `tool`'s executable, when the manifest names one.
    pub(crate) fn get(&self, tool: Tool) -> Option<&Path> {
        self.0.get(&tool).map(PathBuf::as_path)
    }

    /// `tool`'s executable, which a command that runs it needs.
    pub(crate) fn path(&self, tool: Tool) -> Result<&Path> {
        self.get(tool).with_context(|| {
            format!(
                "the test tool manifest names no executable of {}",
                tool.entry()
            )
        })
    }
}

/// A manifest's input record: what its tools were built from.
/// `goodfetch-test-support`'s `tools()` is its reader.
#[derive(Serialize)]
struct InputRecord {
    /// The workspace's folder.
    root: PathBuf,
    /// Each source of the tools, as it was when they were built.
    inputs: Vec<Input>,
}

/// One source: its path from the workspace's folder, its size in bytes and
/// its modification time in nanoseconds since the Unix epoch.
#[derive(Debug, PartialEq, Eq, Serialize)]
struct Input(PathBuf, u64, u64);

/// Cargo's lockfile, at the workspace's top.
const LOCKFILE: &str = "Cargo.lock";

/// The files at the workspace's top that every build reads.
const BUILD_FILES: [&str; 4] = [
    "Cargo.toml",
    LOCKFILE,
    "rust-toolchain.toml",
    ".cargo/config.toml",
];

/// Where the input record of the manifest at `manifest` goes.
fn record_path(manifest: &Path) -> PathBuf {
    manifest.with_extension("inputs.json")
}

/// The files a Cargo dep-info file's `text` names as its target's
/// prerequisites; a space in a path is written `\ `.
fn dep_info_inputs(text: &str) -> Vec<PathBuf> {
    let mut inputs = Vec::new();
    for line in text.lines() {
        let Some((_, prerequisites)) = line.split_once(": ") else {
            continue;
        };
        let mut path = String::new();
        let mut characters = prerequisites.chars().peekable();
        while let Some(character) = characters.next() {
            match character {
                '\\' if characters.peek() == Some(&' ') => {
                    path.push(' ');
                    characters.next();
                }
                ' ' if !path.is_empty() => inputs.push(PathBuf::from(std::mem::take(&mut path))),
                ' ' => {}
                other => path.push(other),
            }
        }
        if !path.is_empty() {
            inputs.push(PathBuf::from(path));
        }
    }
    inputs
}

/// The workspace's sources among `prerequisites`, from `root`: those inside
/// it that are neither Git's own files nor in `target_directory`, with the
/// workspace's [`BUILD_FILES`] and the manifest of each of `packages` that
/// holds one.
fn sources(
    prerequisites: &[PathBuf],
    root: &Path,
    target_directory: &Path,
    packages: &[PathBuf],
) -> BTreeSet<PathBuf> {
    let mut sources: BTreeSet<PathBuf> = prerequisites
        .iter()
        .filter(|path| !path.starts_with(target_directory))
        .filter(|path| !path.components().any(|part| part.as_os_str() == ".git"))
        .filter_map(|path| path.strip_prefix(root).ok())
        .map(Path::to_owned)
        .collect();
    let manifests: Vec<PathBuf> = packages
        .iter()
        .filter_map(|manifest| manifest.strip_prefix(root).ok())
        .filter(|manifest| {
            manifest
                .parent()
                .is_some_and(|folder| sources.iter().any(|source| source.starts_with(folder)))
        })
        .map(Path::to_owned)
        .collect();
    sources.extend(manifests);
    if !sources.is_empty() {
        sources.extend(
            BUILD_FILES
                .iter()
                .map(PathBuf::from)
                .filter(|file| root.join(file).exists()),
        );
    }
    sources
}

/// Each of `sources`, from `root`, with its size and modification time. A
/// source modified after `started`, when the build began, fails: Cargo may
/// have compiled it as it was before, and the record would call the tools
/// built from it as it is now. The lockfile alone may be newer.
fn stamps(sources: BTreeSet<PathBuf>, root: &Path, started: SystemTime) -> Result<Vec<Input>> {
    sources
        .into_iter()
        .map(|source| {
            let path = root.join(&source);
            let metadata =
                std::fs::metadata(&path).with_context(|| format!("reading {}", path.display()))?;
            let modified = metadata
                .modified()
                .with_context(|| format!("reading {}'s modification time", path.display()))?;
            // Cargo itself writes the lockfile when a build resolves anew,
            // and builds from what it wrote.
            if modified > started && source != Path::new(LOCKFILE) {
                bail!(
                    "{} changed while the test tools were building: build them again",
                    source.display()
                );
            }
            let nanoseconds = modified
                .duration_since(UNIX_EPOCH)
                .ok()
                .and_then(|since| u64::try_from(since.as_nanos()).ok())
                .with_context(|| {
                    format!("{}'s modification time is out of range", path.display())
                })?;
            Ok(Input(source, metadata.len(), nanoseconds))
        })
        .collect()
}

/// The tools among the executables a Cargo build reports in `output`, its
/// JSON messages.
fn built_tools(output: &[u8]) -> Result<ToolManifest> {
    let mut built = ToolManifest::default();
    for message in crate::child_cargo::messages(output)? {
        let (Some(target), Some(executable)) = (message.target, message.executable) else {
            continue;
        };
        if message.reason != "compiler-artifact" || !target.kind.iter().any(|kind| kind == "bin") {
            continue;
        }
        if let Some(tool) = Tool::of_binary(&target.name) {
            built.0.insert(tool, executable);
        }
    }
    Ok(built)
}

/// What the executables `built` names were built from, in the workspace at
/// `folder`, an absolute path: the [`sources`] among the prerequisites of
/// each one's dep-info file, which Cargo writes beside it, with their
/// [`stamps`].
fn built_inputs(
    built: &ToolManifest,
    folder: &Path,
    target_directory: &Path,
    package_manifests: &[PathBuf],
    started: SystemTime,
) -> Result<Vec<Input>> {
    let mut prerequisites = Vec::new();
    for (tool, executable) in &built.0 {
        let dep_info = executable.with_extension("d");
        let text = std::fs::read_to_string(&dep_info)
            .with_context(|| format!("reading {}", dep_info.display()))?;
        let named = dep_info_inputs(&text);
        // A record that left a tool's sources out would pass tools of any
        // age.
        if !named.iter().any(|path| path.starts_with(folder)) {
            bail!(
                "{} names no source of {} inside {}",
                dep_info.display(),
                tool.binary(),
                folder.display()
            );
        }
        prerequisites.extend(named);
    }
    let sources = sources(&prerequisites, folder, target_directory, package_manifests);
    stamps(sources, folder, started)
}

/// The Cargo of the static musl build, from the workspace's folder: Zig
/// through cargo-zigbuild at the versions `ci/musl/toolchain.sh` pins.
pub(crate) const MUSL_CARGO: &str = "ci/musl/cargo";

/// `xtask test-tools`'s commands.
#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Builds the commit's executables that tests run and writes the test
    /// tool manifest of their paths.
    Build(Arguments),
    /// Prints one tool's path in a test tool manifest, and fails when it
    /// names none: how a script reads the manifest.
    Path {
        /// The tool.
        tool: Tool,
        /// The test tool manifest.
        #[arg(long)]
        manifest: PathBuf,
    },
}

/// `xtask test-tools build`'s arguments.
#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The Cargo profile the tests build with.
    #[arg(long)]
    cargo_profile: String,
    /// The Rust target the tests build for, when they build for one.
    #[arg(long, conflicts_with = "shipped_target")]
    target: Option<String>,
    /// Builds for the Rust target the Products ship for on this host:
    /// `<arch>-unknown-linux-musl` on Linux (ADR 0024), the host's own
    /// elsewhere.
    #[arg(long)]
    shipped_target: bool,
    /// Builds only this tool, repeatable; every tool the workspace has by
    /// default.
    #[arg(long = "tool")]
    tools: Vec<Tool>,
    /// Names this executable as both Products' in the manifest, in place of
    /// the ones built: a skew run's engine (`GOODFETCH_TEST_ENGINE`).
    #[arg(long)]
    engine: Option<PathBuf>,
    /// Where to write the manifest.
    #[arg(long)]
    out: PathBuf,
}

/// The Rust target the Products ship for on `os` and `arch`, as
/// `std::env::consts` name them, when it is not the host's own build: on
/// Linux the static musl binary (ADR 0024); macOS and Windows ship the
/// host's own target.
pub(crate) fn shipped_target_of(os: &str, arch: &str) -> Option<String> {
    (os == "linux").then(|| format!("{arch}-unknown-linux-musl"))
}

/// The Rust target the Products ship for on this host, as
/// [`shipped_target_of`] gives it.
pub(crate) fn shipped_target() -> Option<String> {
    shipped_target_of(std::env::consts::OS, std::env::consts::ARCH)
}

/// The Cargo that builds for `target`: the static musl toolchain's for a
/// `*-linux-musl` one, else `cargo`.
fn build_cargo(target: Option<&str>, cargo: &OsStr, root: &Path) -> OsString {
    if target.is_some_and(|target| target.ends_with("-linux-musl")) {
        let script = root.join(MUSL_CARGO);
        std::path::absolute(&script)
            .unwrap_or(script)
            .into_os_string()
    } else {
        cargo.to_owned()
    }
}

pub(crate) fn run(command: Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Build(arguments) => build(&arguments, environment),
        Command::Path { tool, manifest } => {
            let manifest = ToolManifest::read(&environment.path(&manifest))?;
            output::line(&manifest.path(tool)?.display().to_string());
            Ok(ExitCode::SUCCESS)
        }
    }
}

fn build(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let target = if arguments.shipped_target {
        shipped_target()
    } else {
        arguments.target.clone()
    };
    let (cargo, root) = (environment.cargo.as_os_str(), environment.root.as_path());
    let mut binaries = Vec::new();
    let mut features = BTreeSet::new();
    let mut package_manifests = Vec::new();
    let (packages, target_directory) = metadata::packages_and_target_directory(cargo, root)?;
    for package in packages {
        package_manifests.push(package.manifest_path);
        for target in package.targets {
            if !target.kind.iter().any(|kind| kind == "bin") {
                continue;
            }
            let Some(tool) = Tool::of_binary(&target.name) else {
                continue;
            };
            if !arguments.tools.is_empty() && !arguments.tools.contains(&tool) {
                continue;
            }
            // A Product's binary target requires its `executable` feature
            // (ADR 0094). Named alone, a feature applies to each selected
            // package that has it; `<package>/<feature>` would name the
            // package's dev-dependency on itself, which a build leaves off.
            features.extend(target.required_features);
            binaries.push(tool);
        }
    }
    if let Some(missing) = arguments.tools.iter().find(|tool| !binaries.contains(tool)) {
        bail!(
            "the workspace has no executable {} for the test tool manifest's {}",
            missing.binary(),
            missing.entry()
        );
    }
    // Each Product's executable embeds its Console build, which this
    // workspace's Console packages make first (ADR 0057).
    if binaries
        .iter()
        .any(|tool| products::is_executable(tool.binary()))
        && root.join(console::PACKAGE).join("package.json").is_file()
    {
        console::build(root)?;
    }
    let mut manifest = ToolManifest::default();
    // Cargo names its prerequisites by absolute paths from this folder.
    let folder = std::path::absolute(root).context("finding the workspace's absolute path")?;
    let mut inputs = Vec::new();
    if !binaries.is_empty() {
        let started = SystemTime::now();
        let build = build_cargo(target.as_deref(), cargo, root);
        let mut command = crate::child_cargo::command(&build);
        command
            .args(["build", "--message-format", "json-render-diagnostics"])
            .args(["--profile", &arguments.cargo_profile])
            .args(target.iter().flat_map(|target| ["--target", target]))
            .args(binaries.iter().flat_map(|tool| ["--bin", tool.binary()]))
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
        manifest = built_tools(&output.stdout)?;
        inputs = built_inputs(
            &manifest,
            &folder,
            &target_directory,
            &package_manifests,
            started,
        )?;
    }
    for tool in &binaries {
        if !manifest.0.contains_key(tool) {
            bail!("cargo build reported no executable for {}", tool.binary());
        }
    }
    if let Some(engine) = &arguments.engine {
        let engine = std::path::absolute(environment.path(engine))
            .with_context(|| format!("finding {}'s absolute path", engine.display()))?;
        for product in [Tool::Collie, Tool::Foxhound] {
            manifest.0.insert(product, engine.clone());
        }
    }
    let record = InputRecord {
        root: folder,
        inputs,
    };
    write(&environment.path(&arguments.out), &manifest, &record)?;
    Ok(ExitCode::SUCCESS)
}

/// Writes `manifest` at `out` and `record`, its input record, beside it.
fn write(out: &Path, manifest: &ToolManifest, record: &InputRecord) -> Result<()> {
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("creating {}", parent.display()))?;
    }
    // The record first: a manifest never stands beside an older build's.
    let record_out = record_path(out);
    let mut text = serde_json::to_string(record)?;
    text.push('\n');
    std::fs::write(&record_out, text)
        .with_context(|| format!("writing {}", record_out.display()))?;
    let mut text = serde_json::to_string_pretty(manifest)?;
    text.push('\n');
    std::fs::write(out, text).with_context(|| format!("writing {}", out.display()))
}

#[cfg(test)]
mod tests;
