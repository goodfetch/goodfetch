//! `xtask sweep-target`: removes the dead build artifacts of a Cargo target
//! folder, which no build has read for a while, so a worktree's `target/`
//! stops growing with every dependency, feature or flag change it builds.
//!
//! Cargo keeps no record of what a build used and collects no garbage in a
//! target folder (its `gc` covers the global cache alone), so the sweep reads
//! what a build leaves, as cargo-sweep's `--time` does: every build, fresh or
//! not, reads the fingerprint of each unit of its graph,
//! `<profile>/.fingerprint/<package>-<hash>/`, and each of the unit's files in
//! `deps/`, `build/` and `examples/` carries the same hash. A unit whose
//! fingerprint no process has read for `--unused-hours` is dead: its files
//! and its fingerprint go, as do files whose hash has no fingerprint at all,
//! which Cargo never takes as fresh. So does each `incremental/` folder no
//! compile has written in that time (rustc names it after the crate, not the
//! hash). A unit a build read keeps every file of it, test and example
//! executables included: removing one would have the next build compile its
//! crate again, not only link it.
//!
//! The access times come from the file system: under Linux's default
//! `relatime`, a read updates a file's access time when it is a day old or
//! older than its last change, so a unit read in the last
//! `unused-hours - 24` hours is always kept; the default 48 hours keeps
//! every unit any build of the last day read, a gate's included. On a file
//! system that records no access time (`noatime`) the sweep removes nothing
//! and fails. Each profile is swept holding Cargo's own locks of it, so a
//! build running beside the sweep keeps its profile whole. Symbolic links
//! are never followed, and a `stress-base` folder, the stress run's base
//! build (`just stress`), is never entered.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

use std::collections::BTreeMap;
use std::fs::{File, FileTimes};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, bail};

use crate::environment::Environment;
use crate::output;

/// The lock files Cargo takes in a profile folder while it builds there.
const CARGO_LOCKS: [&str; 3] = [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"];

/// The folders of a profile whose entries carry their unit's hash.
const HASHED: [&str; 3] = ["deps", "build", "examples"];

/// The stress run's base build, which the sweep never enters.
const STRESS_BASE: &str = "stress-base";

/// How deep under the target folder a profile folder is looked for: a
/// target triple's (`<triple>/<profile>`), the MSRV's (`msrv/<profile>`) and
/// a fixture build's (`tmp/<fixture>/target/<profile>`).
const MAX_DEPTH: usize = 4;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// The Cargo target folder to sweep.
    #[arg(long, default_value = "target")]
    target: PathBuf,
    /// Removes what no build has read for this many hours; above 24, so
    /// `relatime`'s day-long resolution keeps what was read.
    #[arg(long, default_value_t = 48, value_parser = clap::value_parser!(u64).range(25..))]
    unused_hours: u64,
    /// Lists what would go, removing nothing.
    #[arg(long)]
    dry_run: bool,
}

/// What a sweep removed, or would remove.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Swept {
    /// The bytes of the files removed.
    pub(crate) bytes: u64,
    /// The units removed, by their fingerprint's hash.
    pub(crate) units: usize,
    /// The units kept.
    pub(crate) kept: usize,
    /// The paths removed: files, and folders whole.
    pub(crate) removed: Vec<PathBuf>,
    /// Each profile left whole, with why.
    pub(crate) skipped: Vec<(PathBuf, String)>,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let target = environment.path(&arguments.target);
    let cutoff = SystemTime::now()
        .checked_sub(Duration::from_secs(arguments.unused_hours * 3600))
        .context("the cutoff precedes the clock's range")?;
    let swept = sweep(&target, cutoff, arguments.dry_run)?;
    for (profile, reason) in &swept.skipped {
        output::line(&format!("{}: left whole, {reason}", profile.display()));
    }
    let verb = if arguments.dry_run {
        "would free"
    } else {
        "freed"
    };
    output::line(&format!(
        "Swept {}: {verb} {:.1} GB, {} units no build read for {} h and {} other paths; kept {} units",
        target.display(),
        swept.bytes as f64 / 1e9,
        swept.units,
        arguments.unused_hours,
        swept.removed.len().saturating_sub(swept.units),
        swept.kept,
    ));
    Ok(ExitCode::SUCCESS)
}

/// Removes, from the target folder `target`, every unit whose fingerprint
/// no process read since `cutoff`, every hashed file with no fingerprint, and
/// every incremental folder unwritten since `cutoff`; with `dry_run`, lists
/// them only.
pub(crate) fn sweep(target: &Path, cutoff: SystemTime, dry_run: bool) -> Result<Swept> {
    let mut swept = Swept::default();
    if !target.is_dir() {
        return Ok(swept);
    }
    records_access_times(target)?;
    let mut profiles = Vec::new();
    find_profiles(target, 0, &mut profiles)?;
    for profile in profiles {
        let locks = match lock(&profile)? {
            Ok(locks) => locks,
            Err(reason) => {
                swept.skipped.push((profile, reason));
                continue;
            }
        };
        sweep_profile(&profile, cutoff, dry_run, &mut swept)?;
        drop(locks);
    }
    Ok(swept)
}

/// Fails unless reading a file updates its access time here, which the
/// sweep's verdicts rest on.
fn records_access_times(target: &Path) -> Result<()> {
    let probe = target.join(".sweep-access-probe");
    std::fs::write(&probe, b"probe").with_context(|| format!("writing {}", probe.display()))?;
    let checked = (|| {
        let old = SystemTime::UNIX_EPOCH + Duration::from_secs(86_400);
        File::options()
            .write(true)
            .open(&probe)?
            .set_times(FileTimes::new().set_accessed(old))?;
        std::fs::read(&probe)?;
        Ok::<bool, std::io::Error>(std::fs::metadata(&probe)?.accessed()? > old)
    })();
    std::fs::remove_file(&probe).with_context(|| format!("removing {}", probe.display()))?;
    if !checked.with_context(|| format!("probing access times in {}", target.display()))? {
        bail!(
            "{} records no access times (mounted noatime?): nothing swept, as no unit's last read can be told",
            target.display()
        );
    }
    Ok(())
}

