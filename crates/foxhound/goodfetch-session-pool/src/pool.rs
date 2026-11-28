//! The pools' rules (ADRs 0006, 0021, 0035, 0059, 0088 and 0093): which
//! Session serves a request, how an attempt's ending scores it, and when
//! it is retired. A crawl keeps the Run's pool, which every Label that sets
//! none of the keys a login rests on shares, and one pool per Label that
//! sets one (`labels`), so a logged-in section of a site and its public
//! section never share a Session. In each pool, under
//! `session.pool.growth: on_demand`, the default (#2178), a request goes to
//! an open Session at its domain's proxy tier that sends from the client it
//! needs, its own HTTP transport or its browser ([`SendsFrom`]), and serves
//! fewer than `session.pool.session_concurrency` requests: one that holds a
//! page first, then, under `session.pool.affinity: host`, one whose last
//! request went to its host, drawn from `Entropy` among those. A new
//! Session opens only when none has room, or when a block left its domain
//! owing one (ADR 0021), and one of the browser client only while a
//! browser-page place can be had, else the request waits. Under `eager`, a
//! request goes to a new Session while the pool has room, as Crawlee's
//! does (#920). Either way a full pool draws among the Sessions open at the
//! tier, and with none open there, a Session drawn among every open one
//! makes room, so `max_size` bounds both clients together. A Session is
//! retired once
//! its error score reaches `max_error_score`, after `max_uses` requests or
//! at `max_age`. A block that escalates retires its Session (ADR 0021):
//! `rate_limit` and `ip_block` move its domain one proxy tier up, where the
//! replacement opens; `challenge` and `captcha` move its domain to the
//! browser tier, where the replacement serves it, leaving the proxy tier to
//! the Challenge solver's ladder (areas 5 and 8); an `ip_block` met by a
//! request the Session's own transport sent on the tier its Label's
//! comparison chose ([`Effect::TransportRefused`], #2419) retires it and
//! moves no tier, the comparison sampling the Label again;
//! `payment_required`, `consent_required` and `decoy` never rotate (ADRs
//! 0045 and 0048). The
//! domains' proxy tiers and browser tier are the crawl's, whichever pool
//! met the block. Retiring is a pool Session's one transition: from open to
//! retired, never back, in [`SessionPool::retire`]; a retired Session's
//! browser context closes once no attempt holds it any more
//! ([`SessionPool::closable`]).
//!
//! **Local addresses (ADR 0044):** under `egress.local_addresses`, each
//! Session takes, when it opens, the addresses of the next position of the
//! Run's rotation ([`LocalAddressing`]), kept in run state so a resumed
//! Session keeps them. When a block retires a Session without a proxy, its
//! addresses' blocks (`egress.rotation_block.ipv4` and `.ipv6`) are blocked
//! on that domain for the rest of the run, and a Session opened for the
//! domain takes the next position outside every one of them; with none
//! left it rotates within a block, which [`SessionPool::rotated_within_block`]
//! hands the crawl for its Run report. With a proxy, whose exit rotates
//! instead (ADR 0021), the replacement keeps the retired Session's
//! addresses.
//!
//! Size: one concern — the Session pools' state machine (draw, settle,
//! proxy-tier escalation, retirement and local-address rotation).

use std::collections::{BTreeMap, BTreeSet};
use std::net::IpAddr;
use std::sync::Arc;
use std::time::SystemTime;

use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::local_addresses::{
    BlockSizes, Host, Rotation, SessionAddresses, Unavailable, block_of,
};
use goodfetch_protocol::foxhound::block::BlockCause;
use goodfetch_protocol::foxhound::session_pool::{ProxyTier, ServingSession};
use goodfetch_protocol::page::SessionId;
use rusqlite::Connection;

pub use crate::escalation::SettleOutcome;
use crate::escalation::{
    DomainEscalation, DrawDecision, Ladder, SettleCause, State, decide_draw, decide_settle,
};
use crate::labels::PoolKey;
use crate::settings::{Affinity, Growth, PoolSettings};
use crate::store::{self, Changes, SendsFrom, StoredSession, StoredTier};

/// What an attempt's ending says of the Session that served it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Its page succeeded.
    Success,
    /// Its site refused the page, for this Block cause.
    Blocked(BlockCause),
    /// Its site refused the page its own transport sent on the tier its
    /// Label's comparison chose, which the comparison answers by sampling
    /// the Label again before any proxy tier moves (#2419).
    TransportRefused,
    /// Its page failed its Page check.
    PageCheckFailed,
    /// Its ending says nothing of the Session, such as a handler's own
    /// failure or skip.
    Neutral,
}

/// What separates a Label's name from a Session's position in the ID of a
/// Session of that Label's pool, `<label>.session-<n>`; the Run's pool's
/// Sessions are `session-<n>`.
const LABEL_MARK: &str = ".session-";

/// What a Session's ID starts with in the Run's pool.
const RUN_MARK: &str = "session-";

