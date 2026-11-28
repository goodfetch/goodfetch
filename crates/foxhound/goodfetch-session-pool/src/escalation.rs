//! What a crawl does on a block (ADRs 0020, 0021, 0035 and 0093; spec
//! #600's `escalation`): [`on_block`] decides, from the Block cause and
//! where its Session met it, whether the request moves to the browser tier
//! or ends `blocked`. A Challenge met on a tier short of the browser
//! (`http`, `discovered_api`, `browser_fetch`, `light_browser`) moves the
//! request to the browser, in the same Session when it sends from its
//! browser, else in one whose client is its browser (#2416), and its
//! domain learns the move ([`crate::SessionPool::moved_to_browser`]), so a
//! Challenge is met where it can be passed; under a `fetch.tier` forced to
//! that tier, it ends `blocked` instead, since a forced tier is never left
//! (ADR 0093).
//! An IP block met by a request a Session's own transport sent, on the
//! tier its Label's comparison chose (`fetch.http_from: learned` or
//! `transport`), is answered on the fetch tier first (#2419): the Label's
//! choice on the domain is dropped and the request runs again as a
//! sample, whose comparison moves the Label to `browser_fetch` or the
//! browser; the proxy tier moves only if the browser is refused too. A
//! sample's own runs, a forced tier and a request sent from the browser
//! keep the paths below.
//! A block the program's own code names (a handler's `blocked`, as an
//! after-response Hook's `block(cause)` will, ADR 0104) is decided the
//! same way as one the Block classification rules detected. Every other
//! block ends the request `blocked`; the pool's `settle` then retires its
//! Session as the cause says. Until Collie's Challenge ladder is wired
//! into a Session pool, a Challenge on the browser ends `blocked` too.
//! The per-domain record and the proxy-tier climb arrive with the next
//! tickets of the spec.

use std::time::{Duration, SystemTime};

use goodfetch_protocol::foxhound::block::BlockCause;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::session_pool::ProxyTier;
use goodfetch_protocol::page::proxy::ProxyKind;

pub mod state;

pub use state::{
    Event, InvalidEscalationTransition, OBJECT, State, TierPosition, Transition, TransitionEffect,
    apply, refusal_error,
};

/// What settling an attempt on a Session tells the crawl to do with the
/// request's proxy-tier escalation (spec #600; ADR 0021).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettleOutcome {
    /// No proxy-tier escalation or rotation was triggered.
    Unchanged,
    /// The domain climbed to a costlier proxy kind (or kept its escalated
    /// kind after a blocked trial) and the Session was retired; the request
    /// may be re-admitted on the replacement Session while its used
    /// escalations are under `budget`.
    Escalated {
        /// Per-request escalation budget (one retirement per kind in the
        /// ladder).
        budget: usize,
    },
    /// No costlier proxy kind was left, the Session was retired, and the
    /// provider has more than one exit at the current kind; the request
    /// may rotate within the provider while its used escalations are under
    /// `budget`.
    Rotated {
        /// Per-request escalation budget.
        budget: usize,
    },
    /// A late block on an already-retired Session: the domain's tier did
    /// not change again, and the request may re-run on the domain's
    /// current tier while under `budget`.
    LateBlock {
        /// Per-request escalation budget.
        budget: usize,
    },
}

impl SettleOutcome {
    /// Whether the request should be released and re-admitted (without
    /// spending `crawl.retry.max`) given it has already used `used`
    /// escalations.
    #[must_use]
    pub const fn should_re_admit(self, used: usize) -> bool {
        match self {
            Self::Unchanged => false,
            Self::Escalated { budget } | Self::Rotated { budget } | Self::LateBlock { budget } => {
                used < budget
            }
        }
    }
}

/// A domain's escalation state at a point in time: its current proxy tier,
/// its lifecycle state (`Base`, `Escalated`, `Trialing`), and when its next
/// periodic trial on the cheaper kind is due (spec #600; ADR 0021).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DomainEscalation {
    /// The domain's current proxy tier.
    pub tier: ProxyTier,
    /// Its escalation lifecycle state.
    pub state: State,
    /// When the next periodic trial on the cheaper kind is due; `None` when
    /// at `Base`, when a trial is in flight, or when `escalation.trial_interval`
    /// is unset (`null`).
    pub next_trial_at: Option<SystemTime>,
}

