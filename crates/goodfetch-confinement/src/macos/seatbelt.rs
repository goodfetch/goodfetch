//! In-house macOS Seatbelt FFI binding and raw OS primitives (ADR 0033,
//! issue 867): the one module under `macos` that opts into `unsafe_code`
//! (`STYLE.md`).
//!
//! Binds Apple's two-function Seatbelt API (`sandbox_init`,
//! `sandbox_free_error`), plus `sandbox_check` to assert that a live confined
//! process is sandboxed, `setrlimit` for `RLIMIT_CPU` and `RLIMIT_NOFILE`,
//! and descriptor cleanup beyond stdio.
#![expect(
    unsafe_code,
    reason = "in-house two-function Seatbelt sandbox_init/sandbox_free_error binding, sandbox_check live sandbox check, setrlimit, and fd cleanup on macOS (ADR 0033, STYLE.md)"
)]

use std::ffi::{CStr, c_char, c_int};
use std::io;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

/// `SANDBOX_FILTER_NONE` from `<sandbox.h>`: queries whether `pid` has any
/// active Seatbelt sandbox profile.
const SANDBOX_FILTER_NONE: c_int = 0;

unsafe extern "C" {
    /// Compiles and applies an SBPL `profile` (when `flags == 0`) to the
    /// calling process. On failure, writes a heap-allocated message to
    /// `errorbuf` that must be freed with [`sandbox_free_error`].
    fn sandbox_init(profile: *const c_char, flags: u64, errorbuf: *mut *mut c_char) -> c_int;

    /// Releases the error string allocated by [`sandbox_init`].
    fn sandbox_free_error(errorbuf: *mut c_char);

    /// Returns `1` when `pid` is sandboxed (or denied `operation`), `0` when
    /// unconfined/allowed, and `-1` on error.
    fn sandbox_check(pid: libc::pid_t, operation: *const c_char, filter_type: c_int, ...) -> c_int;
}

/// Why [`apply`] did not install the Seatbelt profile.
pub(super) enum ApplyFailure {
    /// The host itself refused confinement (already sandboxed or `EPERM`).
    Unavailable(String),
    /// Any other compilation or kernel error.
    Other(String),
}

/// Applies the SBPL `profile` to the calling process via `sandbox_init`.
pub(super) fn apply(profile: &CStr) -> Result<(), ApplyFailure> {
    // SAFETY: `getpid` reads this process's own ID and cannot fail.
    let pid = unsafe { libc::getpid() };
    if is_sandboxed(pid) {
        return Err(ApplyFailure::Unavailable(
            "process is already inside a Seatbelt sandbox; nested confinement is unavailable"
                .to_owned(),
        ));
    }
    let mut errorbuf: *mut c_char = std::ptr::null_mut();
    // SAFETY: `profile` is a valid NUL-terminated C string outliving the call,
    // `flags == 0` compiles a raw SBPL profile string, and `errorbuf` is a
    // writable pointer initialized to null.
    let rc = unsafe { sandbox_init(profile.as_ptr(), 0, &raw mut errorbuf) };
    if rc == 0 {
        if !errorbuf.is_null() {
            // SAFETY: `errorbuf` was allocated by `sandbox_init`.
            unsafe { sandbox_free_error(errorbuf) };
        }
        return Ok(());
    }
    let errno = io::Error::last_os_error().raw_os_error();
    let message = if errorbuf.is_null() {
        io::Error::last_os_error().to_string()
    } else {
        // SAFETY: `sandbox_init` sets `errorbuf` to a valid NUL-terminated C
        // string on error; we copy it before freeing with `sandbox_free_error`.
        let text = unsafe { CStr::from_ptr(errorbuf) }
            .to_string_lossy()
            .into_owned();
        // SAFETY: `errorbuf` was allocated by `sandbox_init` and is freed once.
        unsafe { sandbox_free_error(errorbuf) };
        text
    };
    if errno == Some(libc::EPERM)
        || errno == Some(libc::ENOSYS)
        || message.contains("Operation not permitted")
    {
        Err(ApplyFailure::Unavailable(message))
    } else {
        Err(ApplyFailure::Other(message))
    }
}

