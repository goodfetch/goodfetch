//! What a refused lifecycle transition is (ADR 0128), and the engine's
//! transition table ([`engine`]). The engine (an Attached engine and a
//! Daemon), each of its Connections and each Run keep their lifecycle as a
//! state enum and change it only through one transition function, which
//! matches the state, then the event inside each state's arm, with no
//! wildcard, and refuses a pair its table does not allow with an
//! [`InvalidTransition`] naming the state and the event. A refusal a
//! program caused answers the program with the user-facing Error code that
//! already names its remedy; one only a bug in Goodfetch can cause is an
//! `InternalError`, logged at `error` ([`InvalidTransition::logged`]). It is
//! never a warning that carries on.
//!
//! The Run's table is `runs::lifecycle`, beside the registry that owns
//! Runs. The engine's is here, a module of its own, since the Connections
//! (`rpc`), the Daemon (`daemon`) and the Attached engine (`engine`) that
//! change its states cannot import one another both ways.

use std::fmt::Debug;

pub mod engine;

/// Who can cause a refused transition, which says what it answers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refusal {
    /// A program can cause it: it gets the user-facing Error code named
    /// here, which already says its remedy.
    Program(&'static str),
    /// Only a bug in Goodfetch can cause it: an `InternalError`, logged at
    /// `error` with the state and the event.
    Internal,
}

/// A transition a table does not allow: the state, the event, and who
/// could have caused it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{event:?} is refused in the state {state:?}")]
pub struct InvalidTransition<S, E> {
    /// The state it was refused in, which it left as it was.
    pub state: S,
    /// The event refused.
    pub event: E,
    /// What it answers.
    pub refusal: Refusal,
}

impl<S: Debug, E: Debug> InvalidTransition<S, E> {
    /// The same, logged at `error` with its state and event when only a
    /// bug can have caused it; a program's is the program's to handle.
    #[must_use]
    pub fn logged(self) -> Self {
        match self.refusal {
            Refusal::Internal => {
                tracing::error!(
                    state = ?self.state,
                    event = ?self.event,
                    "an invalid transition was refused: only a bug in Goodfetch causes it"
                );
            }
            Refusal::Program(_) => {}
        }
        self
    }
}
