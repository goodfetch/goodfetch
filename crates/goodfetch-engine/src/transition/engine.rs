//! The engine's transition table (ADR 0128): a Connection's states, which
//! `rpc` changes through [`connection`], and the engine process's, which an
//! Attached engine and a Daemon change through [`engine`].
//!
//! A Connection opens with `hello` (ADR 0024), serves its calls, and once
//! its Client's side ends cancels the calls in flight and closes when none
//! is left. A dash changes nothing; *bug* is an `InternalError`.
//!
//! | Event, in the state: | `Opening` | `Open` | `Closing` | `Closed` |
//! |---|---|---|---|---|
//! | `Hello` | `Opening`, check it | `invalid_request` | bug | bug |
//! | `Request`, `Notification`, `Reply` | `Closed`, refused | `Open`, served | bug | bug |
//! | `Unreadable` | `Closed`, refused | `Open`, answered | bug | bug |
//! | `Accepted` | `Open` | bug | bug | bug |
//! | `Rejected` | `Closed` | bug | bug | bug |
//! | `ClientLost` | `Closing`, cancel calls | `Closing`, cancel calls | — | — |
//! | `CallsEnded` | bug | bug | `Closed` | bug |
//!
//! An Attached engine serves its Client's Connection and its private
//! endpoint's until that Connection ends or its parent is gone; a Daemon
//! serves every Client until it stops idle, is asked to stop, hands over to
//! its major's pick (ADR 0098) or, on `SIGTERM` or once its listener is
//! closed for good, drains its runs to Checkpoints within its grace (ADR
//! 0049).
//!
//! | Event, in the state: | `Attached` | `Serving` | `HandingOver` | `Draining` | `Stopping` |
//! |---|---|---|---|---|---|
//! | `ClientLost` | — | — | — | — | — |
//! | `StdioEnded`, `ParentGone` | `Stopping` | bug | bug | bug | — |
//! | `Idle` | bug | `Stopping` | — | — | — |
//! | `StopAsked` | `not_a_daemon` | `Stopping` | — | — | — |
//! | `HandOver` | bug | `HandingOver` | — | — | — |
//! | `Drain` | bug | `Draining` | — | — | — |
//! | `Drained` | bug | bug | bug | `Stopping` | — |
//! | `ListenerFailed` | bug | `Draining` | — | — | — |
//! | `PartPanicked` | `Stopping` | `Draining` | — | — | — |
//! | `RegistryPanicked` | `Stopping` | `Stopping` | — | — | — |
//!
//! A dash in `HandingOver`, `Draining` or `Stopping` is a late event: the end
//! already decided ends the engine too. A Client lost by a Daemon changes
//! nothing; its idle stop counts the Clients still connected. A part every
//! Run shares that panicked stops the engine (ADR 0148): a Daemon drains
//! while its Runs' registry runs, an Attached engine, which has no drain,
//! stops its Runs through the registry, and either stops at once when the
//! registry is what panicked.

#![deny(clippy::wildcard_enum_match_arm)]

use goodfetch_protocol::daemon::NOT_A_DAEMON;
use goodfetch_protocol::jsonrpc::ProtocolFailure;

use super::{InvalidTransition, Refusal};

/// Where a Connection is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionState {
    /// It has not opened: its first message must be a `hello`.
    Opening,
    /// Its `hello` was accepted: it serves calls.
    Open,
    /// Its Client's side ended: the calls in flight are cancelled, and it
    /// closes once none is left.
    Closing,
    /// It is closed.
    Closed,
}

/// What reaches a Connection.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionEvent {
    /// A `hello` request.
    Hello,
    /// A request other than `hello`.
    Request,
    /// A notification.
    Notification,
    /// A reply to a handler request the engine sent.
    Reply,
    /// A line that is no JSON-RPC message, or too large or deep to read.
    Unreadable,
    /// The listener's token and the handshake accepted its `hello`.
    Accepted,
    /// The listener's token or the handshake refused its `hello`.
    Rejected,
    /// Its Client's side ended.
    ClientLost,
    /// No call is left in flight.
    CallsEnded,
}

