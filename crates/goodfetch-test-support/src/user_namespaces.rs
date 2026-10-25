//! The hosts that refuse the user namespace Chromium's Linux sandbox starts
//! in (ADR 0049), each set on a child's command before it starts, so an
//! environment test runs Collie's executable on such a host and, with the
//! same command unchanged, on the host with its fix applied:
//!
//! - [`under_default_seccomp`]: the child runs under the seccomp rules
//!   Docker's and containerd's default profiles, Kubernetes'
//!   `RuntimeDefault` included, apply to namespaces: `unshare` and a
//!   `clone` that makes a namespace refused with `EPERM`, `clone3` with
//!   `ENOSYS` so that its callers fall back to `clone`. The fix, a profile
//!   that allows them, is the host's own rules.
//! - [`under_apparmor_restriction`]: the child runs under the
//!   `unprivileged_userns` profile Ubuntu's AppArmor restriction moves an
//!   unconfined program to once it makes a user namespace, so every program
//!   it starts, the browser included, gets a namespace without the
//!   capabilities to use it, as a browser that no AppArmor profile gives
//!   `userns` does. The fix, Chrome for Testing's profile
//!   (`ci/apparmor/goodfetch-chrome-for-testing`, which `ci/os-services.sh`
//!   loads), applies once the child runs unconfined.
//!
//! Linux only, on x86-64 and 64-bit Arm.
#![expect(
    unsafe_code,
    reason = "the forked child installs a seccomp filter or changes its AppArmor profile with raw system calls before its program starts, which Rust's standard library does not offer (ADR 0049)"
)]

use std::io;
use std::os::unix::process::CommandExt as _;
use std::process::Command;

/// The profile Ubuntu's restriction moves a program to once it makes a
/// user namespace, as `/etc/apparmor.d/unprivileged_userns` names it.
const RESTRICTION_PROFILE: &str = "unprivileged_userns";

/// Whether AppArmor restricts unprivileged user namespaces on this host
/// (`kernel.apparmor_restrict_unprivileged_userns=1`), as Ubuntu 23.10 and
/// later do.
#[must_use]
#[expect(
    clippy::disallowed_methods,
    reason = "a test reads it on its own thread, never an async worker"
)]
pub fn apparmor_restricts_user_namespaces() -> bool {
    std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
        .is_ok_and(|value| value.trim() == "1")
}

