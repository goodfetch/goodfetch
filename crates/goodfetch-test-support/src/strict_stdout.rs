//! The strict reader of a child's stdout (ADRs 0069, 0070): stdout is the
//! protocol channel, so every line on it must be one JSON-RPC message, and
//! anything else fails the test, with the offending bytes and the child's
//! stderr, which is kept apart, shown.

use std::fmt;
use std::io::{self, BufRead as _, BufReader, Cursor, Read};
use std::process::Output;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};

use crate::process::Process;

/// How long [`StrictStdout`] waits for each line.
pub const MESSAGE_DEADLINE: Duration = Duration::from_secs(10);

/// How long a failure waits for more of the child's stderr, once nothing
/// new has come, before showing what it has.
pub const STDERR_GRACE: Duration = Duration::from_secs(1);

/// The longest a failure waits for the child's stderr in all, for a child
/// that keeps logging (at `trace`, mid-Run) and is never quiet for
/// [`STDERR_GRACE`]: it then shows what came so far.
pub const STDERR_LIMIT: Duration = Duration::from_secs(3);

/// What the stdout reader thread sends.
type Lines = Receiver<io::Result<Chunk>>;

/// A child's stdout, read as newline-delimited JSON-RPC: it yields each
/// message as a plain JSON value and fails the test on any line that is not
/// one, while the child's stderr is kept apart and shown on failure.
#[derive(Debug)]
pub struct StrictStdout {
    lines: Lines,
    stderr: Receiver<Vec<u8>>,
    stderr_read: Vec<u8>,
    deadline: Duration,
    read: usize,
    waiting: Option<io::Result<Chunk>>,
}

impl StrictStdout {
    /// Reads `stdout` as newline-delimited JSON-RPC, keeping `stderr` apart.
    pub fn new(stdout: impl Read + Send + 'static, stderr: impl Read + Send + 'static) -> Self {
        Self {
            lines: read_lines(stdout),
            stderr: read_stderr(stderr),
            stderr_read: Vec::new(),
            deadline: MESSAGE_DEADLINE,
            read: 0,
            waiting: None,
        }
    }

    /// Reads the stdout of a process started through `env.spawn`, whose
    /// stderr the command piped (`command.stderr(Stdio::piped())`). Both
    /// pipes are taken; run the child with logging at `trace`, so a log line
    /// that leaks onto stdout fails the test.
    ///
    /// # Errors
    ///
    /// Its stdout or stderr is not piped, or was taken already.
    pub fn of(process: &Process) -> Result<Self, StrictStdoutError> {
        let mut child = process.child();
        let kind = match (&child.stdout, &child.stderr) {
            (None, _) => StrictStdoutErrorKind::StdoutNotPiped,
            (Some(_), None) => StrictStdoutErrorKind::StderrNotPiped,
            (Some(_), Some(_)) => {
                let stdout = child.stdout.take().expect("invariant: matched as piped");
                let stderr = child.stderr.take().expect("invariant: matched as piped");
                return Ok(Self::new(stdout, stderr));
            }
        };
        Err(StrictStdoutError {
            kind,
            stderr: String::new(),
        })
    }

    /// Reads the stdout of a child that has exited, such as the output of
    /// `command(&env, program)`, keeping its stderr apart.
    pub fn from_output(output: &Output) -> Self {
        Self::new(
            Cursor::new(output.stdout.clone()),
            Cursor::new(output.stderr.clone()),
        )
    }

    /// The same reader, waiting at most `deadline` for each line instead of
    /// [`MESSAGE_DEADLINE`].
    #[must_use]
    pub fn within(self, deadline: Duration) -> Self {
        Self { deadline, ..self }
    }

    /// The next message, `None` once stdout ends after a whole line.
    ///
    /// # Errors
    ///
    /// The line that is not one JSON-RPC message, or why no line came, with
    /// the child's stderr.
    pub fn try_next(&mut self) -> Result<Option<serde_json::Value>, StrictStdoutError> {
        let received = match self.waiting.take() {
            Some(waiting) => Ok(waiting),
            None => self.lines.recv_timeout(self.deadline),
        };
        let chunk = match received {
            Ok(Ok(chunk)) => chunk,
            Ok(Err(error)) => return Err(self.fail(StrictStdoutErrorKind::Read(error))),
            Err(RecvTimeoutError::Timeout) => {
                return Err(self.fail(StrictStdoutErrorKind::Deadline(self.deadline)));
            }
            Err(RecvTimeoutError::Disconnected) => return Ok(None),
        };
        self.read += 1;
        message(chunk, self.read)
            .map(Some)
            .map_err(|kind| self.fail(kind))
    }

