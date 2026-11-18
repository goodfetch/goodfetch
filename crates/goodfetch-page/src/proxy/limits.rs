//! A proxy's limits (ADRs 0053, 0086 and 0088; spec #577): what one proxy
//! endpoint and credential admits, shared by every run of the user that
//! shares Politeness, since a vendor's plan limits the whole account. A
//! navigation through a Session's proxy takes a Permit under the proxy's
//! key, a program's own navigation included, and gives it back once its
//! hop ended. A run admits one while the count in flight there, across
//! every sharing run, is below its own `politeness.proxy.concurrency` and
//! below the ceiling the proxy's own refusals taught, and once a second
//! divided by its own `politeness.proxy.rate` passed since the last start
//! there by any sharing run.
//!
//! A refusal of the proxy itself (its own 429 or 502, or a tunnel it
//! refused) halves the key's ceiling, from the count in flight with it, or
//! from the ceiling when that is lower; the ceiling grows back by one with
//! each request that ends well at the ceiling, while it binds, after a
//! quiet [`RECOVERY`] since it last changed. A site's answer, a 429
//! included, never changes it. Each rule is a pure function of the wall
//! time it is given (ADR 0059).
//!
//! The per-proxy state lives in the `proxy_limits` table the page crate
//! adds to the per-user Politeness file; the engine's own tables hold the
//! Permits and their waiters.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::politeness::{
    Ending, Hold, Holder, KeyState, MemoryTables, Permit, PermitRule, Politeness, PolitenessTables,
    SharedRule, Step, Taken, Verdict, milliseconds, moment,
};
use goodfetch_engine::runs::Run;
use goodfetch_engine::settings::Resolved;
use goodfetch_protocol::page::proxy::ProxyServer;
use goodfetch_protocol::politeness::{POLITENESS_PROXY_CONCURRENCY, POLITENESS_PROXY_RATE};
use rusqlite::{Connection, OptionalExtension as _};
use tokio::sync::broadcast::error::RecvError;
use tokio_util::sync::CancellationToken;

/// How long a lowered ceiling stays quiet before it grows back by one.
pub(crate) const RECOVERY: Duration = Duration::from_secs(10);

/// How long a wait for room lasts before the key is tried again, only to
/// find a holder that died.
const RECLAIM_CHECK: Duration = Duration::from_secs(30);

/// The table the rule keeps each proxy key's shared state in.
pub(crate) const TABLES: PolitenessTables = PolitenessTables {
    name: "goodfetch-page",
    statements: &["CREATE TABLE IF NOT EXISTS proxy_limits (
            key TEXT NOT NULL PRIMARY KEY,
            ceiling INTEGER,
            ceiling_changed_at INTEGER,
            last_start INTEGER
        ) STRICT"],
    prepared: &[READ_LIMITS, WRITE_LIMITS],
};

// The statements the rule runs on the Politeness file, prepared when it opens.
const READ_LIMITS: &str =
    "SELECT ceiling, ceiling_changed_at, last_start FROM proxy_limits WHERE key = ?1";
const WRITE_LIMITS: &str = "INSERT INTO proxy_limits (key, ceiling, ceiling_changed_at, last_start)
    VALUES (?1, ?2, ?3, ?4)
    ON CONFLICT (key) DO UPDATE SET ceiling = ?2, ceiling_changed_at = ?3, last_start = ?4";

/// One proxy key's shared state, as plain values the Politeness file
/// loads and saves whole.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The most requests in flight the proxy's refusals left it.
    ceiling: Option<u64>,
    /// When the ceiling last changed, in milliseconds.
    ceiling_changed_at: Option<i64>,
    /// The last request start by any sharing run, in milliseconds.
    last_start: Option<i64>,
}

impl Limits {
    fn read(connection: &Connection, key: &str) -> rusqlite::Result<Self> {
        Ok(connection
            .prepare_cached(READ_LIMITS)?
            .query_row([key], |row| {
                Ok(Self {
                    ceiling: row
                        .get::<_, Option<i64>>(0)?
                        .and_then(|ceiling| u64::try_from(ceiling).ok()),
                    ceiling_changed_at: row.get(1)?,
                    last_start: row.get(2)?,
                })
            })
            .optional()?
            .unwrap_or_default())
    }

