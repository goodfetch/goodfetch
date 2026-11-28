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
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::process::CommandExt as _;
use std::path::Path;
use std::process::{Command, Stdio};

use crate::host::{HostFacts, UserNamespace, is_wsl1};

/// Default installation path of the root-owned `goodfetch-sandbox` helper on
/// Linux (ADRs 0033 and 0039, issue 884).
pub const DEFAULT_SANDBOX_HELPER_PATH: &str = "/usr/local/libexec/goodfetch/goodfetch-sandbox";

/// Default installation path of `goodfetch-sandbox`'s AppArmor profile on
/// Ubuntu 23.10 and later (ADR 0033, issue 884).
pub const DEFAULT_APPARMOR_PROFILE_PATH: &str = "/etc/apparmor.d/goodfetch-sandbox";

/// Flag passed to `goodfetch-sandbox` to probe whether its AppArmor profile
/// grants it `userns` without starting any user program.
pub const PROBE_USERNS_ARG: &str = "--probe-userns";

/// The child's exit codes, a byte: the namespace made and mapped;
/// `unshare` refused, and the map refused, each plus its `errno`, which
/// stays below 56 for every refusal either call names.
const CREATED: i32 = 0;
const REFUSED: i32 = 100;
const UNMAPPED: i32 = 200;
const ERRNO_RANGE: i32 = 55;

/// What this host allows. It forks a child, so call it once per process,
/// from blocking work.
pub fn read() -> HostFacts {
    read_with_helper(Path::new(DEFAULT_SANDBOX_HELPER_PATH))
}

/// What this host allows, probing `helper_path` for `goodfetch-sandbox` when
/// direct unprivileged user namespaces are unavailable.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe runs once per process, on the caller's blocking work"
)]
pub fn read_with_helper(helper_path: &Path) -> HostFacts {
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
    let user_namespace = user_namespace();
    let sandbox_helper =
        !wsl1 && user_namespace != UserNamespace::Created && probe_sandbox_helper(helper_path);
    HostFacts {
        user_namespace,
        seccomp_filtered,
        apparmor_restricts,
        landlock_abi: super::kernel_abi(),
        wsl1,
        sandbox_helper,
        seatbelt_refused: false,
    }
}

/// Probes whether `helper_path` is a valid non-setuid `goodfetch-sandbox`
/// binary that can create and map a user namespace on this host.
///
/// For the system installation path [`DEFAULT_SANDBOX_HELPER_PATH`], also
/// verifies that the helper is root-owned and not group/world-writable
/// (ADRs 0033 and 0039).
#[expect(
    clippy::disallowed_methods,
    reason = "the probe runs once per process, on the caller's blocking work"
)]
pub fn probe_sandbox_helper(helper_path: &Path) -> bool {
    let Ok(meta) = std::fs::metadata(helper_path) else {
        return false;
    };
    if !meta.is_file() {
        return false;
    }
    let mode = meta.mode();
    // Never trust a setuid or setgid helper (ADR 0033).
    if mode & 0o6000 != 0 {
        return false;
    }
    if helper_path == Path::new(DEFAULT_SANDBOX_HELPER_PATH)
        && (meta.uid() != 0 || mode & 0o022 != 0)
    {
        return false;
    }
    Command::new(helper_path)
        .arg(PROBE_USERNS_ARG)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
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
