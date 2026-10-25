//! The pools' rules (ADRs 0006, 0021, 0035, 0059, 0088 and 0093): which
//! Session serves a request, how an attempt's ending scores it, and when
//! it is retired. A crawl keeps the Run's pool, which every Label that sets
//! none of the keys a login rests on shares, and one pool per Label that
//! sets one (`labels`), so a logged-in section of a site and its public
//! section never share a Session. In each pool, a request goes to a new
//! Session while the pool has room, as Crawlee's does, then to one drawn
//! from `Entropy` among the Sessions open at its domain's proxy tier; with
//! none open there, the drawn Session makes room. A Session is retired once
//! its error score reaches `max_error_score`, after `max_uses` requests or
//! at `max_age`. A block that escalates retires its Session (ADR 0021):
//! `rate_limit` and `ip_block` move its domain one proxy tier up, where the
//! replacement opens; `challenge` and `captcha` move its domain to the
//! browser tier, where the replacement serves it, leaving the proxy tier to
//! the Challenge solver's ladder (areas 5 and 8); `payment_required`,
//! `consent_required` and `decoy` never rotate (ADRs 0045 and 0048). The
//! domains' proxy tiers and browser tier are the crawl's, whichever pool
//! met the block. Retiring is a pool Session's one transition: from open to
//! retired, never back, in [`SessionPool::retire`]; a retired Session's
//! browser context closes once no attempt holds it any more
//! ([`SessionPool::closable`]).

use std::collections::{BTreeMap, BTreeSet};
use std::time::SystemTime;

use goodfetch_engine::entropy::Entropy;
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::session_pool::{ProxyTier, ServingSession};
use goodfetch_protocol::page::SessionId;
use rusqlite::Connection;

use crate::labels::PoolKey;
use crate::settings::PoolSettings;
use crate::store::{self, Changes, StoredSession, StoredTier};

/// What an attempt's ending says of the Session that served it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Effect {
    /// Its page succeeded.
    Success,
    /// Its site refused the page, for this Block cause.
    Blocked(BlockCause),
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
    /// Challenge.
    Blocked,
    /// It made room for a Session at another proxy tier.
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
}

/// The Sessions of one pool.
#[derive(Debug, Default)]
struct Sessions {
    open: BTreeMap<u64, Open>,
    /// How many it ever opened.
    opened: u64,
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
    /// The retired Sessions an attempt still holds.
    retiring: BTreeSet<SessionId>,
    /// The retired Sessions no attempt holds, not yet taken.
    closable: Vec<SessionId>,
    changes: Changes,
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
            pools.entry(pool).or_default().open.insert(
                position,
                Open {
                    stored,
                    revalidate: true,
                },
            );
        }
        Ok(Self {
            entropy,
            pools,
            tiers: stored.tiers,
            serving: BTreeMap::new(),
            retiring: BTreeSet::new(),
            closable: Vec::new(),
            changes: Changes::default(),
        })
    }

    /// The Session of `pool` that serves a request on `domain_key` at
    /// `now`, which counts as one of its uses, and as one attempt it serves
    /// until [`SessionPool::settle`].
    pub fn draw(
        &mut self,
        pool: &PoolKey,
        domain_key: &str,
        settings: &PoolSettings,
        now: SystemTime,
    ) -> ServingSession {
        self.retire_spent(pool, settings, now);
        let tier = self.tier_of(domain_key);
        let (room, at_tier, every) = {
            let sessions = self.pools.entry(pool.clone()).or_default();
            let at_tier: Vec<u64> = sessions
                .open
                .iter()
                .filter(|(_, open)| open.stored.proxy_tier == tier)
                .map(|(position, _)| *position)
                .collect();
            let every: Vec<u64> = sessions.open.keys().copied().collect();
            (sessions.open.len() < settings.max_size, at_tier, every)
        };
        let position = if room {
            self.open_session(pool, tier, now)
        } else if at_tier.is_empty() {
            let making_room = self.pick(&every);
            self.retire(pool, making_room, Retirement::PoolFull);
            self.open_session(pool, tier, now)
        } else {
            self.pick(&at_tier)
        };
        let open = self
            .pools
            .get_mut(pool)
            .and_then(|sessions| sessions.open.get_mut(&position))
            .expect("invariant: the drawn Session is open");
        open.stored.uses += 1;
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

    /// Scores `session` on how its attempt on `domain_key` ended, retiring
    /// it as the ending says; a block that escalates moves the domain up
    /// even when its Session was already retired. The attempt no longer
    /// holds the Session.
    pub fn settle(
        &mut self,
        session: &ServingSession,
        domain_key: &str,
        effect: Effect,
        settings: &PoolSettings,
    ) {
        let (pool, position) = key(&session.id);
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
            Effect::Blocked(cause) => match cause {
                BlockCause::RateLimit | BlockCause::IpBlock => {
                    self.scored_down(&pool, position, settings);
                    self.escalate(domain_key, session.proxy_tier.next());
                    Some(Retirement::Escalated)
                }
                BlockCause::Challenge | BlockCause::Captcha => {
                    self.scored_down(&pool, position, settings);
                    self.move_to_browser(domain_key);
                    Some(Retirement::Blocked)
                }
                BlockCause::PaymentRequired | BlockCause::ConsentRequired | BlockCause::Decoy => {
                    None
                }
            },
        };
        if let Some(retirement) = retirement {
            self.retire(&pool, position, retirement);
        }
        self.released(&session.id);
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
            if self.retiring.remove(session) {
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

    /// Moves `domain_key` up to `tier`, never down.
    fn escalate(&mut self, domain_key: &str, tier: ProxyTier) {
        let stored = self.tiers.entry(domain_key.to_owned()).or_default();
        if stored.proxy_tier < tier {
            stored.proxy_tier = tier;
            self.changes.tiers.insert(domain_key.to_owned(), *stored);
        }
    }

    /// Moves `domain_key` to the browser tier.
    fn move_to_browser(&mut self, domain_key: &str) {
        let stored = self.tiers.entry(domain_key.to_owned()).or_default();
        if !stored.browser {
            stored.browser = true;
            self.changes.tiers.insert(domain_key.to_owned(), *stored);
        }
    }

    fn tier_of(&self, domain_key: &str) -> ProxyTier {
        self.tiers
            .get(domain_key)
            .map_or(ProxyTier::None, |stored| stored.proxy_tier)
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

    /// Opens a Session of `pool` at `tier` and answers its position.
    fn open_session(&mut self, pool: &PoolKey, tier: ProxyTier, now: SystemTime) -> u64 {
        let sessions = self.pools.entry(pool.clone()).or_default();
        sessions.opened += 1;
        let position = sessions.opened;
        let stored = StoredSession {
            proxy_tier: tier,
            opened_at: now,
            uses: 0,
            error_score: 0.0,
            retired_because: None,
        };
        sessions.open.insert(
            position,
            Open {
                stored,
                revalidate: false,
            },
        );
        position
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
        self.changes
            .sessions
            .insert((pool.clone(), position), open.stored);
        let session = id(pool, position);
        if self.serving.contains_key(&session) {
            self.retiring.insert(session);
        } else {
            self.closable.push(session);
        }
    }
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
