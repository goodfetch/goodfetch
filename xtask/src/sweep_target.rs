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
//! every unit any build of the last day read. `--read-since` takes a Unix
//! time instead and keeps every unit read since then, the cutoff falling a
//! day before it: the gate's sweep passes the gate's start, so whatever the
//! gate read stays however long it ran, with no rounding to hours. On a file
//! system that records no access time (`noatime`) the sweep removes nothing
//! and fails. Each profile is swept holding Cargo's own locks of it, so a
//! build running beside the sweep keeps its profile whole. Symbolic links
//! are never followed, and a `stress-base` folder, the stress run's base
//! build (`just stress`), is never entered, nor is a folder `--leave` names:
//! a profile that a run reads less often than the sweep's window gets a
//! sweep of its own, with the profile's folder as its target (the backstop's
//! worktree, `ci/gate-backstop.sh`).
//!
//! `--free-gb` sweeps by free space instead, for the slots' build folders on
//! a shared host: while the file system of a target has less than that many
//! GB free, it removes the units no build has read for longest, until the
//! line is free again, and never one a build may have read within
//! `--keep-hours`. A day of `relatime` lag would make that window a day
//! longer, so each such run re-arms the units it keeps: it writes back the
//! access time of each fingerprint file read since its last change, which
//! moves that change to now, and the next read is recorded at once. A file
//! whose access time is older than its last change has not been read since
//! that change, so a unit re-armed before the window and unread since goes,
//! even within the day. A host's disk monitor runs it every few minutes,
//! over and under the line, so the re-arms are never far behind.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous; the blocking-fs ban targets async crates (STYLE.md)"
)]

mod recency;

use std::collections::BTreeMap;
use std::fs::{File, FileTimes};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, SystemTime};

use anyhow::{Context as _, Result, bail};

use crate::environment::Environment;
use crate::output;
use recency::{Candidate, Times, least_read_first};

/// The lock files Cargo takes in a profile folder while it builds there.
const CARGO_LOCKS: [&str; 3] = [".cargo-lock", ".cargo-build-lock", ".cargo-artifact-lock"];

/// The folders of a profile whose entries carry their unit's hash.
const HASHED: [&str; 3] = ["deps", "build", "examples"];

/// The stress run's base build, which the sweep never enters.
const STRESS_BASE: &str = "stress-base";

/// How long after a read `relatime` may leave a file's access time behind.
const RELATIME_LAG: Duration = Duration::from_hours(24);

/// How deep under the target folder a profile folder is looked for: a
/// target triple's (`<triple>/<profile>`), the MSRV's (`msrv/<profile>`) and
/// a fixture build's (`tmp/<fixture>/target/<profile>`).
const MAX_DEPTH: usize = 4;

#[derive(clap::Args)]
pub(crate) struct Arguments {
    /// A Cargo target folder to sweep; repeat it for several.
    #[arg(long, default_value = "target")]
    target: Vec<PathBuf>,
    /// Removes what no build has read for this many hours; above 24, so
    /// `relatime`'s day-long resolution keeps what was read.
    #[arg(long, default_value_t = 48, value_parser = clap::value_parser!(u64).range(25..))]
    unused_hours: u64,
    /// Removes what no build has read since this Unix time, in seconds, or
    /// in the day before it, which `relatime` may lag; not after now.
    #[arg(long, conflicts_with = "unused_hours")]
    read_since: Option<u64>,
    /// Sweeps only while a target's file system has less than this many GB
    /// free, least recently read units first, until it has them; re-arms the
    /// access times of the units it keeps.
    #[arg(long, value_name = "GB", conflicts_with_all = ["unused_hours", "read_since"])]
    free_gb: Option<u64>,
    /// With `--free-gb`: never removes a unit a build may have read within
    /// this many hours.
    #[arg(long, default_value_t = 2, requires = "free_gb")]
    keep_hours: u64,
    /// Never enters a folder of this name, wherever it is under the target
    /// folder: a profile another sweep gives a window of its own.
    #[arg(long, value_name = "FOLDER")]
    leave: Vec<String>,
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
    /// The units kept whose next read `relatime` would not have recorded
    /// until the re-arm: by the free-space sweep alone.
    pub(crate) rearmed: usize,
    /// The paths removed: files, and folders whole.
    pub(crate) removed: Vec<PathBuf>,
    /// Each profile left whole, with why.
    pub(crate) skipped: Vec<(PathBuf, String)>,
}

