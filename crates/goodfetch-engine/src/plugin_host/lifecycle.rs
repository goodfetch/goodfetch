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

use goodfetch_protocol::plugin::PLUGIN_QUARANTINED;

use crate::transition::{InvalidTransition, Refusal};

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
