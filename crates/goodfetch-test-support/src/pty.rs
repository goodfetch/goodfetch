//! The CLI helper's pseudo-terminal runs (ADRs 0069 and 0073): `pty(&env,
//! executable)` runs the Product's executable a test gives under expectrl,
//! on a pseudo-terminal on Linux and macOS and on `ConPTY` on Windows, with
//! the test's cleared environment, `COLUMNS=100` and `TERM=xterm-256color`
//! and no `NO_COLOR`, so that colour, progress and person-only prompts show
//! as they do to a person. A test reads the terminal with
//! [`PtyRun::expect`], answers with [`PtyRun::send_line`] and ends with
//! [`PtyRun::finish`], which gives the whole transcript and the exit code
//! and appends the command to the test's command record, as `cli` does.
//!
//! Unlike `cli`, a run here never refuses an escape byte: a terminal is
//! where colour belongs. [`PtyOutput::stripped`] gives the transcript
//! without its escape sequences.
use std::ffi::OsStr;
use std::fmt;
use std::sync::LazyLock;
use std::time::Duration;

use expectrl::session::OsSession;
use expectrl::{Expect as _, Session};
use regex::bytes::Regex;

use crate::cli::{CliError, Invocation, record_command};
use crate::env::TestEnv;

/// How long [`PtyRun::expect`] and [`PtyRun::finish`] wait for the text or
/// the end they wait for: an upper bound for a loaded runner, never a
/// pause.
const DEADLINE: Duration = Duration::from_secs(60);

/// Every escape sequence a terminal program writes: CSI (colour, cursor
/// and erase), OSC (titles, links) up to its BEL or ST, and the two-byte
/// ones.
static ESCAPE_SEQUENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u)\x1b\[[0-?]*[ -/]*[@-~]|\x1b\][^\x07\x1b]*(?:\x07|\x1b\\)|\x1b[@-Z\\-_]")
        .expect("invariant: the escape-sequence pattern parses")
});

/// One SGR sequence, which sets colour and style, with its parameters.
static SGR: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?-u)\x1b\[([0-9;:]*)m").expect("invariant: the SGR pattern parses")
});

/// One run of a Product's executable on a pseudo-terminal, which
/// [`Pty::spawn`] starts.
pub struct Pty<'env> {
    env: &'env TestEnv,
    invocation: Invocation,
}

/// A run of `executable`, the path of the Product's executable the test
/// gives, on a pseudo-terminal (`ConPTY` on Windows), in the test's folder
/// with its cleared environment, `COLUMNS=100` and `TERM=xterm-256color`.
pub fn pty(env: &TestEnv, executable: impl AsRef<OsStr>) -> Pty<'_> {
    Pty {
        env,
        invocation: Invocation::of(
            env,
            executable,
            &[("COLUMNS", "100"), ("TERM", "xterm-256color")],
        ),
    }
}

impl<'env> Pty<'env> {
    /// Adds one argument.
    #[must_use]
    pub fn arg(mut self, argument: impl AsRef<str>) -> Self {
        self.invocation.arg(argument.as_ref());
        self
    }

    /// Adds these arguments.
    #[must_use]
    pub fn args<I>(self, arguments: I) -> Self
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        arguments.into_iter().fold(self, Pty::arg)
    }

    /// Sets one more variable, over the test's map and the helper's own.
    #[must_use]
    pub fn env(mut self, name: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> Self {
        self.invocation.env(name, value);
        self
    }

    /// Starts the executable on its terminal.
    pub fn spawn(self) -> Result<PtyRun<'env>, PtyError> {
        let (command, arguments) = self.invocation.into_parts();
        let mut session = Session::spawn(command).map_err(PtyError::Start)?;
        session.set_expect_timeout(Some(DEADLINE));
        Ok(PtyRun {
            env: self.env,
            session,
            arguments,
            transcript: Vec::new(),
        })
    }

    /// Runs the executable to its end, answering nothing, and gives its
    /// output.
    pub fn run(self) -> Result<PtyOutput, PtyError> {
        self.spawn()?.finish()
    }
}

/// A running executable on its pseudo-terminal.
pub struct PtyRun<'env> {
    env: &'env TestEnv,
    session: OsSession,
    arguments: Vec<String>,
    transcript: Vec<u8>,
}

impl PtyRun<'_> {
    /// Waits until the terminal shows `text`, such as a prompt.
    pub fn expect(&mut self, text: &str) -> Result<(), PtyError> {
        match self.session.expect(text) {
            Ok(found) => {
                self.transcript.extend_from_slice(found.as_bytes());
                Ok(())
            }
            Err(error) => Err(PtyError::NotShown {
                text: text.to_owned(),
                reason: error.to_string(),
            }),
        }
    }

    /// Types `line` and Enter, as a person answering a prompt does.
    pub fn send_line(&mut self, line: &str) -> Result<(), PtyError> {
        self.session
            .send_line(line)
            .map_err(|error| PtyError::Io(error.to_string()))
    }

    /// Reads the terminal until the executable ended, then appends the
    /// command and its exit code to the test's command record, and gives
    /// the whole transcript.
    pub fn finish(self) -> Result<PtyOutput, PtyError> {
        let Self {
            env,
            session,
            arguments,
            mut transcript,
        } = self;
        let (rest, exit) = end(session)?;
        transcript.extend_from_slice(&rest);
        record_command(env, &arguments, exit).map_err(PtyError::Record)?;
        let text = String::from_utf8(transcript).map_err(|_| PtyError::NotUtf8)?;
        Ok(PtyOutput { exit, text })
    }
}