/// The pure result of deciding which proxy tier a draw for a domain uses at
/// `now` (spec #600; ADR 0021).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DrawDecision {
    /// The proxy tier the drawn Session serves at (either the domain's
    /// current tier or the cheaper kind below when starting a trial).
    pub tier: ProxyTier,
    /// The domain's updated escalation record.
    pub next: DomainEscalation,
    /// Whether this draw started a periodic trial on the cheaper kind.
    pub started_trial: bool,
}

/// The cause passed to [`decide_settle`] when an attempt finishes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettleCause {
    /// The attempt succeeded.
    Success,
    /// A 401, 403 or 429 without a challenge page, on a Session at
    /// `session_tier`; `retired_by_block` is true when another attempt
    /// already retired that Session on a block.
    BlockWithoutChallenge {
        /// The proxy tier the Session opened at.
        session_tier: ProxyTier,
        /// Whether the Session was already retired by a block.
        retired_by_block: bool,
    },
    /// A condition a site set (`payment_required`, `consent_required`,
    /// `decoy`): never escalates, rotates or retries.
    StoppingCause,
    /// A Challenge or Captcha.
    Challenge,
    /// Any other attempt ending (neutral, page check failed, transport
    /// refused).
    Other,
}

/// The pure result of settling an attempt on a domain at `now` (spec #600;
/// ADR 0021).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SettleDecision {
    /// The domain's updated escalation record.
    pub next: DomainEscalation,
    /// Whether the crawl should re-admit the request on a replacement
    /// Session.
    pub outcome: SettleOutcome,
    /// Whether the Session that served the attempt should be retired for
    /// escalation/rotation.
    pub retire_session: bool,
}

/// Pure decision for drawing a Session for a domain at `now`: when the
/// domain is `Escalated`, `trial_interval` is set, and `now` has reached
/// `next_trial_at`, transitions `Escalated -> Trialing` via
/// [`Event::TrialDue`] and draws at `ladder.lower_tier(current)` (spec #600;
/// ADR 0021).
#[must_use]
pub fn decide_draw(
    record: DomainEscalation,
    ladder: &Ladder,
    trial_interval: Option<Duration>,
    now: SystemTime,
) -> DrawDecision {
    if ladder.is_empty() {
        return DrawDecision {
            tier: ProxyTier::None,
            next: DomainEscalation {
                tier: ProxyTier::None,
                state: State::Base,
                next_trial_at: None,
            },
            started_trial: false,
        };
    }
    let current = ladder.clamp(record.tier);
    if record.state == State::Escalated {
        if let (Some(interval), Some(lower)) = (trial_interval, ladder.lower_tier(current)) {
            match record.next_trial_at {
                Some(due) if now >= due => {
                    let transition =
                        apply(State::Escalated, Event::TrialDue, ladder.position(current))
                            .expect("invariant: TrialDue is valid in Escalated");
                    return DrawDecision {
                        tier: lower,
                        next: DomainEscalation {
                            tier: current,
                            state: transition.next,
                            next_trial_at: None,
                        },
                        started_trial: true,
                    };
                }
                Some(due) => {
                    return DrawDecision {
                        tier: current,
                        next: DomainEscalation {
                            tier: current,
                            state: State::Escalated,
                            next_trial_at: Some(due),
                        },
                        started_trial: false,
                    };
                }
                None => {
                    return DrawDecision {
                        tier: current,
                        next: DomainEscalation {
                            tier: current,
                            state: State::Escalated,
                            next_trial_at: Some(now + interval),
                        },
                        started_trial: false,
                    };
                }
            }
        }
        return DrawDecision {
            tier: current,
            next: DomainEscalation {
                tier: current,
                state: record.state,
                next_trial_at: None,
            },
            started_trial: false,
        };
    }
    DrawDecision {
        tier: current,
        next: DomainEscalation {
            tier: current,
            state: record.state,
            next_trial_at: if trial_interval.is_some() {
                record.next_trial_at
            } else {
                None
            },
        },
        started_trial: false,
    }
}