/// What a Connection does on an allowed event, besides changing state.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ConnectionEffect {
    /// Check the `hello`: the listener's token, then the handshake.
    Handshake,
    /// Serve the message: call its handler, route its reply, or act on
    /// the notification.
    Serve,
    /// Answer the unreadable line with its error.
    Answer,
    /// Refuse the message that did not open the Connection, then close.
    Refuse,
    /// Cancel the calls in flight.
    CancelCalls,
    /// Nothing more.
    Nothing,
}

/// A transition the Connection's table refuses.
pub type InvalidConnectionTransition = InvalidTransition<ConnectionState, ConnectionEvent>;

/// What `event` does to a Connection in `state`: its next state and its
/// effect, or why it is refused.
pub fn connection(
    state: ConnectionState,
    event: ConnectionEvent,
) -> Result<(ConnectionState, ConnectionEffect), InvalidConnectionTransition> {
    let refused = |refusal| {
        Err(InvalidTransition {
            state,
            event,
            refusal,
        })
    };
    match state {
        ConnectionState::Opening => match event {
            ConnectionEvent::Hello => Ok((ConnectionState::Opening, ConnectionEffect::Handshake)),
            ConnectionEvent::Request
            | ConnectionEvent::Notification
            | ConnectionEvent::Reply
            | ConnectionEvent::Unreadable => {
                Ok((ConnectionState::Closed, ConnectionEffect::Refuse))
            }
            ConnectionEvent::Accepted => Ok((ConnectionState::Open, ConnectionEffect::Nothing)),
            ConnectionEvent::Rejected => Ok((ConnectionState::Closed, ConnectionEffect::Nothing)),
            ConnectionEvent::ClientLost => {
                Ok((ConnectionState::Closing, ConnectionEffect::CancelCalls))
            }
            ConnectionEvent::CallsEnded => refused(Refusal::Internal),
        },
        ConnectionState::Open => match event {
            ConnectionEvent::Hello => {
                refused(Refusal::Program(ProtocolFailure::InvalidRequest.name()))
            }
            ConnectionEvent::Request | ConnectionEvent::Notification | ConnectionEvent::Reply => {
                Ok((ConnectionState::Open, ConnectionEffect::Serve))
            }
            ConnectionEvent::Unreadable => Ok((ConnectionState::Open, ConnectionEffect::Answer)),
            ConnectionEvent::ClientLost => {
                Ok((ConnectionState::Closing, ConnectionEffect::CancelCalls))
            }
            ConnectionEvent::Accepted | ConnectionEvent::Rejected | ConnectionEvent::CallsEnded => {
                refused(Refusal::Internal)
            }
        },
        ConnectionState::Closing => match event {
            ConnectionEvent::ClientLost => {
                Ok((ConnectionState::Closing, ConnectionEffect::Nothing))
            }
            ConnectionEvent::CallsEnded => Ok((ConnectionState::Closed, ConnectionEffect::Nothing)),
            ConnectionEvent::Hello
            | ConnectionEvent::Request
            | ConnectionEvent::Notification
            | ConnectionEvent::Reply
            | ConnectionEvent::Unreadable
            | ConnectionEvent::Accepted
            | ConnectionEvent::Rejected => refused(Refusal::Internal),
        },
        ConnectionState::Closed => match event {
            ConnectionEvent::ClientLost => Ok((ConnectionState::Closed, ConnectionEffect::Nothing)),
            ConnectionEvent::Hello
            | ConnectionEvent::Request
            | ConnectionEvent::Notification
            | ConnectionEvent::Reply
            | ConnectionEvent::Unreadable
            | ConnectionEvent::Accepted
            | ConnectionEvent::Rejected
            | ConnectionEvent::CallsEnded => refused(Refusal::Internal),
        },
    }
}

/// Where the engine process is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineState {
    /// An Attached engine serving its Client.
    Attached,
    /// A Daemon serving its Clients.
    Serving,
    /// A Daemon handing over to its major's pick.
    HandingOver,
    /// A Daemon draining its runs to Checkpoints, after `SIGTERM` or once
    /// its listener is closed for good.
    Draining,
    /// It closes its Connections and stops its Runs.
    Stopping,
}