/// Why a Session was retired, as run state keeps it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Retirement {
    /// Its error score reached `max_error_score`.
    ErrorScore,
    /// It served `max_uses` requests.
    MaxUses,
    /// It reached `max_age`.
    MaxAge,
    /// A block moved its domain up a proxy tier.
    Escalated,
    /// A block that moves its domain to the browser tier, such as a
    /// Challenge, or a refusal of its transport its Label's comparison
    /// answers.
    Blocked,
    /// It made room for a Session at another proxy tier, or of the other
    /// client.
    PoolFull,
}

impl Retirement {
    const fn as_str(self) -> &'static str {
        match self {
            Self::ErrorScore => "error_score",
            Self::MaxUses => "max_uses",
            Self::MaxAge => "max_age",
            Self::Escalated => "escalated",
            Self::Blocked => "blocked",
            Self::PoolFull => "pool_full",
        }
    }
}

/// An open Session of a pool.
#[derive(Debug)]
struct Open {
    stored: StoredSession,
    /// Whether its next page is its first since the crawl resumed.
    revalidate: bool,
    /// The host its last request went to, which `session.pool.affinity:
    /// host` prefers; in memory only.
    last_host: Option<String>,
    /// Whether it keeps a page open between two requests, which a draw
    /// prefers; in memory only.
    holds_page: bool,
}

impl Open {
    fn new(stored: StoredSession, revalidate: bool) -> Self {
        Self {
            stored,
            revalidate,
            last_host: None,
            holds_page: false,
        }
    }
}

/// What a request a pool's Session serves wants of it: the domain key its
/// proxy tier and blocks are kept under, the host it goes to, and the
/// client it sends from.
#[derive(Clone, Copy, Debug)]
pub struct Wanted<'a> {
    /// Its domain key.
    pub domain_key: &'a str,
    /// Its URL's host.
    pub host: &'a str,
    /// The client it sends from.
    pub sends_from: SendsFrom,
}

/// What a pool holds open for a draw: the proxy tier the draw's domain is
/// at, whether the pool has room for one Session more, its open Sessions
/// of the draw's client at that tier and every open Session, by their
/// positions.
struct Drawable {
    tier: ProxyTier,
    room: bool,
    at_tier: Vec<u64>,
    every: Vec<u64>,
}

/// The Sessions of one pool.
#[derive(Debug, Default)]
struct Sessions {
    open: BTreeMap<u64, Open>,
    /// How many it ever opened.
    opened: u64,
}

/// How a crawl's Sessions take local addresses (ADR 0044): the Run's
/// rotation over `egress.local_addresses`, the block sizes it spreads them
/// over, the host that holds them, and whether the Run's Sessions send
/// through a proxy.
pub struct LocalAddressing {
    rotation: Rotation,
    blocks: BlockSizes,
    host: Arc<dyn Host>,
    proxied: bool,
}

impl std::fmt::Debug for LocalAddressing {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("LocalAddressing")
            .field("rotation", &self.rotation)
            .field("blocks", &self.blocks)
            .field("proxied", &self.proxied)
            .finish_non_exhaustive()
    }
}

impl LocalAddressing {
    /// Sessions taking `rotation`'s addresses on `host`, in blocks of
    /// `blocks`, sending through a proxy when `proxied`.
    pub fn new(rotation: Rotation, blocks: BlockSizes, host: Arc<dyn Host>, proxied: bool) -> Self {
        Self {
            rotation,
            blocks,
            host,
            proxied,
        }
    }

    /// The addresses of the rotation's `ordinal`-th position.
    fn at(&self, ordinal: u64) -> Result<SessionAddresses, Unavailable> {
        self.rotation.session(self.host.as_ref(), ordinal)
    }
}

/// A Session opened for a domain whose every block was blocked there, so
/// it rotated within one (ADR 0044).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rotated {
    /// The Session.
    pub session: SessionId,
    /// The domain key it opened for.
    pub domain_key: String,
}

/// A crawl's Session pools: each pool's open Sessions, each domain's proxy
/// tier and whether it moved to the browser tier, the attempts each
/// Session serves, and what changed since run state last took it.
#[derive(Debug)]
pub struct SessionPool {
    entropy: Entropy,
    pools: BTreeMap<PoolKey, Sessions>,
    tiers: BTreeMap<String, StoredTier>,
    /// How many attempts each Session drawn serves now.
    serving: BTreeMap<SessionId, u64>,
    /// The retired Sessions an attempt still holds, each with its rotation
    /// position (which a block one of those attempts meets still blocks)
    /// and whether it was retired by a block (so a second in-flight block
    /// on it is a late row and does not double-escalate).
    retiring: BTreeMap<SessionId, (Option<u64>, bool)>,
    /// The retired Sessions no attempt holds, not yet taken.
    closable: Vec<SessionId>,
    changes: Changes,
    /// How its Sessions take local addresses, if they do.
    local: Option<LocalAddressing>,
    /// The rotation's next position.
    next_ordinal: u64,
    /// The address blocks a block retired a Session of, by domain key.
    blocked: BTreeMap<String, BTreeSet<String>>,
    /// The position of the Session a block retired, by domain key, which
    /// its replacement keeps through a proxy.
    replaced: BTreeMap<String, u64>,
    /// The Sessions that rotated within a block, not yet taken.
    rotated: Vec<Rotated>,
    /// The domain keys a block that retired a Session left owing a new
    /// one, which their next on-demand draw opens (ADR 0021); in memory
    /// only.
    owed: BTreeSet<String>,
    /// The Run's configured proxy-tier escalation ladder (ADR 0021; spec
    /// #600): empty when no paid proxy pool is configured.
    ladder: Ladder,
    /// When each escalated domain's next periodic trial on the cheaper kind
    /// is due (spec #600; ADR 0021); in memory only.
    next_trial_at: BTreeMap<String, SystemTime>,
    /// The active trial Session for each domain currently in
    /// [`State::Trialing`]; in memory only.
    trial_sessions: BTreeMap<String, SessionId>,
}

