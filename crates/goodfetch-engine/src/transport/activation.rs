//! The listening socket a service manager hands the Daemon it starts at the
//! first connection (ADRs 0024 and 0031): launchd's, asked for by its name
//! in the launch agent's `Sockets`, and systemd's, the first descriptor of
//! `LISTEN_FDS` when `LISTEN_PID` names this process. Each is taken once,
//! by the Daemon at its start. FFI and system calls that Miri cannot run:
//! the tests on each real OS cover them (STYLE.md, `unsafe`).
#![expect(
    unsafe_code,
    reason = "a socket the service manager passed is a raw descriptor, which only an unsafe call turns into an owned one; launchd hands it over through its C API (ADR 0031)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "`TAKEN` is a flag of the process, set once by the Daemon at its start, which no thread synchronizes on and no loom test reaches"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::os::fd::{FromRawFd as _, OwnedFd};
use std::sync::atomic::{AtomicBool, Ordering};

/// Whether this process already took its activated socket, which it owns
/// once.
static TAKEN: AtomicBool = AtomicBool::new(false);

/// The activated socket's descriptor, owned, or `None` when no service
/// manager started this process with one.
#[cfg_attr(
    not(target_os = "macos"),
    expect(
        clippy::unnecessary_wraps,
        reason = "launchd's hand-over can fail, systemd's cannot: one signature serves both"
    )
)]
pub(super) fn take(
    environment: &BTreeMap<OsString, OsString>,
    launchd_sockets: &str,
) -> io::Result<Option<OwnedFd>> {
    if TAKEN.swap(true, Ordering::SeqCst) {
        return Ok(None);
    }
    #[cfg(target_os = "macos")]
    {
        let _ = environment;
        launchd(launchd_sockets)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = launchd_sockets;
        Ok(systemd(environment))
    }
}

/// systemd's first passed descriptor, when `LISTEN_PID` is this process.
#[cfg(not(target_os = "macos"))]
fn systemd(environment: &BTreeMap<OsString, OsString>) -> Option<OwnedFd> {
    /// systemd passes descriptors from 3 on (`SD_LISTEN_FDS_START`).
    const FIRST: std::os::fd::RawFd = 3;
    let number = |name: &str| {
        environment
            .get(std::ffi::OsStr::new(name))
            .and_then(|value| value.to_str())
            .and_then(|value| value.parse::<u64>().ok())
    };
    if number("LISTEN_PID") != Some(u64::from(std::process::id())) {
        return None;
    }
    if number("LISTEN_FDS").is_none_or(|count| count == 0) {
        return None;
    }
    // SAFETY: systemd passed this process (`LISTEN_PID` is its own) its
    // listening socket as descriptor 3, which nothing else in the process
    // opened or owns, and `TAKEN` makes this its one owner.
    Some(unsafe { OwnedFd::from_raw_fd(FIRST) })
}

#[cfg(target_os = "macos")]
fn launchd(name: &str) -> io::Result<Option<OwnedFd>> {
    use std::ffi::{CString, c_char, c_int};

    unsafe extern "C" {
        /// `<launch.h>`: the descriptors of the launch agent's socket
        /// `name`, in a buffer the caller frees.
        fn launch_activate_socket(
            name: *const c_char,
            fds: *mut *mut c_int,
            count: *mut usize,
        ) -> c_int;
    }

    let name = CString::new(name).map_err(io::Error::other)?;
    let mut descriptors: *mut c_int = std::ptr::null_mut();
    let mut count: usize = 0;
    // SAFETY: `name` is a NUL-terminated string that outlives the call, and
    // `descriptors` and `count` are valid places for its two answers.
    let error =
        unsafe { launch_activate_socket(name.as_ptr(), &raw mut descriptors, &raw mut count) };
    match error {
        0 => {}
        // Not started by launchd, or by a job with no socket of that name.
        libc::ESRCH | libc::ENOENT => return Ok(None),
        error => return Err(io::Error::from_raw_os_error(error)),
    }
    let owned: Vec<OwnedFd> = if descriptors.is_null() {
        Vec::new()
    } else {
        // SAFETY: launchd answered `count` descriptors at `descriptors`, in
        // a buffer it allocated with `malloc` for the caller, who owns each
        // descriptor from then on; the buffer is freed once they are read.
        unsafe {
            let owned = std::slice::from_raw_parts(descriptors, count)
                .iter()
                .map(|&descriptor| OwnedFd::from_raw_fd(descriptor))
                .collect();
            libc::free(descriptors.cast());
            owned
        }
    };
    // Every descriptor but the first closes as it drops.
    Ok(owned.into_iter().next())
}
