//! The counting relay of the Collie scenario (issue 2282): a TCP relay on
//! loopback between a tool and the remote-debugging endpoint of the browser
//! `xtask compare` launched, which counts the CDP commands the tool sends.
//! Each tool, Collie's engine or Playwright, reaches the browser only
//! through it, so both tools' commands are counted the same way, from
//! outside them: one command is one whole WebSocket data message from the
//! tool to the browser (text or binary, however many frames carry it;
//! pings, pongs and closes are no command).
//!
//! It also counts the rounds the tool waited on the browser (issue 2319):
//! on each connection, a round starts whenever the tool sends a command
//! while none of its commands is awaiting its reply, so commands sent
//! together, or while others are in flight, share one round, and commands
//! each awaited before the next count one round each. A reply is a message
//! from the browser that starts `{"id":`, as Chrome writes every reply;
//! an event starts `{"method":`.
//!
//! The relay also answers the driver's two marks, `GET /start` and
//! `GET /end`, around the part of the trial that is measured: each mark
//! records the command and round counts so far, the CPU time of the tool's
//! process tree ([`Relay::follow`]) and the moment, before it answers, so
//! the figures between the marks are the measured part's alone. Without a
//! browser ([`Relay::marks_only`]), it answers only the marks; given a
//! reader of the tool's engine's own counts ([`Relay::marks_reading`]),
//! each mark records them too, read outside the measured part: before the
//! start mark's CPU time and moment, after the end mark's.
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
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use anyhow::{Context as _, Result};

use super::{sampler, trial};

/// The longest request head the relay reads.
const HEAD_LIMIT: usize = 16 * 1024;

/// What a reply's payload starts with.
const REPLY: &[u8] = br#"{"id":"#;

/// The streaming count of whole data messages in one side's WebSocket
/// frames (RFC 6455, section 5.2), masked or not, fed the bytes in any
/// chunks.
#[derive(Debug, Default)]
pub(crate) struct Frames {
    /// The current frame's header bytes read so far.
    header: Vec<u8>,
    /// The current frame's payload bytes left to skip, once its header is
    /// whole.
    payload_left: u64,
    /// The current frame's mask key, when it is masked.
    mask: Option<[u8; 4]>,
    /// The current frame's payload bytes read so far.
    payload_read: u64,
    /// Whether the current frame carries a data message's bytes.
    data: bool,
    /// The current data message's first bytes, up to [`REPLY`]'s length.
    start: Vec<u8>,
}

/// What a run of bytes ended.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Ended {
    /// Whole data messages.
    pub(crate) messages: u64,
    /// Those of them that start as a reply does.
    pub(crate) replies: u64,
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
    pub(crate) fn feed(&mut self, bytes: &[u8]) -> u64 {
        self.read(bytes).messages
    }

    /// Reads `bytes`, the next of the stream, and returns the data messages
    /// they ended and how many of those were replies.
    pub(crate) fn read(&mut self, mut bytes: &[u8]) -> Ended {
        let mut read = Ended::default();
        while !bytes.is_empty() {
            if self.payload_left > 0 {
                let taken = usize::try_from(self.payload_left)
                    .unwrap_or(usize::MAX)
                    .min(bytes.len());
                if self.data {
                    for &byte in &bytes[..taken] {
                        if self.start.len() >= REPLY.len() {
                            break;
                        }
                        let key = self.mask.map_or(0, |mask| {
                            mask[usize::try_from(self.payload_read % 4).unwrap_or_default()]
                        });
                        self.start.push(byte ^ key);
                        self.payload_read += 1;
                    }
                }
                bytes = &bytes[taken..];
                self.payload_left -= taken as u64;
                if self.payload_left == 0 {
                    self.frame_ended(&mut read);
                }
                continue;
            }
            self.header.push(bytes[0]);
            bytes = &bytes[1..];
            if self.header.len() < 2 || self.header.len() < self.header_length() {
                continue;
            }
            let opcode = self.header[0] & 0x0f;
            self.data = matches!(opcode, 0..=2);
            if matches!(opcode, 1 | 2) {
                self.start.clear();
            }
            self.mask = (self.header[1] & 0x80 != 0).then(|| {
                let at = self.header.len() - 4;
                [
                    self.header[at],
                    self.header[at + 1],
                    self.header[at + 2],
                    self.header[at + 3],
                ]
            });
            self.payload_read = 0;
            self.payload_left = self.payload_length();
            if self.payload_left == 0 {
                self.frame_ended(&mut read);
            }
        }
        read
    }

    /// Ends the current frame, whose header is whole and whose payload is
    /// read, counting the data message it ends into `read`.
    fn frame_ended(&mut self, read: &mut Ended) {
        let fin = self.header[0] & 0x80 != 0;
        if self.data && fin {
            read.messages += 1;
            if self.start == REPLY {
                read.replies += 1;
            }
        }
        self.header.clear();
    }
}

