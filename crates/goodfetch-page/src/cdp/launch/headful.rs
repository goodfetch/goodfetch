//! A headful browser's virtual display on Linux, where the engine's
//! environment names no display server (ADR 0035): one Xvfb for each
//! headful browser process, started before it through the engine's
//! `children` module on a display number Xvfb picks itself
//! (`-displayfd`), so two browsers never share one, and stopped once its
//! browser exited, which leaves no display behind. Xvfb is a translation
//! point (ADR 0127): what its start came to becomes a [`Failure`], and
//! each one, an unknown one too, fails the launch with
//! `capability_unavailable` naming `browser.headless`, whose fix is a
//! headless launch.

#[cfg(test)]
mod tests;

use std::io;
use std::process::{ExitStatus, Stdio};
use std::time::Duration;

use goodfetch_engine::CallError;
use goodfetch_engine::children::Children;
use goodfetch_engine::clock::Clock;
use goodfetch_protocol::page::BROWSER_HEADLESS;
use goodfetch_protocol::settings::{
    CAPABILITY_UNAVAILABLE, CapabilityUnavailableDetails, SettingsLayer,
};
use tokio::io::{AsyncBufReadExt as _, BufReader};
use tokio::process::{Child, ChildStdout, Command};

/// The virtual display server, found on the engine's `PATH`.
const XVFB: &str = "Xvfb";

/// Its arguments: the display number it picked, written on its standard
/// output once it listens; one screen of a common desktop size; no TCP
/// listener.
const ARGUMENTS: [&str; 7] = [
    "-displayfd",
    "1",
    "-screen",
    "0",
    "1920x1080x24",
    "-nolisten",
    "tcp",
];

/// How long Xvfb has to name its display.
const READY: Duration = Duration::from_secs(10);

/// How long Xvfb has to exit once asked, after which it is killed.
const STOP: Duration = Duration::from_secs(2);

/// What `capability_unavailable` names as the thing a headful launch
/// asked for.
const KIND: &str = "virtual_display";

/// A virtual display one headful browser shows its window on.
pub(super) struct Display {
    /// Its number, `:<number>` in `DISPLAY`.
    number: u32,
    /// Xvfb's process.
    process: Child,
    /// Its standard output, held open so it never writes to a closed pipe.
    _output: BufReader<ChildStdout>,
    /// Its process group, killed with the value.
    group: Children,
}

impl Display {
    /// Starts Xvfb and waits, on `clock`, for the display it names.
    ///
    /// # Errors
    ///
    /// A [`Failure`] when Xvfb does not start or names no display.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future kills Xvfb's process group.
    pub(super) async fn start(clock: &Clock) -> Result<Self, Failure> {
        let mut command = Command::new(XVFB);
        command
            .args(ARGUMENTS)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut group = Children::new().map_err(refused)?;
        let mut process = group.spawn(command).map_err(refused)?;
        let stdout = process
            .stdout
            .take()
            .expect("invariant: Xvfb's standard output is piped");
        let mut output = BufReader::new(stdout);
        let mut line = String::new();
        let read = tokio::select! {
            read = output.read_line(&mut line) => Some(read),
            () = clock.sleep(READY) => None,
        };
        let observed = match read {
            None => Observed::Silent,
            Some(Ok(0)) => Observed::Exited(process.wait().await),
            Some(Ok(_)) => Observed::Wrote(line),
            Some(Err(error)) => Observed::Unreadable(error),
        };
        match display_of(observed) {
            Ok(number) => Ok(Self {
                number,
                process,
                _output: output,
                group,
            }),
            Err(failure) => {
                drop(group);
                Err(failure)
            }
        }
    }

    /// Its `DISPLAY` value.
    pub(super) fn variable(&self) -> String {
        format!(":{}", self.number)
    }

    /// Asks Xvfb to exit, which removes its display's lock and socket,
    /// kills it if it has not within [`STOP`] on `clock`, and kills what
    /// is left of its group.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future kills Xvfb's process group.
    pub(super) async fn stop(self, clock: &Clock) {
        let Self {
            mut process, group, ..
        } = self;
        if let Some(id) = process.id().and_then(|id| i32::try_from(id).ok()) {
            #[expect(
                unsafe_code,
                reason = "Xvfb removes its display's lock only on a signal it handles, sent through libc (ADR 0024)"
            )]
            // SAFETY: `kill` takes a process ID and a signal, no pointer; a
            // process already gone answers `ESRCH`, which leaves nothing to
            // do.
            unsafe {
                libc::kill(id, libc::SIGTERM);
            }
            let exited = tokio::select! {
                status = process.wait() => status.is_ok(),
                () = clock.sleep(STOP) => false,
            };
            if !exited {
                let _ = process.start_kill();
                let _ = process.wait().await;
            }
        }
        drop(group);
    }
}