    /// Whether the child writes nothing on its stdout for `quiet`, a short
    /// duration the test sets, such as while a call it has not let through
    /// must stay unanswered. A line written meanwhile is kept for the next
    /// read; a stdout that ended is not quiet.
    pub fn is_quiet_for(&mut self, quiet: Duration) -> bool {
        if self.waiting.is_some() {
            return false;
        }
        match self.lines.recv_timeout(quiet) {
            Ok(read) => {
                self.waiting = Some(read);
                false
            }
            Err(RecvTimeoutError::Timeout) => true,
            Err(RecvTimeoutError::Disconnected) => false,
        }
    }

    /// Everything the child wrote on its stderr, once it closed it: read it
    /// after the child exited, such as to check its logs.
    pub fn stderr_to_end(&mut self) -> String {
        while let Ok(bytes) = self.stderr.recv() {
            self.stderr_read.extend_from_slice(&bytes);
        }
        String::from_utf8_lossy(&self.stderr_read).into_owned()
    }

    /// The failure of `kind`, with the child's stderr: all of it if the
    /// child closes its stderr, or what came until it was quiet for
    /// [`STDERR_GRACE`], or for [`STDERR_LIMIT`] in all.
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper's wall-clock limit on a child's output, outside any async worker (ADR 0062)"
    )]
    fn fail(&mut self, kind: StrictStdoutErrorKind) -> StrictStdoutError {
        let limit = Instant::now() + STDERR_LIMIT;
        loop {
            let left = limit.saturating_duration_since(Instant::now());
            match self.stderr.recv_timeout(STDERR_GRACE.min(left)) {
                Ok(bytes) => self.stderr_read.extend_from_slice(&bytes),
                Err(_) => break,
            }
            if left.is_zero() {
                break;
            }
        }
        StrictStdoutError {
            kind,
            stderr: String::from_utf8_lossy(&self.stderr_read).into_owned(),
        }
    }
}

impl Iterator for StrictStdout {
    type Item = serde_json::Value;

    /// The next message, `None` once stdout ends after a whole line.
    ///
    /// # Panics
    ///
    /// With a [`StrictStdoutError`] as its payload, printed first, wherever
    /// [`StrictStdout::try_next`] fails: this fails the test.
    fn next(&mut self) -> Option<serde_json::Value> {
        match self.try_next() {
            Ok(message) => message,
            Err(error) => {
                #[expect(
                    clippy::print_stderr,
                    reason = "the failure's reason, since a typed panic payload prints none (ADR 0069)"
                )]
                {
                    eprintln!("{error}");
                }
                std::panic::panic_any(error)
            }
        }
    }
}

/// The message a chunk of stdout holds, or why it holds none.
fn message(chunk: Chunk, number: usize) -> Result<serde_json::Value, StrictStdoutErrorKind> {
    let bytes = match chunk {
        Chunk::Line(bytes) => bytes,
        Chunk::Partial(bytes) => {
            return Err(StrictStdoutErrorKind::PartialLine(Line { number, bytes }));
        }
    };
    let line = Line { number, bytes };
    if std::str::from_utf8(&line.bytes).is_err() {
        return Err(StrictStdoutErrorKind::NotUtf8(line));
    }
    let framed = line
        .bytes
        .first()
        .zip(line.bytes.last())
        .is_some_and(|(first, last)| !first.is_ascii_whitespace() && !last.is_ascii_whitespace());
    let Some(message) = framed
        .then(|| serde_json::from_slice::<serde_json::Value>(&line.bytes).ok())
        .flatten()
    else {
        return Err(StrictStdoutErrorKind::NotJson(line));
    };
    if !is_json_rpc(&message) {
        return Err(StrictStdoutErrorKind::NotJsonRpc(line));
    }
    Ok(message)
}

/// Whether `value` is one JSON-RPC 2.0 message: a request (`method` and
/// `id`), a notification (`method`, no `id`) or a reply (`id` and either
/// `result` or `error`), with no member JSON-RPC does not define. A batch
/// is none.
pub(crate) fn is_json_rpc(value: &serde_json::Value) -> bool {
    let Some(message) = value.as_object() else {
        return false;
    };
    let only = |members: &[&str]| message.keys().all(|key| members.contains(&key.as_str()));
    if message.get("jsonrpc").and_then(serde_json::Value::as_str) != Some("2.0") {
        return false;
    }
    if let Some(method) = message.get("method") {
        return method.is_string()
            && message
                .get("id")
                .is_none_or(|id| id.is_string() || id.is_number())
            && message
                .get("params")
                .is_none_or(|params| params.is_object() || params.is_array())
            && only(&["jsonrpc", "method", "id", "params"]);
    }
    let id = message
        .get("id")
        .is_some_and(|id| id.is_string() || id.is_number() || id.is_null());
    let outcome = match (message.get("result"), message.get("error")) {
        (Some(_), None) => true,
        (None, Some(error)) => is_error_object(error),
        _ => false,
    };
    id && outcome && only(&["jsonrpc", "id", "result", "error"])
}