    fn write(&self, connection: &Connection, key: &str) -> rusqlite::Result<()> {
        connection.prepare_cached(WRITE_LIMITS)?.execute((
            key,
            self.ceiling
                .map(|ceiling| i64::try_from(ceiling).unwrap_or(i64::MAX)),
            self.ceiling_changed_at,
            self.last_start,
        ))?;
        Ok(())
    }
}

impl KeyState for Limits {
    type Key = str;
    /// Every change reads it whole.
    type Reads = ();
    /// Every change writes it whole.
    type Writes = ();

    fn load(connection: &Connection, key: &str, (): ()) -> rusqlite::Result<Self> {
        Self::read(connection, key)
    }

    fn save(&self, connection: &Connection, key: &str, (): &()) -> rusqlite::Result<()> {
        self.write(connection, key)
    }

    fn load_from(tables: &MemoryTables, key: &str, (): ()) -> Self {
        tables
            .get::<Proxies>()
            .and_then(|proxies| proxies.0.get(key).copied())
            .unwrap_or_default()
    }

    fn save_to(&self, tables: &mut MemoryTables, key: &str, (): &()) {
        tables.get_mut::<Proxies>().0.insert(key.to_owned(), *self);
    }
}

/// The proxy keys' states a Run's own Politeness keeps in memory, by key.
#[derive(Default)]
struct Proxies(HashMap<String, Limits>);

/// A run's rule for its Sessions' proxies, under its own settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct ProxyRule {
    /// `politeness.proxy.concurrency`.
    concurrency: Option<u64>,
    /// The least time between two starts, from `politeness.proxy.rate`.
    interval: Option<Duration>,
}

impl ProxyRule {
    /// The rule `settings` give a Session's proxy.
    ///
    /// # Errors
    ///
    /// Why `politeness.proxy.concurrency` is refused: `0` admits no
    /// request, so every navigation through the proxy would wait forever.
    pub(crate) fn of(settings: &Resolved) -> Result<Self, String> {
        let concurrency = settings.get(&POLITENESS_PROXY_CONCURRENCY);
        if concurrency == Some(0) {
            return Err(
                "it is 0, which admits no request through the proxy: it is at least 1, or unset for no cap"
                    .to_owned(),
            );
        }
        Ok(Self {
            concurrency,
            interval: settings
                .get::<Option<u64>>(&POLITENESS_PROXY_RATE)
                .filter(|rate| *rate > 0)
                .map(|rate| Duration::from_secs(1) / u32::try_from(rate).unwrap_or(u32::MAX)),
        })
    }
}

fn duration_milliseconds(duration: Duration) -> i64 {
    i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
}

impl PermitRule for ProxyRule {
    type State = Limits;

    fn reads(&self, _: Step) {}

    fn admit(&self, limits: &Limits, in_flight: u64, now: SystemTime) -> Verdict {
        let now = milliseconds(now);
        if let (Some(interval), Some(last_start)) = (self.interval, limits.last_start) {
            let next = last_start.saturating_add(duration_milliseconds(interval));
            if next > now {
                return Verdict::Until(moment(next), Hold::Delay);
            }
        }
        let cap = match (self.concurrency, limits.ceiling) {
            (Some(own), Some(ceiling)) => Some(own.min(ceiling)),
            (own, ceiling) => own.or(ceiling),
        };
        if cap.is_some_and(|cap| in_flight >= cap) {
            Verdict::Busy
        } else {
            Verdict::Go
        }
    }

    fn started(&self, limits: &mut Limits, now: SystemTime) {
        limits.last_start = Some(milliseconds(now));
    }