/// What a started Xvfb came to, as its output shows it.
#[derive(Debug)]
enum Observed {
    /// It exited, or its end could not be read, before it named a display.
    Exited(io::Result<ExitStatus>),
    /// The first line it wrote.
    Wrote(String),
    /// Its output, unreadable as a line of text.
    Unreadable(io::Error),
    /// It wrote nothing within [`READY`].
    Silent,
}

/// Why no virtual display started.
#[derive(Debug, thiserror::Error)]
pub(super) enum Failure {
    /// Xvfb is not on the engine's `PATH`.
    #[error("Xvfb is not installed")]
    NotInstalled,
    /// The OS refused to start it.
    #[error("Xvfb did not start: {0}")]
    FailedToStart(io::Error),
    /// It exited before naming a display.
    #[error("Xvfb exited before naming its display ({0})")]
    ExitedEarly(ExitStatus),
    /// Anything else: what it was, kept and logged.
    #[error("Xvfb named no display")]
    Unrecognized(Unrecognized),
}

/// A start of Xvfb the translation point does not know: what it was,
/// kept and logged, never returned.
pub(super) struct Unrecognized {
    #[expect(
        dead_code,
        reason = "ADR 0127: the text is kept for the failure's life and logged where it is read, and no code reads it"
    )]
    text: String,
}

impl Unrecognized {
    /// The unknown start `text` describes, logged now.
    fn new(text: String) -> Self {
        tracing::warn!(
            answer = %text,
            "Xvfb started in a way its translation point does not recognize"
        );
        Self { text }
    }
}

impl std::fmt::Debug for Unrecognized {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("Unrecognized")
            .finish_non_exhaustive()
    }
}

/// The display number `observed` names, or the failure it is.
fn display_of(observed: Observed) -> Result<u32, Failure> {
    match observed {
        Observed::Wrote(line) => line
            .trim()
            .parse()
            .map_err(|_| Failure::Unrecognized(Unrecognized::new(format!("wrote {line:?}")))),
        Observed::Exited(Ok(status)) => Err(Failure::ExitedEarly(status)),
        Observed::Exited(Err(error)) => Err(Failure::Unrecognized(Unrecognized::new(format!(
            "its end could not be read: {error}"
        )))),
        Observed::Unreadable(error) => Err(Failure::Unrecognized(Unrecognized::new(format!(
            "its output could not be read: {error}"
        )))),
        Observed::Silent => Err(Failure::Unrecognized(Unrecognized::new(format!(
            "it named no display within {} s",
            READY.as_secs()
        )))),
    }
}

/// The failure the OS's refusal to start Xvfb, `error`, is.
fn refused(error: io::Error) -> Failure {
    if error.kind() == io::ErrorKind::NotFound {
        Failure::NotInstalled
    } else {
        Failure::FailedToStart(error)
    }
}

impl Failure {
    /// `capability_unavailable` for a headful launch that `layer` asked
    /// for with `browser.headless`, which shows its window on no display.
    pub(super) fn call_error(&self, layer: SettingsLayer) -> CallError {
        let key = BROWSER_HEADLESS.name;
        let remedy = match self {
            Self::NotInstalled => "install Xvfb (the `xvfb` package)",
            Self::FailedToStart(_) | Self::ExitedEarly(_) | Self::Unrecognized(_) => {
                "see the engine's log for why Xvfb did not start"
            }
        };
        CallError::new(
            &CAPABILITY_UNAVAILABLE,
            format!(
                "A headful browser shows its window on a virtual display where no display server runs, and none started: {self}. {remedy}, or set `{key}: true`."
            ),
            &CapabilityUnavailableDetails {
                key: key.to_owned(),
                layer,
                kind: KIND.to_owned(),
                fix: format!("`{key}: true`"),
            },
        )
    }
}