pub(crate) fn run(arguments: &Arguments, environment: &Environment) -> Result<ExitCode> {
    let targets: Vec<PathBuf> = arguments
        .target
        .iter()
        .map(|target| environment.path(target))
        .collect();
    let now = SystemTime::now();
    if let Some(line_gb) = arguments.free_gb {
        return run_to_line(arguments, &targets, line_gb, now);
    }
    let (cutoff, unread) = match arguments.read_since {
        Some(seconds) => {
            let since = SystemTime::UNIX_EPOCH + Duration::from_secs(seconds);
            if since > now {
                bail!(
                    "--read-since {seconds} is after now: the cutoff would fall within the day `relatime` lags, and a unit read today could go"
                );
            }
            (
                since
                    .checked_sub(RELATIME_LAG)
                    .context("the cutoff precedes the clock's range")?,
                format!("since the day before {seconds}"),
            )
        }
        None => (
            now.checked_sub(Duration::from_hours(arguments.unused_hours))
                .context("the cutoff precedes the clock's range")?,
            format!("for {} h", arguments.unused_hours),
        ),
    };
    for target in &targets {
        let swept = sweep(target, cutoff, &arguments.leave, arguments.dry_run)?;
        report(
            target,
            &swept,
            arguments.dry_run,
            &format!("no build read {unread}"),
            "",
        );
    }
    Ok(ExitCode::SUCCESS)
}

fn run_to_line(
    arguments: &Arguments,
    targets: &[PathBuf],
    line_gb: u64,
    now: SystemTime,
) -> Result<ExitCode> {
    let since = now
        .checked_sub(Duration::from_hours(arguments.keep_hours))
        .context("the keep window precedes the clock's range")?;
    let line = line_gb.saturating_mul(GB);
    let swept = sweep_to_line(targets, line, since, &arguments.leave, arguments.dry_run)?;
    let rearm = if arguments.dry_run {
        "would re-arm"
    } else {
        "re-armed"
    };
    for (target, (swept, free)) in targets.iter().zip(&swept) {
        let Some(free) = free else {
            output::line(&format!(
                "{}: no such folder, nothing swept",
                target.display()
            ));
            continue;
        };
        report(
            target,
            swept,
            arguments.dry_run,
            &format!("no build read in the last {} h", arguments.keep_hours),
            &format!(
                ", {rearm} {}; {} GB free, line {line_gb} GB",
                swept.rearmed,
                tenths(*free)
            ),
        );
    }
    Ok(ExitCode::SUCCESS)
}

const GB: u64 = 1_000_000_000;

/// `bytes` in GB, to a tenth.
fn tenths(bytes: u64) -> String {
    let tenths = bytes / (GB / 10);
    format!("{}.{}", tenths / 10, tenths % 10)
}

/// Prints what the sweep of `target` did: `which` says which units went,
/// and `tail` ends the line.
fn report(target: &Path, swept: &Swept, dry_run: bool, which: &str, tail: &str) {
    for (profile, reason) in &swept.skipped {
        output::line(&format!("{}: left whole, {reason}", profile.display()));
    }
    let verb = if dry_run { "would free" } else { "freed" };
    output::line(&format!(
        "Swept {}: {verb} {} GB, {} units {which} and {} other paths; kept {} units{tail}",
        target.display(),
        tenths(swept.bytes),
        swept.units,
        swept.removed.len().saturating_sub(swept.units),
        swept.kept,
    ));
}

