//! The Console's build (ADRs 0057 and 0068): `build` runs Collie's Console
//! package's `build` script into a folder of its own, then compresses every
//! file it wrote with brotli into `crates/goodfetch-collie/console-build/`,
//! which holds brotli files only and which Collie's executable embeds and
//! serves as they are. Building Collie's executable without it fails with
//! an error naming this command. A build whose inputs (the package's files
//! and the pnpm lockfile) did not change since the last one runs nothing.
//! The package's build reads only what is installed: it calls nowhere (ADR
//! 0043).
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result, bail};
use sha2::{Digest as _, Sha256};

use crate::environment::Environment;
use crate::output;

/// Collie's Console package, from the root.
pub(crate) const PACKAGE: &str = "console/collie";
/// The folder Collie's executable embeds, from the root.
pub(crate) const BUILD: &str = "crates/goodfetch-collie/console-build";
/// The digest of the inputs of the last build, from the root.
const INPUTS: &str = "target/console/collie.inputs";
/// What, beside the package, the build reads.
const LOCKFILE: &str = "pnpm-lock.yaml";
/// What changes the output without changing an input: this command's own
/// compression.
const COMPRESSION: &str = "brotli quality 11, window 22";
/// The page every build has.
const PAGE: &str = "index.html";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Builds Collie's Console into `crates/goodfetch-collie/console-build/`
    /// as brotli files only, unless its inputs did not change since the
    /// last build.
    Build,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Build => {
            build(&environment.root)?;
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Builds Collie's Console in the workspace at `root`, unless the build
/// there is of the same inputs.
pub(crate) fn build(root: &Path) -> Result<()> {
    let digest = inputs(root)?;
    let stamp = root.join(INPUTS);
    let out = root.join(BUILD);
    if out.join(format!("{PAGE}.br")).is_file()
        && std::fs::read_to_string(&stamp).is_ok_and(|kept| kept == digest)
    {
        output::line("Collie's Console build is up to date");
        return Ok(());
    }
    let target = root.join("target");
    std::fs::create_dir_all(&target).with_context(|| format!("creating {}", target.display()))?;
    let staging = tempfile::tempdir_in(&target).context("creating the build's own folder")?;
    let built = staging.path().join("built");
    let status = Process::new("pnpm")
        .arg("--dir")
        .arg(root.join(PACKAGE))
        .args(["run", "build", "--outDir"])
        .arg(&built)
        .status()
        .context("running pnpm; is it installed?")?;
    if !status.success() {
        bail!(
            "the build of {PACKAGE} failed ({status}); run `pnpm install --frozen-lockfile` (`just fetch`) first if its packages are not installed"
        );
    }
    if !built.join(PAGE).is_file() {
        bail!("the build of {PACKAGE} wrote no {PAGE}");
    }
    // The compressed files are staged beside the build folder, not under
    // `target`, so that the rename crosses no volume: the Windows cell's
    // `target` is a junction to its Dev Drive, another volume.
    let parent = out
        .parent()
        .context("the Console's build folder has a parent")?;
    std::fs::create_dir_all(parent).with_context(|| format!("creating {}", parent.display()))?;
    let beside = tempfile::Builder::new()
        .prefix(".console-build-")
        .tempdir_in(parent)
        .with_context(|| format!("creating a folder in {}", parent.display()))?;
    let compressed = beside.path().join("compressed");
    compress_tree(&built, &compressed)?;
    if out.exists() {
        std::fs::remove_dir_all(&out).with_context(|| format!("removing {}", out.display()))?;
    }
    std::fs::rename(&compressed, &out).with_context(|| format!("writing {}", out.display()))?;
    if let Some(folder) = stamp.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(&stamp, &digest).with_context(|| format!("writing {}", stamp.display()))?;
    output::line(&format!("Built Collie's Console into {BUILD}"));
    Ok(())
}

/// Gives the workspace checked out at `tree` the Console build of the one
/// at `root` when its Collie executable embeds one and it has none, as the
/// base's tree of `xtask perf build` lacks it: no benchmark reads its
/// bytes, which only a Daemon's Console serves.
pub(crate) fn lend_build(root: &Path, tree: &Path) -> Result<()> {
    let (from, to) = (root.join(BUILD), tree.join(BUILD));
    if !from.is_dir() || to.exists() || !tree.join("crates/goodfetch-collie/build.rs").is_file() {
        return Ok(());
    }
    copy_tree(&from, &to)
}

fn copy_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
    for entry in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry.with_context(|| format!("reading {}", from.display()))?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target)?;
        } else {
            std::fs::copy(entry.path(), &target)
                .with_context(|| format!("copying {}", entry.path().display()))?;
        }
    }
    Ok(())
}

/// Writes each file under `from` into `to` at the same path, brotli
/// compressed, with `.br` added to its name.
fn compress_tree(from: &Path, to: &Path) -> Result<()> {
    std::fs::create_dir_all(to).with_context(|| format!("creating {}", to.display()))?;
    for entry in std::fs::read_dir(from).with_context(|| format!("reading {}", from.display()))? {
        let entry = entry.with_context(|| format!("reading {}", from.display()))?;
        let path = entry.path();
        if entry.file_type()?.is_dir() {
            compress_tree(&path, &to.join(entry.file_name()))?;
            continue;
        }
        let bytes = std::fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        let mut name = entry.file_name();
        name.push(".br");
        let file = std::fs::File::create(to.join(&name))
            .with_context(|| format!("creating {}", to.join(&name).display()))?;
        let mut writer = brotli::CompressorWriter::new(file, 4096, 11, 22);
        writer
            .write_all(&bytes)
            .and_then(|()| writer.flush())
            .with_context(|| format!("compressing {}", path.display()))?;
    }
    Ok(())
}

/// The digest of everything the build reads: each file of the package but
/// its installed packages, by its path, and the pnpm lockfile.
fn inputs(root: &Path) -> Result<String> {
    let mut files = Vec::new();
    collect(&root.join(PACKAGE), &mut files)?;
    files.sort();
    let lockfile = root.join(LOCKFILE);
    if lockfile.is_file() {
        files.push(lockfile);
    }
    let mut digest = Sha256::new();
    digest.update(COMPRESSION.as_bytes());
    for file in files {
        let relative = file.strip_prefix(root).unwrap_or(&file);
        let bytes = std::fs::read(&file).with_context(|| format!("reading {}", file.display()))?;
        digest.update(relative.to_string_lossy().as_bytes());
        digest.update([0]);
        digest.update(u64::try_from(bytes.len()).unwrap_or(u64::MAX).to_le_bytes());
        digest.update(&bytes);
    }
    Ok(format!("{:x}", digest.finalize()))
}

fn collect(folder: &Path, files: &mut Vec<PathBuf>) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let entry = entry.with_context(|| format!("reading {}", folder.display()))?;
        if entry.file_type()?.is_dir() {
            if entry.file_name() != "node_modules" {
                collect(&entry.path(), files)?;
            }
        } else {
            files.push(entry.path());
        }
    }
    Ok(())
}
