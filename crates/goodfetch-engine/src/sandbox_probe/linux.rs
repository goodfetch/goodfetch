//! The probe on Linux: a forked child of the engine tries the user
//! namespace Chromium's sandbox starts in, `unshare(CLONE_NEWUSER)` then
//! its own `uid_map`, and exits with what it met, so the engine's own
//! process never changes namespace; the rest is read from `/proc` and the
//! environment.
#![expect(
    unsafe_code,
    reason = "the forked child makes the user namespace and exits with raw system calls before any program starts, which Rust's standard library does not offer (ADR 0049)"
)]

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Write as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Stdio};

use super::{Facts, NoSandboxPlatform, UserNamespace};

/// The child's exit codes, a byte: the namespace made and mapped;
/// `unshare` refused, and the map refused, each plus its `errno`, which
/// stays below 56 for every refusal either call names.
const CREATED: i32 = 0;
const REFUSED: i32 = 100;
const UNMAPPED: i32 = 200;
const ERRNO_RANGE: i32 = 55;

/// What this host gives, beside the engine's `environment` map.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe runs on the tasks module's blocking policy"
)]
pub(super) fn facts(environment: &BTreeMap<OsString, OsString>) -> Facts {
    let variable = |name: &str| {
        environment
            .get(OsStr::new(name))
            .and_then(|value| value.to_str())
            .map(str::to_owned)
    };
    let status = std::fs::read_to_string("/proc/self/status").unwrap_or_default();
    let seccomp_filtered = status
        .lines()
        .filter_map(|line| line.strip_prefix("Seccomp:"))
        .any(|mode| mode.trim() == "2");
    let apparmor_restricts =
        std::fs::read_to_string("/proc/sys/kernel/apparmor_restrict_unprivileged_userns")
            .is_ok_and(|value| value.trim() == "1");
    Facts {
        user_namespace: user_namespace(),
        seccomp_filtered,
        apparmor_restricts,
        kubernetes: variable("KUBERNETES_SERVICE_HOST").is_some(),
        platform: platform(variable),
    }
}

/// The platform that allows no sandbox, from the variables `variable`
/// reads, as each platform sets them.
pub(super) fn platform(variable: impl Fn(&str) -> Option<String>) -> Option<NoSandboxPlatform> {
    if variable("AWS_LAMBDA_FUNCTION_NAME").is_some() {
        return Some(NoSandboxPlatform::AwsLambda);
    }
    if variable("AWS_EXECUTION_ENV").is_some_and(|value| value == "AWS_ECS_FARGATE") {
        return Some(NoSandboxPlatform::Fargate);
    }
    if variable("CONTAINER_APP_NAME").is_some() {
        return Some(NoSandboxPlatform::AzureContainerApps);
    }
    None
}

/// What a forked child met making a user namespace.
#[expect(
    clippy::disallowed_methods,
    reason = "the probe runs on the tasks module's blocking policy"
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

/// The AppArmor label of the process `pid`.
#[expect(
    clippy::disallowed_methods,
    reason = "a launch reads it right after its browser started, a read of `/proc` that does not block"
)]
pub(super) fn label_of(pid: u32) -> Option<String> {
    [
        format!("/proc/{pid}/attr/apparmor/current"),
        format!("/proc/{pid}/attr/current"),
    ]
    .iter()
    .find_map(|path| std::fs::read_to_string(path).ok())
    .filter(|label| !label.trim().is_empty())
}