/// Pure decision for settling an attempt on a domain at `now`: handles
/// trial completion (`TrialPassed` -> step down, `TrialBlocked` -> stay
/// escalated and reschedule), non-trial blocks (climb or rotate within the
/// provider, or ignore late blocks on already-retired Sessions), and
/// stopping causes (spec #600; ADR 0021).
#[must_use]
pub fn decide_settle(
    record: DomainEscalation,
    cause: SettleCause,
    was_trial: bool,
    ladder: &Ladder,
    trial_interval: Option<Duration>,
    now: SystemTime,
) -> SettleDecision {
    let next_due = || trial_interval.map(|interval| now + interval);
    if ladder.is_empty() {
        let retire_session = matches!(
            cause,
            SettleCause::BlockWithoutChallenge {
                retired_by_block: false,
                ..
            }
        );
        return SettleDecision {
            next: DomainEscalation {
                tier: ProxyTier::None,
                state: State::Base,
                next_trial_at: None,
            },
            outcome: SettleOutcome::Unchanged,
            retire_session,
        };
    }
    let current = ladder.clamp(record.tier);
    if was_trial && record.state == State::Trialing {
        return decide_trial_settle(current, cause, ladder, next_due);
    }
    let kept = DomainEscalation {
        tier: current,
        state: record.state,
        next_trial_at: trial_interval.and(record.next_trial_at),
    };
    match cause {
        SettleCause::BlockWithoutChallenge {
            retired_by_block: true,
            ..
        } => {
            let _ = apply(
                record.state,
                Event::BlockOnRetiredSession,
                ladder.position(current),
            );
            SettleDecision {
                next: kept,
                outcome: SettleOutcome::LateBlock {
                    budget: ladder.budget_per_request(),
                },
                retire_session: false,
            }
        }
        SettleCause::BlockWithoutChallenge {
            session_tier,
            retired_by_block: false,
        } => {
            let effective = current.max(ladder.clamp(session_tier));
            let position = ladder.position(effective);
            let transition = apply(record.state, Event::BlockWithoutChallenge, position)
                .expect("invariant: BlockWithoutChallenge is valid in every escalation state");
            let next_tier = ladder.next_tier(effective);
            let target_tier = next_tier.unwrap_or(effective);
            let budget = ladder.budget_per_request();
            let next_trial_at = (transition.next == State::Escalated)
                .then(next_due)
                .flatten();
            let outcome = if next_tier.is_some() {
                SettleOutcome::Escalated { budget }
            } else if ladder.is_rotatable(effective) {
                SettleOutcome::Rotated { budget }
            } else {
                SettleOutcome::Unchanged
            };
            SettleDecision {
                next: DomainEscalation {
                    tier: target_tier,
                    state: transition.next,
                    next_trial_at,
                },
                outcome,
                retire_session: true,
            }
        }
        SettleCause::StoppingCause => {
            let _ = apply(record.state, Event::StoppingCause, ladder.position(current));
            SettleDecision {
                next: kept,
                outcome: SettleOutcome::Unchanged,
                retire_session: false,
            }
        }
        SettleCause::Success | SettleCause::Challenge | SettleCause::Other => SettleDecision {
            next: kept,
            outcome: SettleOutcome::Unchanged,
            retire_session: false,
        },
    }
}

