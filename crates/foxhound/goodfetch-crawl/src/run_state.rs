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
//!
//! It also owns ADR 0140's F3 whole (#2167): every admission, the writes
//! that claim work items and those that end their attempts go through it.
//! The visited set and the pending work items are still the run file's, so
//! those operations are jobs of the Run's run-state writer, each queued by
//! the loop in its own order ([`Stored`]); #2168 and #2169 move what they
//! read into memory behind the same operations. Every other task reaches
//! it over the loop's one mailbox ([`Asking`]) and queues none of these
//! jobs with the writer itself. Its operations name no run file's type,
//! and fail with [`StateError`]. One job of a running crawl is not its
//! own yet and stays with the page's task that queues it: a handler
//! attempt's lesson to the Frontier (#2169).
//!
//! It counts the crawl's requests by state ([`Tally`]): read once from
//! the run file as the crawl opens, then counted from what each job
//! answers that it changed, so a live crawl's progress is built from this
//! state and reads no count from the run file (#2167).

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use goodfetch_engine::admission::Waiting;
use goodfetch_engine::local_addresses::{SessionAddresses, Unavailable};
use goodfetch_engine::run_state::{BatchRows, BatchWriter, RunStateError};
use goodfetch_engine::runs::{ChargeError, Run, UnjournaledCharges};
use goodfetch_fetch_tier::{Choosing, HttpFrom, PooledSession, SetTier, TierChoice};
use goodfetch_protocol::foxhound::fetch::FetchTier;
use goodfetch_protocol::foxhound::session_pool::ServingSession;
use goodfetch_session_pool::{Effect, LabelPools, PoolKey, PoolSettings, SendsFrom, SessionPool};

use crate::labels;
use crate::tier_choice::{AttemptPlan, Sampled, TierChoices, Unit, Weighed};
use crate::tier_race::Latencies;
use crate::work_items::{Changed, Leased};

pub(crate) mod admission;
mod asks;
mod stored;
mod tally;

pub(crate) use asks::{Asked, Asking, Escalated, Question, mailbox};
pub(crate) use stored::{QueuedEnd, Stored, Unkeyed, Within};
pub(crate) use tally::Tally;

