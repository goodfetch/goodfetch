//! The mount-namespace mechanism (ADR 0033): where the kernel has no
//! Landlock, the first process of the namespaces sees a root of its own
//! that holds only the policy's paths and the system's shared libraries,
//! as bubblewrap builds one. Its reads, executables and libraries are
//! bound read-only, its reads without the right to start them, and its
//! writes read-write; every other path is absent
//! (`ENOENT`), and the root itself is read-only.
#![expect(
    unsafe_code,
    reason = "mounting, pivoting the root and asking a mount's flags are raw system calls"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the applier runs in a launcher's one thread before the Plugin starts, never on an async worker"
)]

use std::collections::BTreeMap;
use std::ffi::{CString, OsString};
use std::fs;
use std::io;
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

use super::ConfineError;
use crate::policy::Policy;

/// Where the new root is built: a `tmpfs` over `/tmp`, which hides it only
/// inside the mount namespace, as bubblewrap does.
const BASE: &str = "/tmp";
/// The new root and the old one, inside `BASE` once it is the root.
const NEW_ROOT: &str = "/newroot";
const OLD_ROOT: &str = "/oldroot";

/// A mount's flags a remount in a user namespace must keep, since the
/// kernel locks them, with the `statvfs` flag that reports each.
const KEPT: &[(libc::c_ulong, libc::c_ulong)] = &[
    (libc::ST_NOSUID, libc::MS_NOSUID),
    (libc::ST_NODEV, libc::MS_NODEV),
    (libc::ST_NOEXEC, libc::MS_NOEXEC),
    (libc::ST_NOATIME, libc::MS_NOATIME),
    (libc::ST_NODIRATIME, libc::MS_NODIRATIME),
    (ST_RELATIME, libc::MS_RELATIME),
];

/// `statvfs`'s flag for a `relatime` mount, which the `libc` crate gives
/// for glibc only.
const ST_RELATIME: libc::c_ulong = 0x1000;

/// What a path is exposed for; a path the policy names twice gets the
/// widest.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Exposed {
    Read,
    Start,
    Write,
}

/// Gives the calling process, which must be the only one in a fresh user,
/// PID and mount namespace, a root that holds only `policy`'s paths.
pub(super) fn expose_only(policy: &Policy) -> Result<(), ConfineError> {
    let mut exposed = BTreeMap::new();
    // A loader maps the libraries it reads executable, which a mount
    // without the right to start a file refuses.
    let libraries = super::libraries();
    for (paths, access) in [
        (&policy.reads, Exposed::Read),
        (&libraries, Exposed::Start),
        (&policy.executables, Exposed::Start),
        (&policy.writes, Exposed::Write),
    ] {
        for path in paths {
            let widest = exposed.entry(path.clone()).or_insert(access);
            *widest = (*widest).max(access);
        }
    }
    // The working folder, entered again in the new root when exposed there:
    // the seccomp lists allow no `chdir` once the process is confined.
    let working = std::env::current_dir().ok();
    // Nothing mounted here reaches the host's namespace.
    mounted(None, Path::new("/"), None, libc::MS_REC | libc::MS_PRIVATE)?;
    mounted(
        Some("tmpfs"),
        Path::new(BASE),
        Some("tmpfs"),
        libc::MS_NODEV | libc::MS_NOSUID,
    )?;
    changed_directory(BASE)?;
    for folder in [NEW_ROOT, OLD_ROOT] {
        fs::create_dir(&folder[1..]).map_err(|error| failed("mkdir", error))?;
    }
    pivoted(".", &OLD_ROOT[1..])?;
    changed_directory("/")?;
    // The new root must be a mount of its own for the last pivot.
    let new_root = Path::new(NEW_ROOT);
    mounted(Some(NEW_ROOT), new_root, None, libc::MS_BIND | libc::MS_REC)?;
    // Parents come before what they hold, in a map ordered by components.
    for (path, access) in &exposed {
        bound(path, *access)?;
    }
    // The root alone: the writes bound in it stay writable.
    remounted_read_only(new_root, 0)?;
    unmounted(OLD_ROOT)?;
    changed_directory(NEW_ROOT)?;
    pivoted(".", ".")?;
    unmounted(".")?;
    changed_directory("/")?;
    // A working folder the policy does not expose is absent: `/` stays.
    if let Some(working) = working {
        let _ = std::env::set_current_dir(working);
    }
    Ok(())
}

/// Binds `path` of the old root at the same place in the new one.
fn bound(path: &Path, access: Exposed) -> Result<(), ConfineError> {
    let source = under(OLD_ROOT, path);
    let target = under(NEW_ROOT, path);
    let grant = |error: io::Error| ConfineError::Grant {
        path: path.to_owned(),
        reason: error.to_string(),
    };
    let folder = fs::metadata(&source).map_err(grant)?.is_dir();
    if fs::symlink_metadata(&target).is_err() {
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(grant)?;
        }
        if folder {
            fs::create_dir(&target).map_err(grant)?;
        } else {
            fs::File::create(&target).map_err(grant)?;
        }
    }
    mount(Some(&source), &target, None, libc::MS_BIND | libc::MS_REC).map_err(grant)?;
    match access {
        Exposed::Write => Ok(()),
        Exposed::Start => read_only(&target, false),
        Exposed::Read => read_only(&target, true),
    }
}

/// Remounts `target`, and every mount under it, read-only, keeping each
/// one's locked flags; without the right to start a file when `no_start`.
fn read_only(target: &Path, no_start: bool) -> Result<(), ConfineError> {
    let extra = if no_start { libc::MS_NOEXEC } else { 0 };
    for mount_point in mounts_under(target)? {
        remounted_read_only(&mount_point, extra)?;
    }
    Ok(())
}