    fn ended(
        &self,
        limits: &mut Limits,
        in_flight: u64,
        ending: Ending,
        now: SystemTime,
    ) -> (Option<SystemTime>, ()) {
        let now = milliseconds(now);
        match ending {
            Ending::TooManyRequests(_) => {
                // The proxy refused this many at once, the ending one
                // included, whatever the ceiling allowed.
                let tried = in_flight.saturating_add(1);
                let base = limits.ceiling.map_or(tried, |ceiling| ceiling.min(tried));
                limits.ceiling = Some((base / 2).max(1));
                limits.ceiling_changed_at = Some(now);
            }
            Ending::Done | Ending::BackOff(_) | Ending::Until(_) => {
                // Only a request that ended well at the ceiling shows the
                // proxy takes as many: one below it teaches nothing.
                if let (Some(ceiling), Some(changed_at)) =
                    (limits.ceiling, limits.ceiling_changed_at)
                    && in_flight.saturating_add(1) >= ceiling
                    && now >= changed_at.saturating_add(duration_milliseconds(RECOVERY))
                {
                    limits.ceiling = Some(ceiling.saturating_add(1));
                    limits.ceiling_changed_at = Some(now);
                }
            }
        }
        (None, ())
    }
}

/// The account a proxy's Permits count under, since a vendor's plan caps
/// the whole account (ADR 0088), so two accounts with one vendor are two
/// keys and every Session of one account shares one.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Account<'a> {
    /// A proxy the Session names, or a `list` server: its endpoint and the
    /// credential it is used with.
    Proxy {
        server: &'a ProxyServer,
        username: Option<&'a str>,
    },
    /// A hosted Proxy provider's: the provider and the account's own
    /// username as configuration names it, never the sticky username each
    /// Session builds on it (the deep review's 599-4 and D18).
    Provider {
        name: &'a str,
        username: Option<&'a str>,
    },
}

/// The key `account`'s Permits count under.
fn key(account: Account<'_>) -> String {
    match account {
        Account::Proxy { server, username } => {
            format!("proxy {server} {}", username.unwrap_or_default())
        }
        Account::Provider { name, username } => {
            format!("provider {name} {}", username.unwrap_or_default())
        }
    }
}

/// A Session's proxy's limits: the Politeness its Run shares, under the
/// proxy's key and the Run's rule.
pub(crate) struct ProxyLimits {
    run: Run,
    politeness: Politeness,
    holder: Holder,
    key: String,
    rule: SharedRule,
}

impl std::fmt::Debug for ProxyLimits {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProxyLimits")
            .field("key", &self.key)
            .finish_non_exhaustive()
    }
}

impl ProxyLimits {
    /// The limits of `account`'s proxy, used by a Session of `run` under
    /// `settings`; none, with a warning, when the Politeness its Run shares
    /// cannot be opened, so the Session goes on unlimited.
    ///
    /// # Errors
    ///
    /// Why `settings` give no rule ([`ProxyRule::of`]).
    pub(crate) async fn open(
        settings: &Resolved,
        run: &Run,
        account: Account<'_>,
    ) -> Result<Option<Arc<Self>>, String> {
        let rule = ProxyRule::of(settings)?;
        Ok(match run.politeness(settings).await {
            Ok((politeness, holder)) => Some(Arc::new(Self {
                run: run.clone(),
                politeness,
                holder,
                key: key(account),
                rule: SharedRule::new(rule),
            })),
            Err(error) => {
                tracing::warn!(%error, "the Politeness file did not open: the Session's proxy goes unlimited");
                None
            }
        })
    }

    /// A Permit for one request through the proxy, once the rule admits
    /// it: a wait for a start's interval sleeps until then, and a wait for
    /// room until the Permit's give back wakes this Run, or the check for a
    /// holder that died. None once its Run is cancelled, as its end does,
    /// and, with a warning, when the Politeness file fails: the request
    /// goes unlimited.
    ///
    /// # Cancel safety
    ///
    /// Dropping the future may leave a Permit taken for no request, which
    /// its Run's end or another engine's reclaim gives back.
    pub(crate) async fn take(self: &Arc<Self>) -> Option<ProxyPermit> {
        let permit = taken(
            &self.politeness,
            &self.key,
            &self.holder,
            &self.rule,
            &self.run.clock(),
            self.run.cancellation(),
        )
        .await?;
        Some(ProxyPermit {
            permit: Some(permit),
            limits: Arc::clone(self),
        })
    }
}