/// Pure decision for settling an active periodic trial on a domain currently
/// in [`State::Trialing`] (spec #600; ADR 0021).
fn decide_trial_settle(
    current: ProxyTier,
    cause: SettleCause,
    ladder: &Ladder,
    next_due: impl Fn() -> Option<SystemTime>,
) -> SettleDecision {
    let position = ladder.position(current);
    match cause {
        SettleCause::Success => {
            let transition = apply(State::Trialing, Event::TrialPassed, position)
                .expect("invariant: TrialPassed is valid in Trialing");
            let lower = ladder.lower_tier(current).unwrap_or(current);
            let next_trial_at = (transition.next == State::Escalated)
                .then(next_due)
                .flatten();
            SettleDecision {
                next: DomainEscalation {
                    tier: lower,
                    state: transition.next,
                    next_trial_at,
                },
                outcome: SettleOutcome::Unchanged,
                retire_session: false,
            }
        }
        SettleCause::BlockWithoutChallenge { .. } => {
            let transition = apply(State::Trialing, Event::TrialBlocked, position)
                .expect("invariant: TrialBlocked is valid in Trialing");
            let budget = ladder.budget_per_request();
            SettleDecision {
                next: DomainEscalation {
                    tier: current,
                    state: transition.next,
                    next_trial_at: next_due(),
                },
                outcome: SettleOutcome::Escalated { budget },
                retire_session: true,
            }
        }
        SettleCause::StoppingCause => {
            let _ = apply(State::Trialing, Event::StoppingCause, position);
            SettleDecision {
                next: DomainEscalation {
                    tier: current,
                    state: State::Escalated,
                    next_trial_at: next_due(),
                },
                outcome: SettleOutcome::Unchanged,
                retire_session: false,
            }
        }
        SettleCause::Challenge | SettleCause::Other => SettleDecision {
            next: DomainEscalation {
                tier: current,
                state: State::Escalated,
                next_trial_at: next_due(),
            },
            outcome: SettleOutcome::Unchanged,
            retire_session: false,
        },
    }
}

/// One rung of a Run's proxy-tier escalation ladder.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LadderRung {
    /// The proxy kind.
    pub kind: ProxyKind,
    /// Whether the Run's provider has more than one exit for this kind
    /// (allowing rotation within the kind when no higher kind is left).
    pub rotatable: bool,
}

/// The ordered proxy kinds a crawl's Session pool climbs on a block without
/// a challenge page, cheapest first (ADR 0021; spec #600): derived from the
/// kinds the Run's Proxy provider configures, narrowed by
/// `escalation.proxy_kinds` when set. Empty when no paid pool is configured,
/// so a run with no paid pool spends nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Ladder {
    rungs: Vec<LadderRung>,
}

impl Ladder {
    /// Builds a ladder from the rungs `configured` by the Run's Proxy
    /// provider, narrowed by `narrowed` (`escalation.proxy_kinds`) when set,
    /// starting at `initial` (`proxy.kind`) when set and `narrowed` is unset.
    #[must_use]
    pub fn build(
        configured: &[LadderRung],
        initial: Option<ProxyKind>,
        narrowed: Option<&[ProxyKind]>,
    ) -> Self {
        let mut rungs = Vec::new();
        let find = |kind: ProxyKind| configured.iter().find(|rung| rung.kind == kind).copied();
        match narrowed {
            Some(allowed) => {
                for &kind in allowed {
                    if let Some(rung) = find(kind)
                        && !rungs.iter().any(|kept: &LadderRung| kept.kind == kind)
                    {
                        rungs.push(rung);
                    }
                }
            }
            None => {
                for kind in ProxyKind::ALL {
                    if let Some(rung) = find(kind)
                        && initial.is_none_or(|start| kind >= start)
                        && !rungs.iter().any(|kept: &LadderRung| kept.kind == kind)
                    {
                        rungs.push(rung);
                    }
                }
            }
        }
        Self { rungs }
    }

    /// A ladder with these exact proxy kinds, in order, each non-rotatable.
    #[must_use]
    pub fn from_kinds(kinds: Vec<ProxyKind>) -> Self {
        Self {
            rungs: kinds
                .into_iter()
                .map(|kind| LadderRung {
                    kind,
                    rotatable: false,
                })
                .collect(),
        }
    }

    /// The ordered proxy kinds in this ladder.
    #[must_use]
    pub fn kinds(&self) -> Vec<ProxyKind> {
        self.rungs.iter().map(|rung| rung.kind).collect()
    }

