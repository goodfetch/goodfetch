//! The crawl's Run state (ADR 0142): what a running crawl decides from,
//! held in memory and owned by one actor, the delivery loop, which is its
//! only caller, so nothing here sits under a lock. The first state it owns
//! is the leasing of work items: the Session each attempt is drawn from the
//! pool, how its tier is chosen, what its Label on its domain learned and
//! the browser-fetch latencies that time a race.
//!
//! The run file stays the truth of what is leased: a writer's job leases
//! the work items, and the draw then runs here, in memory, on what that job
//! answered. What a draw changes is written as rows the next time the loop
//! queues a job, a lease's or an outcome's, and once more when the crawl has
//! nothing left to lease, so a draw adds no job of its own and the file
//! follows memory by one job at most.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime};

use goodfetch_engine::local_addresses::{SessionAddresses, Unavailable};
use goodfetch_engine::run_state::{BatchRows, BatchWriter};
use goodfetch_fetch_tier::{Choosing, PooledSession, SetTier, TierChoice};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_session_pool::{PoolSettings, SessionPool};

use crate::labels;
use crate::tier_choice::{AttemptPlan, Sampled, TierChoices, Unit, Weighed};
use crate::tier_race::Latencies;
use crate::work_items::Leased;

/// The local addresses a Session drawn from the pool holds: `None`
/// without `egress.local_addresses` (ADR 0044).
pub(crate) type Local = Option<Result<SessionAddresses, Unavailable>>;

/// How an attempt's tier is chosen, settled as its Session is drawn.
pub(crate) enum Chosen {
    /// On this plan.
    Planned(AttemptPlan),
    /// As a race, unless its Label on its domain, which learned this plan,
    /// is due a sample, which it then takes.
    Racing(AttemptPlan),
}

/// A work item leased, with the Session drawn for its attempt, as that
/// Session runs in the browser, its local addresses, and how its tier is
/// chosen.
pub(crate) struct Drawn {
    pub(crate) leased: Leased,
    pub(crate) session: ServingSession,
    pub(crate) in_browser: PooledSession,
    pub(crate) local: Local,
    pub(crate) chosen: Chosen,
}

/// What the Run's settings say of a draw, resolved as the Run opened, and
/// the Run's wall clock as the draw's lease was asked.
pub(crate) struct Drawing<'a> {
    pub(crate) labels: &'a labels::Declared,
    pub(crate) settings: PoolSettings,
    pub(crate) run_tier: SetTier,
    pub(crate) races: bool,
    /// The wall clock, and in milliseconds since the Unix epoch.
    pub(crate) wall: (SystemTime, u64),
}

/// The tier choices whose count of requests served moved since the run
/// file last heard of them, by Label and domain key: the latest of each.
#[derive(Default)]
pub(crate) struct Counted(BTreeMap<(String, String), Unit>);

impl Counted {
    /// Whether none moved.
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl BatchRows for Counted {
    fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()> {
        if self.0.is_empty() {
            return Ok(());
        }
        let mut record = writer.statement(crate::tier_choice::RECORD)?;
        for ((label, domain_key), unit) in self.0 {
            record.write((label, domain_key, crate::tier_choice::encoded(&unit)))?;
        }
        Ok(())
    }
}

/// What the Run's Labels learned under `auto`, with the counts the run
/// file has not heard of yet.
#[derive(Default)]
pub(crate) struct Learned {
    choices: TierChoices,
    /// The choices that counted a request since the run file last heard
    /// of them.
    counted: Counted,
}

impl Learned {
    /// What a crawl that learned `choices` goes on from.
    pub(crate) fn from(choices: TierChoices) -> Self {
        Self {
            choices,
            counted: Counted::default(),
        }
    }

    /// How an attempt of a request of `label` at `url` on `domain_key`
    /// runs at `now` (milliseconds since the Unix epoch). A choice whose
    /// count of requests served moved is kept for the run file, so a
    /// resumed crawl goes on counting toward its refresh.
    pub(crate) fn serve(
        &mut self,
        label: &str,
        domain_key: &str,
        url: &str,
        now: u64,
    ) -> AttemptPlan {
        let (plan, unit) = self.choices.serve_attempt(label, domain_key, url, now);
        if let Some(unit) = unit {
            self.counted
                .0
                .insert((label.to_owned(), domain_key.to_owned()), unit.clone());
        }
        plan
    }