/// A Permit under `key` for `holder` from `politeness`, once `rule` admits
/// it: a wait for a start's interval sleeps on `clock` until then, and a
/// wait for room until a give back wakes `holder`'s Run, or the check for a
/// holder that died. None once `ending`, the Run's cancellation, comes, so
/// its end never waits on another Run's request; none, with a warning, when
/// the Politeness file fails.
///
/// # Cancel safety
///
/// Dropping the future may leave a Permit taken for no request, which its
/// Run's end or another engine's reclaim gives back.
async fn taken(
    politeness: &Politeness,
    key: &str,
    holder: &Holder,
    rule: &SharedRule,
    clock: &Clock,
    ending: &CancellationToken,
) -> Option<Permit> {
    loop {
        let mut wakes = politeness.wakes();
        let wall = clock.wall();
        let take = politeness.take(key, holder, rule.clone(), wall);
        let taken = tokio::select! {
            () = ending.cancelled() => return None,
            taken = take => taken,
        };
        match taken {
            Ok(Taken::Granted(permit)) => return Some(permit),
            Ok(Taken::Wait(Some((until, _)))) => {
                tokio::select! {
                    () = ending.cancelled() => return None,
                    () = clock.sleep(until.duration_since(wall).unwrap_or_default()) => {}
                }
            }
            Ok(Taken::Wait(None)) => {
                let check = clock.sleep(RECLAIM_CHECK);
                tokio::pin!(check);
                loop {
                    tokio::select! {
                        () = ending.cancelled() => return None,
                        () = &mut check => break,
                        wake = wakes.recv() => match wake {
                            Ok(wake) if wake.run_id == holder.run_id && wake.key == key => break,
                            Ok(_) => {}
                            Err(RecvError::Lagged(_)) => break,
                            Err(RecvError::Closed) => {
                                tokio::select! {
                                    () = ending.cancelled() => return None,
                                    () = &mut check => break,
                                }
                            }
                        },
                    }
                }
            }
            Ok(Taken::Refused(_)) => return None,
            Err(error) => {
                tracing::warn!(%error, %key, "the Politeness file refused a proxy's take: the request goes unlimited");
                return None;
            }
        }
    }
}

/// One request's Permit through a Session's proxy, given back when dropped
/// as a request that ended well, or by [`ProxyPermit::refused`].
#[must_use = "a proxy's Permit is given back when dropped"]
pub(crate) struct ProxyPermit {
    permit: Option<Permit>,
    limits: Arc<ProxyLimits>,
}

impl std::fmt::Debug for ProxyPermit {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("ProxyPermit")
            .field("key", &self.limits.key)
            .finish_non_exhaustive()
    }
}

impl ProxyPermit {
    /// Gives the Permit back now for a request the proxy itself refused,
    /// lowering the key's ceiling before the request's call answers.
    pub(crate) async fn refused(mut self) {
        if let Some(permit) = self.permit.take() {
            give_back(&self.limits, permit, Ending::TooManyRequests(None)).await;
        }
    }
}

impl Drop for ProxyPermit {
    fn drop(&mut self) {
        if let Some(permit) = self.permit.take() {
            let limits = Arc::clone(&self.limits);
            self.limits.run.spawn(async move {
                give_back(&limits, permit, Ending::Done).await;
            });
        }
    }
}

async fn give_back(limits: &ProxyLimits, permit: Permit, ending: Ending) {
    let rule = limits.rule.clone();
    let wall = limits.run.clock().wall();
    if let Err(error) = limits
        .politeness
        .give_back(permit, rule, ending, wall)
        .await
    {
        tracing::warn!(%error, key = %limits.key, "a proxy's Permit was not given back; its Run's end or another engine's reclaim frees it");
    }
}

#[cfg(test)]
mod tests;