/// Removes, from the target folder `target`, every unit whose fingerprint
/// no process read since `cutoff`, every hashed file with no fingerprint, and
/// every incremental folder unwritten since `cutoff`, entering no folder
/// `leave` names; with `dry_run`, lists them only.
pub(crate) fn sweep(
    target: &Path,
    cutoff: SystemTime,
    leave: &[String],
    dry_run: bool,
) -> Result<Swept> {
    let mut swept = Swept::default();
    if !target.is_dir() {
        return Ok(swept);
    }
    records_access_times(target)?;
    let mut profiles = Vec::new();
    find_profiles(target, 0, leave, &mut profiles)?;
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

/// A unit's fingerprint folder and its files, which the sweep re-arms
/// unless it removes the unit.
struct Fingerprint {
    target: usize,
    folder: PathBuf,
    files: Vec<(PathBuf, Times)>,
    removed: bool,
}

/// Removes, from each of `targets`, while its file system has less than
/// `line` bytes free as `free` reads them, the units, hashed files with no
/// fingerprint and incremental folders no build has read since `since`, the
/// least recently read first; then re-arms the units each kept. Each
/// target's sweep comes with its free bytes once done (estimated, with
/// `dry_run`, which removes and re-arms nothing), or none for a target that
/// is no folder.
pub(crate) fn sweep_to_line(
    targets: &[PathBuf],
    line: u64,
    since: SystemTime,
    leave: &[String],
    dry_run: bool,
) -> Result<Vec<(Swept, Option<u64>)>> {
    let mut swept: Vec<Swept> = targets.iter().map(|_| Swept::default()).collect();
    let mut held = Vec::new();
    let mut candidates = Vec::new();
    let mut fingerprints = Vec::new();
    let mut devices = Vec::new();
    let mut start = Vec::new();
    for (index, target) in targets.iter().enumerate() {
        if !target.is_dir() {
            devices.push(None);
            start.push(0);
            continue;
        }
        records_access_times(target)?;
        devices.push(Some(device(target)?));
        start.push(free_bytes(target)?);
        let mut profiles = Vec::new();
        find_profiles(target, 0, leave, &mut profiles)?;
        for profile in profiles {
            match lock(&profile)? {
                Ok(locks) => {
                    held.push(locks);
                    list_profile(&profile, index, since, &mut candidates, &mut fingerprints)?;
                }
                Err(reason) => swept[index].skipped.push((profile, reason)),
            }
        }
    }
    // A dry run removes nothing, so its free bytes are those it started
    // with plus what it would have removed on the same file system.
    let free_now = |target: usize, swept: &[Swept]| -> Result<u64> {
        if !dry_run {
            return free_bytes(&targets[target]);
        }
        let would = (0..targets.len())
            .filter(|other| devices[*other] == devices[target])
            .map(|other| swept[other].bytes)
            .sum::<u64>();
        Ok(start[target].saturating_add(would))
    };
    least_read_first(
        &mut candidates,
        |target, swept| Ok(free_now(target, swept)? < line),
        |candidate, swept| {
            let target = &mut swept[candidate.target];
            for path in &candidate.paths {
                remove(path, dry_run, target)?;
            }
            if let Some(unit) = candidate.unit {
                let fingerprint = &mut fingerprints[unit];
                remove(&fingerprint.folder, dry_run, target)?;
                fingerprint.removed = true;
                target.units += 1;
            }
            Ok(())
        },
        &mut swept,
    )?;
    for fingerprint in fingerprints.iter().filter(|unit| !unit.removed) {
        let target = &mut swept[fingerprint.target];
        target.kept += 1;
        if fingerprint
            .files
            .iter()
            .any(|(_, times)| times.needs_rearm())
        {
            target.rearmed += 1;
            if !dry_run {
                rearm(fingerprint)?;
            }
        }
    }
    drop(held);
    let mut ended = Vec::new();
    for (index, device) in devices.iter().enumerate() {
        ended.push(match device {
            Some(_) => Some(free_now(index, &swept)?),
            None => None,
        });
    }
    Ok(swept.into_iter().zip(ended).collect())
}

/// Lists the free-space sweep's candidates in `profile`, of target `target`,
/// and every unit's fingerprint.
fn list_profile(
    profile: &Path,
    target: usize,
    since: SystemTime,
    candidates: &mut Vec<Candidate>,
    fingerprints: &mut Vec<Fingerprint>,
) -> Result<()> {
    // Each unit's hash, with its candidate's index when no build read it
    // since `since`.
    let mut units: BTreeMap<String, Option<usize>> = BTreeMap::new();
    for folder in sorted_entries(&profile.join(".fingerprint"))? {
        let Some(hash) = hash_of(&folder) else {
            continue;
        };
        // The folder's own change too: Cargo makes it before the unit's
        // compile and writes the fingerprint after.
        let made = std::fs::symlink_metadata(&folder)
            .with_context(|| format!("reading {}", folder.display()))?
            .modified()?;
        let mut last = made;
        let mut unread = made < since;
        let mut files = Vec::new();
        for file in sorted_entries(&folder)? {
            let times = Times::of(
                &std::fs::symlink_metadata(&file)
                    .with_context(|| format!("reading {}", file.display()))?,
            )?;
            last = last.max(times.accessed).max(times.modified);
            unread &= times.unread_since(since);
            files.push((file, times));
        }
        let hash = hash.to_owned();
        fingerprints.push(Fingerprint {
            target,
            folder,
            files,
            removed: false,
        });
        let candidate = unread.then(|| {
            candidates.push(Candidate {
                last,
                target,
                paths: Vec::new(),
                unit: Some(fingerprints.len() - 1),
            });
            candidates.len() - 1
        });
        units.insert(hash, candidate);
    }
    for folder in HASHED {
        let folder = profile.join(folder);
        if !folder.is_dir() {
            continue;
        }
        for entry in sorted_entries(&folder)? {
            let Some(hash) = hash_of(&entry) else {
                continue;
            };
            match units.get(hash) {
                Some(Some(candidate)) => candidates[*candidate].paths.push(entry),
                Some(None) => {}
                // Cargo never takes a file with no fingerprint as fresh.
                None => candidates.push(Candidate {
                    last: SystemTime::UNIX_EPOCH,
                    target,
                    paths: vec![entry],
                    unit: None,
                }),
            }
        }
    }
    let incremental = profile.join("incremental");
    if incremental.is_dir() {
        for folder in sorted_entries(&incremental)? {
            let last = last_written(&folder)?;
            if last < since {
                candidates.push(Candidate {
                    last,
                    target,
                    paths: vec![folder],
                    unit: None,
                });
            }
        }
    }
    Ok(())
}

/// Writes back the access time of each fingerprint file a read left
/// unchanged since: the write moves the file's last change to now, so the
/// next read moves the access time again.
fn rearm(fingerprint: &Fingerprint) -> Result<()> {
    for (file, times) in &fingerprint.files {
        if times.needs_rearm() {
            File::open(file)
                .and_then(|open| open.set_times(FileTimes::new().set_accessed(times.accessed)))
                .with_context(|| format!("re-arming {}", file.display()))?;
        }
    }
    Ok(())
}

/// The device of the file system holding `folder`.
#[cfg(unix)]
fn device(folder: &Path) -> Result<u64> {
    use std::os::unix::fs::MetadataExt as _;

    Ok(std::fs::metadata(folder)
        .with_context(|| format!("reading {}", folder.display()))?
        .dev())
}

#[cfg(not(unix))]
fn device(_folder: &Path) -> Result<u64> {
    bail!("--free-gb reads a file system's free space, which only a Unix host gives here")
}

/// The bytes free to an unprivileged user on the file system holding
/// `folder`.
#[cfg(unix)]
#[expect(
    unsafe_code,
    reason = "Unix gives a file system's free space only through `statvfs`"
)]
fn free_bytes(folder: &Path) -> Result<u64> {
    use std::os::unix::ffi::OsStrExt as _;

    let path = std::ffi::CString::new(folder.as_os_str().as_bytes())
        .with_context(|| format!("{} holds a NUL byte", folder.display()))?;
    let mut found = std::mem::MaybeUninit::<libc::statvfs>::zeroed();
    // SAFETY: `path` is a NUL-terminated string and `found` a writable
    // `statvfs` the call fills.
    if unsafe { libc::statvfs(path.as_ptr(), found.as_mut_ptr()) } != 0 {
        return Err(std::io::Error::last_os_error())
            .with_context(|| format!("reading the free space of {}", folder.display()));
    }
    // SAFETY: the call succeeded, so it filled `found`.
    let found = unsafe { found.assume_init() };
    Ok(wide(found.f_bavail).saturating_mul(wide(found.f_frsize)))
}