/// The rest of the transcript once the terminal's other end closed, and
/// the exit code.
#[cfg(unix)]
fn end(mut session: OsSession) -> Result<(Vec<u8>, Option<i32>), PtyError> {
    use expectrl::Eof;
    use expectrl::process::unix::WaitStatus;

    let rest = session
        .expect(Eof)
        .map_err(|error| PtyError::NotEnded(error.to_string()))?
        .as_bytes()
        .to_vec();
    let status = session
        .get_process()
        .wait()
        .map_err(|error| PtyError::Io(error.to_string()))?;
    let exit = match status {
        WaitStatus::Exited(_, code) => Some(code),
        _ => None,
    };
    Ok((rest, exit))
}

/// The rest of the transcript once the executable exited and its
/// pseudo-console closed, and the exit code. `ConPTY` keeps its output open
/// until the pseudo-console closes, and closing it may wait for that output
/// to be read, so a second handle reads it meanwhile.
#[cfg(windows)]
#[expect(
    clippy::disallowed_methods,
    reason = "test support reads the pseudo-console on a thread of its own while it closes, outside any async worker (ADR 0062)"
)]
fn end(mut session: OsSession) -> Result<(Vec<u8>, Option<i32>), PtyError> {
    use std::io::{ErrorKind, Read as _};

    let mut rest = Vec::new();
    let mut buffer = [0_u8; 4096];
    loop {
        match session.try_read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => rest.extend_from_slice(&buffer[..read]),
            Err(error) if error.kind() == ErrorKind::WouldBlock => break,
            Err(error) => return Err(PtyError::Io(error.to_string())),
        }
    }
    let mut output = session
        .get_stream()
        .try_clone()
        .map_err(|error| PtyError::Io(error.to_string()))?;
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        output.read_to_end(&mut bytes).map(|_| bytes)
    });
    let deadline = u32::try_from(DEADLINE.as_millis())
        .expect("invariant: the deadline fits in a u32 of milliseconds");
    let code = session
        .get_process()
        .wait(Some(deadline))
        .map_err(|error| PtyError::NotEnded(error.to_string()))?;
    // Closing the pseudo-console ends its output.
    drop(session);
    let tail = reader
        .join()
        .map_err(|_| PtyError::Io("the terminal's reader panicked".to_owned()))?
        .map_err(|error| PtyError::Io(error.to_string()))?;
    rest.extend_from_slice(&tail);
    Ok((rest, Some(i32::from_ne_bytes(code.to_ne_bytes()))))
}

/// What a run on a pseudo-terminal showed: one transcript, since stdout
/// and stderr share the terminal, and the exit code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PtyOutput {
    /// The exit code; none when a signal ended the process.
    pub exit: Option<i32>,
    /// Everything the terminal showed, escape sequences and `\r\n` line
    /// ends included.
    pub text: String,
}

impl PtyOutput {
    /// The transcript without its escape sequences and with `\n` line
    /// ends, as a person reads it.
    pub fn stripped(&self) -> String {
        let text = ESCAPE_SEQUENCE.replace_all(self.text.as_bytes(), &b""[..]);
        String::from_utf8_lossy(&text).replace("\r\n", "\n")
    }

    /// Whether the transcript sets a colour or a style: an SGR sequence
    /// with a parameter other than a reset. A pseudo-console's own cursor
    /// and erase sequences are not colour.
    pub fn has_colour(&self) -> bool {
        SGR.captures_iter(self.text.as_bytes()).any(|sequence| {
            sequence[1]
                .split(|byte| *byte == b';' || *byte == b':')
                .any(|parameter| parameter.iter().any(|digit| *digit != b'0'))
        })
    }
}

impl fmt::Display for PtyOutput {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.exit {
            Some(code) => writeln!(formatter, "exit: {code}")?,
            None => writeln!(formatter, "exit: none, ended by a signal")?,
        }
        writeln!(formatter, "----- terminal -----")?;
        formatter.write_str(&self.text.escape_debug().to_string())
    }
}

/// Why a run on a pseudo-terminal fails.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum PtyError {
    /// The executable cannot be started on a pseudo-terminal.
    #[error("the executable cannot be started on a pseudo-terminal: {0}")]
    Start(expectrl::Error),
    /// The terminal never showed the text the test waited for before the
    /// executable ended or the deadline passed.
    #[error("the terminal never showed {text:?}: {reason}")]
    NotShown {
        /// The text.
        text: String,
        /// Why the wait ended.
        reason: String,
    },
    /// The executable had not ended by the deadline.
    #[error("the executable had not ended: {0}")]
    NotEnded(String),
    /// Reading or writing the terminal failed.
    #[error("the terminal cannot be read or written: {0}")]
    Io(String),
    /// The transcript is not UTF-8.
    #[error("the terminal's transcript is not UTF-8")]
    NotUtf8,
    /// The test's command record cannot be written.
    #[error(transparent)]
    Record(CliError),
}
