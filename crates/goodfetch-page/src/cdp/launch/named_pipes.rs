//! A launched browser's debugging pipe on Windows (ADR 0017): two named
//! pipes, one each way, whose server ends the page's CDP connection
//! reads and writes, and whose client ends the browser inherits and finds
//! by their handle values in `--remote-debugging-io-pipes=<reads>,<writes>`,
//! beside `--remote-debugging-pipe`. Chromium adopts the two handles
//! instead of its descriptors 3 and 4, which only a C runtime's
//! undocumented start-up table could hand it on Windows. No socket and no
//! debugging port. Each pipe's name is random, its first and only instance
//! the engine's, and it refuses remote clients: a process that opens one
//! first makes the launch fail, never reads or writes the browser's
//! commands.
#![expect(
    clippy::disallowed_types,
    reason = "the CDP connection to a browser process, which ADR 0060 lets its module open"
)]

use std::io;
use std::os::windows::io::{AsRawHandle as _, OwnedHandle};

use tokio::net::windows::named_pipe::{NamedPipeServer, PipeEnd, ServerOptions};

/// The engine's ends of a browser's debugging pipe, and the browser's.
pub(super) struct DebuggingPipe {
    /// What the browser writes.
    pub(super) reader: NamedPipeServer,
    /// What the browser reads.
    pub(super) writer: NamedPipeServer,
    /// The browser's ends.
    pub(super) browser: BrowserEnds,
}

/// The browser's ends of its debugging pipe: synchronous handles, as
/// Chromium reads and writes them with no overlapped I/O, which the
/// browser inherits and the engine closes once the browser started, so the
/// engine sees its end of the pipe close when the browser exits.
pub(super) struct BrowserEnds {
    reads: OwnedHandle,
    writes: OwnedHandle,
}

impl BrowserEnds {
    /// Chromium's switch naming them: their handle values, which an
    /// inherited handle keeps in the child, as unsigned integers.
    ///
    /// # Errors
    ///
    /// When a handle's value does not fit Chromium's 32 bits.
    pub(super) fn switch(&self) -> io::Result<String> {
        let value = |handle: &OwnedHandle| {
            u32::try_from(handle.as_raw_handle().addr())
                .map_err(|_| io::Error::other("a debugging pipe's handle does not fit in 32 bits"))
        };
        Ok(format!(
            "--remote-debugging-io-pipes={},{}",
            value(&self.reads)?,
            value(&self.writes)?
        ))
    }

    /// Its handles, which the browser's start hands on to
    /// `Children::spawn_inheriting`.
    pub(super) fn into_handles(self) -> [OwnedHandle; 2] {
        [self.reads, self.writes]
    }
}

/// Opens a debugging pipe: both pipes' server ends, then the browser's
/// client ends, which connect them at once.
///
/// # Errors
///
/// When the OS refuses a pipe or a handle, or another process opened one
/// of the pipes first.
///
/// # Cancel safety
///
/// Cancel-safe: dropping the future closes both pipes and the browser's
/// ends, and no browser has started yet.
pub(super) async fn debugging_pipe() -> io::Result<DebuggingPipe> {
    let random = getrandom::u64().map_err(|error| io::Error::other(error.to_string()))?;
    let name = format!(
        r"\\.\pipe\goodfetch-cdp-{}-{random:016x}",
        std::process::id()
    );
    // The browser reads `-in`, which the engine writes, and writes `-out`.
    let (outbound, browser_reads) = pipe(&format!("{name}-in"), PipeEnd::Server)?;
    let (inbound, browser_writes) = pipe(&format!("{name}-out"), PipeEnd::Client)?;
    // A client that opened before the server's connect leaves it connected
    // (`ERROR_PIPE_CONNECTED`), so each returns at once; the connect is what
    // starts the server end's reads.
    outbound.connect().await?;
    inbound.connect().await?;
    Ok(DebuggingPipe {
        reader: inbound,
        writer: outbound,
        browser: BrowserEnds {
            reads: browser_reads,
            writes: browser_writes,
        },
    })
}

/// One pipe named `name`, which only `writer` writes: its server end and
/// its client end.
fn pipe(name: &str, writer: PipeEnd) -> io::Result<(NamedPipeServer, OwnedHandle)> {
    let server_writes = writer == PipeEnd::Server;
    let server = ServerOptions::new()
        .access_inbound(!server_writes)
        .access_outbound(server_writes)
        .first_pipe_instance(true)
        .max_instances(1)
        .reject_remote_clients(true)
        .create(name)?;
    // Opened without overlapped I/O, which std's `File` never asks for.
    #[expect(
        clippy::disallowed_methods,
        reason = "a named pipe's client end, no file a page's work reads"
    )]
    let client = std::fs::OpenOptions::new()
        .read(server_writes)
        .write(!server_writes)
        .open(name)?;
    Ok((server, OwnedHandle::from(client)))
}