/// The end of an HTTP head, its blank line, found in bytes read in any
/// chunks: the browser's answer to the WebSocket handshake comes before
/// its first frame.
#[derive(Debug, Default)]
struct HeadEnd {
    /// How many of the blank line's bytes the last bytes read matched.
    matched: usize,
    /// Whether the blank line was found.
    found: bool,
}

impl HeadEnd {
    const BLANK_LINE: &[u8] = b"\r\n\r\n";

    /// The part of `bytes`, the next of the stream, past the head: none
    /// until its blank line, all of them after it.
    fn past<'a>(&mut self, bytes: &'a [u8]) -> &'a [u8] {
        if self.found {
            return bytes;
        }
        for (index, &byte) in bytes.iter().enumerate() {
            self.matched = if byte == Self::BLANK_LINE[self.matched] {
                self.matched + 1
            } else {
                usize::from(byte == b'\r')
            };
            if self.matched == Self::BLANK_LINE.len() {
                self.found = true;
                return &bytes[index + 1..];
            }
        }
        &[]
    }
}

/// One connection's commands awaiting their reply, which start a round
/// whenever the tool sends with none in flight.
#[derive(Debug, Default)]
struct Awaited {
    in_flight: AtomicI64,
}

impl Awaited {
    /// The tool sent `commands`: a round starts when none was in flight.
    fn sent(&self, commands: u64, rounds: &AtomicU64) {
        if commands == 0 {
            return;
        }
        let commands = i64::try_from(commands).unwrap_or(i64::MAX);
        if self.in_flight.fetch_add(commands, Ordering::SeqCst) <= 0 {
            rounds.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// The browser answered `replies` commands.
    fn answered(&self, replies: u64) {
        let replies = i64::try_from(replies).unwrap_or(i64::MAX);
        self.in_flight.fetch_sub(replies, Ordering::SeqCst);
    }
}

/// The Counted work a tool's engine reported (ADR 0140): the C5 `answers`
/// and C1 `sends` it counted since it started.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EngineCounts {
    /// The paused requests it answered, C5 `answers`.
    pub(crate) paused: u64,
    /// The CDP commands it sent, C1 `sends`.
    pub(crate) commands: u64,
}

/// Reads the tool's engine's counts; none when they cannot be read.
pub(crate) type EngineReader = Box<dyn Fn() -> Option<EngineCounts> + Send + Sync>;

/// What a mark recorded.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Mark {
    /// The commands the relay had counted.
    pub(crate) commands: u64,
    /// The rounds the tool had waited on the browser.
    pub(crate) rounds: u64,
    /// The CPU seconds of the tool's process tree.
    pub(crate) cpu_seconds: f64,
    /// The CPU seconds of the tool's own process, its children's left out
    /// (issue 2406).
    pub(crate) driver_cpu_seconds: f64,
    pub(crate) at: Instant,
    /// The tool's engine's counts, when the relay reads them and could.
    pub(crate) engine: Option<EngineCounts>,
}

