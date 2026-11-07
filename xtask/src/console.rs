//! The Console's builds (ADRs 0057, 0068 and 0087): `build` runs each
//! Product's Console package's `build` script into a folder of its own,
//! then compresses every file it wrote with brotli into the folder its
//! executable embeds, `crates/goodfetch-collie/console-build/` for Collie's
//! and `crates/foxhound/goodfetch-foxhound/console-build/` for Foxhound's,
//! which holds brotli files and, uncompressed, the build's `routes.json`,
//! at which the executable serves the page. Foxhound's build composes
//! Collie's screens with its own, so its routes are Collie's and then its
//! own, and Collie's package is among its inputs. Building an executable
//! without its build fails with an error naming this command. A build whose
//! inputs (its packages' files and the pnpm lockfile) did not change since
//! its last one runs nothing. A package's build reads only what is
//! installed: it calls nowhere (ADR 0043).
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
use crate::products;

/// Collie's Console package, from the root.
pub(crate) const PACKAGE: &str = "console/collie";
/// The folder Collie's executable embeds, from the root.
pub(crate) const BUILD: &str = concat!(products::collie_folder!(), "/console-build");
/// Foxhound's Console package, `@goodfetch/console-foxhound`, from the root.
pub(crate) const FOXHOUND_PACKAGE: &str = "console/foxhound";
/// The folder Foxhound's executable embeds, from the root.
pub(crate) const FOXHOUND_BUILD: &str = concat!(products::foxhound_folder!(), "/console-build");
/// What, beside the packages, a build reads.
const LOCKFILE: &str = "pnpm-lock.yaml";
/// What changes the output without changing an input: this command's own
/// compression.
const COMPRESSION: &str = "brotli quality 11, window 22";
/// The page every build has.
const PAGE: &str = "index.html";
/// The routes of a package's screens (ADR 0072), which the build folder
/// keeps uncompressed, for the executable to serve the page at.
const ROUTES: &str = "routes.json";
/// pnpm's program. On Windows, npm installs it as the `pnpm.cmd` shim, which
/// a process finds only by that name: a bare name is searched as `.exe`.
const PNPM: &str = if cfg!(windows) { "pnpm.cmd" } else { "pnpm" };

/// One Product's Console build.
pub(crate) struct Console {
    /// Its Product's name.
    product: &'static str,
    /// Its package, from the root.
    pub(crate) package: &'static str,
    /// The folder its executable embeds, from the root.
    pub(crate) build: &'static str,
    /// The digest of the inputs of its last build, from the root.
    inputs: &'static str,
    /// The packages whose files it reads and whose routes it serves, in
    /// that order, its own last.
    reads: &'static [&'static str],
}

/// Collie's Console.
const COLLIE: Console = Console {
    product: "Collie",
    package: PACKAGE,
    build: BUILD,
    inputs: "target/console/collie.inputs",
    reads: &[PACKAGE],
};

/// Foxhound's Console, which composes Collie's screens with its own.
const FOXHOUND: Console = Console {
    product: "Foxhound",
    package: FOXHOUND_PACKAGE,
    build: FOXHOUND_BUILD,
    inputs: "target/console/foxhound.inputs",
    reads: &[PACKAGE, FOXHOUND_PACKAGE],
};

/// Each Product's Console.
pub(crate) const CONSOLES: [&Console; 2] = [&COLLIE, &FOXHOUND];

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Builds each Product's Console into the folder its executable embeds
    /// (`crates/goodfetch-collie/console-build/`,
    /// `crates/foxhound/goodfetch-foxhound/console-build/`) as brotli files
    /// and its routes, unless its inputs did not change since its last
    /// build.
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

/// Builds each Product's Console whose package the workspace at `root`
/// holds, unless the build there is of the same inputs.
pub(crate) fn build(root: &Path) -> Result<()> {
    for console in CONSOLES {
        if root.join(console.package).join("package.json").is_file() {
            build_one(root, console)?;
        }
    }
    Ok(())
}

