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

use goodfetch_protocol::foxhound::block::BlockCause;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::session_pool::ProxyTier;
use goodfetch_protocol::page::proxy::ProxyKind;

pub mod state;

pub use state::{
    Event, InvalidEscalationTransition, OBJECT, State, TierPosition, Transition, TransitionEffect,
    apply, refusal_error,
};

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
