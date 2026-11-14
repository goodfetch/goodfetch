//! The counting relay of the Collie scenario (issue 2282): a TCP relay on
//! loopback between a tool and the remote-debugging endpoint of the browser
//! `xtask compare` launched, which counts the CDP commands the tool sends.
//! Each tool, Collie's engine or Playwright, reaches the browser only
//! through it, so both tools' commands are counted the same way, from
//! outside them: one command is one whole WebSocket data message from the
//! tool to the browser (text or binary, however many frames carry it;
//! pings, pongs and closes are no command).
//!
//! The relay also answers the driver's two marks, `GET /start` and
//! `GET /end`, around the part of the trial that is measured: each mark
//! records the command count so far, the CPU time of the tool's process
//! tree ([`Relay::follow`]) and the moment, before it answers, so the
//! figures between the marks are the measured part's alone.
#![expect(
    clippy::disallowed_methods,
    reason = "xtask is synchronous: the relay copies each connection's bytes on threads of its own (STYLE.md)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "xtask is no engine crate (ADR 0060): the relay's threads share its count and marks through std's atomics and a mutex, on std's loopback sockets"
)]

use std::io::{Read as _, Write as _};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::{Context as _, Result};

use super::{sampler, trial};

/// The longest request head the relay reads.
const HEAD_LIMIT: usize = 16 * 1024;

/// The streaming count of whole data messages in a client's WebSocket
/// frames (RFC 6455, section 5.2), fed the bytes in any chunks.
#[derive(Debug, Default)]
pub(crate) struct Frames {
    /// The current frame's header bytes read so far.
    header: Vec<u8>,
    /// The current frame's payload bytes left to skip, once its header is
    /// whole.
    payload_left: u64,
    /// Whether the current frame ends a data message.
    ends_message: bool,
}

impl Frames {
    /// The header's length once its first two bytes are known.
    fn header_length(&self) -> usize {
        let extended = match self.header[1] & 0x7f {
            126 => 2,
            127 => 8,
            _ => 0,
        };
        let mask = if self.header[1] & 0x80 == 0 { 0 } else { 4 };
        2 + extended + mask
    }

    /// The payload's length, from a whole header.
    fn payload_length(&self) -> u64 {
        match self.header[1] & 0x7f {
            126 => u64::from(u16::from_be_bytes([self.header[2], self.header[3]])),
            127 => {
                let mut bytes = [0; 8];
                bytes.copy_from_slice(&self.header[2..10]);
                u64::from_be_bytes(bytes)
            }
            length => u64::from(length),
        }
    }

    /// Reads `bytes`, the next of the stream, and returns how many data
    /// messages they ended.
    pub(crate) fn feed(&mut self, mut bytes: &[u8]) -> u64 {
        let mut ended = 0;
        while !bytes.is_empty() {
            if self.payload_left > 0 {
                let skipped = usize::try_from(self.payload_left)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                bytes = &bytes[skipped..];
                self.payload_left -= skipped as u64;
                continue;
            }
            self.header.push(bytes[0]);
            bytes = &bytes[1..];
            if self.header.len() < 2 || self.header.len() < self.header_length() {
                continue;
            }
            let fin = self.header[0] & 0x80 != 0;
            let opcode = self.header[0] & 0x0f;
            self.ends_message = fin && matches!(opcode, 0..=2);
            self.payload_left = self.payload_length();
            self.header.clear();
            if self.ends_message {
                ended += 1;
            }
        }
        ended
    }
}

/// What a mark recorded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    /// The commands the relay had counted.
    pub(crate) commands: u64,
    /// The CPU seconds of the tool's process tree.
    pub(crate) cpu_seconds: f64,
    pub(crate) at: Instant,
}

/// What the relay's threads share.
struct Shared {
    commands: AtomicU64,
    /// The process whose tree a mark reads the CPU time of; 0 for none.
    tool: AtomicU32,
    marks: Mutex<(Option<Mark>, Option<Mark>)>,
    stop: AtomicBool,
}

/// A running relay to the browser's remote-debugging endpoint on loopback.
pub(crate) struct Relay {
    address: SocketAddr,
    shared: Arc<Shared>,
    thread: Option<JoinHandle<()>>,
}