impl SessionPool {
    /// The pools of the run file `connection` reads: empty for a crawl that
    /// starts, and for one that resumes its Sessions as they were, each
    /// revalidated on its first page (ADR 0006).
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn open(connection: &Connection, entropy: Entropy) -> rusqlite::Result<Self> {
        let stored = store::read(connection)?;
        let mut pools: BTreeMap<PoolKey, Sessions> = BTreeMap::new();
        for (pool, opened) in stored.opened {
            pools.entry(pool).or_default().opened = opened;
        }
        for ((pool, position), stored) in stored.open {
            pools
                .entry(pool)
                .or_default()
                .open
                .insert(position, Open::new(stored, true));
        }
        Ok(Self {
            entropy,
            pools,
            tiers: stored.tiers,
            serving: BTreeMap::new(),
            retiring: BTreeMap::new(),
            closable: Vec::new(),
            changes: Changes::default(),
            local: None,
            next_ordinal: stored.next_ordinal,
            blocked: stored.blocked,
            replaced: BTreeMap::new(),
            rotated: Vec::new(),
            owed: BTreeSet::new(),
            ladder: Ladder::default(),
            next_trial_at: BTreeMap::new(),
            trial_sessions: BTreeMap::new(),
        })
    }

    /// The pools, climbing `ladder` on a 401, 403 or 429 without a
    /// challenge page (ADR 0021; spec #600).
    #[must_use]
    pub fn with_ladder(self, ladder: Ladder) -> Self {
        Self { ladder, ..self }
    }

    /// The Run's proxy-tier escalation ladder.
    #[must_use]
    pub fn ladder(&self) -> &Ladder {
        &self.ladder
    }

    /// The escalation lifecycle state of `domain_key`.
    #[must_use]
    pub fn escalation_state(&self, domain_key: &str) -> State {
        self.tiers
            .get(domain_key)
            .map_or(State::Base, |stored| stored.state)
    }

    /// The pools, their Sessions taking local addresses as `local` says.
    #[must_use]
    pub fn with_local_addresses(self, local: LocalAddressing) -> Self {
        Self {
            local: Some(local),
            ..self
        }
    }

    /// The local addresses the open Session `session` holds: `None`
    /// without `egress.local_addresses`.
    ///
    /// # Errors
    ///
    /// [`Unavailable`] when the host no longer holds them, such as an
    /// interface that lost its addresses.
    pub fn local_addresses(
        &self,
        session: &SessionId,
    ) -> Option<Result<SessionAddresses, Unavailable>> {
        let local = self.local.as_ref()?;
        Some(local.at(self.address_ordinal(session)?))
    }

    /// The position of the Run's rotation the open Session `session` takes
    /// its local addresses from, which its browser context's egress relay
    /// binds too, so each of its requests leaves from one address: `None`
    /// without `egress.local_addresses`.
    pub fn address_ordinal(&self, session: &SessionId) -> Option<u64> {
        self.local.as_ref()?;
        let (pool, position) = key(session);
        self.pools
            .get(&pool)?
            .open
            .get(&position)?
            .stored
            .address_ordinal
    }

    /// The Sessions that rotated within a block since the last call, each
    /// once, for the Run report.
    pub fn rotated_within_block(&mut self) -> Vec<Rotated> {
        std::mem::take(&mut self.rotated)
    }

    /// The Session of `pool` that serves a request `wanted` at `now`, as
    /// `settings.growth` draws it, which counts as one of its uses, and as
    /// one attempt it serves until [`SessionPool::settle`]. It is never a
    /// Session of the other client. On demand, a Session of the browser
    /// client opens only once `place` says a browser-page place can be had,
    /// asked only then; otherwise none is drawn, `None`, and the request
    /// waits for a Session with room or a place. Eager never asks.
    pub fn draw(
        &mut self,
        pool: &PoolKey,
        wanted: &Wanted<'_>,
        settings: &PoolSettings,
        now: SystemTime,
        place: impl FnOnce() -> bool,
    ) -> Option<ServingSession> {
        let position = match settings.growth {
            Growth::Eager => self.eager(pool, wanted, settings, now),
            Growth::OnDemand => self.on_demand(pool, wanted, settings, now, place)?,
        };
        Some(self.serving(pool, position, Some(wanted.host)))
    }

