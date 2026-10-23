//! The pool's rules (ADRs 0006, 0021, 0035, 0059 and 0088): which Session
//! serves a request, how an attempt's ending scores it, and when it is
//! retired. A request goes to a new Session while the pool has room, as
//! Crawlee's does, then to one drawn from `Entropy` among the Sessions open
//! at its domain's proxy tier; with none open there, the drawn Session
//! makes room. A Session is retired once its error score reaches
//! `max_error_score`, after `max_uses` requests or at `max_age`. A block
//! that escalates retires its Session (ADR 0021): `rate_limit` and
//! `ip_block` move its domain one proxy tier up, where the replacement
//! opens; `challenge` and `captcha` leave the proxy tier, their escalation
//! being the fetch tier's and the Challenge solver's ladder (areas 5 and 8);
//! `payment_required`, `consent_required` and `decoy` never rotate (ADRs
//! 0045 and 0048). Retiring is a pool Session's one transition: from open
//! to retired, never back, in [`SessionPool::retire`].

use std::collections::BTreeMap;
use std::time::SystemTime;

use goodfetch_engine::entropy::Entropy;
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::session_pool::{ProxyTier, ServingSession};
use goodfetch_protocol::page::SessionId;
use rusqlite::Connection;

use crate::settings::PoolSettings;
use crate::store::{self, Changes, StoredSession};

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
    /// A block that changes no proxy tier, such as a Challenge.
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

/// An open Session of the pool.
#[derive(Debug)]
struct Open {
    stored: StoredSession,
    /// Whether its next page is its first since the crawl resumed.
    revalidate: bool,
}

/// A crawl's Session pool: its open Sessions, each domain's proxy tier,
/// and what changed since run state last took it.
#[derive(Debug)]
pub struct SessionPool {
    entropy: Entropy,
    open: BTreeMap<u64, Open>,
    opened: u64,
    tiers: BTreeMap<String, ProxyTier>,
    changes: Changes,
}

impl SessionPool {
    /// The pool of the run file `connection` reads: empty for a crawl that
    /// starts, and for one that resumes its Sessions as they were, each
    /// revalidated on its first page (ADR 0006).
    ///
    /// # Errors
    ///
    /// When the run file cannot be read.
    pub fn open(connection: &Connection, entropy: Entropy) -> rusqlite::Result<Self> {
        let stored = store::read(connection)?;
        Ok(Self {
            entropy,
            open: stored
                .open
                .into_iter()
                .map(|(position, stored)| {
                    (
                        position,
                        Open {
                            stored,
                            revalidate: true,
                        },
                    )
                })
                .collect(),
            opened: stored.opened,
            tiers: stored.tiers,
            changes: Changes::default(),
        })
    }

    /// The Session that serves a request on `domain_key` at `now`, which
    /// counts as one of its uses.
    pub fn draw(
        &mut self,
        domain_key: &str,
        settings: &PoolSettings,
        now: SystemTime,
    ) -> ServingSession {
        self.retire_spent(settings, now);
        let tier = self.tier_of(domain_key);
        let position = if self.open.len() < settings.max_size {
            self.open_session(tier, now)
        } else {
            let at_tier: Vec<u64> = self
                .open
                .iter()
                .filter(|(_, open)| open.stored.proxy_tier == tier)
                .map(|(position, _)| *position)
                .collect();
            if at_tier.is_empty() {
                let every: Vec<u64> = self.open.keys().copied().collect();
                let making_room = self.pick(&every);
                self.retire(making_room, Retirement::PoolFull);
                self.open_session(tier, now)
            } else {
                self.pick(&at_tier)
            }
        };
        let open = self
            .open
            .get_mut(&position)
            .expect("invariant: the drawn Session is open");
        open.stored.uses += 1;
        let revalidate = std::mem::take(&mut open.revalidate);
        let serving = ServingSession {
            id: id(position),
            proxy_tier: open.stored.proxy_tier,
            revalidate,
        };
        self.changes.sessions.insert(position, open.stored.clone());
        serving
    }

