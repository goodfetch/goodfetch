//! `xtask pgo refresh` and the `train` step it runs: the committed profile
//! and order file of the shipped Linux target, made again from the training
//! workloads (ADR 0057's note of 2026-11-14).
#![cfg_attr(
    not(target_os = "linux"),
    expect(
        unused_imports,
        dead_code,
        reason = "the touch map, and so the refresh, needs Linux"
    )
)]

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};

use anyhow::{Context as _, Result, bail};
use clap::Args;

use super::{ORDER, PROFILE, committed, order};
use crate::compare::training::{self, Executables};
use crate::environment::Environment;
use crate::output;
use crate::test_tools::{self, Tool, ToolManifest};

#[derive(Args)]
pub(crate) struct Arguments {
    /// The folder the refresh works in: its builds, raw profiles and
    /// manifests.
    #[arg(long, default_value = "target/pgo")]
    work: PathBuf,
}

#[derive(Args)]
pub(crate) struct TrainArguments {
    /// The manifest of the instrumented Products' executables.
    #[arg(long)]
    instrumented: PathBuf,
    /// The manifest that names the Proving ground.
    #[arg(long)]
    proving_ground: PathBuf,
    /// The folder `LLVM_PROFILE_FILE` writes the raw profiles in: each
    /// workload must add one.
    #[arg(long)]
    raw: PathBuf,
}

/// The Products' executables, which the profile is trained on.
const PRODUCTS: [&str; 4] = ["--tool", "collie", "--tool", "foxhound"];

/// The instrumented build's target folder in the work folder.
const INSTRUMENTED_TARGET: &str = "instrumented-target";

#[cfg(not(target_os = "linux"))]
pub(crate) fn refresh(_arguments: &Arguments, _environment: &Environment) -> Result<ExitCode> {
    bail!("`xtask pgo refresh` runs on Linux, whose musl build is profile-guided")
}

