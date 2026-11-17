//! A declared Effect's transition table (ADRs 0106 and 0128): the state of
//! its key in the Run's Effect journal changes only through [`apply`],
//! which the registry calls for every event below. `absent` is a key the
//! journal holds no entry of; `open` and `sent` are an entry whose body
//! runs, before and after the first command the window attributes to it,
//! each in a repeatable form; `done` keeps the output the body returned,
//! `uncertain` waits for a person, and `reconciled` is a person's "it
//! happened".
//!
//! | Event, in the state: | `absent` | `open` | `open`, repeatable | `sent` | `sent`, repeatable | `done` | `uncertain` | `reconciled` |
//! |---|---|---|---|---|---|---|---|---|
//! | `Declare` | `open` | bug | bug | bug | bug | — | `effect_uncertain` | — |
//! | `DeclareRepeatable` | `open`, repeatable | bug | bug | bug | bug | — | `effect_uncertain` | — |
//! | `Send` | — | `sent` | `sent`, repeatable | — | — | — | — | — |
//! | `Return` | — | `done` | `done` | `done` | `done` | — | — | — |
//! | `Fail` | — | dropped | dropped | `uncertain` | dropped | — | — | — |
//! | `Interrupt` | — | dropped | dropped | `uncertain` | dropped | — | — | — |
//! | `Happened` | — | — | — | — | — | — | `reconciled` | — |
//! | `DidNotHappen` | — | — | — | — | — | — | dropped | — |
//! | `LeaveUncertain` | — | — | — | — | — | — | — | — |
//!
//! A dash changes nothing: a declaration of a `done` or `reconciled` key
//! answers the journal's answer, and every other dash is a late event, such
//! as a command that raced its body's close, a close answered again, or a
//! person's answer about an Effect another person already answered, or one
//! that is not uncertain. Dropped takes the entry out of the journal, so the
//! next declaration runs the body. A declaration never meets a body still
//! running: the registry waits for it, refuses it as `effect_key_reused`,
//! or judges a body whose attempt or Connection is gone with `Interrupt`
//! first, so meeting one is a bug in Goodfetch.

#![deny(clippy::wildcard_enum_match_arm)]

use goodfetch_protocol::effect::EFFECT_UNCERTAIN;

use crate::transition::{InvalidTransition, Refusal};

/// The state of an Effect key in the Run's Effect journal.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectState {
    /// The journal holds no entry of it.
    Absent,
    /// Its body runs, and nothing was sent for it.
    Open,
    /// Its body, declared repeatable, runs, and nothing was sent for it.
    OpenRepeatable,
    /// Its body runs, and something was sent for it.
    Sent,
    /// Its body, declared repeatable, runs, and something was sent for it.
    SentRepeatable,
    /// Its body returned a value the journal keeps.
    Done,
    /// Its body sent something and did not return a value the journal
    /// keeps: never run again until a person reconciles it.
    Uncertain,
    /// A person answered "it happened".
    Reconciled,
}

impl EffectState {
    /// The state of an entry the journal holds as `state`, declared
    /// `repeatable` or not; `None` for a state the journal does not write.
    pub fn of(state: &str, repeatable: bool) -> Option<Self> {
        match (state, repeatable) {
            ("open", false) => Some(Self::Open),
            ("open", true) => Some(Self::OpenRepeatable),
            ("sent", false) => Some(Self::Sent),
            ("sent", true) => Some(Self::SentRepeatable),
            ("done", _) => Some(Self::Done),
            ("uncertain", _) => Some(Self::Uncertain),
            ("reconciled", _) => Some(Self::Reconciled),
            _ => None,
        }
    }

    /// The name the journal writes it under; `None` for `Absent`, which
    /// it writes as no entry.
    pub fn stored(self) -> Option<&'static str> {
        match self {
            Self::Absent => None,
            Self::Open | Self::OpenRepeatable => Some("open"),
            Self::Sent | Self::SentRepeatable => Some("sent"),
            Self::Done => Some("done"),
            Self::Uncertain => Some("uncertain"),
            Self::Reconciled => Some("reconciled"),
        }
    }
}

