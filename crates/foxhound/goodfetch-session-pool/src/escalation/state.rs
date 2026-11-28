//! Per-domain escalation state table (ADR 0128; spec #600): every state
//! change of a domain's escalation record goes through [`apply`], which
//! matches the state, then the event inside each state's arm, with no
//! wildcard, and refuses the 6 invalid pairs with an [`InvalidTransition`]
//! whose refusal is [`Refusal::Internal`].
#![deny(clippy::wildcard_enum_match_arm)]

use goodfetch_engine::CallError;
use goodfetch_engine::transition::{InvalidTransition, Refusal};
use goodfetch_protocol::connection::INTERNAL;
use serde_json::json;

/// The object name reported in `internal` error details for refused
/// escalation transitions (spec #600).
pub const OBJECT: &str = "escalation";

/// A domain's escalation lifecycle state (spec #600; ADR 0128).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum State {
    /// The domain runs at the lowest proxy kind the Run configures.
    #[default]
    Base,
    /// The domain runs above `Base`, with the next trial scheduled
    /// `escalation.trial_interval` ahead.
    Escalated,
    /// One request is on trial at the proxy kind below.
    Trialing,
}

impl State {
    /// Its name in run state and error details.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Base => "base",
            Self::Escalated => "escalated",
            Self::Trialing => "trialing",
        }
    }

    /// The state named `name`, if any.
    #[must_use]
    pub fn parse(name: &str) -> Option<Self> {
        match name {
            "base" => Some(Self::Base),
            "escalated" => Some(Self::Escalated),
            "trialing" => Some(Self::Trialing),
            _ => None,
        }
    }

    /// Restores a domain's state from a Checkpoint: a `trialing` record
    /// restores as `escalated`, since no trial survives a resume (spec
    /// #600).
    #[must_use]
    pub const fn from_checkpoint(self) -> Self {
        match self {
            Self::Base => Self::Base,
            Self::Escalated | Self::Trialing => Self::Escalated,
        }
    }
}

/// An event delivered to a domain's escalation state machine (spec #600;
/// ADR 0128).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Event {
    /// A 401, 403 or 429 without a challenge page.
    BlockWithoutChallenge,
    /// A challenge page met on a tier short of the browser.
    ChallengeOnHttpTier,
    /// A challenge met on the browser tier.
    ChallengeOnBrowser,
    /// A Challenge ladder step passed.
    ChallengeCleared,
    /// A condition a site set (`payment_required`, `consent_required`,
    /// `decoy` or a Wall allowance refetch's refusal).
    StoppingCause,
    /// The `Clock` reached the domain's next scheduled trial.
    TrialDue,
    /// The trial request on the cheaper kind succeeded.
    TrialPassed,
    /// The trial request on the cheaper kind was blocked.
    TrialBlocked,
    /// A block reported by a Session the pool already retired for this
    /// domain (e.g. a second request in flight on it).
    BlockOnRetiredSession,
}

impl Event {
    /// Its name in error details.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::BlockWithoutChallenge => "block_without_challenge",
            Self::ChallengeOnHttpTier => "challenge_on_http_tier",
            Self::ChallengeOnBrowser => "challenge_on_browser",
            Self::ChallengeCleared => "challenge_cleared",
            Self::StoppingCause => "stopping_cause",
            Self::TrialDue => "trial_due",
            Self::TrialPassed => "trial_passed",
            Self::TrialBlocked => "trial_blocked",
            Self::BlockOnRetiredSession => "block_on_retired_session",
        }
    }
}

/// The tier position of a domain when [`apply`] runs, which decides the
/// conditional transitions of the state table.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TierPosition {
    /// Whether a costlier proxy kind is left above the domain's current
    /// kind.
    pub has_higher_kind: bool,
    /// Whether the proxy kind one step below the domain's current kind is
    /// the lowest kind configured for the Run.
    pub below_is_lowest: bool,
}

/// What a valid transition of [`apply`] tells its caller to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransitionEffect {
    /// Retire the Session and replace it one kind up when a kind is left,
    /// else rotate within the provider until the escalation budget is
    /// spent, then end `blocked`.
    EscalateOrRotate,
    /// Move the request to the browser tier and learn it for the domain
    /// (unless `fetch.tier` is forced to a non-browser tier).
    MoveToBrowser,
    /// Run the browser Challenge ladder.
    RunLadder,
    /// Record the ladder step that passed as the domain's ladder start.
    RecordLadderStart,
    /// End the request at once; nothing escalates, rotates or retries.
    StopRequest,
    /// Send one request through a Session at the proxy kind below.
    StartTrial,
    /// Move the domain down one proxy kind; schedule the next trial while
    /// still above `Base`.
    StepDown,
    /// Keep the domain at its current proxy kind and schedule the next
    /// trial.
    KeepEscalated,
    /// A late block from an already-retired Session: nothing changes.
    IgnoreLate,
}

