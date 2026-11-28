//! A process Plugin instance's transition table (ADRs 0033 and 0128): its
//! state changes only through [`apply`], which its owner calls for every
//! event below. `starting` is the first process launched and awaiting its
//! `hello`, `restarting` the backoff wait and the new process until its
//! `hello`, and `quarantined` lasts the rest of the run.
//!
//! | Event, in the state: | `starting` | `ready` | `restarting` | `quarantined` | `stopped` |
//! |---|---|---|---|---|---|
//! | `HelloOk` | `ready`, opened | bug | `ready`, opened | bug | — |
//! | `HelloRefused` | `stopped`, refused | bug | `stopped`, refused | bug | — |
//! | `Crash` | `restarting` (1) | `restarting` (1) | `restarting` (1) | — | — |
//! | `PingMissed` | — | `restarting` (1) | — | — | — |
//! | `Call` | `starting`, queued | `ready`, sent | `restarting`, queued | `plugin_quarantined` | bug |
//! | `Result` | bug | `ready`, delivered | bug | bug | bug |
//! | `LateResult` | — | — | — | — | — |
//! | `RunEnded`, `Stop` | `stopped` | `stopped` | `stopped` | `stopped` | — |
//!
//! A dash changes nothing. (1) The crash rules: the in-flight calls fail
//! under them and the instance restarts after its backoff, unless the crash
//! is the third within 10 minutes of the run, which leads to `quarantined`
//! instead. A missed heartbeat outside `ready` is a late one, since only a
//! `ready` instance is pinged; an answer to a call abandoned after a cancel
//! or a stop is refused at the engine (ADR 0003), and its Effect-class call
//! stays uncertain. A Plugin's own stray message (a second `hello`, an
//! answer to no call) is decoded before this table and handled as a
//! malformed answer, so meeting `HelloOk`, `HelloRefused` or `Result` where
//! the table says bug is a bug in Goodfetch; a call on a quarantined
//! instance comes from the run's own need and answers `plugin_quarantined`.

#![deny(clippy::wildcard_enum_match_arm)]

use std::collections::VecDeque;
use std::time::Duration;

use goodfetch_protocol::plugin::PLUGIN_QUARANTINED;
use tokio::time::Instant;

use crate::transition::{InvalidTransition, Refusal};

/// Initial restart backoff after the first crash (ADR 0033).
pub const BACKOFF_INITIAL: Duration = Duration::from_secs(1);

/// Maximum restart backoff cap (ADR 0033): 1 s doubling to 30 s.
pub const BACKOFF_MAX: Duration = Duration::from_secs(30);

/// Sliding window for counting crashes toward quarantine (ADR 0033): 10 minutes.
pub const QUARANTINE_WINDOW: Duration = Duration::from_mins(10);

/// Number of crashes within [`QUARANTINE_WINDOW`] that quarantines the Plugin
/// for the rest of its run (ADR 0033).
pub const QUARANTINE_CRASHES: usize = 3;

/// Decision returned by [`CrashTracker::record`] when a process Plugin crashes
/// or misses its heartbeat.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CrashDecision {
    /// Fewer than [`QUARANTINE_CRASHES`] crashes in the last 10 minutes:
    /// restart after waiting `wait`.
    Restart {
        /// Backoff wait before starting the replacement process.
        wait: Duration,
    },
    /// The third crash within 10 minutes: quarantine for the rest of the run.
    Quarantine {
        /// Total crashes recorded by this instance, this one included.
        crashes: u32,
    },
}

impl CrashDecision {
    /// Translates this decision into the [`RecentCrashes`] tag consumed by
    /// [`apply`].
    #[must_use]
    pub const fn recent(self) -> RecentCrashes {
        match self {
            Self::Restart { .. } => RecentCrashes::Fewer,
            Self::Quarantine { .. } => RecentCrashes::Third,
        }
    }
}

/// Pure crash backoff and 10-minute quarantine tracker for one Plugin
/// instance (ADR 0033, issue 837).
#[derive(Clone, Debug, Default)]
pub struct CrashTracker {
    recent: VecDeque<Instant>,
    total_crashes: u32,
}

impl CrashTracker {
    /// Starts with no crashes recorded.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Computes the restart backoff for the `crash_number`-th crash (1-based):
    /// 1 s, 2 s, 4 s, 8 s, 16 s, 30 s, 30 s, …
    #[must_use]
    pub fn backoff(crash_number: u32) -> Duration {
        let shift = crash_number.saturating_sub(1).min(5);
        let secs = BACKOFF_INITIAL
            .as_secs()
            .saturating_mul(1_u64 << shift)
            .min(BACKOFF_MAX.as_secs());
        Duration::from_secs(secs)
    }

    /// Records a crash at `now`, prunes crashes older than
    /// [`QUARANTINE_WINDOW`], and decides whether to restart with backoff or
    /// quarantine.
    pub fn record(&mut self, now: Instant) -> CrashDecision {
        while let Some(&oldest) = self.recent.front() {
            if now.saturating_duration_since(oldest) > QUARANTINE_WINDOW {
                self.recent.pop_front();
            } else {
                break;
            }
        }
        self.recent.push_back(now);
        self.total_crashes = self.total_crashes.saturating_add(1);
        if self.recent.len() >= QUARANTINE_CRASHES {
            CrashDecision::Quarantine {
                crashes: self.total_crashes,
            }
        } else {
            CrashDecision::Restart {
                wait: Self::backoff(self.total_crashes),
            }
        }
    }

    /// Total crashes recorded by this instance so far.
    #[must_use]
    pub const fn total_crashes(&self) -> u32 {
        self.total_crashes
    }
}