/// Whether `value` is a JSON-RPC 2.0 error object: an integer `code`, a
/// string `message` and an optional `data`.
fn is_error_object(value: &serde_json::Value) -> bool {
    value.as_object().is_some_and(|error| {
        error.get("code").is_some_and(serde_json::Value::is_i64)
            && error
                .get("message")
                .is_some_and(serde_json::Value::is_string)
            && error
                .keys()
                .all(|key| ["code", "message", "data"].contains(&key.as_str()))
    })
}

/// Starts the thread that reads `stdout` a line at a time.
fn read_lines(stdout: impl Read + Send + 'static) -> Lines {
    let (sender, receiver) = mpsc::channel();
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper's reader thread, which ends when the child's output does (ADR 0062)"
    )]
    std::thread::spawn(move || {
        let mut stdout = BufReader::new(stdout);
        loop {
            let mut bytes = Vec::new();
            let read = match stdout.read_until(b'\n', &mut bytes) {
                Ok(0) => break,
                Ok(_) if bytes.pop_if(|last| *last == b'\n').is_some() => Ok(Chunk::Line(bytes)),
                Ok(_) => Ok(Chunk::Partial(bytes)),
                Err(error) => Err(error),
            };
            let last = !matches!(read, Ok(Chunk::Line(_)));
            if sender.send(read).is_err() || last {
                break;
            }
        }
    });
    receiver
}

/// Starts the thread that reads `stderr` as it comes, until it closes or
/// fails.
fn read_stderr(stderr: impl Read + Send + 'static) -> Receiver<Vec<u8>> {
    let (sender, receiver) = mpsc::channel();
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper's reader thread, which ends when the child's output does (ADR 0062)"
    )]
    std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut buffer = [0_u8; 8192];
        loop {
            match stderr.read(&mut buffer) {
                Ok(0) => break,
                Ok(read) => {
                    if sender.send(buffer[..read].to_vec()).is_err() {
                        break;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    });
    receiver
}

/// What the stdout reader thread read, without the newline.
#[derive(Debug)]
enum Chunk {
    /// A line its newline ended.
    Line(Vec<u8>),
    /// The bytes before the end of stdout, which no newline ended.
    Partial(Vec<u8>),
}

/// A line of a child's stdout, without its newline.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    /// Its number, from 1.
    pub number: usize,
    /// Its bytes, as the child wrote them.
    pub bytes: Vec<u8>,
}

impl fmt::Display for Line {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "line {}: \"{}\"",
            self.number,
            self.bytes.escape_ascii()
        )
    }
}

/// Why a child's stdout failed the test, with what the child wrote on its
/// stderr until then.
#[derive(Debug, thiserror::Error)]
#[error("{kind}\nthe child's stderr:\n{stderr}")]
pub struct StrictStdoutError {
    /// What was wrong.
    pub kind: StrictStdoutErrorKind,
    /// The child's stderr, as text with invalid UTF-8 replaced.
    pub stderr: String,
}

/// What was wrong with a child's stdout.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum StrictStdoutErrorKind {
    /// A line is not UTF-8.
    #[error("the child's stdout holds a line that is not UTF-8, {0}")]
    NotUtf8(Line),
    /// Stdout ended after bytes that no newline ended.
    #[error("the child's stdout ended inside a line, {0}")]
    PartialLine(Line),
    /// A line is not one JSON value with nothing around it.
    #[error("the child's stdout holds a line that is not JSON, {0}")]
    NotJson(Line),
    /// A line is JSON but not a JSON-RPC 2.0 message.
    #[error("the child's stdout holds a line that is not a JSON-RPC message, {0}")]
    NotJsonRpc(Line),
    /// No line came within the deadline.
    #[error("the child wrote no line on its stdout within {0:?}")]
    Deadline(Duration),
    /// Reading its stdout failed.
    #[error("reading the child's stdout failed: {0}")]
    Read(io::Error),
    /// The child's stdout is not piped, or was taken already.
    #[error("the child's stdout is not piped")]
    StdoutNotPiped,
    /// The child's stderr is not piped, or was taken already.
    #[error("the child's stderr is not piped: give its command `.stderr(Stdio::piped())`")]
    StderrNotPiped,
}

impl StrictStdoutErrorKind {
    /// The offending line, if a line was.
    pub fn line(&self) -> Option<&Line> {
        match self {
            Self::NotUtf8(line)
            | Self::PartialLine(line)
            | Self::NotJson(line)
            | Self::NotJsonRpc(line) => Some(line),
            Self::Deadline(_) | Self::Read(_) | Self::StdoutNotPiped | Self::StderrNotPiped => None,
        }
    }
}

#[cfg(test)]
mod tests;