    /// The position an eager draw takes (#920): a new Session while the
    /// pool has room; else one drawn among those open at the domain's
    /// proxy tier; else, with none open there, one drawn among every open
    /// Session makes room for a new one.
    fn eager(
        &mut self,
        pool: &PoolKey,
        wanted: &Wanted<'_>,
        settings: &PoolSettings,
        now: SystemTime,
    ) -> u64 {
        let Wanted {
            domain_key,
            sends_from,
            ..
        } = *wanted;
        let decision = self.draw_decision(domain_key, settings, now);
        let Drawable {
            tier,
            room,
            at_tier,
            every,
        } = self.drawable_at(pool, decision.tier, sends_from, (settings, now));
        let position = if room {
            self.open_session(pool, tier, sends_from, now, domain_key)
        } else if at_tier.is_empty() {
            let making_room = self.pick(&every);
            self.retire(pool, making_room, Retirement::PoolFull);
            self.open_session(pool, tier, sends_from, now, domain_key)
        } else {
            self.pick(&at_tier)
        };
        self.commit_draw_decision(domain_key, decision, &id(pool, position));
        position
    }

    /// The position an on-demand draw takes (#2178), `None` when the
    /// request waits: an open Session at the domain's proxy tier of the
    /// client wanted, outside every address block blocked on the domain,
    /// that has room under `session_concurrency`, unless the domain is owed
    /// a new Session; else a new Session, once a place can be had for one
    /// of the browser client, while the pool has room, the domain is owed
    /// one or none is open at the tier, one drawn among every open Session
    /// making room in a full pool; else, in a full pool, one of the busy
    /// Sessions at the tier.
    fn on_demand(
        &mut self,
        pool: &PoolKey,
        wanted: &Wanted<'_>,
        settings: &PoolSettings,
        now: SystemTime,
        place: impl FnOnce() -> bool,
    ) -> Option<u64> {
        let Wanted {
            domain_key,
            host,
            sends_from,
        } = *wanted;
        let decision = self.draw_decision(domain_key, settings, now);
        let Drawable {
            tier,
            room,
            mut at_tier,
            every,
        } = self.drawable_at(pool, decision.tier, sends_from, (settings, now));
        at_tier.retain(|position| !self.in_blocked_block(pool, *position, domain_key));
        let free: Vec<u64> = at_tier
            .iter()
            .copied()
            .filter(|position| {
                let serving = self.serving.get(&id(pool, *position)).copied();
                settings
                    .session_concurrency
                    .is_none_or(|concurrency| serving.unwrap_or_default() < concurrency)
            })
            .collect();
        let owed = self.owed.contains(domain_key);
        let position = if !owed && !free.is_empty() {
            self.prefer(pool, &free, host, settings.affinity)
        } else if !(room || owed || at_tier.is_empty()) {
            self.prefer(pool, &at_tier, host, settings.affinity)
        } else {
            if sends_from == SendsFrom::Browser && !place() {
                return None;
            }
            if !room {
                let making_room = self.pick(&every);
                self.retire(pool, making_room, Retirement::PoolFull);
            }
            self.owed.remove(domain_key);
            self.open_session(pool, tier, sends_from, now, domain_key)
        };
        self.commit_draw_decision(domain_key, decision, &id(pool, position));
        Some(position)
    }

    /// The position drawn among `positions` of `pool`, which hold at least
    /// one: among those that hold a page if any does, then, under
    /// `affinity: host`, among those whose last request went to `host` if
    /// any did.
    fn prefer(&mut self, pool: &PoolKey, positions: &[u64], host: &str, affinity: Affinity) -> u64 {
        let open = self.pools.get(pool).map(|sessions| &sessions.open);
        let narrowed = |positions: &[u64], keeps: &dyn Fn(&Open) -> bool| -> Vec<u64> {
            let kept: Vec<u64> = positions
                .iter()
                .copied()
                .filter(|position| open.and_then(|open| open.get(position)).is_some_and(keeps))
                .collect();
            if kept.is_empty() {
                positions.to_vec()
            } else {
                kept
            }
        };
        let mut drawn = narrowed(positions, &|open| open.holds_page);
        if affinity == Affinity::Host {
            drawn = narrowed(&drawn, &|open| open.last_host.as_deref() == Some(host));
        }
        self.pick(&drawn)
    }

    /// Whether the open Session at `position` of `pool` takes its local
    /// addresses, with no proxy, from a block blocked on `domain_key`.
    fn in_blocked_block(&self, pool: &PoolKey, position: u64, domain_key: &str) -> bool {
        let Some(local) = self.local.as_ref().filter(|local| !local.proxied) else {
            return false;
        };
        let Some(blocked) = self
            .blocked
            .get(domain_key)
            .filter(|blocked| !blocked.is_empty())
        else {
            return false;
        };
        let ordinal = self
            .pools
            .get(pool)
            .and_then(|sessions| sessions.open.get(&position))
            .and_then(|open| open.stored.address_ordinal);
        ordinal
            .and_then(|ordinal| local.at(ordinal).ok())
            .is_some_and(|addresses| {
                blocks_of(addresses, local.blocks)
                    .iter()
                    .any(|block| blocked.contains(block))
            })
    }