/// The result of a valid transition in [`apply`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Transition {
    /// The domain's next state.
    pub next: State,
    /// What happens on this transition.
    pub effect: TransitionEffect,
}

/// A refused escalation transition (ADR 0128).
pub type InvalidEscalationTransition = InvalidTransition<State, Event>;

/// Applies `event` to `state` at `position`, returning the next state and
/// effect, or [`InvalidEscalationTransition`] for the 6 pairs the table
/// refuses (spec #600; ADR 0128).
///
/// # Errors
///
/// [`InvalidEscalationTransition`] with [`Refusal::Internal`] when
/// `(state, event)` is one of the 6 refused pairs.
pub fn apply(
    state: State,
    event: Event,
    position: TierPosition,
) -> Result<Transition, InvalidEscalationTransition> {
    let refused = |event: Event| {
        Err(InvalidTransition {
            state,
            event,
            refusal: Refusal::Internal,
        })
    };
    let stay = |effect: TransitionEffect| {
        Ok(Transition {
            next: state,
            effect,
        })
    };
    match state {
        State::Base => match event {
            Event::BlockWithoutChallenge => Ok(Transition {
                next: if position.has_higher_kind {
                    State::Escalated
                } else {
                    State::Base
                },
                effect: TransitionEffect::EscalateOrRotate,
            }),
            Event::ChallengeOnHttpTier => stay(TransitionEffect::MoveToBrowser),
            Event::ChallengeOnBrowser => stay(TransitionEffect::RunLadder),
            Event::ChallengeCleared => stay(TransitionEffect::RecordLadderStart),
            Event::StoppingCause => stay(TransitionEffect::StopRequest),
            Event::TrialDue => refused(Event::TrialDue),
            Event::TrialPassed => refused(Event::TrialPassed),
            Event::TrialBlocked => refused(Event::TrialBlocked),
            Event::BlockOnRetiredSession => stay(TransitionEffect::IgnoreLate),
        },
        State::Escalated => match event {
            Event::BlockWithoutChallenge => stay(TransitionEffect::EscalateOrRotate),
            Event::ChallengeOnHttpTier => stay(TransitionEffect::MoveToBrowser),
            Event::ChallengeOnBrowser => stay(TransitionEffect::RunLadder),
            Event::ChallengeCleared => stay(TransitionEffect::RecordLadderStart),
            Event::StoppingCause => stay(TransitionEffect::StopRequest),
            Event::TrialDue => Ok(Transition {
                next: State::Trialing,
                effect: TransitionEffect::StartTrial,
            }),
            Event::TrialPassed => refused(Event::TrialPassed),
            Event::TrialBlocked => refused(Event::TrialBlocked),
            Event::BlockOnRetiredSession => stay(TransitionEffect::IgnoreLate),
        },
        State::Trialing => match event {
            Event::BlockWithoutChallenge => stay(TransitionEffect::EscalateOrRotate),
            Event::ChallengeOnHttpTier => stay(TransitionEffect::MoveToBrowser),
            Event::ChallengeOnBrowser => stay(TransitionEffect::RunLadder),
            Event::ChallengeCleared => stay(TransitionEffect::RecordLadderStart),
            Event::StoppingCause => stay(TransitionEffect::StopRequest),
            Event::TrialDue => refused(Event::TrialDue),
            Event::TrialPassed => Ok(Transition {
                next: if position.below_is_lowest {
                    State::Base
                } else {
                    State::Escalated
                },
                effect: TransitionEffect::StepDown,
            }),
            Event::TrialBlocked => Ok(Transition {
                next: State::Escalated,
                effect: TransitionEffect::KeepEscalated,
            }),
            Event::BlockOnRetiredSession => stay(TransitionEffect::IgnoreLate),
        },
    }
}

/// Logs `invalid` at `error` and builds the `internal` [`CallError`]
/// naming the object `escalation`, the state and the event (spec #600;
/// ADR 0079 and ADR 0128).
#[must_use]
pub fn refusal_error(invalid: InvalidEscalationTransition) -> CallError {
    let invalid = invalid.logged();
    let state = invalid.state.as_str();
    let event = invalid.event.as_str();
    CallError::new(
        &INTERNAL,
        format!("invalid {OBJECT} transition: `{event}` is refused in `{state}`"),
        &json!({
            "object": OBJECT,
            "state": state,
            "event": event,
        }),
    )
}