/// Returns `true` when `pid` has an active Seatbelt profile
/// (`sandbox_check(pid, NULL, SANDBOX_FILTER_NONE) == 1`).
pub(super) fn is_sandboxed(pid: i32) -> bool {
    // SAFETY: `operation` is null and `SANDBOX_FILTER_NONE` requires no
    // variadic arguments; `sandbox_check` reads kernel state for `pid`.
    unsafe { sandbox_check(pid, std::ptr::null(), SANDBOX_FILTER_NONE) == 1 }
}

/// Probes whether this macOS host allows `sandbox_init` in a forked child so
/// the engine process itself is never confined by the probe (ADR 0049).
#[expect(
    clippy::disallowed_methods,
    reason = "startup probe runs once per process on the caller's blocking work"
)]
pub(super) fn probe_seatbelt() -> bool {
    // SAFETY: `getpid` cannot fail.
    if is_sandboxed(unsafe { libc::getpid() }) {
        return false;
    }
    let Ok(exe) = std::env::current_exe() else {
        return false;
    };
    let mut command = Command::new(exe);
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // SAFETY: runs in the forked child before exec; calls only async-signal-safe
    // `sandbox_init` on a static literal profile and `_exit` immediately so no
    // program starts and nothing allocates.
    unsafe {
        command.pre_exec(|| {
            let mut errorbuf: *mut c_char = std::ptr::null_mut();
            let rc = sandbox_init(c"(version 1)(allow default)".as_ptr(), 0, &raw mut errorbuf);
            if !errorbuf.is_null() {
                sandbox_free_error(errorbuf);
            }
            if rc == 0 {
                libc::_exit(0);
            }
            libc::_exit(1);
        });
    }
    command.status().is_ok_and(|status| status.success())
}

/// Sets soft and hard rlimits for `resource` on the calling process.
pub(super) fn set_rlimit(resource: c_int, soft: u64, hard: u64) -> io::Result<()> {
    let limit = libc::rlimit {
        rlim_cur: soft,
        rlim_max: hard,
    };
    // SAFETY: `limit` is a valid `rlimit` struct read by the kernel.
    let rc = unsafe { libc::setrlimit(resource, &raw const limit) };
    if rc == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

/// Closes every open file descriptor beyond stdio (FD >= 3) on macOS.
#[expect(
    clippy::disallowed_methods,
    reason = "launcher runs single-threaded before exec, outside any async worker"
)]
pub(super) fn close_beyond_stdio() -> io::Result<()> {
    if let Ok(entries) = std::fs::read_dir("/dev/fd") {
        let mut fds = Vec::new();
        for entry in entries.flatten() {
            if let Some(fd) = entry
                .file_name()
                .to_str()
                .and_then(|name| name.parse::<c_int>().ok())
                && fd >= 3
            {
                fds.push(fd);
            }
        }
        for fd in fds {
            // SAFETY: closing an inherited descriptor >= 3 in the single-threaded
            // launcher before exec; `EBADF` for the closed `/dev/fd` dirfd is
            // harmless and ignored.
            let _ = unsafe { libc::close(fd) };
        }
        return Ok(());
    }
    let mut rlim = libc::rlimit {
        rlim_cur: 0,
        rlim_max: 0,
    };
    // SAFETY: `rlim` is writable and initialized.
    let upper = if unsafe { libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut rlim) } == 0 {
        c_int::try_from(rlim.rlim_cur.min(4096)).unwrap_or(4096)
    } else {
        1024
    };
    for fd in 3..upper {
        // SAFETY: closing descriptors >= 3 in the single-threaded launcher.
        let _ = unsafe { libc::close(fd) };
    }
    Ok(())
}
