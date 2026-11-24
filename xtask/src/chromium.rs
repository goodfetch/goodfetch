//! The Goodfetch Chromium's gate tooling (ADR 0074). `package` keeps each
//! suite's runtime dependencies, the files `gn`'s `.runtime_deps` list names
//! (the inputs of Chromium's own isolates), by SHA-256, so the gate's test
//! jobs fetch them by digest and run on standard runners without the build
//! tree. `expectations` checks the expected results under
//! `chromium/expectations/`.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs and process bans target async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fs::{File, Metadata};
use std::io::{self, Read as _, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::{Context as _, Result, bail};
use serde::Serialize;
use sha2::{Digest as _, Sha256};

use crate::check::{self, Finding};
use crate::environment::Environment;
use crate::output;

mod expectations;

/// The largest part a kept file is cut into: a file over 2 GiB is split.
const PART_SIZE: u64 = 2 << 30;

/// The manifest of digests, in the package's folder.
const MANIFEST: &str = "manifest.json";

/// Where each file or part is kept under its SHA-256, in the package's folder.
const BLOBS: &str = "blobs";

/// The ending of the list `gn` writes for a suite, `<suite>.runtime_deps`.
const LIST_ENDING: &str = ".runtime_deps";

#[derive(clap::Subcommand)]
pub(crate) enum Command {
    /// Keeps each suite's runtime dependencies, the files and folders its
    /// `<suite>.runtime_deps` list names from `gn`'s output folder, under
    /// `<into>/blobs/<sha256>`, a file over the part size cut into parts
    /// kept each by its own digest, and writes `<into>/manifest.json`: each
    /// suite's files with their size, SHA-256, executable bit and parts.
    /// Fails, writing nothing, on a listed file that is missing; `<into>`
    /// must be absent or empty.
    Package {
        /// `gn`'s output folder, which the lists' paths start from.
        #[arg(long)]
        out: PathBuf,
        /// The new package's folder.
        #[arg(long)]
        into: PathBuf,
        /// The largest part, in bytes; a file over it is split (2 GiB).
        #[arg(long, default_value_t = PART_SIZE, value_parser = clap::value_parser!(u64).range(1..))]
        part_size: u64,
        /// The suites' `.runtime_deps` lists, each named after its suite.
        #[arg(required = true)]
        lists: Vec<PathBuf>,
    },
    /// The expected results under `chromium/expectations/`.
    Expectations {
        #[command(subcommand)]
        command: expectations::Command,
    },
}

#[derive(Serialize)]
struct Manifest {
    suites: Vec<Suite>,
}

#[derive(Serialize)]
struct Suite {
    name: String,
    files: Vec<Kept>,
}

/// One runtime dependency, by its path from the output folder.
#[derive(Serialize)]
struct Kept {
    path: String,
    size: u64,
    sha256: String,
    executable: bool,
    /// The blobs that, put end to end, give the file back.
    parts: Vec<Part>,
}

#[derive(Serialize)]
struct Part {
    size: u64,
    sha256: String,
}

pub(crate) fn run(command: &Command, environment: &Environment) -> Result<ExitCode> {
    match command {
        Command::Package {
            out,
            into,
            part_size,
            lists,
        } => package(out, into, *part_size, lists, environment),
        Command::Expectations { command } => expectations::run(command, environment),
    }
}

fn package(
    out: &Path,
    into: &Path,
    part_size: u64,
    lists: &[PathBuf],
    environment: &Environment,
) -> Result<ExitCode> {
    let out = environment.path(out);
    let into = environment.path(into);
    let mut findings = Vec::new();
    let mut suites = BTreeMap::new();
    for list in lists {
        let list = environment.path(list);
        let name = suite(&list)?;
        let text = std::fs::read_to_string(&list)
            .with_context(|| format!("reading {}", list.display()))?;
        let files = resolved(&out, &name, &text, &mut findings)?;
        if suites.insert(name.clone(), files).is_some() {
            bail!("two lists name the suite {name}");
        }
    }
    if !findings.is_empty() {
        return Ok(check::report(&findings));
    }
    let blobs = into.join(BLOBS);
    if into.exists()
        && std::fs::read_dir(&into)
            .with_context(|| format!("reading {}", into.display()))?
            .next()
            .is_some()
    {
        bail!("{} is not empty: package into a new folder", into.display());
    }
    std::fs::create_dir_all(&blobs).with_context(|| format!("creating {}", blobs.display()))?;
    let mut manifest = Manifest { suites: Vec::new() };
    for (name, files) in suites {
        let files = files
            .into_iter()
            .map(|(path, source)| keep(&source, path, &blobs, part_size))
            .collect::<Result<_>>()?;
        manifest.suites.push(Suite { name, files });
    }
    let path = into.join(MANIFEST);
    let mut text = serde_json::to_string_pretty(&manifest)?;
    text.push('\n');
    std::fs::write(&path, text).with_context(|| format!("writing {}", path.display()))?;
    output::line(&path.display().to_string());
    Ok(ExitCode::SUCCESS)
}

/// The suite a list is for, from its name `<suite>.runtime_deps`.
fn suite(list: &Path) -> Result<String> {
    list.file_name()
        .and_then(|name| name.to_str())
        .and_then(|name| name.strip_suffix(LIST_ENDING))
        .filter(|name| !name.is_empty())
        .map(str::to_owned)
        .with_context(|| format!("{} is not named <suite>{LIST_ENDING}", list.display()))
}

/// Each file a suite's list names, by its path from the output folder, a
/// listed folder standing for every file under it; a listed path that is
/// neither is a finding.
fn resolved(
    out: &Path,
    name: &str,
    list: &str,
    findings: &mut Vec<Finding>,
) -> Result<BTreeMap<String, PathBuf>> {
    let mut files = BTreeMap::new();
    for path in list.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let source = out.join(path);
        if source.is_dir() {
            under(&source, path.trim_end_matches('/'), &mut files)?;
        } else if source.is_file() {
            files.insert(path.to_owned(), source);
        } else {
            findings.push(Finding::new(
                "missing-runtime-dep",
                format!("{name} {path}"),
                format!(
                    "`{name}{LIST_ENDING}` lists it, but {} is no file or folder",
                    source.display()
                ),
            ));
        }
    }
    Ok(files)
}

