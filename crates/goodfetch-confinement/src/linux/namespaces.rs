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
#![expect(
    clippy::disallowed_types,
    reason = "returns the loopback listener bound inside the Plugin's network namespace before exec (ADR 0033)"
)]

use std::io;
use std::net::TcpListener;

use super::{ConfineError, mounts, relay as proxy_relay};
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
/// mount namespace, where the child sees only that policy's paths. With
/// `network`, it also enters a network namespace (`CLONE_NEWNET`), brings up
/// `lo`, and when `bind_relay` is set binds a loopback TCP listener before
/// dropping capabilities. The child, the PID namespace's first process,
/// returns with no capability and ends when its parent does. The parent never
/// returns: it waits for the child and ends with its status, or by its signal.
///
/// The calling process must have one thread: the kernel refuses a user
/// namespace to any other.
pub(super) fn enter(
    files: Option<&Policy>,
    network: bool,
    bind_relay: bool,
) -> Result<Option<(TcpListener, u16)>, ConfineError> {
    // SAFETY: neither call reads or writes memory.
    let (user, group) = unsafe { (libc::getuid(), libc::getgid()) };
    let mount = if files.is_some() {
        libc::CLONE_NEWNS
    } else {
        0
    };
    let net = if network { libc::CLONE_NEWNET } else { 0 };
    // SAFETY: `unshare` takes flags only.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWPID | mount | net) } == -1 {
        return Err(unshare_error(io::Error::last_os_error()));
    }
    map_self(user, group)?;
    // SAFETY: the process has one thread, as `unshare` checked, so the child
    // inherits no lock another thread holds.
    match unsafe { libc::fork() } {
        -1 => Err(ConfineError::Namespace {
            call: "fork",
            error: io::Error::last_os_error(),
        }),
        0 => first_process(files, network, bind_relay),
        child => relay(child),
    }
}

/// Enters a user namespace and a network namespace (`CLONE_NEWUSER |
/// CLONE_NEWNET`) without forking a PID namespace (for Landlock ABI 6 where
/// Landlock already scopes signals). Brings up `lo`, optionally binds the
/// relay's loopback listener, and drops all capabilities.
pub(super) fn enter_network_only(
    bind_relay: bool,
) -> Result<Option<(TcpListener, u16)>, ConfineError> {
    // SAFETY: neither call reads or writes memory.
    let (user, group) = unsafe { (libc::getuid(), libc::getgid()) };
    // SAFETY: `unshare` takes flags only.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } == -1 {
        return Err(unshare_error(io::Error::last_os_error()));
    }
    map_self(user, group)?;
    let bound = setup_network(bind_relay)?;
    drop_capabilities()?;
    Ok(bound)
}

fn unshare_error(error: io::Error) -> ConfineError {
    match error.raw_os_error() {
        Some(libc::EPERM | libc::ENOSPC | libc::EUSERS) => ConfineError::Unavailable(
            "this host refuses an unprivileged user namespace (a sysctl, a seccomp profile or AppArmor)",
        ),
        _ => ConfineError::Namespace {
            call: "unshare",
            error,
        },
    }
}

fn map_self(user: libc::uid_t, group: libc::gid_t) -> Result<(), ConfineError> {
    // The maps are written by the process itself, which the kernel allows
    // for its own user and group once it denies `setgroups`.
    written("/proc/self/setgroups", "deny")?;
    written("/proc/self/uid_map", &format!("{user} {user} 1"))?;
    written("/proc/self/gid_map", &format!("{group} {group} 1"))?;
    Ok(())
}

fn setup_network(bind_relay: bool) -> Result<Option<(TcpListener, u16)>, ConfineError> {
    proxy_relay::bring_up_loopback().map_err(|error| ConfineError::Namespace {
        call: "SIOCSIFFLAGS",
        error,
    })?;
    if bind_relay {
        let bound = proxy_relay::bind_loopback().map_err(|error| ConfineError::Namespace {
            call: "bind(127.0.0.1:0)",
            error,
        })?;
        Ok(Some(bound))
    } else {
        Ok(None)
    }
}

/// Probes whether this process can enter a user and network namespace, map
/// itself, and drop every capability: used by `goodfetch-sandbox --probe-userns`
/// so the engine's startup probe verifies both the helper binary and its
/// AppArmor profile without running any user program (ADR 0033, issue 884).
pub fn probe_user_namespace() -> Result<(), ConfineError> {
    // SAFETY: neither call reads or writes memory.
    let (user, group) = unsafe { (libc::getuid(), libc::getgid()) };
    // SAFETY: `unshare` takes flags only.
    if unsafe { libc::unshare(libc::CLONE_NEWUSER | libc::CLONE_NEWNET) } == -1 {
        return Err(unshare_error(io::Error::last_os_error()));
    }
    map_self(user, group)?;
    drop_capabilities()?;
    Ok(())
}

/// Drops every capability (effective, permitted, inheritable and ambient) and
/// sets `PR_SET_NO_NEW_PRIVS` so no subsequent `execve` can regain privileges
/// (ADR 0033, issue 884).
pub(super) fn drop_capabilities() -> Result<(), ConfineError> {
    // Clear ambient capabilities first (best-effort when the kernel supports
    // ambient capabilities; ignored only on EINVAL).
    // SAFETY: `prctl` with `PR_CAP_AMBIENT` / `PR_CAP_AMBIENT_CLEAR_ALL` takes
    // integer arguments only.
    let ambient = unsafe {
        libc::prctl(
            libc::PR_CAP_AMBIENT,
            libc::PR_CAP_AMBIENT_CLEAR_ALL,
            0,
            0,
            0,
        )
    };
    if ambient == -1 {
        let error = io::Error::last_os_error();
        if error.raw_os_error() != Some(libc::EINVAL) {
            return Err(ConfineError::Namespace {
                call: "prctl(PR_CAP_AMBIENT_CLEAR_ALL)",
                error,
            });
        }
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
    // SAFETY: `prctl(PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0)` takes integer arguments only.
    if unsafe { libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) } == -1 {
        return Err(ConfineError::Namespace {
            call: "prctl(PR_SET_NO_NEW_PRIVS)",
            error: io::Error::last_os_error(),
        });
    }
    Ok(())
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

/// The child: ends with its parent, brings up loopback when entering a network
/// namespace, sees only `files`' paths when given, and drops every capability
/// the new user namespace gave it.
fn first_process(
    files: Option<&Policy>,
    network: bool,
    bind_relay: bool,
) -> Result<Option<(TcpListener, u16)>, ConfineError> {
    // SAFETY: `prctl` takes numbers only.
    if unsafe { libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) } == -1 {
        return Err(ConfineError::Namespace {
            call: "prctl",
            error: io::Error::last_os_error(),
        });
    }
    let bound = if network {
        setup_network(bind_relay)?
    } else {
        None
    };
    if let Some(policy) = files {
        mounts::expose_only(policy)?;
    }
    drop_capabilities()?;
    Ok(bound)
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