    /// Takes the sample `sampled`, as the weighing judged its runs at
    /// `now`; answers what its Label on its domain now learned, which its
    /// attempt's outcome writes into the run file in place of any count
    /// kept for it.
    pub(crate) fn classify(&mut self, sampled: &Sampled, weighed: Weighed, now: u64) -> Unit {
        let unit = self.choices.classify(sampled, weighed, now).clone();
        self.counted
            .0
            .remove(&(sampled.label.clone(), sampled.domain_key.clone()));
        unit
    }

    /// The choices that counted a request since the last call, for the
    /// run file.
    pub(crate) fn counted(&mut self) -> Counted {
        std::mem::take(&mut self.counted)
    }
}

/// The crawl's Run state, which its delivery loop owns.
pub(crate) struct RunState {
    pool: SessionPool,
    learned: Learned,
    latencies: Latencies,
}

impl RunState {
    /// The state of a crawl that draws from `pool` and learned `choices`.
    pub(crate) fn new(pool: SessionPool, choices: TierChoices) -> Self {
        Self {
            pool,
            learned: Learned::from(choices),
            latencies: Latencies::default(),
        }
    }

    /// The Session pool.
    pub(crate) fn pool(&mut self) -> &mut SessionPool {
        &mut self.pool
    }

    /// Draws the Session of the pool that serves `leased`, and chooses how
    /// its tier is chosen, as its Label and the Run's `fetch.tier` choose
    /// and, under `auto`, on the browser for a domain a Challenge moved
    /// there (ADR 0021), else, when the Run races its tiers, as a sample
    /// while its Label on its domain is due one, else as a race, else as
    /// its Label on its domain learned. A choice whose count of requests
    /// served moved is kept for the run file, so a resumed crawl goes on
    /// counting toward its refresh.
    pub(crate) fn draw(&mut self, drawing: &Drawing<'_>, leased: Leased) -> Drawn {
        let request = &leased.crawl_request;
        let labels = drawing.labels;
        let key = labels.pools.pool(&request.label);
        let session = self
            .pool
            .draw(&key, &leased.waiting.key, &drawing.settings, drawing.wall.0);
        let in_browser = PooledSession {
            id: session.id.clone(),
            source: labels.pools.source(&key),
            address_ordinal: self.pool.address_ordinal(&session.id),
        };
        let to_browser = self.pool.browser(&leased.waiting.key);
        let local = self.pool.local_addresses(&session.id);
        // A sitemap's fetch runs no handler and takes no tier choice: its
        // attempt fetches it on the Run's HTTP tier.
        let choosing =
            (!leased.sitemap).then(|| labels.settings.choose(&request.label, drawing.run_tier));
        // A request keeping its URL's fragment is served only by a tier
        // that runs the page's scripts, which the engine's choice is then
        // (ADR 0120); the start refused it on any forced tier but those.
        let keeps_fragment = !leased.sitemap && labels.fragments.keeps(&request.label);
        let browser = AttemptPlan::On(TierChoice {
            tier: FetchTier::Browser,
            forced: None,
        });
        let learned = &mut self.learned;
        let mut learned = || {
            learned.serve(
                &request.label,
                &leased.waiting.key,
                &request.url,
                drawing.wall.1,
            )
        };
        let chosen = match choosing {
            None => Chosen::Planned(AttemptPlan::Sitemap),
            Some(Choosing::Engine) if keeps_fragment => Chosen::Planned(browser),
            Some(Choosing::Engine) if drawing.races && !to_browser => Chosen::Racing(learned()),
            Some(Choosing::Forced(choice)) => Chosen::Planned(AttemptPlan::On(choice)),
            Some(Choosing::Engine) if to_browser => Chosen::Planned(browser),
            Some(Choosing::Engine) => Chosen::Planned(learned()),
        };
        Drawn {
            leased,
            session,
            in_browser,
            local,
            chosen,
        }
    }

    /// The choices a draw counted a request on since the last call, for
    /// the run file.
    pub(crate) fn counted(&mut self) -> Counted {
        self.learned.counted()
    }

    /// Takes the sample `sampled`, as [`Learned::classify`] does.
    pub(crate) fn classify(&mut self, sampled: &Sampled, weighed: Weighed, now: u64) -> Unit {
        self.learned.classify(sampled, weighed, now)
    }

    /// Takes `latency`, the browser-fetch latency of a race of `label` on
    /// `domain_key`.
    pub(crate) fn time(&mut self, label: &str, domain_key: &str, latency: Duration) {
        self.latencies.record(label, domain_key, latency);
    }

    /// How long a race of `label` on `domain_key` waits for its browser's
    /// navigation.
    pub(crate) fn race_delay(&self, label: &str, domain_key: &str) -> Duration {
        self.latencies.delay(label, domain_key)
    }
}

#[cfg(test)]
mod tests;
