//! The foreground Daemon's reaper on Linux (ADR 0049): `daemon run` makes
//! itself a child subreaper (`PR_SET_CHILD_SUBREAPER`), starts the Daemon
//! as its one child (`daemon run --reaped`), forwards `SIGTERM` and
//! `SIGINT` to it, and reaps every process that ends under it: the Daemon,
//! and each browser descendant the Daemon's own children leave behind,
//! which the kernel gives to the nearest subreaper. It exits with the
//! Daemon's exit code once the Daemon ended, so the executable runs as PID
//! 1 of a container without `tini`. The reaper waits on any child, which
//! is why the Daemon, whose Tokio runtime waits on its own children, runs
//! in a process of its own.
//!
//! The kernel's calls have no safe wrapper here: system calls that Miri
//! cannot run, which the PID 1 test on Linux covers (STYLE.md, `unsafe`).
#![expect(
    unsafe_code,
    reason = "the subreaper, the parent-death signal, `waitpid` and `kill` are only reached through the OS's C calls (ADR 0049)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the reaper starts the Daemon's process and builds its runtime outside any async worker (ADR 0060)"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io;
use std::path::Path;
use std::process::ExitCode;

use tokio::signal::unix::{SignalKind, signal};

/// Runs the foreground Daemon from `executable` under this process as its
/// reaper, with `environment`, and answers its exit code.
pub(in crate::executable) fn reap_around(
    executable: &Path,
    environment: &BTreeMap<OsString, OsString>,
    json: bool,
) -> ExitCode {
    match reap(executable, environment, json) {
        Ok(exit) => exit,
        Err(error) => {
            tracing::error!(%error, "the foreground Daemon's reaper failed");
            ExitCode::FAILURE
        }
    }
}

fn reap(
    executable: &Path,
    environment: &BTreeMap<OsString, OsString>,
    json: bool,
) -> io::Result<ExitCode> {
    // SAFETY: `prctl` with integer arguments only.
    if unsafe { libc::prctl(libc::PR_SET_CHILD_SUBREAPER, 1, 0, 0, 0) } != 0 {
        return Err(io::Error::last_os_error());
    }
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        // Listening comes before the Daemon starts, so neither its end nor a
        // signal sent at once is missed.
        let mut ended = signal(SignalKind::child())?;
        let mut terminate = signal(SignalKind::terminate())?;
        let mut interrupt = signal(SignalKind::interrupt())?;
        let mut command = std::process::Command::new(executable);
        command
            .args(["daemon", "run", "--reaped"])
            .args(json.then_some("--json"))
            .env_clear()
            .envs(environment);
        {
            use std::os::unix::process::CommandExt as _;
            // SAFETY: the closure runs in the child between fork and exec,
            // and only makes one async-signal-safe system call.
            unsafe {
                command.pre_exec(|| {
                    if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) == 0 {
                        Ok(())
                    } else {
                        Err(io::Error::last_os_error())
                    }
                });
            }
        }
        let daemon = command.spawn()?;
        let daemon = libc::pid_t::try_from(daemon.id())
            .map_err(|_| io::Error::other("the Daemon's process ID is out of range"))?;
        loop {
            if let Some(exit) = reap_every_ended(daemon) {
                return Ok(exit);
            }
            tokio::select! {
                _ = ended.recv() => {}
                _ = terminate.recv() => forward(daemon, libc::SIGTERM),
                _ = interrupt.recv() => forward(daemon, libc::SIGINT),
            }
        }
    })
}

/// Reaps every child that ended; the Daemon's exit code once it is among
/// them.
fn reap_every_ended(daemon: libc::pid_t) -> Option<ExitCode> {
    let mut exit = None;
    loop {
        let mut status = 0;
        // SAFETY: `waitpid` writes the status into a local integer.
        let reaped = unsafe { libc::waitpid(-1, &raw mut status, libc::WNOHANG) };
        if reaped <= 0 {
            return exit;
        }
        if reaped == daemon {
            exit = Some(exit_code(status));
        } else {
            tracing::debug!(pid = reaped, "the reaper reaped a descendant");
        }
    }
}

/// The exit code a shell gives a child that ended with `status`.
fn exit_code(status: libc::c_int) -> ExitCode {
    let code = if libc::WIFEXITED(status) {
        libc::WEXITSTATUS(status)
    } else {
        128 + libc::WTERMSIG(status)
    };
    ExitCode::from(u8::try_from(code).unwrap_or(1))
}

fn forward(daemon: libc::pid_t, signal: libc::c_int) {
    // SAFETY: `kill` takes a process ID and a signal, no pointer.
    if unsafe { libc::kill(daemon, signal) } != 0 {
        tracing::warn!(error = %io::Error::last_os_error(), "the reaper cannot pass a signal to the Daemon");
    }
}