/// Where a process Plugin instance is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceState {
    /// Its first process is launched and awaits its `hello`.
    Starting,
    /// Its `hello` matched its approved manifest: it serves calls.
    Ready,
    /// It crashed: it waits for its backoff, then its new process awaits
    /// its `hello`.
    Restarting,
    /// It crashed three times within 10 minutes of its run: it serves no
    /// call for the rest of the run.
    Quarantined,
    /// Its process is killed and its temporary folder deleted.
    Stopped,
}

/// The crashes of the run's instance within the last 10 minutes, this one
/// included.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecentCrashes {
    /// One or two: the instance restarts.
    Fewer,
    /// The third: the instance is quarantined.
    Third,
}

/// What happens to a process Plugin instance.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceEvent {
    /// Its `hello` matches its approved manifest.
    HelloOk,
    /// Its first message is not a `hello`, or the `hello` differs from its
    /// approved manifest or shares no protocol version with the engine.
    HelloRefused,
    /// Its process exited or was killed, the engine's own kills under the
    /// crash rules included.
    Crash(RecentCrashes),
    /// Three `$/ping` went unanswered, which the crash rules count as a
    /// crash.
    PingMissed(RecentCrashes),
    /// The engine needs the instance for a call.
    Call,
    /// An answer to a call still awaited.
    Result,
    /// An answer to a call abandoned after a cancel or a stop.
    LateResult,
    /// Its run stopped, finished or was discarded, or its task ended.
    RunEnded,
    /// The engine stops it for another reason, such as a revocation.
    Stop,
}

/// What the instance's owner does on an allowed event, besides changing its
/// state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum InstanceEffect {
    /// Grant the Plugin its credits and send the queued calls.
    Open,
    /// Fail the queued calls with the `hello`'s Error code, kill the
    /// process and delete its temporary folder.
    Refuse,
    /// Fail the in-flight calls under the crash rules (`plugin_crashed`
    /// for reads and idempotent writes, retried under their lease; an
    /// unchecked Effect uncertain), kill the process if it still runs, and
    /// start a new one after the backoff of 1 s doubling to 30 s.
    Restart,
    /// Fail the in-flight calls under the crash rules, kill the process if
    /// it still runs, fail the queued calls with `plugin_quarantined`, and
    /// send one "a Plugin quarantined" Notification.
    Quarantine,
    /// Hold the call until the `hello` matches.
    Queue,
    /// Send the call, once a credit is free.
    Send,
    /// Hand the answer to its call and release its credit.
    Deliver,
    /// Kill the process and delete its temporary folder.
    Stop,
    /// Nothing more.
    Nothing,
}

/// A transition the instance's table refuses.
pub type InvalidInstanceTransition = InvalidTransition<InstanceState, InstanceEvent>;

/// What `event` does to an instance in `state`: its next state and its
/// effect, or why it is refused.
pub fn apply(
    state: InstanceState,
    event: InstanceEvent,
) -> Result<(InstanceState, InstanceEffect), InvalidInstanceTransition> {
    let refused = |refusal| {
        Err(InvalidTransition {
            state,
            event,
            refusal,
        })
    };
    let unchanged = Ok((state, InstanceEffect::Nothing));
    let stopped = Ok((InstanceState::Stopped, InstanceEffect::Stop));
    let crashed = |recent| match recent {
        RecentCrashes::Fewer => Ok((InstanceState::Restarting, InstanceEffect::Restart)),
        RecentCrashes::Third => Ok((InstanceState::Quarantined, InstanceEffect::Quarantine)),
    };
    match state {
        InstanceState::Starting | InstanceState::Restarting => match event {
            InstanceEvent::HelloOk => Ok((InstanceState::Ready, InstanceEffect::Open)),
            InstanceEvent::HelloRefused => Ok((InstanceState::Stopped, InstanceEffect::Refuse)),
            InstanceEvent::Crash(recent) => crashed(recent),
            InstanceEvent::PingMissed(_) | InstanceEvent::LateResult => unchanged,
            InstanceEvent::Call => Ok((state, InstanceEffect::Queue)),
            InstanceEvent::Result => refused(Refusal::Internal),
            InstanceEvent::RunEnded | InstanceEvent::Stop => stopped,
        },
        InstanceState::Ready => match event {
            InstanceEvent::HelloOk | InstanceEvent::HelloRefused => refused(Refusal::Internal),
            InstanceEvent::LateResult => unchanged,
            InstanceEvent::Crash(recent) | InstanceEvent::PingMissed(recent) => crashed(recent),
            InstanceEvent::Call => Ok((InstanceState::Ready, InstanceEffect::Send)),
            InstanceEvent::Result => Ok((InstanceState::Ready, InstanceEffect::Deliver)),
            InstanceEvent::RunEnded | InstanceEvent::Stop => stopped,
        },
        InstanceState::Quarantined => match event {
            InstanceEvent::HelloOk | InstanceEvent::HelloRefused | InstanceEvent::Result => {
                refused(Refusal::Internal)
            }
            InstanceEvent::Crash(_) | InstanceEvent::PingMissed(_) | InstanceEvent::LateResult => {
                unchanged
            }
            InstanceEvent::Call => refused(Refusal::Program(PLUGIN_QUARANTINED.name)),
            InstanceEvent::RunEnded | InstanceEvent::Stop => stopped,
        },
        InstanceState::Stopped => match event {
            InstanceEvent::Call | InstanceEvent::Result => refused(Refusal::Internal),
            InstanceEvent::HelloOk
            | InstanceEvent::HelloRefused
            | InstanceEvent::Crash(_)
            | InstanceEvent::PingMissed(_)
            | InstanceEvent::LateResult
            | InstanceEvent::RunEnded
            | InstanceEvent::Stop => unchanged,
        },
    }
}

#[cfg(test)]
mod tests;
