//! The browser's debugging pipe (ADR 0017): two OS pipes, whose browser
//! ends become its file descriptors 3 (what it reads) and 4 (what it
//! writes), as `--remote-debugging-pipe` expects, and whose engine ends the
//! CDP connection reads and writes. No socket and no debugging port.
#![expect(
    unsafe_code,
    reason = "the child's descriptors 3 and 4 are set between fork and exec, the pipe file descriptors of ADR 0017"
)]

use std::io::{self, PipeReader, PipeWriter};
use std::os::fd::{AsRawFd as _, OwnedFd};

use tokio::net::unix::pipe::{Receiver, Sender};
use tokio::process::Command;

/// The engine's ends of the pipe, once the browser started.
pub(super) struct EngineEnds {
    /// What the browser writes.
    pub(super) reader: Receiver,
    /// What the browser reads.
    pub(super) writer: Sender,
}

/// The browser's ends of the pipe, which the parent closes once the child
/// started, so the engine sees the browser's end of the pipe close with it.
pub(super) struct BrowserEnds {
    _reads: PipeReader,
    _writes: PipeWriter,
}

/// Sets `command` up to hand the browser its ends of a new pipe as
/// descriptors 3 and 4, and returns both sides' ends.
pub(super) fn attach(command: &mut Command) -> io::Result<(EngineEnds, BrowserEnds)> {
    let (browser_reads, engine_writes) = io::pipe()?;
    let (engine_reads, browser_writes) = io::pipe()?;
    let reads = browser_reads.as_raw_fd();
    let writes = browser_writes.as_raw_fd();
    // SAFETY: the closure runs in the child between fork and exec and calls
    // only `fcntl`, `dup2` and `close`, which are async-signal-safe; the
    // descriptors it reads were open in the parent at fork. Each end is first
    // duplicated above 4, so moving one onto 3 never closes the other.
    unsafe {
        command.pre_exec(move || {
            let high_reads = libc::fcntl(reads, libc::F_DUPFD, 5);
            let high_writes = libc::fcntl(writes, libc::F_DUPFD, 5);
            if high_reads < 0
                || high_writes < 0
                || libc::dup2(high_reads, 3) < 0
                || libc::dup2(high_writes, 4) < 0
            {
                return Err(io::Error::last_os_error());
            }
            libc::close(high_reads);
            libc::close(high_writes);
            Ok(())
        });
    }
    let engine = EngineEnds {
        reader: Receiver::from_owned_fd(OwnedFd::from(engine_reads))?,
        writer: Sender::from_owned_fd(OwnedFd::from(engine_writes))?,
    };
    Ok((
        engine,
        BrowserEnds {
            _reads: browser_reads,
            _writes: browser_writes,
        },
    ))
}