#[cfg(target_os = "linux")]
pub(crate) fn refresh(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let Some(target) = test_tools::shipped_target() else {
        bail!("this host ships no musl build to guide");
    };
    let work = std::path::absolute(environment.path(&arguments.work))
        .context("finding the work folder's absolute path")?;
    let raw = work.join("raw");
    let guided = work.join("guided");
    for folder in [&raw, &guided] {
        if folder.exists() {
            std::fs::remove_dir_all(folder)
                .with_context(|| format!("removing {}", folder.display()))?;
        }
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    let proving_ground = work.join("proving-ground.json");
    let hot = profile(environment, &work, &proving_ground)?;
    let profile = guided.join(PROFILE);

    output::line("pgo: building the Products guided by the profile");
    let guided_tools = work.join("guided.json");
    build(
        environment,
        &guided_tools,
        &PRODUCTS,
        &[(
            "GOODFETCH_PGO",
            format!("use={}", guided.display()).as_ref(),
        )],
    )?;
    let guided_tools = ToolManifest::read(&guided_tools)?;
    let foxhound = guided_tools.path(Tool::Foxhound)?;
    let Some(map) = super::link_map(foxhound)? else {
        bail!(
            "the guided build of {} wrote no link map",
            foxhound.display()
        );
    };
    let placed = order::placed(
        &std::fs::read_to_string(&map).with_context(|| format!("reading {}", map.display()))?,
    );

    output::line("pgo: mapping the pages a crawl touches");
    let proving_ground = ToolManifest::read(&proving_ground)?;
    let browser = training::browser()?;
    let driver = training::collie_driver(&environment.root)?;
    let touched = super::touch::touched_pages(&Executables {
        proving_ground: proving_ground.path(Tool::ProvingGround)?,
        foxhound,
        collie: guided_tools.path(Tool::Collie)?,
        browser: &browser,
        collie_driver: &driver,
        goodfetch_variables: &environment.goodfetch_variables,
    })?;
    let order = order::order_file(
        &hot,
        &order::touched_sections(&placed, &touched, super::touch::PAGE),
    );

    let committed = committed(&environment.root, &target);
    std::fs::create_dir_all(&committed)
        .with_context(|| format!("creating {}", committed.display()))?;
    let compressed = committed.join(format!("{PROFILE}.zst"));
    run(
        Command::new("zstd")
            .args(["-q", "-19", "-f", "-o"])
            .arg(&compressed)
            .arg(&profile),
        "zstd",
    )?;
    let plain = committed.join(PROFILE);
    if plain.exists() {
        std::fs::remove_file(&plain).with_context(|| format!("removing {}", plain.display()))?;
    }
    write_lines(&committed.join(ORDER), &order)?;
    // The instrumented build is the largest; the next refresh builds it
    // again from sccache.
    let instrumented_target = work.join(INSTRUMENTED_TARGET);
    std::fs::remove_dir_all(&instrumented_target)
        .with_context(|| format!("removing {}", instrumented_target.display()))?;
    output::line(&format!(
        "pgo: {} profiled functions; the crawls touched {:.1} MiB of Foxhound's image; {} entries ordered in {}",
        hot.len(),
        super::touch::mebibytes(&touched),
        order.len(),
        committed.join(ORDER).display()
    ));
    Ok(ExitCode::SUCCESS)
}

/// The instrumented build's profile of the training workloads, merged into
/// `<work>/guided`, and its functions hottest first, which that folder's
/// order file lists; the Proving ground's manifest written to
/// `proving_ground`.
#[cfg(target_os = "linux")]
fn profile(environment: &Environment, work: &Path, proving_ground: &Path) -> Result<Vec<String>> {
    let (raw, guided) = (work.join("raw"), work.join("guided"));
    let instrumented_target = work.join(INSTRUMENTED_TARGET);

    output::line("pgo: building the instrumented Products");
    let instrumented = work.join("instrumented.json");
    build(
        environment,
        &instrumented,
        &PRODUCTS,
        &[
            ("CARGO_TARGET_DIR", instrumented_target.as_os_str()),
            (
                "GOODFETCH_PGO",
                format!("generate={}", raw.display()).as_ref(),
            ),
        ],
    )?;
    // As `just targets` and `just compare` build it.
    build(
        environment,
        proving_ground,
        &["--tool", "proving_ground"],
        &[],
    )?;

    output::line("pgo: training");
    let status = Command::new(std::env::current_exe().context("finding xtask's own executable")?)
        .args(["pgo", "train", "--instrumented"])
        .arg(&instrumented)
        .arg("--proving-ground")
        .arg(proving_ground)
        .arg("--raw")
        .arg(&raw)
        .env("LLVM_PROFILE_FILE", raw.join("%p-%m.profraw"))
        .status()
        .context("running xtask pgo train")?;
    if !status.success() {
        bail!("the training failed");
    }

    output::line("pgo: merging the profiles");
    let profile = guided.join(PROFILE);
    let mut merge = llvm_tool("llvm-profdata")?;
    merge.args(["merge", "--output"]).arg(&profile);
    merge.args(profraws(&raw)?);
    run(&mut merge, "llvm-profdata merge")?;
    let shown = llvm_tool("llvm-profdata")?
        .args(["show", "--all-functions", "--counts"])
        .arg(&profile)
        .stderr(Stdio::inherit())
        .output()
        .context("running llvm-profdata show")?;
    if !shown.status.success() {
        bail!("llvm-profdata show failed");
    }
    let hot = order::hot_functions(&String::from_utf8_lossy(&shown.stdout));
    write_lines(&guided.join(ORDER), &hot)?;
    Ok(hot)
}

/// Runs every training workload once on the instrumented executables,
/// failing on one that wrote no raw profile.
pub(crate) fn train(arguments: &TrainArguments, environment: &Environment) -> Result<ExitCode> {
    let instrumented = ToolManifest::read(&environment.path(&arguments.instrumented))?;
    let proving_ground = ToolManifest::read(&environment.path(&arguments.proving_ground))?;
    let raw = environment.path(&arguments.raw);
    let browser = training::browser()?;
    let driver = training::collie_driver(&environment.root)?;
    let executables = Executables {
        proving_ground: proving_ground.path(Tool::ProvingGround)?,
        foxhound: instrumented.path(Tool::Foxhound)?,
        collie: instrumented.path(Tool::Collie)?,
        browser: &browser,
        collie_driver: &driver,
        goodfetch_variables: &environment.goodfetch_variables,
    };
    for workload in training::TRAINING {
        let before = profraws(&raw)?.len();
        let seconds = workload
            .run(&executables)
            .with_context(|| format!("running {}", workload.name()))?;
        if profraws(&raw)?.len() == before {
            bail!(
                "{} wrote no raw profile in {}: is the executable instrumented?",
                workload.name(),
                raw.display()
            );
        }
        output::line(&format!("pgo: {} in {seconds:.2} s", workload.name()));
    }
    Ok(ExitCode::SUCCESS)
}

/// Builds `tools` for the shipped target with the release profile, the
/// manifest written to `out`, Cargo's environment changed by `variables`.
#[cfg(target_os = "linux")]
fn build(
    environment: &Environment,
    out: &Path,
    tools: &[&str],
    variables: &[(&str, &std::ffi::OsStr)],
) -> Result<()> {
    let mut command =
        Command::new(std::env::current_exe().context("finding xtask's own executable")?);
    command
        .args(["test-tools", "build", "--cargo-profile", "release"])
        .args(["--shipped-target", "--out"])
        .arg(out)
        .args(tools)
        .current_dir(&environment.root)
        .env_remove("GOODFETCH_PGO");
    for (name, value) in variables {
        command.env(name, value);
    }
    run(&mut command, "xtask test-tools build")
}

/// The raw profiles in `folder`.
fn profraws(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut found = Vec::new();
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let path = entry
            .with_context(|| format!("reading {}", folder.display()))?
            .path();
        if path
            .extension()
            .is_some_and(|extension| extension == "profraw")
        {
            found.push(path);
        }
    }
    found.sort();
    Ok(found)
}

/// An LLVM tool of rustup's `llvm-tools` component (rust-toolchain.toml).
#[cfg(target_os = "linux")]
fn llvm_tool(name: &str) -> Result<Command> {
    let print = |what: &str| -> Result<String> {
        let output = Command::new("rustc")
            .args(["--print", what])
            .output()
            .with_context(|| format!("running rustc --print {what}"))?;
        if !output.status.success() {
            bail!("rustc --print {what} failed");
        }
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_owned())
    };
    let tool = Path::new(&print("sysroot")?)
        .join("lib/rustlib")
        .join(print("host-tuple")?)
        .join("bin")
        .join(name);
    if !tool.is_file() {
        bail!(
            "{} is missing: rustup's llvm-tools component is",
            tool.display()
        );
    }
    Ok(Command::new(tool))
}

#[cfg(target_os = "linux")]
fn run(command: &mut Command, what: &str) -> Result<()> {
    let status = command
        .status()
        .with_context(|| format!("running {what}"))?;
    if !status.success() {
        bail!("{what} failed");
    }
    Ok(())
}

/// Writes `lines` to `path`, each ended by a newline.
#[cfg(target_os = "linux")]
fn write_lines(path: &Path, lines: &[String]) -> Result<()> {
    let mut text = lines.join("\n");
    text.push('\n');
    std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
}
