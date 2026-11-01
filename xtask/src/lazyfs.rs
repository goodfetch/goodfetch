//! LazyFS, the FUSE file system that power-cuts a real executable on Linux
//! (ADR 0060): built from source archives pinned by SHA-256 in
//! `test-support/lazyfs/sources.toml`. `fetch` keeps them before the
//! outbound-traffic deny; `build` builds from the kept archives alone, with
//! `cmake`'s `FetchContent` downloads turned off, into `target/lazyfs/bin/`,
//! where `GOODFETCH_LAZYFS` names the executable for the LazyFS tests.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command as Process, ExitCode};

use anyhow::{Context as _, Result, bail};
use serde::Deserialize;

use crate::check;
use crate::environment::Environment;
use crate::github;
use crate::output;
use crate::pinned::{self, Pin};

/// The pinned sources, from the repository root.
pub(crate) const SOURCES_FILE: &str = "test-support/lazyfs/sources.toml";

/// Where `fetch` keeps the archives and `build` builds, from the root.
const FOLDER: &str = "target/lazyfs";

/// The largest archive kept; each is under a megabyte.
const MAXIMUM_DOWNLOAD: u64 = 16 * 1024 * 1024;

/// The archives `build` needs, by their pinned names.
const LAZYFS: &str = "lazyfs.tar.gz";
const TOML11: &str = "toml11.tar.gz";
const SPDLOG: &str = "spdlog.tar.gz";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Downloads each source archive `test-support/lazyfs/sources.toml`
    /// pins into `target/lazyfs/sources/`, unless the kept file already has
    /// its pinned SHA-256, and fails on a download whose SHA-256 differs.
    Fetch,
    /// Builds LazyFS (Linux, with `cmake`, a C++17 compiler and the headers
    /// of `libfuse3`) from the kept archives into `target/lazyfs/bin/`, and prints
    /// the executable's path; fails unless each archive is its pin.
    Build,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SourcesFile {
    source: Vec<Pin>,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    let root = &environment.root;
    let pins = pinned::read(root, SOURCES_FILE, |file: SourcesFile| file.source)?;
    let sources = root.join(FOLDER).join("sources");
    match command {
        Command::Fetch => Ok(check::report(&pinned::fetch(
            &sources,
            &pins,
            SOURCES_FILE,
            MAXIMUM_DOWNLOAD,
            github::Api {
                url: github::API_URL,
                token: environment.token(),
            },
        )?)),
        Command::Build => {
            if let Err(findings) = pinned::kept(&sources, &pins, "just lazyfs-fetch")? {
                return Ok(check::report(&findings));
            }
            let executable = build(&root.join(FOLDER), &sources, &pins)?;
            let executable = std::path::absolute(&executable)
                .with_context(|| format!("resolving {}", executable.display()))?;
            output::line(&executable.display().to_string());
            Ok(ExitCode::SUCCESS)
        }
    }
}

