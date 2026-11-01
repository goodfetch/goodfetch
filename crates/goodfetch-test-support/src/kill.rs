//! Seeded kills of real processes (ADR 0060): a moment drawn from the
//! test's seed among the protocol events the caller names, and the kill of a
//! child (`kill -9`, or `TerminateProcess` on Windows) when it sends that
//! event, recorded in the Observation log.

use std::io::{self, BufRead as _, BufReader};
use std::process::ExitStatus;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;

use chacha20::ChaCha20Rng;
use chacha20::rand_core::Rng as _;
use serde::Serialize;

use crate::observation::{Fault, ObservationLog, Operation, Record};
use crate::process::Process;
use crate::seeded;

/// How long [`kill_at`] waits for each protocol message.
pub const KILL_DEADLINE: Duration = Duration::from_secs(10);

/// When to kill: at the `occurrence`-th protocol message, from 1, whose
/// JSON-RPC `method` is `event`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moment {
    /// The protocol event.
    pub event: String,
    /// Which occurrence of it, from 1.
    pub occurrence: u32,
}

impl Moment {
    /// The moment at this occurrence of this event.
    pub fn new(event: impl Into<String>, occurrence: u32) -> Self {
        Self {
            event: event.into(),
            occurrence,
        }
    }

    /// One of `events`, at one of its first `occurrences` occurrences,
    /// drawn from `seed` through `ChaCha20`, so a seed always draws the same
    /// moment.
    ///
    /// # Panics
    ///
    /// If `events` is empty or `occurrences` is 0.
    pub fn drawn(seed: u64, events: &[&str], occurrences: u32) -> Self {
        assert!(
            !events.is_empty() && occurrences > 0,
            "a moment is drawn from at least one event and one occurrence"
        );
        let mut stream = seeded::stream(seed, b"");
        let event = events[draw(&mut stream, events.len() as u64)];
        let occurrence = draw(&mut stream, u64::from(occurrences)) + 1;
        Self::new(
            event,
            u32::try_from(occurrence).expect("invariant: drawn below a u32"),
        )
    }
}

/// A number below `bound`, from the stream (with a modulo bias far below
/// anything a test notices).
fn draw(stream: &mut ChaCha20Rng, bound: u64) -> usize {
    usize::try_from(stream.next_u64() % bound).expect("invariant: drawn below a usize bound")
}

/// The kill: the message it came at, and how the process ended.
#[derive(Debug)]
pub struct Killed {
    /// The protocol message that named the moment's event.
    pub message: serde_json::Value,
    /// The killed process's exit status.
    pub status: ExitStatus,
}

/// Kills a process at `moment`, waiting at most [`KILL_DEADLINE`] for each
/// protocol message, and records the kill in `log`.
pub fn kill_at<R: Serialize>(
    process: &Process,
    moment: &Moment,
    log: &mut ObservationLog<R>,
) -> Result<Killed, KillAtError> {
    kill_at_within(process, moment, log, KILL_DEADLINE)
}

/// Kills a process at `moment`: it reads the process's standard output as
/// newline-delimited JSON-RPC, skipping lines that are not JSON, and kills
/// the process at the moment's message, before reading on. The kill is
/// recorded in `log` as a `Kill` fault on the process's program. Its
/// standard output must be piped, and is consumed.
pub fn kill_at_within<R: Serialize>(
    process: &Process,
    moment: &Moment,
    log: &mut ObservationLog<R>,
    deadline: Duration,
) -> Result<Killed, KillAtError> {
    let stdout = process.child().stdout.take().ok_or(KillAtError::NoStdout)?;
    // Bounded (STYLE.md): the reader waits while the test has not taken a
    // line, and ends once the receiver is gone.
    let (sender, receiver) = mpsc::sync_channel(16);
    #[expect(
        clippy::disallowed_methods,
        reason = "a test helper's reader thread, which ends when the child's output does (ADR 0062)"
    )]
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line).is_err() {
                break;
            }
        }
    });
    let mut seen = 0;
    let message = loop {
        let line = match receiver.recv_timeout(deadline) {
            Ok(line) => line.map_err(KillAtError::Read)?,
            Err(RecvTimeoutError::Timeout) => return Err(KillAtError::Deadline(deadline)),
            Err(RecvTimeoutError::Disconnected) => return Err(KillAtError::Ended),
        };
        let Ok(message) = serde_json::from_str::<serde_json::Value>(&line) else {
            continue;
        };
        if message["method"].as_str() == Some(moment.event.as_str()) {
            seen += 1;
            if seen == moment.occurrence {
                break message;
            }
        }
    };
    let status = {
        let mut child = process.child();
        child.kill().map_err(KillAtError::Kill)?;
        child.wait().map_err(KillAtError::Kill)?
    };
    log.push(Record::new(
        process.program(),
        Operation::Fault(Fault::Kill {
            event: moment.event.clone(),
            occurrence: moment.occurrence,
        }),
    ));
    Ok(Killed { message, status })
}

/// Why a process was not killed at its moment.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum KillAtError {
    /// Its standard output is not piped.
    #[error("the process's standard output is not piped")]
    NoStdout,
    /// No message came within the deadline.
    #[error("the process sent no message within {0:?}")]
    Deadline(Duration),
    /// Its output closed before the moment.
    #[error("the process's output closed before the moment")]
    Ended,
    /// Reading its output failed.
    #[error("reading the process's output failed: {0}")]
    Read(io::Error),
    /// Killing it or waiting for it failed.
    #[error("the process cannot be killed: {0}")]
    Kill(io::Error),
}

#[cfg(test)]
mod tests;
