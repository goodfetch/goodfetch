//! The PID-namespace mechanism (ADR 0033): where Landlock cannot scope
//! signals (before ABI 6, kernel 6.12), the process to confine becomes the
//! first process of a PID namespace of its own, inside a user namespace of
//! its own. No process outside can be named from there, so none can be
//! signalled. Its former self stays outside, waits for it and ends as it
//! ends. The mount-namespace mechanism adds a mount namespace, in which
//! the first process sees only the policy's paths (`mounts`).
#![expect(
    unsafe_code,
    reason = "entering namespaces, forking, waiting and dropping capabilities are raw system calls"
)]

use std::io;

use super::{ConfineError, mounts};
use crate::policy::Policy;

/// `capset`'s version for 64-bit capability sets, held in two words
/// (`_LINUX_CAPABILITY_VERSION_3`).
const CAPABILITY_VERSION_3: u32 = 0x2008_0522;

#[repr(C)]
struct CapabilityHeader {
    version: u32,
    pid: libc::c_int,
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct CapabilitySets {
    effective: u32,
    permitted: u32,
    inheritable: u32,
}

/// Enters a user namespace and a PID namespace of its own, mapping only the
/// caller's own user and group, then forks. With `files`, it also enters a
/// mount namespace, where the child sees only that policy's paths. The
/// child, the PID namespace's first process, returns with no capability
/// and ends when its parent does. The parent never returns: it waits for
/// the child and ends with its status, or by its signal.
///
/// The calling process must have one thread: the kernel refuses a user
/// namespace to any other.
pub(super) fn enter(files: Option<&Policy>) -> Result<(), ConfineError> {
    // SAFETY: neither call reads or writes memory.
    let (user, group) = unsafe { (libc::getuid(), libc::getgid()) };
    let mount = if files.is_some() {
        libc::CLONE_NEWNS
    } else {
        0
    };
    // SAFETY: `unshare` takes flags only.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWPID | mount) } == -1 {
        let error = io::Error::last_os_error();
        return Err(match error.raw_os_error() {
            Some(libc::EPERM | libc::ENOSPC | libc::EUSERS) => ConfineError::Unavailable(
                "this host refuses an unprivileged user namespace (a sysctl, a seccomp profile or AppArmor)",
            ),
            _ => ConfineError::Namespace {
                call: "unshare",
                error,
            },
        });
    }
    // The maps are written by the process itself, which the kernel allows
    // for its own user and group once it denies `setgroups`.
    written("/proc/self/setgroups", "deny")?;
    written("/proc/self/uid_map", &format!("{user} {user} 1"))?;
    written("/proc/self/gid_map", &format!("{group} {group} 1"))?;
    // SAFETY: the process has one thread, as `unshare` checked, so the child
    // inherits no lock another thread holds.
    match unsafe { libc::fork() } {
        -1 => Err(ConfineError::Namespace {
            call: "fork",
            error: io::Error::last_os_error(),
        }),
        0 => first_process(files),
        child => relay(child),
    }
}

#[expect(
    clippy::disallowed_methods,
    reason = "the applier runs in a launcher's one thread before the Plugin starts, never on an async worker"
)]
fn written(path: &'static str, map: &str) -> Result<(), ConfineError> {
    std::fs::write(path, map).map_err(|error| match error.raw_os_error() {
        // AppArmor's restriction of unprivileged user namespaces (Ubuntu
        // 23.10 and later) leaves the namespace without the capability to
        // map: the `goodfetch-sandbox` helper's profile is the way there.
        Some(libc::EPERM | libc::EACCES) => ConfineError::Unavailable(
            "this host refuses to map a user namespace's user (AppArmor's restriction of unprivileged user namespaces)",
        ),
        _ => ConfineError::Namespace {
            call: path,
            error,
        },
    })
}

/// The child: ends with its parent, sees only `files`' paths when given,
/// and drops every capability the new user namespace gave it.
fn first_process(files: Option<&Policy>) -> Result<(), ConfineError> {
    // SAFETY: `prctl` takes numbers only.
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } == -1 {
        return Err(ConfineError::Namespace {
            call: "prctl",
            error: io::Error::last_os_error(),
        });
    }
    if let Some(policy) = files {
        mounts::expose_only(policy)?;
    }
    let mut header = CapabilityHeader {
        version: CAPABILITY_VERSION_3,
        pid: 0,
    };
    let sets = [CapabilitySets::default(); 2];
    // SAFETY: `header` and `sets` have the layout `capset` reads, version 3
    // with its two words, and outlive the call; pid 0 is this thread.
    if unsafe { libc::syscall(libc::SYS_capset, &raw mut header, sets.as_ptr()) } == -1 {
        return Err(ConfineError::Namespace {
            call: "capset",
            error: io::Error::last_os_error(),
        });
    }
    Ok(())
}

/// The parent: waits for `child`, then ends as it ended.
fn relay(child: libc::pid_t) -> ! {
    let mut status: libc::c_int = 0;
    // SAFETY: `waitpid` writes the child's status into `status`.
    while unsafe { libc::waitpid(child, &raw mut status, 0) } != child {
        if io::Error::last_os_error().kind() != io::ErrorKind::Interrupted {
            std::process::abort();
        }
    }
    if libc::WIFSIGNALED(status) {
        let signal = libc::WTERMSIG(status);
        // SAFETY: restores the signal's default action and unblocks it, then
        // raises it, so this process ends by the signal that ended the child;
        // `set` is a signal set initialised by `sigemptyset`.
        unsafe {
            let mut set: libc::sigset_t = std::mem::zeroed();
            libc::sigemptyset(&raw mut set);
            libc::sigaddset(&raw mut set, signal);
            libc::sigprocmask(libc::SIG_UNBLOCK, &raw const set, std::ptr::null_mut());
            libc::signal(signal, libc::SIG_DFL);
            libc::raise(signal);
        }
        std::process::abort();
    }
    // SAFETY: ends this process at once with the child's status, running
    // nothing the child's own exit already ran.
    unsafe { libc::_exit(libc::WEXITSTATUS(status)) }
}