    /// Whether no paid proxy pool is configured.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.rungs.is_empty()
    }

    /// The lowest tier of the ladder (`ProxyTier::None` when empty).
    #[must_use]
    pub fn base_tier(&self) -> ProxyTier {
        self.rungs
            .first()
            .map_or(ProxyTier::None, |rung| ProxyTier::of_kind(rung.kind))
    }

    /// The tier one step above `current`, if a costlier kind is left.
    #[must_use]
    pub fn next_tier(&self, current: ProxyTier) -> Option<ProxyTier> {
        let index = self.index_of(current)?;
        self.rungs
            .get(index + 1)
            .map(|rung| ProxyTier::of_kind(rung.kind))
    }

    /// The tier one step below `current`, if `current` is above `base_tier`.
    #[must_use]
    pub fn lower_tier(&self, current: ProxyTier) -> Option<ProxyTier> {
        let index = self.index_of(current)?;
        index
            .checked_sub(1)
            .and_then(|below| self.rungs.get(below))
            .map(|rung| ProxyTier::of_kind(rung.kind))
    }

    /// Whether `current` can rotate to a fresh exit within its provider
    /// when no costlier kind is left.
    #[must_use]
    pub fn is_rotatable(&self, current: ProxyTier) -> bool {
        self.index_of(current)
            .and_then(|index| self.rungs.get(index))
            .is_some_and(|rung| rung.rotatable)
    }

    /// The tier position of `current` for [`apply`].
    #[must_use]
    pub fn position(&self, current: ProxyTier) -> TierPosition {
        let Some(index) = self.index_of(current) else {
            return TierPosition::default();
        };
        TierPosition {
            has_higher_kind: index + 1 < self.rungs.len(),
            below_is_lowest: index == 1,
        }
    }

    /// The per-request escalation budget: one retirement per kind in the
    /// ladder (spec #600).
    #[must_use]
    pub fn budget_per_request(&self) -> usize {
        self.rungs.len()
    }

    /// Resolves a stored tier against this ladder: if the stored tier is in
    /// the ladder, keeps it; otherwise falls back to [`Self::base_tier`].
    #[must_use]
    pub fn clamp(&self, stored: ProxyTier) -> ProxyTier {
        if self.index_of(stored).is_some() {
            stored
        } else {
            self.base_tier()
        }
    }

    fn index_of(&self, tier: ProxyTier) -> Option<usize> {
        let kind = tier.kind()?;
        self.rungs.iter().position(|rung| rung.kind == kind)
    }
}

/// Where a Session met a block: the tier its request was fetched on, none
/// when the handler fetched it itself, whether `fetch.tier` forced that
/// tier, and whether the Session's own transport sent it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Met {
    /// The tier.
    pub tier: Option<FetchTier>,
    /// Whether `fetch.tier` forced it.
    pub forced: bool,
    /// Whether the Session's own transport sent the request, on the tier
    /// its Label's comparison chose, not as one of a sample's runs.
    pub from_transport: bool,
}

/// What a crawl does with a request that met a block.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// The request runs again on the browser tier, in the same attempt
    /// and Session, and its domain's later requests start there.
    MoveToBrowser,
    /// The transport was refused: the Label's choice on the domain is
    /// dropped, and the request runs again as a sample.
    Sample,
    /// The request ends `blocked` with its cause.
    Blocked,
}

/// What a crawl does with a request whose Session met a block of `cause`
/// where `met` says (the module's doc).
pub fn on_block(cause: &BlockCause, met: Met) -> Action {
    let challenge = matches!(cause, BlockCause::Challenge | BlockCause::Captcha);
    if *cause == BlockCause::IpBlock && met.from_transport && !met.forced {
        return Action::Sample;
    }
    match met.tier {
        Some(
            FetchTier::Http
            | FetchTier::DiscoveredApi
            | FetchTier::BrowserFetch
            | FetchTier::LightBrowser,
        ) if challenge && !met.forced => Action::MoveToBrowser,
        Some(_) | None => Action::Blocked,
    }
}

#[cfg(test)]
mod tests;