/// Builds LazyFS and its cache library `libpcache` from the kept archives,
/// unless `bin/` already holds a build of these pins, and returns the
/// executable's path. The library sits beside it, found through `$ORIGIN`.
fn build(folder: &Path, sources: &Path, pins: &[Pin]) -> Result<PathBuf> {
    if !cfg!(target_os = "linux") {
        bail!("LazyFS is a Linux file system: build it on Linux");
    }
    for name in [LAZYFS, TOML11, SPDLOG] {
        if !pins.iter().any(|pin| pin.name == name) {
            bail!("{SOURCES_FILE} pins no `{name}`, which the build needs");
        }
    }
    let bin = folder.join("bin");
    let executable = bin.join("lazyfs");
    let stamp_path = bin.join("pins");
    let stamp = pins.iter().fold(String::new(), |mut stamp, pin| {
        let _ = writeln!(stamp, "{} {}", pin.sha256, pin.name);
        stamp
    });
    if executable.exists() && std::fs::read_to_string(&stamp_path).ok().as_deref() == Some(&stamp) {
        return Ok(executable);
    }

    let work = folder.join("work");
    if work.exists() {
        std::fs::remove_dir_all(&work).with_context(|| format!("removing {}", work.display()))?;
    }
    let lazyfs = work.join("lazyfs");
    let pcache = lazyfs.join("libs").join("libpcache");
    let pcache_build = pcache.join("build");
    // LazyFS's own CMake file reads spdlog's headers and library from
    // libpcache's `_deps/spdlog-*`, where FetchContent would put them.
    let toml_source = pcache_build.join("_deps").join("toml-src");
    let spdlog_source = pcache_build.join("_deps").join("spdlog-src");
    extract(&sources.join(LAZYFS), &lazyfs)?;
    extract(&sources.join(TOML11), &toml_source)?;
    extract(&sources.join(SPDLOG), &spdlog_source)?;

    cmake(&[
        "-S".into(),
        pcache.display().to_string(),
        "-B".into(),
        pcache_build.display().to_string(),
        "-DCMAKE_BUILD_TYPE=Release".into(),
        // The pinned toml11 declares a CMake minimum below 3.5, which CMake
        // 4 refuses unless told the oldest policy version to assume.
        POLICY_MINIMUM.into(),
        "-DFETCHCONTENT_FULLY_DISCONNECTED=ON".into(),
        format!("-DFETCHCONTENT_SOURCE_DIR_TOML={}", toml_source.display()),
        format!(
            "-DFETCHCONTENT_SOURCE_DIR_SPDLOG={}",
            spdlog_source.display()
        ),
    ])?;
    cmake(&[
        "--build".into(),
        pcache_build.display().to_string(),
        "--parallel".into(),
    ])?;
    let lazyfs_build = lazyfs.join("lazyfs").join("build");
    cmake(&[
        "-S".into(),
        lazyfs.join("lazyfs").display().to_string(),
        "-B".into(),
        lazyfs_build.display().to_string(),
        "-DCMAKE_BUILD_TYPE=Release".into(),
        "-DLAZYFS_BUILD_TESTS=OFF".into(),
        POLICY_MINIMUM.into(),
        "-DCMAKE_BUILD_RPATH=$ORIGIN".into(),
    ])?;
    cmake(&[
        "--build".into(),
        lazyfs_build.display().to_string(),
        "--parallel".into(),
    ])?;

    std::fs::create_dir_all(&bin).with_context(|| format!("creating {}", bin.display()))?;
    copy(&lazyfs_build.join("lazyfs"), &executable)?;
    // The executable needs the library under its soname.
    copy(
        &pcache_build.join("libpcache.so.0"),
        &bin.join("libpcache.so.0"),
    )?;
    std::fs::write(&stamp_path, stamp)
        .with_context(|| format!("writing {}", stamp_path.display()))?;
    Ok(executable)
}

/// Unpacks a GitHub source archive's one top folder into `into`.
fn extract(archive: &Path, into: &Path) -> Result<()> {
    std::fs::create_dir_all(into).with_context(|| format!("creating {}", into.display()))?;
    let status = Process::new("tar")
        .arg("-xzf")
        .arg(archive)
        .arg("-C")
        .arg(into)
        .arg("--strip-components=1")
        .status()
        .context("starting tar")?;
    if !status.success() {
        bail!("tar could not unpack {} ({status})", archive.display());
    }
    Ok(())
}

/// The oldest `CMake` policy version the pinned sources' `CMake` files are
/// configured under, which `CMake` 3 ignores.
const POLICY_MINIMUM: &str = "-DCMAKE_POLICY_VERSION_MINIMUM=3.5";

fn cmake(arguments: &[String]) -> Result<()> {
    // Its output goes to stderr through xtask's writer, so stdout holds only
    // the executable's path.
    let output = Process::new("cmake")
        .args(arguments)
        .output()
        .context("starting cmake, which the LazyFS build needs")?;
    for stream in [&output.stdout, &output.stderr] {
        let text = String::from_utf8_lossy(stream);
        if !text.trim().is_empty() {
            output::error(text.trim_end());
        }
    }
    if !output.status.success() {
        bail!("cmake {} failed ({})", arguments.join(" "), output.status);
    }
    Ok(())
}

fn copy(from: &Path, to: &Path) -> Result<()> {
    std::fs::copy(from, to)
        .map(|_| ())
        .with_context(|| format!("copying {} to {}", from.display(), to.display()))
}