    /// Scores `session` on how its attempt on `domain_key` ended, retiring
    /// it as the ending says; a block that escalates moves the domain up
    /// even when its Session was already retired.
    pub fn settle(
        &mut self,
        session: &ServingSession,
        domain_key: &str,
        effect: Effect,
        settings: &PoolSettings,
    ) {
        let position = position(&session.id);
        let retirement = match effect {
            Effect::Neutral => None,
            Effect::Success => {
                if let Some(open) = self.open.get_mut(&position) {
                    open.stored.error_score = (open.stored.error_score - 0.5).max(0.0);
                    self.changes.sessions.insert(position, open.stored.clone());
                }
                None
            }
            Effect::PageCheckFailed => self
                .scored_down(position, settings)
                .then_some(Retirement::ErrorScore),
            Effect::Blocked(cause) => match cause {
                BlockCause::RateLimit | BlockCause::IpBlock => {
                    self.scored_down(position, settings);
                    self.escalate(domain_key, session.proxy_tier.next());
                    Some(Retirement::Escalated)
                }
                BlockCause::Challenge | BlockCause::Captcha => {
                    self.scored_down(position, settings);
                    Some(Retirement::Blocked)
                }
                BlockCause::PaymentRequired | BlockCause::ConsentRequired | BlockCause::Decoy => {
                    None
                }
            },
        };
        if let Some(retirement) = retirement {
            self.retire(position, retirement);
        }
    }

    /// What changed since the last call, for the run-state writer.
    pub fn changes(&mut self) -> Changes {
        std::mem::take(&mut self.changes)
    }

    /// Adds 1 to the error score of the Session at `position`, if open,
    /// and answers whether it reached `max_error_score`.
    fn scored_down(&mut self, position: u64, settings: &PoolSettings) -> bool {
        let Some(open) = self.open.get_mut(&position) else {
            return false;
        };
        open.stored.error_score += 1.0;
        self.changes.sessions.insert(position, open.stored.clone());
        #[expect(
            clippy::cast_precision_loss,
            reason = "a maximum error score is a small whole number"
        )]
        let maximum = settings.max_error_score as f64;
        open.stored.error_score >= maximum
    }

    /// Moves `domain_key` up to `tier`, never down.
    fn escalate(&mut self, domain_key: &str, tier: ProxyTier) {
        if self.tier_of(domain_key) < tier {
            self.tiers.insert(domain_key.to_owned(), tier);
            self.changes.tiers.insert(domain_key.to_owned(), tier);
        }
    }

    fn tier_of(&self, domain_key: &str) -> ProxyTier {
        self.tiers
            .get(domain_key)
            .copied()
            .unwrap_or(ProxyTier::None)
    }

    /// Retires each open Session past its uses or its age at `now`.
    fn retire_spent(&mut self, settings: &PoolSettings, now: SystemTime) {
        let spent: Vec<(u64, Retirement)> = self
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
            self.retire(position, retirement);
        }
    }

    /// Opens a Session at `tier` and answers its position.
    fn open_session(&mut self, tier: ProxyTier, now: SystemTime) -> u64 {
        self.opened += 1;
        let stored = StoredSession {
            position: self.opened,
            proxy_tier: tier,
            opened_at: now,
            uses: 0,
            error_score: 0.0,
            retired_because: None,
        };
        self.open.insert(
            self.opened,
            Open {
                stored,
                revalidate: false,
            },
        );
        self.opened
    }

    /// The position drawn among `positions`, which hold at least one.
    fn pick(&mut self, positions: &[u64]) -> u64 {
        let count = u64::try_from(positions.len()).unwrap_or(u64::MAX);
        let drawn = usize::try_from(self.entropy.next_u64() % count).unwrap_or_default();
        positions[drawn]
    }

    /// Retires the open Session at `position` for `retirement`; one
    /// already retired stays as it was.
    fn retire(&mut self, position: u64, retirement: Retirement) {
        if let Some(mut open) = self.open.remove(&position) {
            open.stored.retired_because = Some(retirement.as_str());
            self.changes.sessions.insert(position, open.stored);
        }
    }
}

/// The ID of the Session at `position`.
fn id(position: u64) -> SessionId {
    SessionId(format!("session-{position}"))
}

/// The position a Session's ID names; 0, which names none, for an ID the
/// pool never gave.
fn position(id: &SessionId) -> u64 {
    id.0.strip_prefix("session-")
        .and_then(|position| position.parse().ok())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
