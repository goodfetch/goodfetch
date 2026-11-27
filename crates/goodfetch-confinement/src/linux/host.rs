//! The startup probe on Linux (ADRs 0033 and 0049): a forked child tries
//! the user namespace that Chromium's sandbox and a Plugin's namespaces
//! start in, `unshare(CLONE_NEWUSER)` then its own `uid_map`, and exits
//! with what it met, so the caller's own process never changes namespace;
//! the rest is read from `/proc` and asked of the kernel.
#![expect(
    unsafe_code,
    reason = "the forked child makes the user namespace and exits with raw system calls before any program starts, which Rust's standard library does not offer (ADR 0049)"
)]

use std::io::Write as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

use crate::host::{HostFacts, UserNamespace, is_wsl1};

/// The child's exit codes, a byte: the namespace made and mapped;
/// `unshare` refused, and the map refused, each plus its `errno`, which
/// stays below 56 for every refusal either call names.
const CREATED: i32 = 0;
const REFUSED: i32 = 100;
const UNMAPPED: i32 = 200;
const ERRNO_RANGE: i32 = 55;

/// What this host allows. It forks a child, so call it once per process,
/// from blocking work.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe runs once per process, on the caller's blocking work"
)]
pub fn read() -> HostFacts {
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let seccomp_filtered = status
        .lines()
        .filter_map(|line| line.strip_prefix("Seccomp:"))
        .any(|mode| mode.trim() == "2");
    let apparmor_restricts =
        std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
            .is_ok_and(|value| value.trim() == "1");
    let wsl1 = std::fs::read_to_string("/proc/sys/kernel/osrelease")
        .is_ok_and(|release| is_wsl1(&release));
    HostFacts {
        user_namespace: user_namespace(),
        seccomp_filtered,
        apparmor_restricts,
        landlock_abi: super::kernel_abi(),
        wsl1,
    }
}

/// What a forked child met making a user namespace.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe runs once per process, on the caller's blocking work"
)]
fn user_namespace() -> UserNamespace {
    // SAFETY: `getuid` reads the process's own user ID and cannot fail.
    let uid = unsafe { libc::getuid() };
    // Prepared before the fork, so the child allocates nothing.
    let mut map = Vec::new();
    let _ = write!(map, "0 {uid} 1");
    let mut command = Command::new("/proc/self/exe");
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: the closure runs in the forked child, which is single-threaded
    // there. It allocates nothing and takes no lock: it makes system calls
    // on `map`, prepared before the fork, and on a literal path, then exits
    // without returning, so no program starts.
    unsafe {
        command.pre_exec(move || {
            if libc::unshare(libc::CLONE_NEWUSER) != 0 {
                libc::_exit(REFUSED + errno());
            }
            let file = libc::open(c"/proc/self/uid_map".as_ptr(), libc::O_WRONLY);
            if file < 0 {
                libc::_exit(UNMAPPED + errno());
            }
            let written = libc::write(file, map.as_ptr().cast(), map.len());
            if written < 0 {
                libc::_exit(UNMAPPED + errno());
            }
            libc::_exit(CREATED)
        });
    }
    let status = match command.status() {
        Ok(status) => status,
        Err(error) => return UserNamespace::Refused(error.raw_os_error().unwrap_or(libc::EIO)),
    };
    match status.code() {
        Some(CREATED) => UserNamespace::Created,
        Some(code) if (UNMAPPED..=UNMAPPED + ERRNO_RANGE).contains(&code) => {
            UserNamespace::Unmapped(code - UNMAPPED)
        }
        Some(code) if (REFUSED..=REFUSED + ERRNO_RANGE).contains(&code) => {
            UserNamespace::Refused(code - REFUSED)
        }
        _ => UserNamespace::Refused(libc::EIO),
    }
}

/// The calling thread's `errno`, within the exit codes' range: `EIO`
/// beyond it.
fn errno() -> i32 {
    std::io::Error::last_os_error()
        .raw_os_error()
        .filter(|errno| (1..=ERRNO_RANGE).contains(errno))
        .unwrap_or(libc::EIO)
}