/// Builds `console` in the workspace at `root`, unless the build there is
/// of the same inputs.
fn build_one(root: &Path, console: &Console) -> Result<()> {
    let Console {
        product,
        package,
        build,
        inputs: kept_inputs,
        reads,
    } = *console;
    let digest = inputs(root, reads)?;
    let stamp = root.join(kept_inputs);
    let out = root.join(build);
    if out.join(format!("{PAGE}.br")).is_file()
        && std::fs::read_to_string(&stamp).is_ok_and(|kept| kept == digest)
    {
        output::line(&format!("{product}'s Console build is up to date"));
        return Ok(());
    }
    let target = root.join("target");
    std::fs::create_dir_all(&target).with_context(|| format!("creating {}", target.display()))?;
    let staging = tempfile::tempdir_in(&target).context("creating the build's own folder")?;
    let built = staging.path().join("built");
    let status = Process::new(PNPM)
        .arg("--dir")
        .arg(root.join(package))
        .args(["run", "build", "--outDir"])
        .arg(&built)
        .status()
        .context("running pnpm; is it installed?")?;
    if !status.success() {
        bail!(
            "the build of {package} failed ({status}); run `pnpm install --frozen-lockfile` (`just fetch`) first if its packages are not installed"
        );
    }
    if !built.join(PAGE).is_file() {
        bail!("the build of {package} wrote no {PAGE}");
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
    if let Some(routes) = routes(root, reads)? {
        std::fs::write(compressed.join(ROUTES), routes)
            .with_context(|| format!("writing {ROUTES} into {build}"))?;
    }
    if out.exists() {
        std::fs::remove_dir_all(&out).with_context(|| format!("removing {}", out.display()))?;
    }
    std::fs::rename(&compressed, &out).with_context(|| format!("writing {}", out.display()))?;
    if let Some(folder) = stamp.parent() {
        std::fs::create_dir_all(folder)
            .with_context(|| format!("creating {}", folder.display()))?;
    }
    std::fs::write(&stamp, &digest).with_context(|| format!("writing {}", stamp.display()))?;
    output::line(&format!("Built {product}'s Console into {build}"));
    Ok(())
}

/// The routes of the build that reads `packages`: each package's
/// `routes.json`, in their order, as one `{ "routes": [...] }` list without
/// repeats; none when no package has one.
fn routes(root: &Path, packages: &[&str]) -> Result<Option<String>> {
    let mut all: Vec<serde_json::Value> = Vec::new();
    let mut listed = false;
    for package in packages {
        let path = root.join(package).join(ROUTES);
        if !path.is_file() {
            continue;
        }
        listed = true;
        let text = std::fs::read_to_string(&path)
            .with_context(|| format!("reading {}", path.display()))?;
        let list: serde_json::Value = serde_json::from_str(&text)
            .with_context(|| format!("{} is no route list", path.display()))?;
        let package_routes = list
            .get("routes")
            .and_then(serde_json::Value::as_array)
            .with_context(|| format!("{} has no `routes` list", path.display()))?;
        for route in package_routes {
            if !all.contains(route) {
                all.push(route.clone());
            }
        }
    }
    if !listed {
        return Ok(None);
    }
    let text = serde_json::to_string(&serde_json::json!({ "routes": all }))
        .context("writing the build's routes")?;
    Ok(Some(format!("{text}\n")))
}

/// Installs the pnpm packages of the workspace at `root`, as its lockfile
/// names them, so its Console builds from its own.
pub(crate) fn install(root: &Path) -> Result<()> {
    let status = Process::new(PNPM)
        .arg("--dir")
        .arg(root)
        .args(["install", "--frozen-lockfile"])
        .status()
        .context("running pnpm; is it installed?")?;
    if !status.success() {
        bail!(
            "installing the packages of {} failed ({status})",
            root.display()
        );
    }
    Ok(())
}

/// Gives the workspace checked out at `tree` each Console build of the one
/// at `root` that its executable embeds and it lacks, as the base's tree of
/// `xtask perf build` lacks them: no benchmark reads their bytes, which
/// only a Daemon's Console serves.
pub(crate) fn lend_build(root: &Path, tree: &Path) -> Result<()> {
    for console in CONSOLES {
        let (from, to) = (root.join(console.build), tree.join(console.build));
        let embeds = to
            .parent()
            .is_some_and(|executable| executable.join("build.rs").is_file());
        if from.is_dir() && !to.exists() && embeds {
            copy_tree(&from, &to)?;
        }
    }
    Ok(())
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

/// The digest of everything a build reads: each file of `packages` but
/// their installed packages, by its path, and the pnpm lockfile.
fn inputs(root: &Path, packages: &[&str]) -> Result<String> {
    let mut files = Vec::new();
    for package in packages {
        collect(&root.join(package), &mut files)?;
    }
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