impl Relay {
    /// Starts relaying a loopback port of its own to the browser's
    /// remote-debugging port `browser`.
    pub(crate) fn start(browser: u16) -> Result<Self> {
        let listener =
            TcpListener::bind("127.0.0.1:0").context("binding the counting relay's port")?;
        let address = listener
            .local_addr()
            .context("reading the counting relay's port")?;
        let shared = Arc::new(Shared {
            commands: AtomicU64::new(0),
            tool: AtomicU32::new(0),
            marks: Mutex::new((None, None)),
            stop: AtomicBool::new(false),
        });
        let accepting = Arc::clone(&shared);
        let thread = std::thread::spawn(move || {
            for stream in listener.incoming() {
                if accepting.stop.load(Ordering::Relaxed) {
                    break;
                }
                let Ok(stream) = stream else { continue };
                let shared = Arc::clone(&accepting);
                // A connection's threads end with the connection: the
                // browser's or the tool's side closing it.
                std::thread::spawn(move || {
                    // A connection that fails ends; the trial sees its
                    // tool fail.
                    let _ = connection(stream, browser, &shared);
                });
            }
        });
        Ok(Self {
            address,
            shared,
            thread: Some(thread),
        })
    }

    /// The relay's own address, which the tool connects to in place of
    /// the browser's.
    pub(crate) fn address(&self) -> SocketAddr {
        self.address
    }

    /// Reads the CPU time of the tree under `pid` at each mark.
    pub(crate) fn follow(&self, pid: u32) {
        self.shared.tool.store(pid, Ordering::Relaxed);
    }

    /// The commands counted so far.
    #[cfg(test)]
    pub(crate) fn commands(&self) -> u64 {
        self.shared.commands.load(Ordering::SeqCst)
    }

    /// The `/start` and `/end` marks, each once asked.
    pub(crate) fn marks(&self) -> (Option<Mark>, Option<Mark>) {
        *self
            .shared
            .marks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

impl Drop for Relay {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::Relaxed);
        // Wakes the accepting thread, which then sees the stop.
        let _ = TcpStream::connect(self.address);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The request head of `stream`, up to its blank line, with any bytes read
/// past it.
fn head(stream: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>)> {
    let mut read = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        if let Some(end) = read.windows(4).position(|window| window == b"\r\n\r\n") {
            let rest = read.split_off(end + 4);
            return Ok((read, rest));
        }
        anyhow::ensure!(read.len() <= HEAD_LIMIT, "a request head over 16 KiB");
        let length = stream.read(&mut buffer).context("reading a request")?;
        anyhow::ensure!(length > 0, "a connection closed before its request head");
        read.extend_from_slice(&buffer[..length]);
    }
}

/// Serves one connection: a mark, or a relayed request to the browser.
fn connection(mut client: TcpStream, browser: u16, shared: &Shared) -> Result<()> {
    if shared.stop.load(Ordering::Relaxed) {
        return Ok(());
    }
    let (request, rest) = head(&mut client)?;
    let text = String::from_utf8_lossy(&request);
    let target = text
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or_default();
    if let Some(which) = match target {
        "/start" => Some(0),
        "/end" => Some(1),
        _ => None,
    } {
        let mark = Mark {
            commands: shared.commands.load(Ordering::SeqCst),
            cpu_seconds: match shared.tool.load(Ordering::Relaxed) {
                0 => 0.0,
                pid => sampler::cpu_seconds(pid),
            },
            at: trial::now(),
        };
        let answer = {
            let mut marks = shared.marks.lock().unwrap_or_else(PoisonError::into_inner);
            let slot = if which == 0 {
                &mut marks.0
            } else {
                &mut marks.1
            };
            if slot.is_some() {
                "HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            } else {
                *slot = Some(mark);
                "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
            }
        };
        client.write_all(answer.as_bytes())?;
        return Ok(());
    }
    let upgrade = text.to_ascii_lowercase().contains("upgrade: websocket");
    let mut upstream =
        TcpStream::connect(("127.0.0.1", browser)).context("connecting to the browser")?;
    upstream.write_all(&request)?;
    let mut frames = Frames::default();
    if upgrade {
        shared
            .commands
            .fetch_add(frames.feed(&rest), Ordering::SeqCst);
    }
    upstream.write_all(&rest)?;
    let (mut from_browser, mut to_client) = (upstream.try_clone()?, client.try_clone()?);
    let back = std::thread::spawn(move || {
        let _ = std::io::copy(&mut from_browser, &mut to_client);
        let _ = to_client.shutdown(Shutdown::Write);
    });
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let length = match client.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(length) => length,
        };
        let bytes = &buffer[..length];
        if upgrade {
            shared
                .commands
                .fetch_add(frames.feed(bytes), Ordering::SeqCst);
        }
        if upstream.write_all(bytes).is_err() {
            break;
        }
    }
    let _ = upstream.shutdown(Shutdown::Write);
    let _ = back.join();
    Ok(())
}

#[cfg(test)]
mod tests;