/// Remounts the one mount at `mount_point` read-only, keeping its locked
/// flags, with the `extra` ones.
fn remounted_read_only(mount_point: &Path, extra: libc::c_ulong) -> Result<(), ConfineError> {
    let flags = libc::MS_BIND | libc::MS_REMOUNT | libc::MS_RDONLY | kept(mount_point)? | extra;
    mount(None, mount_point, None, flags).map_err(|error| ConfineError::Grant {
        path: mount_point.to_owned(),
        reason: error.to_string(),
    })
}

/// `target` and the mount points under it, from the old root's view of
/// this process's mounts.
fn mounts_under(target: &Path) -> Result<Vec<PathBuf>, ConfineError> {
    let table = fs::read(under(OLD_ROOT, Path::new("/proc/self/mountinfo")))
        .map_err(|error| failed("/proc/self/mountinfo", error))?;
    let mut found: Vec<PathBuf> = table
        .split(|byte| *byte == b'\n')
        .filter_map(|line| line.split(|byte| *byte == b' ').nth(4))
        .map(|field| PathBuf::from(OsString::from_vec(unescaped(field))))
        .filter(|mount_point| mount_point.starts_with(target))
        .collect();
    found.sort();
    found.dedup();
    if found.is_empty() {
        found.push(target.to_owned());
    }
    Ok(found)
}

/// A `mountinfo` field with its octal escapes (`\040` for a space) read.
fn unescaped(field: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(field.len());
    let mut rest = field;
    while let Some((&first, tail)) = rest.split_first() {
        let digits = tail.get(..3).filter(|digits| {
            first == b'\\' && digits.iter().all(|digit| (b'0'..=b'7').contains(digit))
        });
        if let Some(digits) = digits {
            let value = digits
                .iter()
                .fold(0_u32, |value, digit| value * 8 + u32::from(digit - b'0'));
            bytes.push(u8::try_from(value).unwrap_or(u8::MAX));
            rest = &tail[3..];
        } else {
            bytes.push(first);
            rest = tail;
        }
    }
    bytes
}

/// The flags of the mount at `target` a remount must keep.
fn kept(target: &Path) -> Result<libc::c_ulong, ConfineError> {
    let path = c_path(target);
    // SAFETY: an all-zero `statvfs` is a valid value for the kernel to fill.
    let mut stats: libc::statvfs = unsafe { std::mem::zeroed() };
    // SAFETY: `path` ends in NUL and `stats` is writable; both outlive the
    // call.
    if unsafe { libc::statvfs(path.as_ptr(), &raw mut stats) } == -1 {
        return Err(failed("statvfs", io::Error::last_os_error()));
    }
    Ok(KEPT
        .iter()
        .filter(|(reported, _)| stats.f_flag & reported != 0)
        .fold(0, |flags, (_, kept)| flags | kept))
}

/// `path`, an absolute path, under `root`.
fn under(root: &str, path: &Path) -> PathBuf {
    let mut joined = OsString::from(root);
    joined.push(path.as_os_str());
    PathBuf::from(joined)
}

fn c_path(path: &Path) -> CString {
    CString::new(path.as_os_str().as_bytes()).expect("invariant: a path holds no NUL byte")
}

fn failed(call: &'static str, error: io::Error) -> ConfineError {
    ConfineError::Namespace { call, error }
}

fn mounted(
    source: Option<&str>,
    target: &Path,
    kind: Option<&str>,
    flags: libc::c_ulong,
) -> Result<(), ConfineError> {
    mount(source.map(Path::new), target, kind, flags).map_err(|error| failed("mount", error))
}

fn mount(
    source: Option<&Path>,
    target: &Path,
    kind: Option<&str>,
    flags: libc::c_ulong,
) -> io::Result<()> {
    let source = source.map(c_path);
    let target = c_path(target);
    let kind = kind
        .map(|kind| CString::new(kind).expect("invariant: a file system's name holds no NUL byte"));
    let pointer =
        |text: &Option<CString>| text.as_ref().map_or(std::ptr::null(), |text| text.as_ptr());
    // SAFETY: each string ends in NUL or is null, and outlives the call; no
    // file system mounted here reads data.
    let result = unsafe {
        libc::mount(
            pointer(&source),
            target.as_ptr(),
            pointer(&kind),
            flags,
            std::ptr::null(),
        )
    };
    if result == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn pivoted(new_root: &str, old_root: &str) -> Result<(), ConfineError> {
    let new_root = c_path(Path::new(new_root));
    let old_root = c_path(Path::new(old_root));
    // SAFETY: both strings end in NUL and outlive the call.
    if unsafe { libc::syscall(libc::SYS_pivot_root, new_root.as_ptr(), old_root.as_ptr()) } == -1 {
        return Err(failed("pivot_root", io::Error::last_os_error()));
    }
    Ok(())
}

fn unmounted(target: &str) -> Result<(), ConfineError> {
    let target = c_path(Path::new(target));
    // SAFETY: the string ends in NUL and outlives the call.
    if unsafe { libc::umount2(target.as_ptr(), libc::MNT_DETACH) } == -1 {
        return Err(failed("umount2", io::Error::last_os_error()));
    }
    Ok(())
}

fn changed_directory(path: &str) -> Result<(), ConfineError> {
    std::env::set_current_dir(path).map_err(|error| failed("chdir", error))
}