/// What the relay's threads share.
struct Shared {
    commands: AtomicU64,
    rounds: AtomicU64,
    /// The process whose tree a mark reads the CPU time of; 0 for none.
    tool: AtomicU32,
    marks: Mutex<(Option<Mark>, Option<Mark>)>,
    /// What reads the tool's engine's counts at each mark, if anything.
    engine: Option<EngineReader>,
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
        Self::bind(Some(browser), None)
    }

    /// Starts answering the marks alone, for a trial whose tool launches
    /// its own browser.
    pub(crate) fn marks_only() -> Result<Self> {
        Self::bind(None, None)
    }

    /// Starts answering the marks alone, each also recording what `engine`
    /// reads of the tool's engine's counts.
    pub(crate) fn marks_reading(engine: EngineReader) -> Result<Self> {
        Self::bind(None, Some(engine))
    }

    fn bind(browser: Option<u16>, engine: Option<EngineReader>) -> Result<Self> {
        let listener =
            TcpListener::bind("127.0.0.1:0").context("binding the counting relay's port")?;
        let address = listener
            .local_addr()
            .context("reading the counting relay's port")?;
        let shared = Arc::new(Shared {
            commands: AtomicU64::new(0),
            rounds: AtomicU64::new(0),
            tool: AtomicU32::new(0),
            marks: Mutex::new((None, None)),
            engine,
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

/// Which of the two marks a request asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Which {
    Start,
    End,
}

/// Records the mark `which` once, and returns the answer to its request.
fn mark(shared: &Shared, which: Which) -> &'static str {
    // The engine's counts are read outside the measured part, so its
    // answering them is no part of the figures between the marks.
    let engine = || shared.engine.as_ref().and_then(|read| read());
    let before = if which == Which::Start {
        engine()
    } else {
        None
    };
    let (cpu_seconds, driver_cpu_seconds) = match shared.tool.load(Ordering::Relaxed) {
        0 => (0.0, 0.0),
        pid => (sampler::cpu_seconds(pid), sampler::own_cpu_seconds(pid)),
    };
    let mut mark = Mark {
        commands: shared.commands.load(Ordering::SeqCst),
        rounds: shared.rounds.load(Ordering::SeqCst),
        cpu_seconds,
        driver_cpu_seconds,
        at: trial::now(),
        engine: before,
    };
    if which == Which::End {
        mark.engine = engine();
    }
    let mut marks = shared.marks.lock().unwrap_or_else(PoisonError::into_inner);
    let slot = match which {
        Which::Start => &mut marks.0,
        Which::End => &mut marks.1,
    };
    if slot.is_some() {
        "HTTP/1.1 409 Conflict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
    } else {
        *slot = Some(mark);
        "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n"
    }
}

/// Serves one connection: a mark, or a relayed request to the browser,
/// when there is one.
fn connection(mut client: TcpStream, browser: Option<u16>, shared: &Shared) -> Result<()> {
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
        "/start" => Some(Which::Start),
        "/end" => Some(Which::End),
        _ => None,
    } {
        client.write_all(mark(shared, which).as_bytes())?;
        return Ok(());
    }
    let Some(browser) = browser else {
        client.write_all(
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
        )?;
        return Ok(());
    };
    let upgrade = text.to_ascii_lowercase().contains("upgrade: websocket");
    let mut upstream =
        TcpStream::connect(("127.0.0.1", browser)).context("connecting to the browser")?;
    upstream.write_all(&request)?;
    let awaited = Arc::new(Awaited::default());
    let mut frames = Frames::default();
    let mut sent = |bytes: &[u8]| {
        if upgrade {
            let commands = frames.feed(bytes);
            shared.commands.fetch_add(commands, Ordering::SeqCst);
            awaited.sent(commands, &shared.rounds);
        }
    };
    sent(&rest);
    upstream.write_all(&rest)?;
    let (mut from_browser, mut to_client) = (upstream.try_clone()?, client.try_clone()?);
    let answers = Arc::clone(&awaited);
    let back = std::thread::spawn(move || {
        let mut replies = Frames::default();
        let mut head = HeadEnd::default();
        let mut buffer = vec![0; 64 * 1024];
        loop {
            let length = match from_browser.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(length) => length,
            };
            let bytes = &buffer[..length];
            if upgrade {
                // Before the tool can see the replies, so its next command
                // finds them answered.
                answers.answered(replies.read(head.past(bytes)).replies);
            }
            if to_client.write_all(bytes).is_err() {
                break;
            }
        }
        let _ = to_client.shutdown(Shutdown::Write);
    });
    let mut buffer = vec![0; 64 * 1024];
    loop {
        let length = match client.read(&mut buffer) {
            Ok(0) | Err(_) => break,
            Ok(length) => length,
        };
        let bytes = &buffer[..length];
        sent(bytes);
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