/// Runs the child under a container's default seccomp rules for
/// namespaces.
pub fn under_default_seccomp(command: &mut Command) {
    let filter = filter();
    // SAFETY: the closure runs in the forked child before its program
    // starts. It allocates nothing and takes no lock: it makes system calls
    // on `filter`, built before the fork, and on its own stack.
    unsafe {
        command.pre_exec(move || {
            let program = libc::sock_fprog {
                len: FILTER_LENGTH,
                filter: filter.as_ptr().cast_mut(),
            };
            if libc::prctl(libc::PR_SET_NO_NEW_PRIVS, 1, 0, 0, 0) != 0 {
                return Err(io::Error::last_os_error());
            }
            let no_flags: libc::c_long = 0;
            if libc::syscall(
                libc::SYS_seccomp,
                libc::c_long::from(libc::SECCOMP_SET_MODE_FILTER),
                no_flags,
                &raw const program,
            ) != 0
            {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
}

/// Runs the child under the `unprivileged_userns` profile, which its
/// program starts under.
pub fn under_apparmor_restriction(command: &mut Command) {
    let request = format!("exec {RESTRICTION_PROFILE}").into_bytes();
    // SAFETY: the closure runs in the forked child, single-threaded there,
    // before its program starts. It allocates nothing and takes no lock: it
    // makes system calls on `request`, built before the fork, and on a
    // literal path.
    unsafe {
        command.pre_exec(move || {
            let file = libc::open(c"/proc/self/attr/exec".as_ptr(), libc::O_WRONLY);
            if file < 0 {
                return Err(io::Error::last_os_error());
            }
            let written = libc::write(file, request.as_ptr().cast(), request.len());
            let failed = (written < 0).then(io::Error::last_os_error);
            libc::close(file);
            failed.map_or(Ok(()), Err)
        });
    }
}

/// `AUDIT_ARCH_*` of the architecture the filter is built for
/// (`linux/audit.h`).
#[cfg(target_arch = "x86_64")]
const AUDIT_ARCH: u32 = 0xC000_003E;
#[cfg(target_arch = "aarch64")]
const AUDIT_ARCH: u32 = 0xC000_00B7;

/// The bit of the x32 ABI's system call numbers on x86-64.
const X32_SYSCALL_BIT: u32 = 0x4000_0000;

/// Offsets into `struct seccomp_data`: `nr`, `arch`, and the low 32 bits of
/// `args[0]`, `clone`'s flags on both architectures.
const NR: u32 = 0;
const ARCH: u32 = 4;
const ARG0_LOW: u32 = 16;

/// Every `CLONE_NEW*` flag, which Docker's default profile refuses on
/// `clone` without `CAP_SYS_ADMIN`: `CLONE_NEWNS`, `CLONE_NEWCGROUP`,
/// `CLONE_NEWUTS`, `CLONE_NEWIPC`, `CLONE_NEWUSER`, `CLONE_NEWPID` and
/// `CLONE_NEWNET`.
const NAMESPACE_FLAGS: u32 = 0x7E02_0000;

const FILTER_LENGTH: u16 = 12;

fn statement(code: u32, k: u32) -> libc::sock_filter {
    jump(code, k, 0, 0)
}

fn jump(code: u32, k: u32, jt: u8, jf: u8) -> libc::sock_filter {
    libc::sock_filter {
        code: u16::try_from(code).expect("invariant: a BPF opcode fits 16 bits"),
        jt,
        jf,
        k,
    }
}

fn number(call: libc::c_long) -> u32 {
    u32::try_from(call).expect("invariant: a system call number fits")
}

/// The filter: refuse `unshare`, a `clone` with a namespace flag and every
/// call of another ABI with `EPERM`, `clone3` with `ENOSYS`; allow the
/// rest. Each jump counts the instructions it skips.
fn filter() -> [libc::sock_filter; FILTER_LENGTH as usize] {
    let load = libc::BPF_LD | libc::BPF_W | libc::BPF_ABS;
    let equal = libc::BPF_JMP | libc::BPF_JEQ | libc::BPF_K;
    let at_least = libc::BPF_JMP | libc::BPF_JGE | libc::BPF_K;
    let any_bit = libc::BPF_JMP | libc::BPF_JSET | libc::BPF_K;
    let ret = libc::BPF_RET | libc::BPF_K;
    let deny = libc::SECCOMP_RET_ERRNO | libc::EPERM.cast_unsigned();
    let no_such_call = libc::SECCOMP_RET_ERRNO | libc::ENOSYS.cast_unsigned();
    [
        /* 0 */ statement(load, ARCH),
        /* 1 */ jump(equal, AUDIT_ARCH, 0, 8),
        /* 2 */ statement(load, NR),
        /* 3 */ jump(at_least, X32_SYSCALL_BIT, 6, 0),
        /* 4 */ jump(equal, number(libc::SYS_unshare), 5, 0),
        /* 5 */ jump(equal, number(libc::SYS_clone3), 5, 0),
        /* 6 */ jump(equal, number(libc::SYS_clone), 0, 2),
        /* 7 */ statement(load, ARG0_LOW),
        /* 8 */ jump(any_bit, NAMESPACE_FLAGS, 1, 0),
        /* 9 */ statement(ret, libc::SECCOMP_RET_ALLOW),
        /* 10 */ statement(ret, deny),
        /* 11 */ statement(ret, no_such_call),
    ]
}