    /// Notes whether the open Session `session` keeps a page open between
    /// two requests, which a draw prefers among those with room; one no
    /// longer open changes nothing.
    pub fn holds_page(&mut self, session: &SessionId, holds: bool) {
        let (pool, position) = key(session);
        if let Some(open) = self.open_mut(&pool, position) {
            open.holds_page = holds;
        }
    }

    /// As [`SessionPool::draw`], for a Session that only reads a site's
    /// files for an attempt, a robots.txt, an AI-use signal file or an RSL
    /// license (#2422): an open Session of `sends_from`'s client at the
    /// domain's proxy tier serves even while the pool has room, one opens
    /// only while none is open and the pool has room, and none is drawn,
    /// `None`, rather than retire a Session to make room.
    pub fn draw_reusing(
        &mut self,
        pool: &PoolKey,
        domain_key: &str,
        sends_from: SendsFrom,
        settings: &PoolSettings,
        now: SystemTime,
    ) -> Option<ServingSession> {
        let Drawable {
            tier,
            room,
            at_tier,
            ..
        } = self.drawable(pool, domain_key, sends_from, (settings, now));
        let position = if !at_tier.is_empty() {
            self.pick(&at_tier)
        } else if room {
            self.open_session(pool, tier, sends_from, now, domain_key)
        } else {
            return None;
        };
        Some(self.serving(pool, position, None))
    }

    /// What `pool` holds open for a draw on `domain_key` at `now`, sent
    /// from `sends_from`, once the Sessions spent by then are retired.
    fn drawable(
        &mut self,
        pool: &PoolKey,
        domain_key: &str,
        sends_from: SendsFrom,
        (settings, now): (&PoolSettings, SystemTime),
    ) -> Drawable {
        let tier = self.tier_of(domain_key);
        self.drawable_at(pool, tier, sends_from, (settings, now))
    }

    /// What `pool` holds open at `tier` at `now`, sent from `sends_from`,
    /// once the Sessions spent by then are retired.
    fn drawable_at(
        &mut self,
        pool: &PoolKey,
        tier: ProxyTier,
        sends_from: SendsFrom,
        (settings, now): (&PoolSettings, SystemTime),
    ) -> Drawable {
        self.retire_spent(pool, settings, now);
        let sessions = self.pools.entry(pool.clone()).or_default();
        let at_tier = sessions
            .open
            .iter()
            .filter(|(_, open)| {
                open.stored.proxy_tier == tier && open.stored.sends_from == sends_from
            })
            .map(|(position, _)| *position)
            .collect();
        Drawable {
            tier,
            room: sessions.open.len() < settings.max_size,
            at_tier,
            every: sessions.open.keys().copied().collect(),
        }
    }

    fn domain_escalation(&self, domain_key: &str) -> DomainEscalation {
        DomainEscalation {
            tier: self.tier_of(domain_key),
            state: self.escalation_state(domain_key),
            next_trial_at: self.next_trial_at.get(domain_key).copied(),
        }
    }

    fn draw_decision(
        &self,
        domain_key: &str,
        settings: &PoolSettings,
        now: SystemTime,
    ) -> DrawDecision {
        decide_draw(
            self.domain_escalation(domain_key),
            &self.ladder,
            settings.trial_interval,
            now,
        )
    }

    fn commit_draw_decision(
        &mut self,
        domain_key: &str,
        decision: DrawDecision,
        session_id: &SessionId,
    ) {
        self.apply_domain_escalation(domain_key, decision.next);
        if decision.started_trial {
            self.trial_sessions
                .insert(domain_key.to_owned(), session_id.clone());
        }
    }

    fn apply_domain_escalation(&mut self, domain_key: &str, next: DomainEscalation) {
        match next.next_trial_at {
            Some(due) => {
                self.next_trial_at.insert(domain_key.to_owned(), due);
            }
            None => {
                self.next_trial_at.remove(domain_key);
            }
        }
        if let Some(stored) = self.tiers.get_mut(domain_key) {
            if stored.proxy_tier != next.tier || stored.state != next.state {
                stored.proxy_tier = next.tier;
                stored.state = next.state;
                self.changes.tiers.insert(domain_key.to_owned(), *stored);
            }
        } else if next.tier != self.ladder.base_tier() || next.state != State::Base {
            let stored = StoredTier {
                proxy_tier: next.tier,
                browser: false,
                state: next.state,
            };
            self.tiers.insert(domain_key.to_owned(), stored);
            self.changes.tiers.insert(domain_key.to_owned(), stored);
        }
    }