/// Why the crawl's Run state did not take an operation.
#[derive(Debug, thiserror::Error)]
pub(crate) enum StateError {
    /// The crawl ended: no loop owns its Run state any more.
    #[error("the crawl ended, so nothing takes its Run state's operations")]
    Closed,
    /// The run file did not take a write or give a read.
    #[error(transparent)]
    RunFile(#[from] RunStateError),
    /// A claim's charges were refused or could not be journaled.
    #[error(transparent)]
    Charge(#[from] ChargeError),
}

impl StateError {
    /// The same as the run file's own error, for a seam that speaks it:
    /// what is no run file's error reads as its writer closed.
    pub(crate) fn run_file(self) -> RunStateError {
        match self {
            Self::RunFile(error) | Self::Charge(ChargeError::State(error)) => error,
            Self::Closed | Self::Charge(_) => RunStateError::Closed,
        }
    }
}

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

/// A Session drawn from the pool for an attempt, as it runs in the
/// browser, with its local addresses.
pub(crate) struct DrawnSession {
    pub(crate) session: ServingSession,
    pub(crate) in_browser: PooledSession,
    pub(crate) local: Local,
    /// The client it sends from.
    pub(crate) sends_from: SendsFrom,
}

/// A work item leased, with the Session drawn for its attempt, how its
/// tier is chosen, and, for a sample whose `http` run is sent from the
/// transport (`fetch.http_from`), the Session drawn for that run, which
/// shares nothing with the first.
pub(crate) struct Drawn {
    pub(crate) leased: Leased,
    pub(crate) session: DrawnSession,
    pub(crate) chosen: Chosen,
    pub(crate) transport: Option<DrawnSession>,
    /// The Session whose own transport reads the site files the attempt
    /// reads, robots.txt and the AI-use signal files, when neither of the
    /// others sends from one (#2422); none when its pool has no room for
    /// one and none is open, and the attempt's Session reads them from
    /// its own transport.
    pub(crate) site_files: Option<DrawnSession>,
}

/// What the Run's settings say of a draw, resolved as the Run opened, and
/// the Run's wall clock as the draw's lease was asked.
pub(crate) struct Drawing<'a> {
    pub(crate) labels: &'a labels::Declared,
    pub(crate) settings: PoolSettings,
    pub(crate) run_tier: SetTier,
    /// The Run's `fetch.http_from`, which a Label's own setting overrides.
    pub(crate) http_from: HttpFrom,
    pub(crate) races: bool,
    /// Whether the Run's crawl requests read site files, robots.txt or the
    /// AI-use signal files, and the Run has a transport to read them with.
    pub(crate) reads_site_files: bool,
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

    /// How an attempt of a request of `label` at `url` on `domain_key`,
    /// its HTTP sent `from` that client, runs at `now` (milliseconds since
    /// the Unix epoch). A choice whose count of requests served moved is
    /// kept for the run file, so a resumed crawl goes on counting toward
    /// its refresh.
    pub(crate) fn serve(
        &mut self,
        label: &str,
        domain_key: &str,
        url: &str,
        (from, now): (SendsFrom, u64),
    ) -> AttemptPlan {
        let (plan, unit) = self
            .choices
            .serve_attempt(label, domain_key, url, from, now);
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
    /// Its requests by state. Each value of the Run's progress shares the
    /// counts it was shown, so a change copies them once and no reader
    /// waits for the loop.
    tally: Arc<Tally>,
    stored: Stored,
}

impl RunState {
    /// The state of a crawl that draws from `pool`, learned `choices` and
    /// counts its requests on from `tally`, what is still the run file's
    /// read and written through `stored`.
    pub(crate) fn new(
        pool: SessionPool,
        choices: TierChoices,
        tally: Tally,
        stored: Stored,
    ) -> Self {
        Self {
            pool,
            learned: Learned::from(choices),
            latencies: Latencies::default(),
            tally: Arc::new(tally),
            stored,
        }
    }

    /// What is still the run file's, as the loop reads and writes it.
    pub(crate) fn stored(&self) -> &Stored {
        &self.stored
    }

    /// Counts what a job of the run file `changed` of the crawl's requests
    /// by state.
    pub(crate) fn changed(&mut self, changed: Changed) {
        if !changed.is_empty() {
            Arc::make_mut(&mut self.tally).change(changed);
        }
    }

    /// The crawl's requests by state as counted now, for a value of its
    /// Run's progress to share.
    pub(crate) fn tally(&self) -> Arc<Tally> {
        Arc::clone(&self.tally)
    }

    /// Answers `question`, what another task asks of the crawl's Run
    /// state: an escalated attempt's draw from memory
    /// ([`RunState::escalated`]), any other from the run file
    /// ([`Stored::answer`]), counting what its write changed: the work
    /// items it added, each keyed, for the loop to offer their domains'
    /// queues, when the loop waited for the write itself.
    pub(crate) async fn answer(&mut self, question: Box<Question>) -> Option<Vec<Waiting>> {
        let ask = match *question {
            Question::Stored(ask) => ask,
            Question::Escalated { escalated, answer } => {
                let drawn = self.escalated(&escalated);
                // The attempt no longer waits for it: it served nothing.
                if let Err(drawn) = answer.send(drawn) {
                    self.pool.settle(
                        &drawn.session,
                        &escalated.domain_key,
                        Effect::Neutral,
                        &escalated.settings,
                    );
                }
                return None;
            }
        };
        let (added, changed) = self.stored.answer(ask).await?;
        self.changed(changed);
        Some(added)
    }

    /// Draws the Session whose client is its browser that an attempt which
    /// escalated from a Session's own transport runs again in (#2416), as
    /// `escalated` says: as a lease's draw does, so one already open serves
    /// it while the pool has no room, and the pool keeps it for the run
    /// file as it keeps any draw.
    fn escalated(&mut self, escalated: &Escalated) -> DrawnSession {
        let session = self.pool.draw(
            &escalated.pool,
            &escalated.domain_key,
            SendsFrom::Browser,
            &escalated.settings,
            escalated.now,
        );
        self.drawn_session(
            (&escalated.labels.pools, &escalated.pool),
            session,
            SendsFrom::Browser,
        )
    }

    /// Claims each pending work item of `ids`, in that order, in one job
    /// of the run-state writer that also journals `charges`, what `run`
    /// charged for them (ADR 0138), and writes the rows the draws before it
    /// decided: what they changed in the pool, and each tier choice's
    /// count of requests served. Then draws, in memory, the Session that
    /// serves each and how its tier is chosen.
    ///
    /// # Errors
    ///
    /// The charges' refusal, or the run file's.
    pub(crate) async fn lease(
        &mut self,
        run: &Run,
        charges: UnjournaledCharges,
        ids: Vec<String>,
        drawing: &Drawing<'_>,
    ) -> Result<Vec<Drawn>, StateError> {
        let rows = (self.pool.changes(), self.learned.counted());
        let leased = run.write_charged(charges, Stored::claim(ids, rows)).await?;
        Ok(leased
            .into_iter()
            .map(|leased| self.draw(drawing, leased))
            .collect())
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
    /// its Label on its domain learned, its HTTP sent from the client its
    /// Label's `fetch.http_from`, else the Run's, says. A sample whose
    /// `http` run is sent from the transport draws a second Session for
    /// that run in the same draw. While the Run reads site files, an
    /// attempt whose Sessions all send from the browser draws, in the same
    /// draw, a Session of the transport to read them, one already open
    /// while there is one (#2422). A choice whose count of requests served
    /// moved is kept for the run file, so a resumed crawl goes on counting
    /// toward its refresh.
    fn draw(&mut self, drawing: &Drawing<'_>, leased: Leased) -> Drawn {
        let request = &leased.crawl_request;
        let labels = drawing.labels;
        let key = labels.pools.pool(&request.label);
        let to_browser = self.pool.browser(&leased.waiting.key);
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
        let from = if labels
            .settings
            .http_from(&request.label, drawing.http_from)
            .from_transport()
        {
            SendsFrom::Transport
        } else {
            SendsFrom::Browser
        };
        let learned = &mut self.learned;
        let mut learned = || {
            learned.serve(
                &request.label,
                &leased.waiting.key,
                &request.url,
                (from, drawing.wall.1),
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
        // The Session is drawn for the client its plan sends from, so it
        // never serves a request of the other one. A race, and a sample
        // that takes a Label's turn to race, serve their pages from the
        // browser.
        let sends_from = match &chosen {
            Chosen::Planned(plan) => plan.sends_from(from),
            Chosen::Racing(_) => SendsFrom::Browser,
        };
        let sampled_from_transport = match &chosen {
            Chosen::Planned(AttemptPlan::Sample(sampled))
            | Chosen::Racing(AttemptPlan::Sample(sampled)) => sampled.from == SendsFrom::Transport,
            Chosen::Planned(_) | Chosen::Racing(_) => false,
        };
        let domain_key = leased.waiting.key.as_str();
        let session = self.pool.draw(
            &key,
            domain_key,
            sends_from,
            &drawing.settings,
            drawing.wall.0,
        );
        let session = self.drawn_session((&labels.pools, &key), session, sends_from);
        let transport = sampled_from_transport.then(|| {
            let transport = self.pool.draw(
                &key,
                domain_key,
                SendsFrom::Transport,
                &drawing.settings,
                drawing.wall.0,
            );
            self.drawn_session((&labels.pools, &key), transport, SendsFrom::Transport)
        });
        // A site file is never read from a browser page, nor from a client
        // other than a Session's own transport (#2422).
        let site_files = (drawing.reads_site_files
            && sends_from == SendsFrom::Browser
            && transport.is_none())
        .then(|| {
            self.pool.draw_reusing(
                &key,
                domain_key,
                SendsFrom::Transport,
                &drawing.settings,
                drawing.wall.0,
            )
        })
        .flatten()
        .map(|reader| self.drawn_session((&labels.pools, &key), reader, SendsFrom::Transport));
        Drawn {
            leased,
            session,
            chosen,
            transport,
            site_files,
        }
    }

    /// `session`, drawn from the pool `key` of `pools`, sending from
    /// `sends_from`, as it runs in the browser, with its local addresses.
    fn drawn_session(
        &self,
        (pools, key): (&LabelPools, &PoolKey),
        session: ServingSession,
        sends_from: SendsFrom,
    ) -> DrawnSession {
        let in_browser = PooledSession {
            id: session.id.clone(),
            source: pools.source(key),
            address_ordinal: self.pool.address_ordinal(&session.id),
        };
        let local = self.pool.local_addresses(&session.id);
        DrawnSession {
            session,
            in_browser,
            local,
            sends_from,
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