#[cfg(not(unix))]
fn free_bytes(_folder: &Path) -> Result<u64> {
    bail!("--free-gb reads a file system's free space, which only a Unix host gives here")
}

/// A `statvfs` count, whose width differs by OS, as a `u64`.
#[cfg(unix)]
fn wide<T: Into<u64>>(count: T) -> u64 {
    count.into()
}

/// Fails unless reading a file updates its access time here, which the
/// sweep's verdicts rest on.
fn records_access_times(target: &Path) -> Result<()> {
    let probe = target.join(".sweep-access-probe");
    std::fs::write(&probe, b"probe").with_context(|| format!("writing {}", probe.display()))?;
    let checked = (|| {
        let old = SystemTime::UNIX_EPOCH + Duration::from_hours(24);
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
/// folder, or one `leave` names.
fn find_profiles(
    folder: &Path,
    depth: usize,
    leave: &[String],
    profiles: &mut Vec<PathBuf>,
) -> Result<()> {
    if folder.join(".fingerprint").is_dir() {
        profiles.push(folder.to_path_buf());
        return Ok(());
    }
    if depth == MAX_DEPTH {
        return Ok(());
    }
    for entry in sorted_entries(folder)? {
        let name = entry
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        let metadata = std::fs::symlink_metadata(&entry)
            .with_context(|| format!("reading {}", entry.display()))?;
        let left = name == STRESS_BASE || leave.iter().any(|left| left == name);
        if metadata.is_dir() && !name.starts_with('.') && !left {
            find_profiles(&entry, depth + 1, leave, profiles)?;
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

fn sweep_profile(
    profile: &Path,
    cutoff: SystemTime,
    dry_run: bool,
    swept: &mut Swept,
) -> Result<()> {
    // Each unit's hash, with its fingerprint folder and its last read.
    let mut units: BTreeMap<String, (PathBuf, SystemTime)> = BTreeMap::new();
    for folder in sorted_entries(&profile.join(".fingerprint"))? {
        let Some(hash) = hash_of(&folder) else {
            continue;
        };
        // The folder's own change too: Cargo makes it before the unit's
        // compile and writes the fingerprint after.
        let mut last = std::fs::symlink_metadata(&folder)
            .with_context(|| format!("reading {}", folder.display()))?
            .modified()?;
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