    /// The open Session at `position` of `pool`, drawn for a request to
    /// `host`, if it says: one use more, and one attempt more it serves.
    fn serving(&mut self, pool: &PoolKey, position: u64, host: Option<&str>) -> ServingSession {
        let open = self
            .pools
            .get_mut(pool)
            .and_then(|sessions| sessions.open.get_mut(&position))
            .expect("invariant: the drawn Session is open");
        open.stored.uses += 1;
        if let Some(host) = host {
            open.last_host = Some(host.to_owned());
        }
        let revalidate = std::mem::take(&mut open.revalidate);
        let serving = ServingSession {
            id: id(pool, position),
            proxy_tier: open.stored.proxy_tier,
            revalidate,
        };
        self.changes
            .sessions
            .insert((pool.clone(), position), open.stored.clone());
        *self.serving.entry(serving.id.clone()).or_default() += 1;
        serving
    }

    /// Scores `session` on how its attempt on `domain_key` ended at `now`,
    /// retiring it as the ending says; a block that escalates moves the
    /// domain up one kind in its configured [`Ladder`] when the Session was
    /// still open, or ignores a late block on an already-retired Session
    /// (ADR 0128). If this attempt was an active periodic trial on the
    /// cheaper kind, `Success` steps the domain down (`TrialPassed`) and a
    /// block keeps it escalated and schedules the next trial
    /// (`TrialBlocked`). The attempt no longer holds the Session.
    pub fn settle(
        &mut self,
        session: &ServingSession,
        domain_key: &str,
        effect: Effect,
        settings: &PoolSettings,
        now: SystemTime,
    ) -> SettleOutcome {
        let (pool, position) = key(&session.id);
        let was_trial = self
            .trial_sessions
            .get(domain_key)
            .is_some_and(|trial_id| trial_id == &session.id);
        if was_trial {
            self.trial_sessions.remove(domain_key);
        }
        let already_retired_by_block = self
            .retiring
            .get(&session.id)
            .is_some_and(|(_, by_block)| *by_block);
        let settle_cause = match &effect {
            Effect::Success => SettleCause::Success,
            Effect::Blocked(BlockCause::RateLimit | BlockCause::IpBlock) => {
                SettleCause::BlockWithoutChallenge {
                    session_tier: session.proxy_tier,
                    retired_by_block: already_retired_by_block,
                }
            }
            Effect::Blocked(
                BlockCause::PaymentRequired | BlockCause::ConsentRequired | BlockCause::Decoy,
            ) => SettleCause::StoppingCause,
            Effect::Blocked(BlockCause::Challenge | BlockCause::Captcha) => SettleCause::Challenge,
            Effect::Neutral
            | Effect::PageCheckFailed
            | Effect::TransportRefused
            | Effect::Blocked(_) => SettleCause::Other,
        };
        let decision = decide_settle(
            self.domain_escalation(domain_key),
            settle_cause,
            was_trial,
            &self.ladder,
            settings.trial_interval,
            now,
        );
        self.apply_domain_escalation(domain_key, decision.next);
        let outcome = decision.outcome;
        let retirement = match effect {
            Effect::Neutral => None,
            Effect::Success => {
                if let Some(open) = self.open_mut(&pool, position) {
                    open.stored.error_score = (open.stored.error_score - 0.5).max(0.0);
                    let stored = open.stored.clone();
                    self.changes
                        .sessions
                        .insert((pool.clone(), position), stored);
                }
                None
            }
            Effect::PageCheckFailed => self
                .scored_down(&pool, position, settings)
                .then_some(Retirement::ErrorScore),
            Effect::TransportRefused => {
                self.scored_down(&pool, position, settings);
                Some(Retirement::Blocked)
            }
            Effect::Blocked(cause) => match cause {
                BlockCause::RateLimit | BlockCause::IpBlock => {
                    if already_retired_by_block && !was_trial {
                        // A late block from a Session already retired by a
                        // block for this domain (spec #600's
                        // `block_on_retired_session`): state does not
                        // change again, though its local-address block is
                        // still recorded.
                        self.blocked_on(&pool, position, domain_key);
                        None
                    } else {
                        self.scored_down(&pool, position, settings);
                        decision.retire_session.then_some(Retirement::Escalated)
                    }
                }
                BlockCause::Challenge | BlockCause::Captcha => {
                    self.scored_down(&pool, position, settings);
                    self.move_to_browser(domain_key);
                    Some(Retirement::Blocked)
                }
                BlockCause::PaymentRequired | BlockCause::ConsentRequired | BlockCause::Decoy => {
                    None
                }
                // A cause this engine does not know says nothing of the
                // Session: its classification never gives one.
                _ => None,
            },
        };
        if let Some(retirement) = retirement {
            if matches!(retirement, Retirement::Escalated | Retirement::Blocked) {
                self.blocked_on(&pool, position, domain_key);
                // Escalation by new Session (ADR 0021): the domain's next
                // draw opens one whatever the growth rule says.
                if settings.growth == Growth::OnDemand {
                    self.owed.insert(domain_key.to_owned());
                }
            }
            self.retire(&pool, position, retirement);
        }
        self.released(&session.id);
        outcome
    }