/// What happens to an Effect key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EffectEvent {
    /// `effect.open` declares it, not repeatable.
    Declare,
    /// `effect.open` declares it repeatable.
    DeclareRepeatable,
    /// The window attributes a command that may change a page or a site
    /// to its body, whose sent mark is written before the command goes
    /// out.
    Send,
    /// `effect.close` says its body returned a value the journal keeps.
    Return,
    /// `effect.close` says its body raised, or returned a value the
    /// journal does not keep.
    Fail,
    /// Its body was cut short: `effect.close` says so, or its attempt,
    /// its Connection or the engine that ran it is gone.
    Interrupt,
    /// A person answers "it happened".
    Happened,
    /// A person answers "it did not happen".
    DidNotHappen,
    /// A person answers "leave uncertain".
    LeaveUncertain,
}

/// What an allowed event does to an Effect key.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// Its entry becomes this state.
    To(EffectState),
    /// Its entry leaves the journal, so the next declaration runs the
    /// body.
    Dropped,
    /// Nothing changes.
    Unchanged,
}

/// A transition the Effect's table refuses.
pub type InvalidEffectTransition = InvalidTransition<EffectState, EffectEvent>;

/// What `event` does to an Effect key in `state`, or why it is refused.
pub fn apply(state: EffectState, event: EffectEvent) -> Result<Next, InvalidEffectTransition> {
    let refused = |refusal| {
        Err(InvalidTransition {
            state,
            event,
            refusal,
        })
    };
    let uncertain = refused(Refusal::Program(EFFECT_UNCERTAIN.name));
    let running = refused(Refusal::Internal);
    match state {
        EffectState::Absent => match event {
            EffectEvent::Declare => Ok(Next::To(EffectState::Open)),
            EffectEvent::DeclareRepeatable => Ok(Next::To(EffectState::OpenRepeatable)),
            EffectEvent::Send
            | EffectEvent::Return
            | EffectEvent::Fail
            | EffectEvent::Interrupt
            | EffectEvent::Happened
            | EffectEvent::DidNotHappen
            | EffectEvent::LeaveUncertain => Ok(Next::Unchanged),
        },
        EffectState::Open | EffectState::OpenRepeatable => match event {
            EffectEvent::Declare | EffectEvent::DeclareRepeatable => running,
            EffectEvent::Send => Ok(Next::To(if state == EffectState::Open {
                EffectState::Sent
            } else {
                EffectState::SentRepeatable
            })),
            EffectEvent::Return => Ok(Next::To(EffectState::Done)),
            EffectEvent::Fail | EffectEvent::Interrupt => Ok(Next::Dropped),
            EffectEvent::Happened | EffectEvent::DidNotHappen | EffectEvent::LeaveUncertain => {
                Ok(Next::Unchanged)
            }
        },
        EffectState::Sent => match event {
            EffectEvent::Declare | EffectEvent::DeclareRepeatable => running,
            EffectEvent::Return => Ok(Next::To(EffectState::Done)),
            EffectEvent::Fail | EffectEvent::Interrupt => Ok(Next::To(EffectState::Uncertain)),
            EffectEvent::Send
            | EffectEvent::Happened
            | EffectEvent::DidNotHappen
            | EffectEvent::LeaveUncertain => Ok(Next::Unchanged),
        },
        EffectState::SentRepeatable => match event {
            EffectEvent::Declare | EffectEvent::DeclareRepeatable => running,
            EffectEvent::Return => Ok(Next::To(EffectState::Done)),
            EffectEvent::Fail | EffectEvent::Interrupt => Ok(Next::Dropped),
            EffectEvent::Send
            | EffectEvent::Happened
            | EffectEvent::DidNotHappen
            | EffectEvent::LeaveUncertain => Ok(Next::Unchanged),
        },
        EffectState::Uncertain => match event {
            EffectEvent::Declare | EffectEvent::DeclareRepeatable => uncertain,
            EffectEvent::Happened => Ok(Next::To(EffectState::Reconciled)),
            EffectEvent::DidNotHappen => Ok(Next::Dropped),
            EffectEvent::Send
            | EffectEvent::Return
            | EffectEvent::Fail
            | EffectEvent::Interrupt
            | EffectEvent::LeaveUncertain => Ok(Next::Unchanged),
        },
        EffectState::Done | EffectState::Reconciled => match event {
            EffectEvent::Declare
            | EffectEvent::DeclareRepeatable
            | EffectEvent::Send
            | EffectEvent::Return
            | EffectEvent::Fail
            | EffectEvent::Interrupt
            | EffectEvent::Happened
            | EffectEvent::DidNotHappen
            | EffectEvent::LeaveUncertain => Ok(Next::Unchanged),
        },
    }
}

#[cfg(test)]
mod tests;