/// Adds every file under `folder`, by `path`, its path from the output
/// folder, joined to its own path under it.
fn under(folder: &Path, path: &str, files: &mut BTreeMap<String, PathBuf>) -> Result<()> {
    for entry in
        std::fs::read_dir(folder).with_context(|| format!("reading {}", folder.display()))?
    {
        let source = entry?.path();
        let name = source
            .file_name()
            .and_then(|name| name.to_str())
            .with_context(|| format!("{} is not named in UTF-8", source.display()))?;
        let path = format!("{path}/{name}");
        if source.is_dir() {
            under(&source, &path, files)?;
        } else {
            files.insert(path, source);
        }
    }
    Ok(())
}

/// Keeps `source` in `blobs`, cut into parts of at most `part_size` bytes,
/// each under its SHA-256.
fn keep(source: &Path, path: String, blobs: &Path, part_size: u64) -> Result<Kept> {
    let mut file = File::open(source).with_context(|| format!("opening {}", source.display()))?;
    let metadata = file.metadata()?;
    let size = metadata.len();
    let mut whole = Sha256::new();
    let mut parts = Vec::new();
    let mut left = size;
    loop {
        let length = left.min(part_size);
        let partial = blobs.join(".partial");
        let mut sink = Hashing {
            file: File::create(&partial)
                .with_context(|| format!("creating {}", partial.display()))?,
            part: Sha256::new(),
            whole: &mut whole,
        };
        let copied = io::copy(&mut (&mut file).take(length), &mut sink)
            .with_context(|| format!("copying {}", source.display()))?;
        if copied != length {
            bail!("{} changed while it was packaged", source.display());
        }
        let Hashing {
            file: written,
            part,
            ..
        } = sink;
        drop(written);
        let digest = format!("{:x}", part.finalize());
        let blob = blobs.join(&digest);
        if blob.exists() {
            std::fs::remove_file(&partial)?;
        } else {
            std::fs::rename(&partial, &blob)
                .with_context(|| format!("keeping {}", blob.display()))?;
        }
        parts.push(Part {
            size: length,
            sha256: digest,
        });
        left -= length;
        if left == 0 {
            break;
        }
    }
    Ok(Kept {
        path,
        size,
        sha256: format!("{:x}", whole.finalize()),
        executable: executable(&metadata),
        parts,
    })
}

/// A part's file, hashed as it is written, alone and as part of the whole.
struct Hashing<'a> {
    file: File,
    part: Sha256,
    whole: &'a mut Sha256,
}

impl Write for Hashing<'_> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let written = self.file.write(buffer)?;
        self.part.update(&buffer[..written]);
        self.whole.update(&buffer[..written]);
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.file.flush()
    }
}

#[cfg(unix)]
fn executable(metadata: &Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt as _;
    metadata.permissions().mode() & 0o111 != 0
}

#[cfg(not(unix))]
fn executable(_metadata: &Metadata) -> bool {
    false
}