    /// Learns that a Challenge met on a tier short of the browser moved a
    /// request of `domain_key` to the browser tier ([`crate::escalation`]):
    /// the domain's later requests start there
    /// under `fetch.tier: auto`, and a resumed crawl keeps it.
    pub fn moved_to_browser(&mut self, domain_key: &str) {
        self.move_to_browser(domain_key);
    }

    /// Whether a Challenge moved `domain_key` to the browser tier, where
    /// its requests are served under `fetch.tier: auto` (ADR 0021).
    pub fn browser(&self, domain_key: &str) -> bool {
        self.tiers
            .get(domain_key)
            .is_some_and(|stored| stored.browser)
    }

    /// The retired Sessions no attempt holds any more, each once: their
    /// browser contexts may close.
    pub fn closable(&mut self) -> Vec<SessionId> {
        std::mem::take(&mut self.closable)
    }

    /// What changed since the last call, for the run-state writer.
    pub fn changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }

    fn open_mut(&mut self, pool: &PoolKey, position: u64) -> Option<&mut Open> {
        self.pools
            .get_mut(pool)
            .and_then(|sessions| sessions.open.get_mut(&position))
    }

    /// One attempt fewer holds `session`; a retired one no attempt holds
    /// becomes closable.
    fn released(&mut self, session: &SessionId) {
        let left = match self.serving.get_mut(session) {
            Some(count) => {
                *count = count.saturating_sub(1);
                *count
            }
            None => 0,
        };
        if left == 0 {
            self.serving.remove(session);
            if self.retiring.remove(session).is_some() {
                self.closable.push(session.clone());
            }
        }
    }

    /// Adds 1 to the error score of the Session at `position` of `pool`,
    /// if open, and answers whether it reached `max_error_score`.
    fn scored_down(&mut self, pool: &PoolKey, position: u64, settings: &PoolSettings) -> bool {
        let Some(open) = self.open_mut(pool, position) else {
            return false;
        };
        open.stored.error_score += 1.0;
        let stored = open.stored.clone();
        #[expect(
            clippy::cast_precision_loss,
            reason = "a maximum error score is a small whole number"
        )]
        let maximum = settings.max_error_score as f64;
        let reached = stored.error_score >= maximum;
        self.changes
            .sessions
            .insert((pool.clone(), position), stored);
        reached
    }

    /// Moves `domain_key` to the browser tier.
    fn move_to_browser(&mut self, domain_key: &str) {
        let base = self.ladder.base_tier();
        let stored = self
            .tiers
            .entry(domain_key.to_owned())
            .or_insert_with(|| StoredTier {
                proxy_tier: base,
                browser: false,
                state: State::Base,
            });
        if !stored.browser {
            stored.browser = true;
            self.changes.tiers.insert(domain_key.to_owned(), *stored);
        }
    }

    fn tier_of(&self, domain_key: &str) -> ProxyTier {
        match self.tiers.get(domain_key) {
            Some(stored) => self.ladder.clamp(stored.proxy_tier),
            None => self.ladder.base_tier(),
        }
    }

    /// Retires each open Session of `pool` past its uses or its age at
    /// `now`.
    fn retire_spent(&mut self, pool: &PoolKey, settings: &PoolSettings, now: SystemTime) {
        let Some(sessions) = self.pools.get(pool) else {
            return;
        };
        let spent: Vec<(u64, Retirement)> = sessions
            .open
            .iter()
            .filter_map(|(position, open)| {
                let age = now
                    .duration_since(open.stored.opened_at)
                    .unwrap_or_default();
                if open.stored.uses >= settings.max_uses {
                    Some((*position, Retirement::MaxUses))
                } else if age >= settings.max_age {
                    Some((*position, Retirement::MaxAge))
                } else {
                    None
                }
            })
            .collect();
        for (position, retirement) in spent {
            self.retire(pool, position, retirement);
        }
    }

    /// Opens a Session of `pool` at `tier`, sending from `sends_from`, for
    /// a request on `domain_key` and answers its position.
    fn open_session(
        &mut self,
        pool: &PoolKey,
        tier: ProxyTier,
        sends_from: SendsFrom,
        now: SystemTime,
        domain_key: &str,
    ) -> u64 {
        let position = self.pools.get(pool).map_or(0, |sessions| sessions.opened) + 1;
        let address_ordinal = self.opening_ordinal(&id(pool, position), domain_key);
        let sessions = self.pools.entry(pool.clone()).or_default();
        sessions.opened = position;
        let stored = StoredSession {
            proxy_tier: tier,
            sends_from,
            opened_at: now,
            uses: 0,
            error_score: 0.0,
            retired_because: None,
            address_ordinal,
        };
        sessions.open.insert(position, Open::new(stored, false));
        position
    }

    /// The rotation's position the Session `session`, opening for a
    /// request on `domain_key`, takes its local addresses from: through a
    /// proxy, that of the Session a block retired there, if any; else the
    /// next one whose addresses lie outside every block blocked there,
    /// within [`SEARCHED`] positions, or the next one, noted as rotating
    /// within a block, when none does.
    fn opening_ordinal(&mut self, session: &SessionId, domain_key: &str) -> Option<u64> {
        let local = self.local.as_ref()?;
        if local.proxied
            && let Some(ordinal) = self.replaced.remove(domain_key)
        {
            return Some(ordinal);
        }
        let next = self.next_ordinal;
        let ordinal = match self.blocked.get(domain_key) {
            Some(blocked) if !blocked.is_empty() => {
                let outside = (next..next.saturating_add(SEARCHED)).find(|ordinal| {
                    local.at(*ordinal).is_ok_and(|addresses| {
                        blocks_of(addresses, local.blocks)
                            .iter()
                            .all(|block| !blocked.contains(block))
                    })
                });
                outside.unwrap_or_else(|| {
                    self.rotated.push(Rotated {
                        session: session.clone(),
                        domain_key: domain_key.to_owned(),
                    });
                    next
                })
            }
            _ => next,
        };
        self.next_ordinal = ordinal.saturating_add(1);
        Some(ordinal)
    }

    /// Keeps what a block on `domain_key` met by the Session at `position`
    /// of `pool`, open or retired while an attempt held it, means for its
    /// replacement's local addresses: through a proxy, its position, which
    /// the replacement keeps; else its addresses' blocks, blocked on the
    /// domain for the rest of the run.
    fn blocked_on(&mut self, pool: &PoolKey, position: u64, domain_key: &str) {
        let Some(local) = self.local.as_ref() else {
            return;
        };
        let open = self
            .pools
            .get(pool)
            .and_then(|sessions| sessions.open.get(&position))
            .map(|open| open.stored.address_ordinal);
        let Some(ordinal) = open
            .or_else(|| self.retiring.get(&id(pool, position)).map(|(ord, _)| *ord))
            .flatten()
        else {
            return;
        };
        if local.proxied {
            self.replaced.insert(domain_key.to_owned(), ordinal);
            return;
        }
        let Ok(addresses) = local.at(ordinal) else {
            return;
        };
        for block in blocks_of(addresses, local.blocks) {
            if self
                .blocked
                .entry(domain_key.to_owned())
                .or_default()
                .insert(block.clone())
            {
                self.changes.blocked.insert((domain_key.to_owned(), block));
            }
        }
    }

    /// The position drawn among `positions`, which hold at least one.
    fn pick(&mut self, positions: &[u64]) -> u64 {
        let count = u64::try_from(positions.len()).unwrap_or(u64::MAX);
        let drawn = usize::try_from(self.entropy.next_u64() % count).unwrap_or_default();
        positions[drawn]
    }

    /// Retires the open Session at `position` of `pool` for `retirement`;
    /// one already retired stays as it was. It is closable at once unless
    /// an attempt holds it.
    fn retire(&mut self, pool: &PoolKey, position: u64, retirement: Retirement) {
        let Some(mut open) = self
            .pools
            .get_mut(pool)
            .and_then(|sessions| sessions.open.remove(&position))
        else {
            return;
        };
        open.stored.retired_because = Some(retirement.as_str());
        let address_ordinal = open.stored.address_ordinal;
        self.changes
            .sessions
            .insert((pool.clone(), position), open.stored);
        let session = id(pool, position);
        if self.serving.contains_key(&session) {
            self.retiring.insert(
                session,
                (address_ordinal, retirement == Retirement::Escalated),
            );
        } else {
            self.closable.push(session);
        }
    }
}