/// Each folder holding a `.fingerprint/`, at most [`MAX_DEPTH`] below
/// `folder`, without following a link or entering a hidden or stress-base
/// folder.
fn find_profiles(folder: &Path, depth: usize, profiles: &mut Vec<PathBuf>) -> Result<()> {
    if folder.join(".fingerprint").is_dir() {
        profiles.push(folder.to_path_buf());
        return Ok(());
    }
    if depth == MAX_DEPTH {
        return Ok(());
    }
    for entry in sorted_entries(folder)? {
        let name = entry.file_name().and_then(|name| name.to_str()).unwrap_or("");
        let metadata = std::fs::symlink_metadata(&entry)
            .with_context(|| format!("reading {}", entry.display()))?;
        if metadata.is_dir() && !name.starts_with('.') && name != STRESS_BASE {
            find_profiles(&entry, depth + 1, profiles)?;
        }
    }
    Ok(())
}

/// Cargo's locks of `profile`, held; or why one could not be taken.
fn lock(profile: &Path) -> Result<Result<Vec<File>, String>> {
    let mut held = Vec::new();
    for name in CARGO_LOCKS {
        let path = profile.join(name);
        if !path.is_file() {
            continue;
        }
        let file = File::options()
            .read(true)
            .write(true)
            .open(&path)
            .with_context(|| format!("opening {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => held.push(file),
            Err(std::fs::TryLockError::WouldBlock) => {
                return Ok(Err(format!("a Cargo process holds its {name}")));
            }
            Err(std::fs::TryLockError::Error(error)) => {
                return Err(error).with_context(|| format!("locking {}", path.display()));
            }
        }
    }
    Ok(Ok(held))
}

fn sweep_profile(profile: &Path, cutoff: SystemTime, dry_run: bool, swept: &mut Swept) -> Result<()> {
    // Each unit's hash, with its fingerprint folder and its last read.
    let mut units: BTreeMap<String, (PathBuf, SystemTime)> = BTreeMap::new();
    for folder in sorted_entries(&profile.join(".fingerprint"))? {
        let Some(hash) = hash_of(&folder) else {
            continue;
        };
        let mut last = SystemTime::UNIX_EPOCH;
        for file in sorted_entries(&folder)? {
            let metadata = std::fs::symlink_metadata(&file)
                .with_context(|| format!("reading {}", file.display()))?;
            last = last.max(metadata.accessed()?).max(metadata.modified()?);
        }
        units.insert(hash.to_owned(), (folder, last));
    }
    let live = |hash: &str| units.get(hash).is_some_and(|(_, last)| *last >= cutoff);
    for folder in HASHED {
        let folder = profile.join(folder);
        if !folder.is_dir() {
            continue;
        }
        for entry in sorted_entries(&folder)? {
            if hash_of(&entry).is_some_and(|hash| !live(hash)) {
                remove(&entry, dry_run, swept)?;
            }
        }
    }
    for (folder, last) in units.values() {
        if *last >= cutoff {
            swept.kept += 1;
        } else {
            swept.units += 1;
            remove(folder, dry_run, swept)?;
        }
    }
    let incremental = profile.join("incremental");
    if incremental.is_dir() {
        for folder in sorted_entries(&incremental)? {
            if last_written(&folder)? < cutoff {
                remove(&folder, dry_run, swept)?;
            }
        }
    }
    Ok(())
}

/// The latest change of `path` or of an entry directly in it: rustc writes a
/// new session folder in a crate's incremental folder at each compile.
fn last_written(path: &Path) -> Result<SystemTime> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    let mut last = metadata.modified()?;
    if metadata.is_dir() {
        for entry in sorted_entries(path)? {
            last = last.max(std::fs::symlink_metadata(&entry)?.modified()?);
        }
    }
    Ok(last)
}

/// The unit hash in an entry's name: `<name>-<16 hex digits>`, then nothing
/// or an extension.
fn hash_of(path: &Path) -> Option<&str> {
    let name = path.file_name()?.to_str()?;
    let stem = name.split_once('.').map_or(name, |(stem, _)| stem);
    let (_, hash) = stem.rsplit_once('-')?;
    (hash.len() == 16 && hash.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(hash)
}

fn remove(path: &Path, dry_run: bool, swept: &mut Swept) -> Result<()> {
    swept.bytes += size(path)?;
    swept.removed.push(path.to_path_buf());
    if dry_run {
        return Ok(());
    }
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if metadata.is_dir() {
        std::fs::remove_dir_all(path)
    } else {
        std::fs::remove_file(path)
    }
    .with_context(|| format!("removing {}", path.display()))
}

/// The bytes of the files at or under `path`, links not followed.
fn size(path: &Path) -> Result<u64> {
    let metadata =
        std::fs::symlink_metadata(path).with_context(|| format!("reading {}", path.display()))?;
    if !metadata.is_dir() {
        return Ok(metadata.len());
    }
    let mut bytes = 0;
    for entry in sorted_entries(path)? {
        bytes += size(&entry)?;
    }
    Ok(bytes)
}

fn sorted_entries(folder: &Path) -> Result<Vec<PathBuf>> {
    let mut entries = std::fs::read_dir(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .with_context(|| format!("reading {}", folder.display()))?;
    entries.sort();
    Ok(entries)
}

#[cfg(test)]
mod tests;
