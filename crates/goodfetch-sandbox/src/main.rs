//! `goodfetch-sandbox` (ADRs 0033 and 0039, issue 884): the small root-owned,
//! non-setuid Linux helper whose AppArmor profile grants `userns` on Ubuntu
//! 23.10 and later.
//!
//! Its security boundary is that it confines whatever it starts: it creates
//! the namespaces, drops every capability, sets `no_new_privs`, installs its
//! own unconditional baseline seccomp filter (no further namespaces, no
//! netlink), then installs Landlock and the caller's policy filter before
//! replacing itself with the Plugin. As hygiene, it starts only entries of
//! the Plugin store or runtime cache and refuses any other path.
//!
//! It links `goodfetch-confinement` and never `goodfetch-engine`.

use std::path::PathBuf;
use std::process::ExitCode;

use goodfetch_confinement::launch::{LAUNCH_VARIABLE, LaunchError, allowed_roots, sandbox_shim};
use goodfetch_protocol::home::HOME_FOLDER;
use goodfetch_protocol::settings::{BROWSERS_PATH_VARIABLE, HOME_VARIABLE};

fn main() -> ExitCode {
    #[cfg(target_os = "linux")]
    {
        if std::env::args_os()
            .nth(1)
            .is_some_and(|arg| arg == goodfetch_confinement::linux::host::PROBE_USERNS_ARG)
        {
            return probe_userns();
        }
        #[expect(
            clippy::disallowed_methods,
            reason = "the sandbox helper runs single-threaded outside any async runtime and reads its launch and home roots from its environment (ADR 0033)"
        )]
        let launch = std::env::var_os(LAUNCH_VARIABLE);
        let roots = resolve_allowed_roots();
        ExitCode::from(sandbox_shim(launch.as_deref(), &roots).status())
    }
    #[cfg(not(target_os = "linux"))]
    {
        ExitCode::from(LaunchError::Unsupported.status())
    }
}

#[cfg(target_os = "linux")]
fn probe_userns() -> ExitCode {
    use goodfetch_confinement::linux::{ConfineError, probe_user_namespace};

    match probe_user_namespace() {
        Ok(()) => ExitCode::SUCCESS,
        Err(ConfineError::Unavailable(reason)) => {
            ExitCode::from(LaunchError::Unavailable(reason.to_owned()).status())
        }
        Err(error) => ExitCode::from(LaunchError::Confine(error.to_string()).status()),
    }
}

/// Resolves the allowed Plugin store and runtime cache roots for the invoking
/// user from `GOODFETCH_HOME` (or the XDG / passwd data directory) and
/// optional `GOODFETCH_BROWSERS_PATH`.
#[cfg(target_os = "linux")]
#[expect(
    clippy::disallowed_methods,
    reason = "single-threaded launcher outside any async runtime"
)]
fn resolve_allowed_roots() -> Vec<PathBuf> {
    let home = std::env::var_os(HOME_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            std::env::var_os("XDG_DATA_HOME")
                .filter(|value| !value.is_empty())
                .map(|dir| PathBuf::from(dir).join(HOME_FOLDER))
        })
        .or_else(|| {
            std::env::var_os("HOME")
                .filter(|value| !value.is_empty())
                .map(|dir| PathBuf::from(dir).join(".local/share").join(HOME_FOLDER))
        })
        .or_else(|| passwd_home().map(|dir| dir.join(".local/share").join(HOME_FOLDER)));
    let browsers = std::env::var_os(BROWSERS_PATH_VARIABLE)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from);
    match home {
        Some(home) => allowed_roots(&home, browsers.as_deref()),
        None => browsers.into_iter().collect(),
    }
}

/// Reads the current user's home directory from `/etc/passwd` via `getpwuid`
/// when `HOME`, `XDG_DATA_HOME`, and `GOODFETCH_HOME` are all unset.
#[cfg(target_os = "linux")]
#[expect(
    unsafe_code,
    reason = "single-threaded launcher reading the invoking user's passwd home directory"
)]
fn passwd_home() -> Option<PathBuf> {
    use std::ffi::{CStr, OsStr};
    use std::os::unix::ffi::OsStrExt as _;

    // SAFETY: `getuid` and `getpwuid` are called on the launcher's single
    // thread before any other thread exists; `pw_dir` is copied before
    // returning.
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if pw.is_null() || (*pw).pw_dir.is_null() {
            return None;
        }
        let dir = CStr::from_ptr((*pw).pw_dir);
        Some(PathBuf::from(OsStr::from_bytes(dir.to_bytes())))
    }
}