/// How many positions of the rotation a Session opening for a domain with
/// blocked blocks looks through for one outside them.
const SEARCHED: u64 = 1024;

/// The blocks `addresses` lie in, sized by `sizes`, as run state names
/// them: the family, the prefix length and the block's number.
fn blocks_of(addresses: SessionAddresses, sizes: BlockSizes) -> Vec<String> {
    let ipv4 = addresses.ipv4.map(|address| {
        format!(
            "ipv4/{}:{}",
            sizes.ipv4,
            block_of(IpAddr::V4(address), sizes.ipv4)
        )
    });
    let ipv6 = addresses.ipv6.map(|address| {
        format!(
            "ipv6/{}:{}",
            sizes.ipv6,
            block_of(IpAddr::V6(address), sizes.ipv6)
        )
    });
    ipv4.into_iter().chain(ipv6).collect()
}

/// The ID of the Session at `position` of `pool`.
fn id(pool: &PoolKey, position: u64) -> SessionId {
    match pool {
        PoolKey::Run => SessionId(format!("{RUN_MARK}{position}")),
        PoolKey::Label(label) => SessionId(format!("{label}{LABEL_MARK}{position}")),
    }
}

/// The pool and the position a Session's ID names; position 0, which
/// names none, for an ID the pool never gave.
fn key(id: &SessionId) -> (PoolKey, u64) {
    let parsed = |position: &str| position.parse().unwrap_or_default();
    match id.0.rsplit_once(LABEL_MARK) {
        Some((label, position)) => (PoolKey::Label(label.to_owned()), parsed(position)),
        None => (
            PoolKey::Run,
            id.0.strip_prefix(RUN_MARK).map(parsed).unwrap_or_default(),
        ),
    }
}

#[cfg(test)]
mod tests;
