//! A Run's transition table (ADRs 0029, 0102 and 0128): its Lifecycle
//! (`running`, `stopped`, `finished`, `failed`) changes only through
//! [`apply`], which the registry calls for every event below. A start
//! begins a Run `running`; a start that names a Run already there is the
//! event `Start`.
//!
//! | Event, in the state: | `running` | `stopped` | `finished` | `failed` |
//! |---|---|---|---|---|
//! | `Start` | — | — | — | — |
//! | `Resume` | `running` | `running` | — | `running` |
//! | `Checkpoint` | — | — | — | — |
//! | `Pause`, `Migrate`, `Abort`, `Timeout`, `Limit`, `LimitNotUnderstood`, `ResourcesExhausted`, `EngineLost`, `EnginePanicked`, `Drain` | `stopped` | — | — | — |
//! | `Finish` | `finished` | — | — | — |
//! | `Fail` | `failed` | — | — | — |
//! | `RetryFailed` | — | `running` | `running` | `running` |
//! | `WriterFound` | `run_in_use` | `run_in_use` | `run_in_use` | `run_in_use` |
//!
//! A dash changes nothing. In an ended state it is a late event: an end
//! racing another (a driver's finish after an abort), a stop of a Run that
//! already ended, which answers its report, or a start or resume of a
//! finished Run, which returns it as it is (ADR 0102). A resume of a
//! `running` Run takes the writer role of a Run whose engine is gone, at the
//! next run epoch; a Run this engine writes is answered as it is before the
//! table is asked. `run_in_use` is the program's refusal when another
//! writer holds the Run's lock. Whether a Limit a stopped Run reached has
//! something left again is the Usage ledger's to say, not this table's.
//! Areas 9, 10 and 14 add their events here (ADR 0128).

#![deny(clippy::wildcard_enum_match_arm)]

use goodfetch_protocol::run::{Lifecycle, RUN_IN_USE};

use crate::transition::{InvalidTransition, Refusal};

/// What happens to a Run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RunEvent {
    /// A start names its Run ID (ADR 0102).
    Start,
    /// A resume takes its writer role at the next run epoch (ADR 0003).
    Resume,
    /// A Checkpoint of it is written (ADR 0002).
    Checkpoint,
    /// It is paused: checkpointed and stopped, to be resumed by its Run ID
    /// (RT-11).
    Pause,
    /// It is migrated: checkpointed and stopped, to be resumed on another
    /// machine (ADR 0002).
    Migrate,
    /// Its program, a person or a Client aborts it (ADR 0078).
    Abort,
    /// Its `timeout.run` passed (ADR 0080).
    Timeout,
    /// A Limit that covers it has nothing left (ADR 0041).
    Limit,
    /// A Limit flagged *must understand* that its engine cannot evaluate
    /// came to cover it (ADR 0041): stopped, to be resumed by an engine
    /// that can.
    LimitNotUnderstood,
    /// The resource governor ran out of resources for it (ADR 0040).
    ResourcesExhausted,
    /// Its engine is gone: its state says `running` and no writer holds it.
    EngineLost,
    /// Its Attached engine stops on a panic in a part every Run shares
    /// (ADR 0148): stopped, to be resumed by its Run ID.
    EnginePanicked,
    /// Its Daemon drains (ADR 0049): checkpointed and stopped, to be
    /// resumed by its Run ID.
    Drain,
    /// It reached its end: its program finished it, or its driver's work
    /// is done.
    Finish,
    /// An error in the engine or the program's code ended it.
    Fail,
    /// `run.retry_failed` re-admits its failed requests (ADR 0102).
    RetryFailed,
    /// Another writer holds its lock.
    WriterFound,
}

/// What an allowed event does to a Run's Lifecycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Next {
    /// It becomes this Lifecycle; `running` again, from `running`, is a
    /// resume at the next run epoch.
    To(Lifecycle),
    /// Nothing changes.
    Unchanged,
}

/// A transition the Run's table refuses.
pub type InvalidRunTransition = InvalidTransition<Lifecycle, RunEvent>;

/// What `event` does to a Run in `state`, or why it is refused.
pub fn apply(state: Lifecycle, event: RunEvent) -> Result<Next, InvalidRunTransition> {
    let in_use = Err(InvalidTransition {
        state,
        event,
        refusal: Refusal::Program(RUN_IN_USE.name),
    });
    match state {
        Lifecycle::Running => match event {
            RunEvent::Start | RunEvent::Checkpoint | RunEvent::RetryFailed => Ok(Next::Unchanged),
            RunEvent::Resume => Ok(Next::To(Lifecycle::Running)),
            RunEvent::Pause
            | RunEvent::Migrate
            | RunEvent::Abort
            | RunEvent::Timeout
            | RunEvent::Limit
            | RunEvent::LimitNotUnderstood
            | RunEvent::ResourcesExhausted
            | RunEvent::EngineLost
            | RunEvent::EnginePanicked
            | RunEvent::Drain => Ok(Next::To(Lifecycle::Stopped)),
            RunEvent::Finish => Ok(Next::To(Lifecycle::Finished)),
            RunEvent::Fail => Ok(Next::To(Lifecycle::Failed)),
            RunEvent::WriterFound => in_use,
        },
        Lifecycle::Stopped | Lifecycle::Failed => match event {
            RunEvent::Resume | RunEvent::RetryFailed => Ok(Next::To(Lifecycle::Running)),
            RunEvent::Start
            | RunEvent::Checkpoint
            | RunEvent::Pause
            | RunEvent::Migrate
            | RunEvent::Abort
            | RunEvent::Timeout
            | RunEvent::Limit
            | RunEvent::LimitNotUnderstood
            | RunEvent::ResourcesExhausted
            | RunEvent::EngineLost
            | RunEvent::EnginePanicked
            | RunEvent::Drain
            | RunEvent::Finish
            | RunEvent::Fail => Ok(Next::Unchanged),
            RunEvent::WriterFound => in_use,
        },
        Lifecycle::Finished => match event {
            RunEvent::RetryFailed => Ok(Next::To(Lifecycle::Running)),
            RunEvent::Start
            | RunEvent::Resume
            | RunEvent::Checkpoint
            | RunEvent::Pause
            | RunEvent::Migrate
            | RunEvent::Abort
            | RunEvent::Timeout
            | RunEvent::Limit
            | RunEvent::LimitNotUnderstood
            | RunEvent::ResourcesExhausted
            | RunEvent::EngineLost
            | RunEvent::EnginePanicked
            | RunEvent::Drain
            | RunEvent::Finish
            | RunEvent::Fail => Ok(Next::Unchanged),
            RunEvent::WriterFound => in_use,
        },
    }
}

#[cfg(test)]
mod tests;