/// What happens to the engine process.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EngineEvent {
    /// A Client's Connection ended (a Daemon's, or one of an Attached
    /// engine's private endpoint).
    ClientLost,
    /// An Attached engine's Client's Connection ended.
    StdioEnded,
    /// An Attached engine's parent is gone.
    ParentGone,
    /// No Client was connected for `daemon.idle_timeout`.
    Idle,
    /// A Client called `daemon.stop`.
    StopAsked,
    /// A refused Client found the Daemon idle and not its major's pick.
    HandOver,
    /// `SIGTERM`.
    Drain,
    /// Every run was drained to a Checkpoint, or the grace passed.
    Drained,
    /// A Daemon's listener is closed for good.
    ListenerFailed,
    /// A task of the engine's own owner that belongs to no Run and no
    /// Connection panicked, the Runs' registry still running (ADR 0148).
    PartPanicked,
    /// The Runs' registry's own task panicked (ADR 0148).
    RegistryPanicked,
}

impl EngineEvent {
    /// The panic of a part every Run shares, which is the Runs' registry
    /// when `registry` (ADR 0148).
    pub fn panicked(registry: bool) -> Self {
        if registry {
            Self::RegistryPanicked
        } else {
            Self::PartPanicked
        }
    }
}

/// A transition the engine's table refuses.
pub type InvalidEngineTransition = InvalidTransition<EngineState, EngineEvent>;

/// What `event` does to the engine in `state`: its next state, or why it
/// is refused.
pub fn engine(
    state: EngineState,
    event: EngineEvent,
) -> Result<EngineState, InvalidEngineTransition> {
    let refused = |refusal| {
        Err(InvalidTransition {
            state,
            event,
            refusal,
        })
    };
    match state {
        EngineState::Attached => match event {
            EngineEvent::ClientLost => Ok(EngineState::Attached),
            EngineEvent::StdioEnded
            | EngineEvent::ParentGone
            | EngineEvent::PartPanicked
            | EngineEvent::RegistryPanicked => Ok(EngineState::Stopping),
            EngineEvent::StopAsked => refused(Refusal::Program(NOT_A_DAEMON.name)),
            EngineEvent::Idle
            | EngineEvent::HandOver
            | EngineEvent::Drain
            | EngineEvent::Drained
            | EngineEvent::ListenerFailed => refused(Refusal::Internal),
        },
        EngineState::Serving => match event {
            EngineEvent::ClientLost => Ok(EngineState::Serving),
            EngineEvent::Idle | EngineEvent::StopAsked | EngineEvent::RegistryPanicked => {
                Ok(EngineState::Stopping)
            }
            EngineEvent::HandOver => Ok(EngineState::HandingOver),
            EngineEvent::Drain | EngineEvent::ListenerFailed | EngineEvent::PartPanicked => {
                Ok(EngineState::Draining)
            }
            EngineEvent::StdioEnded | EngineEvent::ParentGone | EngineEvent::Drained => {
                refused(Refusal::Internal)
            }
        },
        EngineState::HandingOver => match event {
            EngineEvent::ClientLost
            | EngineEvent::Idle
            | EngineEvent::StopAsked
            | EngineEvent::HandOver
            | EngineEvent::Drain
            | EngineEvent::ListenerFailed
            | EngineEvent::PartPanicked
            | EngineEvent::RegistryPanicked => Ok(EngineState::HandingOver),
            EngineEvent::StdioEnded | EngineEvent::ParentGone | EngineEvent::Drained => {
                refused(Refusal::Internal)
            }
        },
        EngineState::Draining => match event {
            EngineEvent::ClientLost
            | EngineEvent::Idle
            | EngineEvent::StopAsked
            | EngineEvent::HandOver
            | EngineEvent::Drain
            | EngineEvent::ListenerFailed
            | EngineEvent::PartPanicked
            | EngineEvent::RegistryPanicked => Ok(EngineState::Draining),
            EngineEvent::Drained => Ok(EngineState::Stopping),
            EngineEvent::StdioEnded | EngineEvent::ParentGone => refused(Refusal::Internal),
        },
        EngineState::Stopping => match event {
            EngineEvent::ClientLost
            | EngineEvent::StdioEnded
            | EngineEvent::ParentGone
            | EngineEvent::Idle
            | EngineEvent::StopAsked
            | EngineEvent::HandOver
            | EngineEvent::Drain
            | EngineEvent::Drained
            | EngineEvent::ListenerFailed
            | EngineEvent::PartPanicked
            | EngineEvent::RegistryPanicked => Ok(EngineState::Stopping),
        },
    }
}

#[cfg(test)]
mod tests;
