//! A crawl's work items, Records and outcomes in its run state (ADRs 0003,
//! 0079, 0087, 0102 and 0103): the tables Foxhound's contribution adds to
//! every run file, and the statements that move a work item from pending to
//! leased (committed before its handler request is sent, so no handler runs
//! on a lease the run file does not hold), to succeeded with its Records
//! under its lease, back to pending for a retry, or to failed, skipped or
//! blocked. A work item's every ending writes its outcome in the same
//! commit, in commit order, with the errors of the attempts that failed
//! since its previous ending, each kept as it failed. Each statement runs
//! inside one of the run-state writer's commits. Each pending work item
//! also waits under its domain key, priority and `forefront`, from which
//! its per-domain queue fills again in the same order, after a drain or a
//! resume (ADRs 0004 and 0005): a `forefront` work item keeps its own
//! position in `forefront`, so the later of two goes first and both ahead
//! of the rest. A work item is added once per identity, as the Frontier's
//! visited set, in the same run file, decides; it keeps the method and body
//! it is sent with.
//!
//! Size: one concern, the run state's work item tables and the statements
//! that move a work item through them; a fetched document's Provenance
//! marks (encoding, certificate exception, robots.txt) are fields its
//! commit writes, not a concern of their own.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::{Arc, LazyLock, OnceLock};
use std::time::SystemTime;

use goodfetch_counted_work::CountedWork;
use goodfetch_dataset::{Checked, Committing};
use goodfetch_engine::CallError;
use goodfetch_engine::admission::{Keys, Waiting};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::run_state::{BatchRows, BatchWriter, RunStateTables, run_row};
use goodfetch_frontier::payload::{self, Payload};
use goodfetch_frontier::trap_links::{self, Mark};
use goodfetch_frontier::{Candidate, Declared, Entering, Keyed, Url, sitemap};
use goodfetch_protocol::foxhound::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::{CrawlRequest, Seed, SitemapSeed};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpBody, HttpMethod, NotAddedReason};
use goodfetch_protocol::foxhound::outcomes::{
    AttemptError, CrawlRequestOutcome, DatasetRecords, Ending, NotAddedAtCommit, NothingCommitted,
    OutcomeKind, OutcomeRequest, SkipReason,
};
use goodfetch_protocol::foxhound::provenance::{
    CertificateExceptionMark, EncodingProvenance, RecordProvenance, RobotsTxtMark, TierProvenance,
};
use goodfetch_protocol::foxhound::report::SitemapReport;
use goodfetch_protocol::page::read::{TransformProvenance, UnreadField};
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Map, Value};

use crate::invariants::Invariants;

/// The tables of a crawl's work items, Records and outcomes.
pub(crate) const TABLES: RunStateTables = RunStateTables {
    name: "goodfetch-crawl",
    statements: &[
        "CREATE TABLE IF NOT EXISTS work_items (
            id TEXT NOT NULL PRIMARY KEY,
            position INTEGER NOT NULL UNIQUE,
            url TEXT NOT NULL,
            method TEXT NOT NULL,
            body BLOB,
            label TEXT NOT NULL,
            depth INTEGER NOT NULL,
            status TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            retried_from INTEGER NOT NULL,
            lease_epoch INTEGER NOT NULL,
            error_code TEXT,
            domain_key TEXT,
            priority INTEGER NOT NULL,
            forefront INTEGER
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS work_items_by_status ON work_items (status, position)",
        "CREATE INDEX IF NOT EXISTS work_items_unkeyed ON work_items (position)
            WHERE domain_key IS NULL",
        "CREATE INDEX IF NOT EXISTS work_items_by_domain
            ON work_items (status, domain_key, forefront DESC, priority DESC, position)",
        "CREATE TABLE IF NOT EXISTS attempt_errors (
            work_item TEXT NOT NULL,
            attempt INTEGER NOT NULL,
            code TEXT NOT NULL,
            message TEXT NOT NULL,
            details TEXT,
            PRIMARY KEY (work_item, attempt)
        ) STRICT",
        "CREATE TABLE IF NOT EXISTS outcomes (
            position INTEGER NOT NULL PRIMARY KEY,
            work_item TEXT NOT NULL,
            kind TEXT NOT NULL,
            label TEXT NOT NULL,
            attempts INTEGER NOT NULL,
            outcome TEXT NOT NULL
        ) STRICT",
        "CREATE INDEX IF NOT EXISTS outcomes_by_work_item ON outcomes (work_item, position)",
        "CREATE TABLE IF NOT EXISTS work_item_positions (
            id INTEGER NOT NULL PRIMARY KEY CHECK (id = 1),
            reserved INTEGER NOT NULL
        ) STRICT",
        // How many ended requests each latest outcome counts, by kind and
        // by skip reason, Error code or Block cause (`name`, empty for
        // `succeeded`), kept in the commit of each outcome and of each
        // re-admission, so a progress read decodes no outcome (#1547).
        "CREATE TABLE IF NOT EXISTS request_counts (
            kind TEXT NOT NULL,
            name TEXT NOT NULL,
            count INTEGER NOT NULL,
            PRIMARY KEY (kind, name)
        ) STRICT",
        // The Records each Dataset's succeeded requests committed, dropped
        // and rejected, kept as `request_counts` is.
        "CREATE TABLE IF NOT EXISTS record_counts (
            dataset TEXT NOT NULL PRIMARY KEY,
            committed INTEGER NOT NULL,
            dropped INTEGER NOT NULL,
            rejected INTEGER NOT NULL
        ) STRICT",
    ],
};

const PENDING: &str = "pending";
const LEASED: &str = "leased";
const SUCCEEDED: &str = "succeeded";
const FAILED: &str = "failed";
const SKIPPED: &str = "skipped";
const BLOCKED: &str = "blocked";

/// Whether a work item is still leased to an attempt, which every end of an
/// attempt checks first.
const STILL_LEASED: &str =
    "SELECT 1 FROM work_items WHERE id = ?1 AND status = 'leased' AND lease_epoch = ?2";

/// A work item's end with no error code: a page's, a sitemap's read and a
/// skip's.
const ENDED: &str = "UPDATE work_items SET status = ?1, error_code = NULL WHERE id = ?2";

/// The statements a sitemap's read runs beside the Frontier's, which a
/// page's end runs too: the Run prepares them as it starts or resumes, so
/// that the sitemap read never prepares one, whether or not a page ended
/// before it (ADR 0140, `F3.prepares.sitemap`).
pub(crate) const SITEMAP_READ: [&str; 2] = [STILL_LEASED, ENDED];

/// A work item leased to one attempt.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Leased {
    /// The work item as its handler receives it.
    pub(crate) crawl_request: CrawlRequest,
    /// Which attempt this is, from 1: attempts count on across a
    /// re-admission.
    pub(crate) attempt: u32,
    /// Which attempt this is since its latest admission, from 1, which
    /// `crawl.retry.max` bounds: a re-admission gives a fresh budget
    /// (ADR 0102).
    pub(crate) tries: u32,
    /// The lease's epoch.
    pub(crate) lease_epoch: u64,
    /// Its domain key and priority, under which it waits again on a retry.
    pub(crate) waiting: Waiting,
    /// Whether it is a sitemap's fetch, which runs no handler (ADR 0093).
    pub(crate) sitemap: bool,
    /// Whether it was added through a link its served document's markup
    /// hides, a suspected trap, which a browser tier never follows (ADR
    /// 0035); a link a person reveals in a menu, a tab or a disclosure
    /// leaves it unmarked here.
    pub(crate) trap_link_suspected: bool,
}

impl Leased {
    /// The same work item as its next lease takes it, once this attempt
    /// left it pending for a retry: one attempt and one try further, under
    /// the next lease epoch.
    pub(crate) fn next(self) -> Self {
        Self {
            attempt: self.attempt.saturating_add(1),
            tries: self.tries.saturating_add(1),
            lease_epoch: self.lease_epoch.saturating_add(1),
            ..self
        }
    }

    /// The same work item as its next lease takes it, once this attempt
    /// left it pending uncounted, released: the same attempt and try,
    /// under the next lease epoch.
    pub(crate) fn released(self) -> Self {
        Self {
            lease_epoch: self.lease_epoch.saturating_add(1),
            ..self
        }
    }

    /// The same work item as its next lease takes it, once this attempt
    /// left it as `again` says, if pending.
    pub(crate) fn after(self, again: Again) -> Option<Self> {
        match again {
            Again::Not => None,
            Again::Retried => Some(self.next()),
            Again::Released => Some(self.released()),
        }
    }
}

/// Whether an attempt that committed nothing left its work item pending
/// for another lease.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Again {
    /// It ended, or a later lease holds it.
    Not,
    /// Pending for a retry, its attempt counted.
    Retried,
    /// Pending, its attempt uncounted: the program gave its request back
    /// (ADR 0102).
    Released,
}

impl Again {
    /// Whether its work item waits in its queue again.
    pub(crate) fn waits(self) -> bool {
        !matches!(self, Self::Not)
    }
}

/// What an attempt that committed no Record failed with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Failure {
    /// Its Error code.
    pub(crate) code: String,
    /// What it said.
    pub(crate) message: String,
    /// The error's own details, such as the time a site's `Retry-After`
    /// asked for.
    pub(crate) details: Map<String, Value>,
}

impl Failure {
    /// A failure with the Error code `code` that says `message`, with no
    /// details.
    pub(crate) fn new(code: &str, message: &str) -> Self {
        Self {
            code: code.to_owned(),
            message: message.to_owned(),
            details: Map::new(),
        }
    }

    /// The same failure, its details naming `fix`.
    #[must_use]
    pub(crate) fn with_fix(mut self, fix: &str) -> Self {
        self.details
            .insert("fix".to_owned(), Value::String(fix.to_owned()));
        self
    }
}

/// How an attempt that committed no Record ends its work item.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Unsuccessful {
    /// Back to pending, its error kept for the work item's outcome.
    Retried(Failure),
    /// Failed, with no retry left.
    Failed(Failure),
    /// Skipped: by its handler with its message, by the site's
    /// preferences before its handler ran, or at a hop of its document's
    /// redirect chain.
    Skipped {
        reason: SkipReason,
        message: Option<String>,
    },
    /// Blocked by its site.
    Blocked(BlockCause),
    /// Back to pending with its attempt uncounted, its error as it was:
    /// the program gave its request back (ADR 0102).
    Released,
}

fn integer(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

/// What a work item the Frontier admits carries beside its URL.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Adding<'a> {
    /// The method it is sent with.
    pub(crate) method: &'a HttpMethod,
    /// The body it is sent with, if any.
    pub(crate) body: Option<&'a HttpBody>,
    /// Its Label.
    pub(crate) label: &'a str,
    /// Its Payload.
    pub(crate) payload: Option<&'a Arc<Payload>>,
    /// Its priority within its domain.
    pub(crate) priority: i64,
    /// Whether it goes ahead of every request its domain holds pending.
    pub(crate) forefront: bool,
    /// Its depth: 0 for a seed.
    pub(crate) depth: u32,
}

/// A `GET`, the method of a sitemap's fetch and of its entries.
static GET: LazyLock<HttpMethod> = LazyLock::new(HttpMethod::get);

/// What a sitemap's read adds: its fetch, a `GET` at depth 0 of no Label,
/// which runs no handler and writes no outcome, or one of its entries, a
/// `GET` at depth 0 of its rule's Label, with no Payload (ADR 0093).
fn from_sitemap(adding: sitemap::Adding<'_>) -> Adding<'_> {
    Adding {
        method: &GET,
        body: None,
        label: match adding {
            sitemap::Adding::Entry(label) => label,
            sitemap::Adding::Sitemap => "",
        },
        payload: None,
        priority: 0,
        forefront: false,
        depth: 0,
    }
}

/// A seed checked as [`check_seed`] does, its Payload serialized once,
/// for its check and for the run file (#1896).
#[derive(Clone, Debug)]
pub(crate) struct CheckedSeed {
    /// The seed, its Payload taken out into `payload`.
    seed: Seed,
    payload: Option<Arc<Payload>>,
}

impl CheckedSeed {
    /// A sitemap seed the engine adds beside the Client's, which carries
    /// no Payload and whose rules the engine wrote.
    pub(crate) const fn sitemap(seed: SitemapSeed) -> Self {
        Self {
            seed: Seed::Sitemap(seed),
            payload: None,
        }
    }

    /// The seed, its Payload taken out.
    pub(crate) const fn seed(&self) -> &Seed {
        &self.seed
    }
}

/// Checks each of `seeds` as [`check_seed`] does, failing at the first
/// that does not hold.
pub(crate) fn check_seeds(
    invariants: &Invariants,
    seeds: impl IntoIterator<Item = Seed>,
    work: &CountedWork,
) -> Result<Vec<CheckedSeed>, CallError> {
    seeds
        .into_iter()
        .map(|seed| check_seed(invariants, seed, work))
        .collect()
}

/// Checks `seed` before it is admitted: a request's Label, one the crawl
/// declares, and its Payload, serialized once, against its Label's schema
/// and the cap the Run's `invariants` hold, and a sitemap seed's rules,
/// each pattern one the engine matches (`pattern_unsupported`), kept
/// compiled for its sitemaps' reads, and each Label one the crawl declares
/// that takes no Payload, since an entry carries none.
///
/// # Errors
///
/// `label_unknown`, `payload_invalid` or `pattern_unsupported`.
pub(crate) fn check_seed(
    invariants: &Invariants,
    mut seed: Seed,
    work: &CountedWork,
) -> Result<CheckedSeed, CallError> {
    let payloads = invariants.payloads();
    match &mut seed {
        Seed::Request(request) => {
            let payload = request
                .payload
                .take()
                .map(|payload| Arc::new(Payload::of(payload, work)));
            payloads.check(&request.label, payload.as_deref(), work)?;
            Ok(CheckedSeed { seed, payload })
        }
        Seed::Sitemap(sitemap) => {
            invariants
                .frontier()
                .know_sitemap_rules(&sitemap.rules)
                .map_err(CallError::from)?;
            sitemap
                .rules
                .iter()
                .try_for_each(|rule| payloads.check(&rule.label, None, work))?;
            Ok(CheckedSeed {
                seed,
                payload: None,
            })
        }
    }
}

/// Reads `document`, what the sitemap fetch `id` holding its lease under
/// `lease_epoch` gave, admitting its entries and nested sitemaps within the
/// Scope of the Run's `invariants` and under its domain policy `policy`,
/// and ends the fetch, with no outcome. Each work item it adds is keyed
/// under `keys` as it is added, as an attempt's commit keys its links, so
/// its fetch's outcome is the one write its read asks for (#2167); answers
/// them keyed, in order, as they wait for their domains' queues, or `None`
/// when its lease no longer held. What it did counts in `changed`.
pub(crate) fn read_sitemap(
    connection: &Connection,
    invariants: &goodfetch_frontier::Invariants,
    policy: &Policy,
    (id, lease_epoch): (&str, u64),
    document: &sitemap::Document,
    (keys, work, changed): (&Keys, &CountedWork, &mut Changed),
) -> rusqlite::Result<Option<Vec<Waiting>>> {
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(None);
    }
    let mut positions = Positions::keying(connection, keys.clone())?;
    sitemap::read(
        connection,
        invariants,
        policy,
        id,
        document,
        work,
        |url, adding| add(connection, &mut positions, url, from_sitemap(adding)),
    )?;
    connection
        .prepare_cached(ENDED)
        .and_then(|mut cached| cached.execute((SUCCEEDED, id)))?;
    changed.fetches_ended += 1;
    Ok(Some(changed.adds(&mut positions)))
}

/// Admits each seed through the Frontier within the Run's `invariants` and
/// under its domain policy `policy`, as a pending work item at depth 0 when
/// added, carrying the Payload its check serialized, at its position from
/// `positions`, and answers each seed's answer, in order, its key keeping
/// its URL's fragment when its Label does.
pub(crate) fn admit_seeds(
    connection: &Connection,
    positions: &mut Positions,
    invariants: &Invariants,
    policy: &Policy,
    seeds: &[CheckedSeed],
) -> rusqlite::Result<Vec<Enqueued>> {
    seeds
        .iter()
        .map(|checked| {
            let seed = match &checked.seed {
                Seed::Request(seed) => seed,
                Seed::Sitemap(seed) => {
                    let answer = admit_sitemap_seed(
                        connection,
                        positions,
                        invariants.frontier(),
                        policy,
                        seed,
                    )?;
                    if matches!(answer, Enqueued::Added { .. }) {
                        positions.decide(|| {
                            Decided::Sitemap(Box::new((
                                invariants.shared_frontier(),
                                policy.clone(),
                                seed.clone(),
                            )))
                        });
                    }
                    return Ok(answer);
                }
            };
            let declared = Declared {
                method: &seed.method,
                body: seed.body.as_ref().map(HttpBody::bytes),
                key: seed.key.as_deref(),
                keeps_fragment: invariants.keeps_fragment(&seed.label),
            };
            let candidate = Candidate::Seed {
                url: &seed.url,
                declared,
            };
            let adding = Adding {
                method: &seed.method,
                body: seed.body.as_ref(),
                label: &seed.label,
                payload: checked.payload.as_ref(),
                priority: seed.priority,
                forefront: seed.forefront,
                depth: 0,
            };
            admit(connection, positions, invariants, policy, candidate, adding)
        })
        .collect()
}

/// Admits the sitemap seed `seed` through the Frontier within the Run's
/// `invariants` and under its domain policy `policy`: its sitemap's fetch,
/// added at its position from `positions`.
fn admit_sitemap_seed(
    connection: &Connection,
    positions: &mut Positions,
    invariants: &goodfetch_frontier::Invariants,
    policy: &Policy,
    seed: &SitemapSeed,
) -> rusqlite::Result<Enqueued> {
    let mut admitting = std::mem::take(&mut positions.admitting);
    let answer = sitemap::admit_seed_in(
        connection,
        invariants,
        policy,
        seed,
        &mut admitting,
        |url| {
            add(
                connection,
                positions,
                url,
                from_sitemap(sitemap::Adding::Sitemap),
            )
        },
    );
    positions.admitting = admitting;
    if matches!(answer, Ok(Enqueued::Added { .. })) {
        positions.sitemap_seed = true;
    }
    answer
}

/// Admits `candidate` through the Frontier within the Run's `invariants`
/// and under its domain policy `policy`, as a pending work item carrying
/// `adding` when added, at its position from `positions`, against what
/// `positions` holds beside the run file: an attempt's earlier staged
/// admissions while it stages, what its commit does not add at the commit
/// (ADR 0093). While an attempt stages, a candidate whose judgement wrote
/// to the run file is kept in `positions` as decided, for the attempt's
/// commit to apply (#2245).
pub(crate) fn admit(
    connection: &Connection,
    positions: &mut Positions,
    invariants: &Invariants,
    policy: &Policy,
    candidate: Candidate<'_>,
    adding: Adding<'_>,
) -> rusqlite::Result<Enqueued> {
    let (given, keyed, entering) =
        goodfetch_frontier::key_candidate(invariants.frontier(), policy, candidate);
    let coming = Coming {
        given,
        keyed: &keyed,
        entering,
        adding,
        via: Via::Request,
    };
    let (enqueued, again) = coming.admit(connection, positions, invariants.frontier())?;
    if again {
        positions.decide(|| Decided::Request(Box::new(coming.kept(invariants))));
    }
    Ok(enqueued)
}

/// Admits the link given as `given` and keyed as `keyed` through the
/// Frontier as [`admit`] does, entering as `entering` says: the part of a
/// page's link's admission that needs the Run's state (#2244), its key
/// made on the task that serves its call. The request it adds is marked as
/// `suspected` says its link is suspected a trap, and a request it reaches
/// again, answered `duplicate`, is told how it was seen (ADR 0035). While
/// an attempt stages, both are kept in `positions` as decided, for the
/// attempt's commit to apply (#2245).
///
/// # Errors
///
/// The run state's.
pub(crate) fn admit_keyed(
    connection: &Connection,
    positions: &mut Positions,
    invariants: &Invariants,
    (given, keyed): (&str, &Keyed),
    entering: Entering<'_>,
    (adding, suspected): (Adding<'_>, Option<Mark>),
) -> rusqlite::Result<Enqueued> {
    let coming = Coming {
        given,
        keyed,
        entering,
        adding,
        via: Via::Link(suspected),
    };
    let (enqueued, again) = coming.admit(connection, positions, invariants.frontier())?;
    if again {
        positions.decide(|| Decided::Request(Box::new(coming.kept(invariants))));
    } else if let Enqueued::NotAdded {
        reason: NotAddedReason::Duplicate,
        ..
    } = &enqueued
        && let Some(revealable) = reaching_again(suspected)
    {
        let frontier = invariants.shared_frontier();
        positions.decide(|| Decided::SeenAgain(Box::new((frontier, keyed.clone(), revealable))));
    }
    Ok(enqueued)
}

/// How a link suspected a trap as `suspected` says, or one a person sees
/// (`None`), answered `duplicate`, reaches the request it names again: as
/// one a person sees (`Some(false)`), which clears its mark, as one a
/// person reveals (`Some(true)`), which leaves it followed on every tier,
/// or as a suspected trap (`None`), which leaves its mark as it is.
const fn reaching_again(suspected: Option<Mark>) -> Option<bool> {
    match suspected {
        None => Some(false),
        Some(Mark::Revealable) => Some(true),
        Some(Mark::Hidden) => None,
    }
}

/// What gives a keyed candidate to the crawl.
#[derive(Clone, Copy, Debug)]
enum Via {
    /// A call that names the request itself: a seed, a handler's enqueue.
    Request,
    /// A link of a page, suspected a trap as its mark says (ADR 0035);
    /// `None` for one a person sees.
    Link(Option<Mark>),
}

/// A keyed candidate coming to the part of its admission that needs the
/// Run's state: as it was given and keyed, what it enters the crawl with,
/// what its work item carries and what gives it.
#[derive(Clone, Copy)]
struct Coming<'a> {
    given: &'a str,
    keyed: &'a Keyed,
    entering: Entering<'a>,
    adding: Adding<'a>,
    via: Via,
}

impl Coming<'_> {
    /// Admits it through the Frontier within the Run's `invariants`, at
    /// its position from `positions`, and, for a page's link, marks the
    /// request it adds as its link is suspected, or tells the request it
    /// reaches again how it was seen (ADR 0035). Answers, beside its
    /// answer, whether its judgement wrote to the run file what an
    /// attempt's staging rolls back, so that the attempt's commit judges
    /// it again: it was added, a bound refused it, counted, or a learned
    /// rule merged or refused it, recorded.
    fn admit(
        self,
        connection: &Connection,
        positions: &mut Positions,
        invariants: &goodfetch_frontier::Invariants,
    ) -> rusqlite::Result<(Enqueued, bool)> {
        let left = positions.admitting.left();
        let mut admitting = std::mem::take(&mut positions.admitting);
        let answer = goodfetch_frontier::admit_keyed(
            connection,
            invariants,
            (self.given, self.keyed),
            self.entering,
            &mut admitting,
            |url| add(connection, positions, url, self.adding),
        );
        positions.admitting = admitting;
        let enqueued = answer?;
        let again =
            matches!(enqueued, Enqueued::Added { .. }) || positions.admitting.left() != left;
        let Via::Link(suspected) = self.via else {
            return Ok((enqueued, again));
        };
        match &enqueued {
            Enqueued::Added { id, .. } => {
                if let Some(mark) = suspected {
                    trap_links::suspect(connection, invariants.unwritten(), id, mark)?;
                }
            }
            Enqueued::NotAdded {
                reason: NotAddedReason::Duplicate,
                ..
            } => {
                if let Some(revealable) = reaching_again(suspected) {
                    let unwritten = invariants.unwritten();
                    trap_links::seen_again(connection, unwritten, self.keyed, revealable)?;
                }
            }
            Enqueued::NotAdded { .. } => {}
        }
        Ok((enqueued, again))
    }

    /// It, owned, as an attempt's staging keeps a candidate its commit
    /// judges again, within the Run's `invariants`.
    fn kept(self, invariants: &Invariants) -> Request {
        Request {
            invariants: invariants.shared_frontier(),
            given: self.given.to_owned(),
            keyed: self.keyed.clone(),
            from: self.entering.from.cloned(),
            entering: self.entering.depth,
            method: self.adding.method.clone(),
            body: self.adding.body.cloned(),
            label: self.adding.label.to_owned(),
            payload: self.adding.payload.cloned(),
            priority: self.adding.priority,
            forefront: self.adding.forefront,
            depth: self.adding.depth,
            via: self.via,
        }
    }
}

/// What an attempt's staging decided, in the order it decided it, which
/// the attempt's commit applies in place of running the staged admission
/// again (#2245, closing #2069; ADR 0093): each candidate whose judgement
/// wrote to the run file, which the staging rolled back, and each request
/// a link of its page reached again. Any other candidate it answered not
/// added, a plain duplicate or one out of the scope among them, wrote
/// nothing, is decided for good, and is not judged again.
#[derive(Debug)]
pub(crate) enum Decided {
    /// A candidate whose judgement wrote to the run file: a request it
    /// answered `added`, one a bound refused, which the bound counts, or
    /// one a learned rule merged or refused, which the rule records.
    Request(Box<Request>),
    /// A sitemap seed it answered `added`, with the Frontier's invariants
    /// and the domain policy it was admitted under.
    Sitemap(Box<(Arc<goodfetch_frontier::Invariants>, Policy, SitemapSeed)>),
    /// A request the visited set holds that a link of the attempt's page
    /// reached again, seen as `revealable` says (ADR 0035).
    SeenAgain(Box<(Arc<goodfetch_frontier::Invariants>, Keyed, bool)>),
}

/// A candidate an attempt's commit judges again, with all it adds it
/// with: a [`Coming`], owned.
#[derive(Debug)]
pub(crate) struct Request {
    invariants: Arc<goodfetch_frontier::Invariants>,
    given: String,
    keyed: Keyed,
    from: Option<Url>,
    /// The depth it enters the crawl at.
    entering: u32,
    method: HttpMethod,
    body: Option<HttpBody>,
    label: String,
    payload: Option<Arc<Payload>>,
    priority: i64,
    forefront: bool,
    /// Its work item's depth.
    depth: u32,
    via: Via,
}

impl Decided {
    /// Applies it in `connection`'s transaction, at its attempt's commit,
    /// at the position `positions` kept for it. A candidate is judged
    /// again by the part of its admission that needs the Run's state,
    /// which reads the run file as it stands now: a seed that reaches it,
    /// its identity in the visited set, a learned refusal and the Bounds'
    /// counts. It is added, or its refusal counted or recorded, as the
    /// run file now says; a request the staging answered `added` that is
    /// not added is noted in `positions`.
    ///
    /// # Errors
    ///
    /// The run state's.
    pub(crate) fn apply(
        &self,
        connection: &Connection,
        positions: &mut Positions,
    ) -> rusqlite::Result<()> {
        match self {
            Self::Request(request) => {
                let coming = Coming {
                    given: &request.given,
                    keyed: &request.keyed,
                    entering: Entering {
                        from: request.from.as_ref(),
                        depth: request.entering,
                    },
                    adding: Adding {
                        method: &request.method,
                        body: request.body.as_ref(),
                        label: &request.label,
                        payload: request.payload.as_ref(),
                        priority: request.priority,
                        forefront: request.forefront,
                        depth: request.depth,
                    },
                    via: request.via,
                };
                coming
                    .admit(connection, positions, &request.invariants)
                    .map(drop)
            }
            Self::Sitemap(seed) => {
                let (invariants, policy, seed) = &**seed;
                admit_sitemap_seed(connection, positions, invariants, policy, seed).map(drop)
            }
            Self::SeenAgain(seen) => {
                let (invariants, keyed, revealable) = &**seen;
                trap_links::seen_again(connection, invariants.unwritten(), keyed, *revealable)
            }
        }
    }
}

/// The positions work items are added at, each its ID: after every work
/// item the run file holds and every position reserved for one an attempt
/// staged (ADR 0093), so a staged request keeps the ID its enqueue
/// answered once its attempt commits, and a discarded one leaves a gap.
/// Beside them, what the Frontier admits against beside the run file: an
/// attempt's earlier staged admissions while it stages one more, and the
/// URLs its commit does not add.
#[derive(Debug)]
pub(crate) struct Positions {
    /// The positions the staging of the admission being applied gave each
    /// URL it added.
    kept: BTreeMap<String, u64>,
    /// The URLs of `kept` given their position again.
    used: BTreeSet<String>,
    next: u64,
    /// The positions given since the last [`Positions::taken`], by URL.
    taken: BTreeMap<String, u64>,
    admitting: goodfetch_frontier::Admitting,
    /// How each work item it adds is keyed as it is added, and those it
    /// keyed, in order: an attempt's commit keys its links so (ADR 0138);
    /// `None` leaves them for the delivery to key.
    keyed: Option<(Keys, Vec<Waiting>)>,
    /// What the admission staged now decided, in order, while an attempt
    /// stages: what its commit applies (#2245).
    decided: Option<Vec<Decided>>,
    /// How many work items it added.
    added: u64,
    /// Whether a sitemap seed is among them.
    sitemap_seed: bool,
}

impl Positions {
    /// The positions after everything the run file `connection` holds,
    /// admitting against the run file alone.
    pub(crate) fn of(connection: &Connection) -> rusqlite::Result<Self> {
        let next: i64 = connection
            .prepare_cached(
                "SELECT max(
                 (SELECT coalesce(max(position), 0) FROM work_items),
                 (SELECT coalesce(max(reserved), 0) FROM work_item_positions)
             ) + 1",
            )
            .and_then(|mut cached| cached.query_row([], |row| row.get(0)))?;
        Ok(Self {
            kept: BTreeMap::new(),
            used: BTreeSet::new(),
            next: u64::try_from(next).unwrap_or(1),
            taken: BTreeMap::new(),
            admitting: goodfetch_frontier::Admitting::RunFile,
            keyed: None,
            decided: None,
            added: 0,
            sitemap_seed: false,
        })
    }

    /// The same, keying each work item it adds under `keys` as it adds
    /// it: an admission with no attempt, which nothing keys later.
    pub(crate) fn keying(connection: &Connection, keys: Keys) -> rusqlite::Result<Self> {
        Ok(Self {
            keyed: Some((keys, Vec::new())),
            ..Self::of(connection)?
        })
    }

    /// The same, admitting while an attempt stages, against what its
    /// earlier staged admissions did, `earlier`.
    pub(crate) fn staging(
        connection: &Connection,
        earlier: std::sync::Arc<goodfetch_frontier::Staged>,
    ) -> rusqlite::Result<Self> {
        Ok(Self {
            admitting: goodfetch_frontier::Admitting::Staging {
                earlier,
                now: goodfetch_frontier::Staged::default(),
            },
            decided: Some(Vec::new()),
            ..Self::of(connection)?
        })
    }

    /// Keeps what `decided` answers, while an attempt stages.
    fn decide(&mut self, decided: impl FnOnce() -> Decided) {
        if let Some(kept) = &mut self.decided {
            kept.push(decided());
        }
    }

    /// What the admission staged now decided, in order, for its attempt's
    /// commit to apply; nothing outside a staging.
    pub(crate) fn decided(&mut self) -> Vec<Decided> {
        self.decided.take().unwrap_or_default()
    }

    /// The same, admitting at an attempt's commit, which notes each URL it
    /// does not add, and keys each work item it adds under `keys`, when
    /// given, as it adds it.
    pub(crate) fn committing(
        connection: &Connection,
        keys: Option<Keys>,
    ) -> rusqlite::Result<Self> {
        Ok(Self {
            admitting: goodfetch_frontier::Admitting::Commit {
                refused: Vec::new(),
            },
            keyed: keys.map(|keys| (keys, Vec::new())),
            ..Self::of(connection)?
        })
    }

    /// Each work item it added keyed, in order, as it waits for its
    /// domain's queue.
    pub(crate) fn keyed(&mut self) -> Vec<Waiting> {
        self.keyed
            .as_mut()
            .map(|(_, keyed)| std::mem::take(keyed))
            .unwrap_or_default()
    }

    /// Gives each URL in `kept` the position it holds there from now on,
    /// as what an admission staged before decided is applied.
    pub(crate) fn keeping(&mut self, kept: BTreeMap<String, u64>) {
        self.kept = kept;
        self.used.clear();
        self.taken.clear();
        if let goodfetch_frontier::Admitting::Commit { refused } = &mut self.admitting {
            refused.clear();
        }
    }

    /// The position of the work item added at `url`.
    fn take(&mut self, url: &str) -> u64 {
        if let Some(kept) = self.kept.get(url) {
            self.used.insert(url.to_owned());
            return *kept;
        }
        let position = self.next;
        self.next += 1;
        self.taken.insert(url.to_owned(), position);
        position
    }

    /// The positions given since the last call, by URL.
    pub(crate) fn taken(&mut self) -> BTreeMap<String, u64> {
        std::mem::take(&mut self.taken)
    }

    /// What the staged admission did beside the run file, once it ran, as
    /// the attempt's later admissions read it.
    pub(crate) fn staged(self) -> goodfetch_frontier::Staged {
        match self.admitting {
            goodfetch_frontier::Admitting::Staging { now, .. } => now,
            goodfetch_frontier::Admitting::RunFile
            | goodfetch_frontier::Admitting::Commit { .. } => goodfetch_frontier::Staged::default(),
        }
    }

    /// Each request the admission applied at the commit kept a position
    /// for and did not add, since [`Positions::keeping`]: the ID its
    /// staging answered, its URL and why the commit did not add it.
    pub(crate) fn not_added(&self) -> Vec<NotAddedAtCommit> {
        let goodfetch_frontier::Admitting::Commit { refused } = &self.admitting else {
            return Vec::new();
        };
        refused
            .iter()
            .filter(|refused| !self.used.contains(&refused.url))
            .filter_map(|refused| {
                let id = self.kept.get(&refused.url)?;
                Some(NotAddedAtCommit {
                    id: id.to_string(),
                    url: refused.url.clone(),
                    reason: refused.reason.clone(),
                    duplicate_of: refused.duplicate_of.clone(),
                })
            })
            .collect()
    }

    /// Reserves in the run file every position given so far, so that no
    /// later work item takes one, though none of it is written yet.
    pub(crate) fn reserve(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection
            .prepare_cached(
                "INSERT INTO work_item_positions (id, reserved) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET reserved = max(reserved, excluded.reserved)",
            )
            .and_then(|mut cached| cached.execute([integer(self.next.saturating_sub(1))]))?;
        Ok(())
    }
}

/// Adds a request to `url`, the URL it is fetched at, as a pending work
/// item at its position from `positions`, its Payload kept apart in the
/// Frontier's own table, and answers its ID, its position.
pub(crate) fn add(
    connection: &Connection,
    positions: &mut Positions,
    url: &Url,
    adding: Adding<'_>,
) -> rusqlite::Result<String> {
    let position = integer(positions.take(url.as_str()));
    let key = positions
        .keyed
        .as_ref()
        .map(|(keys, _)| keys.key(url.as_str()));
    let id: String = connection.prepare_cached("INSERT INTO work_items
             (id, position, url, method, body, label, depth, status, attempts, retried_from, lease_epoch, priority, forefront, domain_key)
         VALUES (CAST(?1 AS TEXT), ?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, 0, 0, ?7, CASE WHEN ?8 THEN ?1 END, ?9)
         RETURNING id").and_then(|mut cached| cached.query_row((
            position,
            url.as_str(),
            adding.method.as_str(),
            adding.body.map(HttpBody::bytes),
            adding.label,
            adding.depth,
            adding.priority,
            adding.forefront,
            key.as_deref(),
        ), |row| row.get(0)))?;
    payload::store(connection, &id, adding.payload.map(|payload| &**payload))?;
    positions.added += 1;
    if let (Some(key), Some((_, keyed))) = (key, positions.keyed.as_mut()) {
        keyed.push(Waiting {
            id: id.clone(),
            key,
            priority: adding.priority,
            order: position,
            forefront: adding.forefront,
        });
    }
    Ok(id)
}

/// The IDs of the work items of `label` not ended, pending or leased, in
/// order: those a resume would deliver.
pub(crate) fn pending_of(connection: &Connection, label: &str) -> rusqlite::Result<Vec<String>> {
    connection
        .prepare_cached(
            "SELECT id FROM work_items WHERE label = ?1 AND status IN ('pending', 'leased')
             ORDER BY position",
        )?
        .query_map([label], |row| row.get(0))?
        .collect()
}

/// The Labels of the work items not ended, pending or leased, each once, in
/// order: those a resume would deliver. A sitemap's fetch, whose Label is
/// the engine's empty one, is none of them.
pub(crate) fn pending_labels(connection: &Connection) -> rusqlite::Result<Vec<String>> {
    connection
        .prepare_cached(
            "SELECT DISTINCT label FROM work_items
             WHERE status IN ('pending', 'leased') AND label <> '' ORDER BY label",
        )?
        .query_map([], |row| row.get(0))?
        .collect()
}

/// The URL of the crawl request `id`, as its work item holds it; `None`
/// for an ID no work item has. The reader other crates read the
/// `work_items` table through, such as a Dataset's rejection table for each
/// entry's URL.
///
/// # Errors
///
/// When the run file cannot be read.
pub fn url_of(connection: &Connection, id: &str) -> rusqlite::Result<Option<String>> {
    connection
        .prepare_cached("SELECT url FROM work_items WHERE id = ?1")
        .and_then(|mut cached| cached.query_row([id], |row| row.get(0)))
        .optional()
}

/// The URL and depth of the work item `id` while its attempt under
/// `lease_epoch` holds its lease; `None` once that attempt ended.
pub(crate) fn attempt(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
) -> rusqlite::Result<Option<(String, u32)>> {
    connection
        .prepare_cached("SELECT url, depth FROM work_items WHERE id = ?1 AND status = 'leased' AND lease_epoch = ?2").and_then(|mut cached| cached.query_row((id, integer(lease_epoch)), |row| Ok((row.get(0)?, row.get(1)?))))
        .optional()
}

/// The first `limit` work items not keyed yet, in order: each one's URL,
/// and how it waits once keyed, its key still empty.
pub(crate) fn unkeyed(
    connection: &Connection,
    limit: usize,
) -> rusqlite::Result<Vec<(String, Waiting)>> {
    connection
        .prepare_cached(
            "SELECT url, id, priority, position, forefront IS NOT NULL FROM work_items
             WHERE domain_key IS NULL ORDER BY position LIMIT ?1",
        )?
        .query_map([i64::try_from(limit).unwrap_or(i64::MAX)], |row| {
            Ok((
                row.get(0)?,
                Waiting {
                    id: row.get(1)?,
                    key: String::new(),
                    priority: row.get(2)?,
                    order: row.get(3)?,
                    forefront: row.get(4)?,
                },
            ))
        })?
        .collect()
}

/// Work items' domain keys, each with its item's ID, as the delivery loop
/// keyed them: a Run-state batch's rows (ADR 0142).
pub(crate) struct DomainKeys(pub(crate) Vec<(String, String)>);

impl BatchRows for DomainKeys {
    fn write(self, writer: &BatchWriter<'_>) -> rusqlite::Result<()> {
        let mut update = writer.statement("UPDATE work_items SET domain_key = ?2 WHERE id = ?1")?;
        for (id, key) in &self.0 {
            update.write((id, key))?;
        }
        Ok(())
    }
}

/// The pending work items of `key`, or of every key when `None`, at most
/// `bound` of each key, in the order its per-domain queue admits them
/// (`forefront` ones latest first, then highest priority first, then in
/// order): what fills that queue.
pub(crate) fn waiting(
    connection: &Connection,
    key: Option<&str>,
    bound: usize,
) -> rusqlite::Result<Vec<Waiting>> {
    let bound = i64::try_from(bound).unwrap_or(i64::MAX);
    let row = |row: &rusqlite::Row<'_>| {
        Ok(Waiting {
            id: row.get(0)?,
            key: row.get(1)?,
            priority: row.get(2)?,
            order: row.get(3)?,
            forefront: row.get(4)?,
        })
    };
    match key {
        Some(key) => connection
            .prepare_cached(
                "SELECT id, domain_key, priority, position, forefront IS NOT NULL FROM work_items
                 WHERE status = 'pending' AND domain_key = ?1
                 ORDER BY forefront DESC, priority DESC, position LIMIT ?2",
            )?
            .query_map((key, bound), row)?
            .collect(),
        None => connection
            .prepare_cached(
                "SELECT id, domain_key, priority, position, forefront IS NOT NULL FROM (
                     SELECT id, domain_key, priority, position, forefront, row_number() OVER (
                         PARTITION BY domain_key ORDER BY forefront DESC, priority DESC, position
                     ) AS rank
                     FROM work_items WHERE status = 'pending' AND domain_key IS NOT NULL
                 ) WHERE rank <= ?1 ORDER BY position",
            )?
            .query_map([bound], row)?
            .collect(),
    }
}

/// Leases each pending work item of `ids`, in that order: each one's
/// attempts and lease epoch go up by one.
pub(crate) fn lease(connection: &Connection, ids: &[String]) -> rusqlite::Result<Vec<Leased>> {
    let mut statement = connection.prepare_cached("UPDATE work_items SET status = 'leased', attempts = attempts + 1, lease_epoch = lease_epoch + 1
         WHERE id = ?1 AND status = 'pending'
         RETURNING id, url, label, attempts, lease_epoch, domain_key, priority, position,
             forefront IS NOT NULL, method, body, attempts - retried_from")?;
    leased_rows(connection, &mut statement, ids)
}

/// Each pending work item of `ids`, in that order, as its next lease takes
/// it: its attempts and lease epoch one up, which [`claim`] then writes.
/// It changes nothing, so it reads on one of the run file's readers.
pub(crate) fn leasable(connection: &Connection, ids: &[String]) -> rusqlite::Result<Vec<Leased>> {
    let mut statement = connection.prepare_cached(
        "SELECT id, url, label, attempts + 1, lease_epoch + 1, domain_key, priority, position,
             forefront IS NOT NULL, method, body, attempts + 1 - retried_from
         FROM work_items WHERE id = ?1 AND status = 'pending'",
    )?;
    leased_rows(connection, &mut statement, ids)
}

/// Each of `added`, the work items a write just added, keyed, in order, as
/// its first lease takes it, read in that write's transaction.
pub(crate) fn leasable_added(
    connection: &Connection,
    added: &[Waiting],
) -> rusqlite::Result<Vec<Leased>> {
    let ids: Vec<String> = added.iter().map(|waiting| waiting.id.clone()).collect();
    let mut rows = leasable(connection, &ids)?;
    // Each waits as it was keyed, which the row holds once its batch does.
    for (leased, waiting) in rows.iter_mut().zip(added) {
        if leased.waiting.id == waiting.id {
            leased.waiting = waiting.clone();
        }
    }
    Ok(rows)
}

/// Leases each work item of `claims` that is still pending as it was when
/// its row was read ([`leasable`]): its lease epoch one below the one
/// given. Answers the IDs it did not lease, whose attempts then end with
/// their lease lost.
pub(crate) fn claim(
    connection: &Connection,
    claims: &[(String, u64)],
) -> rusqlite::Result<Vec<String>> {
    let mut statement = connection.prepare_cached(
        "UPDATE work_items SET status = 'leased', attempts = attempts + 1, lease_epoch = lease_epoch + 1
         WHERE id = ?1 AND status = 'pending' AND lease_epoch + 1 = ?2",
    )?;
    let mut unclaimed = Vec::new();
    for (id, lease_epoch) in claims {
        if statement.execute((id, integer(*lease_epoch)))? == 0 {
            unclaimed.push(id.clone());
        }
    }
    Ok(unclaimed)
}

/// Each row `statement` answers for each of `ids`, in that order, as a
/// lease of it: its ID, URL, Label, attempts, lease epoch, domain key,
/// priority, position, whether it goes to the forefront, method, body and
/// tries since its latest admission.
fn leased_rows(
    connection: &Connection,
    statement: &mut rusqlite::CachedStatement<'_>,
    ids: &[String],
) -> rusqlite::Result<Vec<Leased>> {
    let mut leased = Vec::with_capacity(ids.len());
    for id in ids {
        let row = statement
            .query_row([id], |row| {
                Ok(Leased {
                    crawl_request: CrawlRequest {
                        id: row.get(0)?,
                        url: row.get(1)?,
                        label: row.get(2)?,
                        payload: None,
                        method: HttpMethod::parse(&row.get::<_, String>(9)?).map_err(|error| {
                            rusqlite::Error::FromSqlConversionFailure(
                                9,
                                rusqlite::types::Type::Text,
                                Box::new(error),
                            )
                        })?,
                        body: row.get::<_, Option<Vec<u8>>>(10)?.map(HttpBody::new),
                    },
                    attempt: row.get(3)?,
                    tries: row.get(11)?,
                    lease_epoch: u64::try_from(row.get::<_, i64>(4)?).unwrap_or_default(),
                    waiting: Waiting {
                        id: row.get(0)?,
                        key: row.get::<_, Option<String>>(5)?.unwrap_or_default(),
                        priority: row.get(6)?,
                        order: row.get(7)?,
                        forefront: row.get(8)?,
                    },
                    sitemap: false,
                    trap_link_suspected: false,
                })
            })
            .optional()?;
        if let Some(mut row) = row {
            row.crawl_request.payload = payload::of_request(connection, &row.crawl_request.id)?;
            row.sitemap = sitemap::is_fetch(connection, &row.crawl_request.id)?;
            row.trap_link_suspected = trap_links::suspected(connection, &row.crawl_request.id)?
                == Some(trap_links::Mark::Hidden);
            leased.push(row);
        }
    }
    Ok(leased)
}

/// Whether the work item `id` is still leased under `lease_epoch`.
fn still_leased(connection: &Connection, id: &str, lease_epoch: u64) -> rusqlite::Result<bool> {
    Ok(connection
        .prepare_cached(STILL_LEASED)
        .and_then(|mut cached| cached.query_row((id, integer(lease_epoch)), |_| Ok(())))
        .optional()?
        .is_some())
}

/// How an attempt's run fetched its document, as its Records' Provenance
/// names it: nothing until a run fetched one.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Fetched {
    /// The tier, and how it was chosen.
    pub(crate) tier: Option<TierProvenance>,
    /// The encoding its document was read in, and where it came from.
    pub(crate) encoding: Option<EncodingProvenance>,
    /// The invalid-certificate exception its document's host was reached
    /// through (ADR 0042).
    pub(crate) certificate_exception: Option<CertificateExceptionMark>,
    /// `ignored` when its request's URL is one its origin's robots.txt
    /// disallows, fetched while `site_preferences.respect_robots_txt` was
    /// off (ADR 0086).
    pub(crate) robots_txt: Option<RobotsTxtMark>,
}

/// What a code-free crawl's attempt read of its page's fields, as its
/// Record's Provenance names it (ADR 0123): each Value transform that gave
/// a field, and each field the page did not give; nothing for a crawl
/// whose program handles its requests.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct FieldsRead {
    pub(crate) value_transforms: Vec<TransformProvenance>,
    pub(crate) unread: Vec<UnreadField>,
}

/// What an attempt commits: its Records, how its run fetched its
/// document, what it read of its page's fields, the masks that hold the
/// values its Run resolved, and when it ended.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Committed<'a> {
    pub(crate) records: &'a Checked,
    pub(crate) fetched: &'a Fetched,
    pub(crate) fields: &'a FieldsRead,
    pub(crate) masks: &'a Masks,
    pub(crate) ended_at: SystemTime,
    /// Each request the attempt staged that the commit did not add.
    pub(crate) not_added: &'a [NotAddedAtCommit],
    /// Where its Records' work is counted (F4, ADR 0140).
    pub(crate) work: &'a CountedWork,
    /// Whether the crawl keeps Records, its `dataset.records`: off, the
    /// commit writes none and reads nothing for one (ADR 0138).
    pub(crate) keeps_records: bool,
    /// The Clean-up rules its Run holds to, which its Records' Provenance
    /// names.
    pub(crate) clean_up_rules: &'a CleanUpRules,
}

/// The digest of the Clean-up rules a Run holds to, which the Provenance of
/// each of its Records names. The Run's start records it and nothing
/// changes it, so a crawl reads it from its run file once, in its first
/// commit that keeps Records, and for no Record after (ADR 0140's F4). A
/// clone shares what was read.
#[derive(Clone, Debug, Default)]
pub(crate) struct CleanUpRules(Arc<OnceLock<Option<String>>>);

impl CleanUpRules {
    /// The digest, read from the run file `connection` holds unless a
    /// commit read it before.
    fn digest(&self, connection: &Connection) -> rusqlite::Result<Option<String>> {
        if let Some(read) = self.0.get() {
            return Ok(read.clone());
        }
        let digest = run_row::clean_up_digest(connection)?;
        Ok(self.0.get_or_init(|| digest).clone())
    }
}

/// Commits an attempt's Records, once, each numbered in its Dataset and
/// naming its Provenance: its request, how its document was fetched and
/// the Clean-up rules the Run holds to; marks its work item succeeded and
/// writes its outcome, ended at `ended_at`, when its lease is the one the
/// run file holds; answers whether it did. Its ending counts in `changed`.
pub(crate) fn commit(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
    committed: Committed<'_>,
    changed: &mut Changed,
) -> rusqlite::Result<bool> {
    let Committed {
        records,
        fetched,
        fields,
        masks,
        ended_at,
        not_added,
        work,
        keeps_records,
        clean_up_rules,
    } = committed;
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    let counted = if keeps_records {
        commit_records(
            connection,
            id,
            (records, fetched, fields),
            masks,
            clean_up_rules,
            ended_at,
            work,
        )?
    } else {
        Vec::new()
    };
    connection
        .prepare_cached(ENDED)
        .and_then(|mut cached| cached.execute((SUCCEEDED, id)))?;
    let nothing_committed = if !keeps_records {
        Some(NothingCommitted::RecordsOff)
    } else if counted.is_empty() {
        Some(NothingCommitted::NonePushed)
    } else if counted.iter().all(|dataset| dataset.committed == 0) {
        Some(NothingCommitted::NoneKept)
    } else {
        None
    };
    write_outcome(
        connection,
        id,
        ended_at,
        Ending::Succeeded {
            records: counted,
            nothing_committed,
            not_added: not_added.to_vec(),
        },
        changed,
    )?;
    Ok(true)
}

/// Commits the attempt `id`'s Records, each numbered in its Dataset and
/// naming its Provenance: its request, how its document was fetched and
/// the Clean-up rules the Run holds to; answers what each Dataset counted.
fn commit_records(
    connection: &Connection,
    id: &str,
    (records, fetched, fields): (&Checked, &Fetched, &FieldsRead),
    masks: &Masks,
    clean_up_rules: &CleanUpRules,
    ended_at: SystemTime,
    work: &CountedWork,
) -> rusqlite::Result<Vec<DatasetRecords>> {
    let url = url_of(connection, id)?.ok_or(rusqlite::Error::QueryReturnedNoRows)?;
    let clean_up_rules = clean_up_rules.digest(connection)?;
    let provenance = RecordProvenance {
        request_id: id.to_owned(),
        tier: fetched.tier.clone(),
        encoding: fetched.encoding.clone(),
        origin: None,
        clean_up_rules,
        wall: None,
        payment: None,
        certificate_exception: fetched.certificate_exception.clone(),
        robots_txt: fetched.robots_txt,
        value_transforms: fields.value_transforms.clone(),
        unread_fields: fields.unread.clone(),
    };
    let committing = Committing {
        request_id: id,
        url: &url,
        at: &ended_at,
        provenance: &provenance,
        masks,
        work,
    };
    goodfetch_dataset::commit(connection, committing, records)
}

/// Ends an attempt that committed nothing as `unsuccessful` says, ended at
/// `ended_at`, when its lease is the one the run file holds; answers
/// whether it did. Its ending counts in `changed`; a retry leaves it
/// pending and counts nothing, and a release leaves it pending as it was
/// before its lease, as [`release_leases`] does.
pub(crate) fn end(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
    unsuccessful: Unsuccessful,
    ended_at: SystemTime,
    changed: &mut Changed,
) -> rusqlite::Result<bool> {
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    let set = |status: &str, error_code: Option<&str>| {
        connection
            .prepare_cached("UPDATE work_items SET status = ?1, error_code = ?2 WHERE id = ?3")
            .and_then(|mut cached| cached.execute((status, error_code, id)))
    };
    let ending = match unsuccessful {
        Unsuccessful::Retried(failure) => {
            set(PENDING, Some(&failure.code))?;
            keep_error(connection, id, &failure)?;
            return Ok(true);
        }
        Unsuccessful::Failed(failure) => {
            set(FAILED, Some(&failure.code))?;
            Ending::Failed {
                error: keep_error(connection, id, &failure)?,
            }
        }
        Unsuccessful::Skipped { reason, message } => {
            set(SKIPPED, None)?;
            Ending::Skipped { reason, message }
        }
        Unsuccessful::Blocked(cause) => {
            set(BLOCKED, None)?;
            Ending::Blocked { cause }
        }
        Unsuccessful::Released => {
            connection
                .prepare_cached(
                    "UPDATE work_items SET status = ?1, attempts = attempts - 1 WHERE id = ?2",
                )
                .and_then(|mut cached| cached.execute((PENDING, id)))?;
            return Ok(true);
        }
    };
    write_outcome(connection, id, ended_at, ending, changed)?;
    Ok(true)
}

/// Ends the pending work items a rule the Frontier learned later merged
/// (ADR 0093): given the crawl requests it showed to be one page, each
/// still pending ends `skipped` with `duplicate` at `ended_at`, its
/// handler never run, unless none of them was leased or ended otherwise
/// than so, when the first pending one stays; one ended so and given back
/// since ([`give_back`]) counts by its latest outcome. Answers each one it
/// ended, with its URL and the domain key it waits under, if keyed; each
/// counts in `changed`.
pub(crate) fn end_merged(
    connection: &Connection,
    ids: &BTreeSet<String>,
    ended_at: SystemTime,
    changed: &mut Changed,
) -> rusqlite::Result<Vec<(String, String, Option<String>)>> {
    let mut members = Vec::with_capacity(ids.len());
    for id in ids {
        let member = connection
            .prepare_cached(
                "SELECT position, status, url, domain_key, coalesce((
                     SELECT kind = ?2 AND json_extract(outcome, '$.reason') = 'duplicate'
                     FROM outcomes WHERE work_item = ?1 ORDER BY position DESC LIMIT 1), 0)
                 FROM work_items WHERE id = ?1",
            )
            .and_then(|mut cached| {
                cached.query_row((id, SKIPPED), |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                })
            })
            .optional()?;
        members.extend(member.map(|member| (id.clone(), member)));
    }
    members.sort_by_key(|(_, (position, ..))| *position);
    let kept = members
        .iter()
        .any(|(_, (_, status, _, _, merged))| status != PENDING && !merged);
    let mut pending = members
        .into_iter()
        .filter(|(_, (_, status, ..))| status == PENDING);
    if !kept {
        pending.next();
    }
    let mut ended = Vec::new();
    for (id, (_, _, url, key, _)) in pending {
        connection
            .prepare_cached(ENDED)
            .and_then(|mut cached| cached.execute((SKIPPED, &id)))?;
        write_outcome(
            connection,
            &id,
            ended_at,
            Ending::Skipped {
                reason: SkipReason::Duplicate,
                message: None,
            },
            changed,
        )?;
        ended.push((id, url, key));
    }
    Ok(ended)
}

/// Keeps the error of the work item `id`'s current attempt for its next
/// outcome.
fn keep_error(
    connection: &Connection,
    id: &str,
    failure: &Failure,
) -> rusqlite::Result<AttemptError> {
    let attempt: u32 = connection
        .prepare_cached("SELECT attempts FROM work_items WHERE id = ?1")
        .and_then(|mut cached| cached.query_row([id], |row| row.get(0)))?;
    connection
        .prepare_cached(
            "INSERT INTO attempt_errors (work_item, attempt, code, message, details)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        )
        .and_then(|mut cached| {
            cached.execute((
                id,
                attempt,
                &failure.code,
                &failure.message,
                (!failure.details.is_empty())
                    .then(|| Value::Object(failure.details.clone()).to_string()),
            ))
        })?;
    Ok(AttemptError {
        attempt,
        code: failure.code.clone(),
        message: failure.message.clone(),
        details: failure.details.clone(),
    })
}

/// Writes the outcome of the work item `id`, next in commit order, with
/// the errors of its attempts since its previous outcome. Attempts count on
/// across a re-admission, so the attempts of each outcome are the work
/// item's attempts so far.
fn write_outcome(
    connection: &Connection,
    id: &str,
    ended_at: SystemTime,
    ending: Ending,
    changed: &mut Changed,
) -> rusqlite::Result<()> {
    // A sitemap's fetch is no crawl request: its sitemap's counts, or why
    // it could not be read, are its ending (ADR 0102).
    if sitemap::is_fetch(connection, id)? {
        changed.fetches_ended += 1;
        return sitemap::ended(connection, id, &ending);
    }
    let (url, label, depth, attempts): (String, String, u32, u32) = connection
        .prepare_cached("SELECT url, label, depth, attempts FROM work_items WHERE id = ?1")
        .and_then(|mut cached| {
            cached.query_row([id], |row| {
                Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
            })
        })?;
    let errors = connection
        .prepare_cached(
            "SELECT attempt, code, message, details FROM attempt_errors
             WHERE work_item = ?1
               AND attempt > (SELECT coalesce(max(attempts), 0) FROM outcomes WHERE work_item = ?1)
             ORDER BY attempt",
        )?
        .query_map([id], |row| {
            Ok(AttemptError {
                attempt: row.get(0)?,
                code: row.get(1)?,
                message: row.get(2)?,
                details: row
                    .get::<_, Option<String>>(3)?
                    .and_then(|details| serde_json::from_str(&details).ok())
                    .unwrap_or_default(),
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let position: u64 = connection
        .prepare_cached("SELECT coalesce(max(position), 0) + 1 FROM outcomes")
        .and_then(|mut cached| {
            cached.query_row([], |row| {
                Ok(u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default())
            })
        })?;
    let kind = ending.kind();
    // The work item had no counted outcome: it was leased or pending, a
    // re-admission having taken its last one out of the counts.
    count(connection, &ending, 1)?;
    changed.ended.push(counted_as(&ending));
    let outcome = CrawlRequestOutcome {
        position,
        request: OutcomeRequest {
            id: id.to_owned(),
            url,
            label: label.clone(),
            depth,
        },
        attempts,
        errors,
        ended_at,
        ending,
    };
    connection
        .prepare_cached(
            "INSERT INTO outcomes (position, work_item, kind, label, attempts, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        )
        .and_then(|mut cached| {
            cached.execute((
                integer(position),
                id,
                kind.as_str(),
                label,
                attempts,
                serde_json::to_string(&outcome).expect("invariant: an outcome encodes as JSON"),
            ))
        })?;
    Ok(())
}

/// What an ended request counts under among its Run's ended requests: its
/// kind, and its skip reason, Error code or Block cause, none for
/// `succeeded`.
pub(crate) type CountedAs = (OutcomeKind, String);

/// What one job of the run file changed of the crawl's requests by state,
/// as the statements that changed them counted it, for the crawl's Run
/// state to count in memory (#2167): the job answers it, and no count is
/// read from the run file for it.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Changed {
    /// How many work items it added, each pending.
    pub(crate) added: u64,
    /// Each request it ended, as it counts.
    pub(crate) ended: Vec<CountedAs>,
    /// How many sitemaps' fetches it ended, which count under nothing.
    pub(crate) fetches_ended: u64,
    /// Each ended request it made pending again, as it counted.
    pub(crate) reopened: Vec<CountedAs>,
    /// How many sitemaps' fetches that had failed it made pending again.
    pub(crate) fetches_reopened: u64,
    /// Whether it added a sitemap seed.
    pub(crate) sitemap_seed: bool,
    /// Each sitemap seed's report after it, when it changed them
    /// ([`Changed::with_sitemaps`]).
    pub(crate) sitemaps: Option<Vec<SitemapReport>>,
}

impl Changed {
    /// Counts the work items `positions` added, and answers them keyed,
    /// in order, as [`Positions::keyed`] does.
    pub(crate) fn adds(&mut self, positions: &mut Positions) -> Vec<Waiting> {
        self.added += std::mem::take(&mut positions.added);
        self.sitemap_seed |= std::mem::take(&mut positions.sitemap_seed);
        positions.keyed()
    }

    /// Counts what another job, or another part of the same one, `other`,
    /// changed beside it.
    pub(crate) fn and(&mut self, other: Self) {
        self.added += other.added;
        self.ended.extend(other.ended);
        self.fetches_ended += other.fetches_ended;
        self.reopened.extend(other.reopened);
        self.fetches_reopened += other.fetches_reopened;
        self.sitemap_seed |= other.sitemap_seed;
        if other.sitemaps.is_some() {
            self.sitemaps = other.sitemaps;
        }
    }

    /// Whether it changed nothing.
    pub(crate) fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// The same, with each sitemap seed's report as `connection`'s
    /// transaction holds them once the job is done, when the job changed
    /// them: it added a sitemap seed, or ended a sitemap's fetch.
    ///
    /// # Errors
    ///
    /// The run file's.
    pub(crate) fn with_sitemaps(mut self, connection: &Connection) -> rusqlite::Result<Self> {
        if self.sitemap_seed || self.fetches_ended > 0 {
            self.sitemaps = Some(sitemap::reports(connection)?);
        }
        Ok(self)
    }
}

/// What `ending` is counted under in `request_counts`: its kind, and its
/// skip reason, Error code or Block cause, none for `succeeded` or an
/// ending a newer engine wrote.
fn counted_as(ending: &Ending) -> CountedAs {
    let name = match ending {
        Ending::Failed { error } => error.code.clone(),
        Ending::Skipped { reason, .. } => match serde_json::to_value(reason) {
            Ok(Value::String(name)) => name,
            _ => String::new(),
        },
        Ending::Blocked { cause } => cause.as_str().to_owned(),
        // `succeeded`, or an ending a newer engine wrote.
        _ => String::new(),
    };
    (ending.kind(), name)
}

/// Counts `ending` once more, `by` 1, or once less, `by` -1, among the
/// ended requests and, for a request that succeeded, its Records.
pub(crate) fn count(connection: &Connection, ending: &Ending, by: i64) -> rusqlite::Result<()> {
    let (kind, name) = counted_as(ending);
    let kind = kind.as_str();
    connection
        .prepare_cached(
            "INSERT INTO request_counts (kind, name, count) VALUES (?1, ?2, ?3)
         ON CONFLICT (kind, name) DO UPDATE SET count = count + excluded.count",
        )
        .and_then(|mut cached| cached.execute((kind, name, by)))?;
    if let Ending::Succeeded { records, .. } = ending {
        let mut counted = connection.prepare_cached(
            "INSERT INTO record_counts (dataset, committed, dropped, rejected)
             VALUES (?1, ?2 * ?5, ?3 * ?5, ?4 * ?5)
             ON CONFLICT (dataset) DO UPDATE SET
                 committed = committed + excluded.committed,
                 dropped = dropped + excluded.dropped,
                 rejected = rejected + excluded.rejected",
        )?;
        for dataset in records {
            counted.execute((
                &dataset.dataset,
                integer(dataset.committed),
                integer(dataset.dropped),
                integer(dataset.rejected),
                by,
            ))?;
        }
    }
    Ok(())
}

/// How many ended requests one kind of latest outcome and one skip reason,
/// Error code or Block cause (empty for a success) count:
/// `(kind, name, count)`.
pub(crate) type KindCount = (String, String, u64);

/// The ended requests, by the kind of each one's latest outcome and its
/// skip reason, Error code or Block cause, and the Records their
/// successes committed, dropped and rejected per Dataset, as their
/// commits counted them; a request pending again counts in none.
///
/// # Errors
///
/// The run file's, when the counts cannot be read.
pub(crate) fn ended_counts(
    connection: &Connection,
) -> rusqlite::Result<(Vec<KindCount>, Vec<DatasetRecords>)> {
    let requests = connection
        .prepare_cached(
            "SELECT kind, name, count FROM request_counts WHERE count > 0 ORDER BY kind, name",
        )?
        .query_map([], |row| {
            Ok((
                row.get(0)?,
                row.get(1)?,
                u64::try_from(row.get::<_, i64>(2)?).unwrap_or_default(),
            ))
        })?
        .collect::<rusqlite::Result<_>>()?;
    let records = connection
        .prepare_cached(
            "SELECT dataset, committed, dropped, rejected FROM record_counts ORDER BY dataset",
        )?
        .query_map([], |row| {
            let count = |at: usize| {
                row.get::<_, i64>(at)
                    .map(|count| u64::try_from(count).unwrap_or_default())
            };
            Ok(DatasetRecords {
                dataset: row.get(0)?,
                committed: count(1)?,
                dropped: count(2)?,
                rejected: count(3)?,
            })
        })?
        .collect::<rusqlite::Result<_>>()?;
    Ok((requests, records))
}

/// Makes pending again each work item of `ids` that a rule the Frontier
/// had learned ended `skipped` with `duplicate` before any fetch, once a
/// contradicting pair stopped that rule (ADR 0005), and answers the IDs of
/// those it gave back: each one still `skipped` whose latest outcome is
/// that ending. Each keeps its ID,
/// position and domain key, and its outcome leaves the ended requests'
/// counts until it ends again, as a re-admitted one's does ([`readmit`]),
/// which `changed` counts.
pub(crate) fn give_back(
    connection: &Connection,
    ids: &[String],
    changed: &mut Changed,
) -> rusqlite::Result<Vec<String>> {
    let mut given = Vec::new();
    for id in ids {
        let skipped: Option<Option<String>> = connection
            .prepare_cached("SELECT domain_key FROM work_items WHERE id = ?1 AND status = ?2")
            .and_then(|mut cached| cached.query_row((id, SKIPPED), |row| row.get(0)).optional())?;
        if skipped.is_none() {
            continue;
        }
        let Some(outcome) = latest_outcome(connection, id)? else {
            continue;
        };
        if !matches!(
            outcome.ending,
            Ending::Skipped {
                reason: SkipReason::Duplicate,
                ..
            }
        ) {
            continue;
        }
        connection
            .prepare_cached("UPDATE work_items SET status = ?1 WHERE id = ?2")
            .and_then(|mut cached| cached.execute((PENDING, id)))?;
        count(connection, &outcome.ending, -1)?;
        changed.reopened.push(counted_as(&outcome.ending));
        given.push(id.clone());
    }
    Ok(given)
}

/// The latest outcome of the work item `id`, if it ended at least once.
fn latest_outcome(
    connection: &Connection,
    id: &str,
) -> rusqlite::Result<Option<CrawlRequestOutcome>> {
    let outcome: Option<String> = connection
        .prepare_cached(
            "SELECT outcome FROM outcomes WHERE work_item = ?1 ORDER BY position DESC LIMIT 1",
        )
        .and_then(|mut cached| cached.query_row([id], |row| row.get(0)).optional())?;
    outcome
        .map(|outcome| {
            serde_json::from_str(&outcome).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })
        })
        .transpose()
}

/// The statuses of the work items `run.retry_failed` re-admits, which
/// `run.failed_requests` lists: `failed`, and with `blocked`, `blocked`.
pub(crate) fn readmitted(blocked: bool) -> &'static [&'static str] {
    if blocked {
        &[FAILED, BLOCKED]
    } else {
        &[FAILED]
    }
}

/// The Labels of the work items [`readmit`] would re-admit with `blocked`
/// now, each once, in order. A sitemap's fetch, whose Label is the
/// engine's empty one, is none of them.
pub(crate) fn readmitted_labels(
    connection: &Connection,
    blocked: bool,
) -> rusqlite::Result<Vec<String>> {
    let statuses = serde_json::to_string(readmitted(blocked)).expect("invariant: names encode");
    connection
        .prepare_cached(
            "SELECT DISTINCT label FROM work_items
             WHERE status IN (SELECT value FROM json_each(?1)) AND label <> '' ORDER BY label",
        )?
        .query_map([statuses], |row| row.get(0))?
        .collect()
}

/// How many work items [`readmit`] would re-admit with `blocked` now: none
/// in a run file no crawl wrote.
pub(crate) fn readmittable(connection: &Connection, blocked: bool) -> rusqlite::Result<u64> {
    let crawled = connection
        .prepare_cached("SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'work_items'")
        .and_then(|mut cached| cached.query_row([], |_| Ok(())))
        .optional()?
        .is_some();
    if !crawled {
        return Ok(0);
    }
    let statuses = serde_json::to_string(readmitted(blocked)).expect("invariant: names encode");
    connection
        .prepare_cached(
            "SELECT COUNT(*) FROM work_items WHERE status IN (SELECT value FROM json_each(?1))",
        )
        .and_then(|mut cached| {
            cached.query_row([statuses], |row| {
                Ok(u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default())
            })
        })
}

/// Re-admits every work item [`readmitted`] names, which ends again with a
/// new outcome, and answers their IDs, in order: `run.retry_failed`'s
/// re-admission. Each keeps its ID, Label, Payload and depth, and its
/// attempts count on from where they are, with a fresh `crawl.retry.max`
/// from there (ADR 0102). Each counts as pending from then on, its latest
/// outcome taken out of the ended requests' counts until it ends again,
/// which `changed` counts.
pub(crate) fn readmit(
    connection: &Connection,
    blocked: bool,
    changed: &mut Changed,
) -> rusqlite::Result<Vec<String>> {
    let statuses = serde_json::to_string(readmitted(blocked)).expect("invariant: names encode");
    let mut ids = connection
        .prepare_cached(
            "UPDATE work_items SET status = 'pending', error_code = NULL, retried_from = attempts
             WHERE status IN (SELECT value FROM json_each(?1))
             RETURNING id, position",
        )?
        .query_map([statuses], |row| Ok((row.get::<_, i64>(1)?, row.get(0)?)))?
        .collect::<rusqlite::Result<Vec<(i64, String)>>>()?;
    ids.sort();
    for (_, id) in &ids {
        // A sitemap's fetch that failed left no outcome and no count.
        match latest_outcome(connection, id)? {
            Some(outcome) => {
                count(connection, &outcome.ending, -1)?;
                changed.reopened.push(counted_as(&outcome.ending));
            }
            None => changed.fetches_reopened += 1,
        }
    }
    Ok(ids.into_iter().map(|(_, id)| id).collect())
}

/// Returns every leased work item to pending without counting its attempt:
/// the program's own stop counts nothing (ADR 0093).
pub(crate) fn release_leases(connection: &Connection) -> rusqlite::Result<()> {
    connection
        .prepare_cached(
            "UPDATE work_items SET status = ?1, attempts = attempts - 1 WHERE status = ?2",
        )
        .and_then(|mut cached| cached.execute((PENDING, LEASED)))?;
    Ok(())
}

/// How many work items are pending, and each leased one, in flight, by its
/// ID with its attempt.
pub(crate) fn pending_and_leased(
    connection: &Connection,
) -> rusqlite::Result<(u64, BTreeSet<(String, u32)>)> {
    let pending = connection
        .prepare_cached("SELECT COUNT(*) FROM work_items WHERE status = ?1")
        .and_then(|mut cached| cached.query_row([PENDING], |row| row.get::<_, i64>(0)))
        .map(|pending| u64::try_from(pending).unwrap_or(0))?;
    let mut statement =
        connection.prepare_cached("SELECT id, attempts FROM work_items WHERE status = ?1")?;
    let leased = statement
        .query_map([LEASED], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((pending, leased))
}

/// How many work items are left to deliver: pending, or leased to an
/// attempt.
pub(crate) fn left(connection: &Connection) -> rusqlite::Result<u64> {
    connection
        .prepare_cached("SELECT COUNT(*) FROM work_items WHERE status IN (?1, ?2)")
        .and_then(|mut cached| cached.query_row([PENDING, LEASED], |row| row.get::<_, i64>(0)))
        .map(|left| u64::try_from(left).unwrap_or(0))
}

/// Settles every work item still leased when its engine stopped without
/// settling it, on a resume, as [`end`] settles a live attempt that failed
/// with `failure`, ended at `ended_at`: its attempt counted and its error
/// kept, it returns to pending while it has retries left of `retry_max`
/// since its latest admission, and fails otherwise, with its outcome
/// (ADR 0102). Each ending counts in `changed`.
pub(crate) fn recover_leases(
    connection: &Connection,
    failure: &Failure,
    retry_max: u32,
    ended_at: SystemTime,
    changed: &mut Changed,
) -> rusqlite::Result<()> {
    let lost = connection
        .prepare_cached(
            "SELECT id, lease_epoch, attempts - retried_from <= ?1 FROM work_items
             WHERE status = ?2 ORDER BY position",
        )?
        .query_map((i64::from(retry_max), LEASED), |row| {
            Ok((
                row.get::<_, String>(0)?,
                u64::try_from(row.get::<_, i64>(1)?).unwrap_or_default(),
                row.get::<_, bool>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, lease_epoch, retried) in lost {
        let unsuccessful = if retried {
            Unsuccessful::Retried(failure.clone())
        } else {
            Unsuccessful::Failed(failure.clone())
        };
        end(
            connection,
            &id,
            lease_epoch,
            unsuccessful,
            ended_at,
            changed,
        )?;
    }
    Ok(())
}

/// Whether any work item is still pending or leased: what a crawl's
/// finish waits for.
pub(crate) fn any_unsettled(connection: &Connection) -> rusqlite::Result<bool> {
    Ok(connection
        .prepare_cached("SELECT 1 FROM work_items WHERE status IN (?1, ?2) LIMIT 1")
        .and_then(|mut cached| cached.query_row([PENDING, LEASED], |_| Ok(())))
        .optional()?
        .is_some())
}

/// The IDs of at most `limit` pending work items.
pub(crate) fn pending(connection: &Connection, limit: usize) -> rusqlite::Result<Vec<String>> {
    connection
        .prepare_cached("SELECT id FROM work_items WHERE status = ?1 LIMIT ?2")?
        .query_map((PENDING, i64::try_from(limit).unwrap_or(i64::MAX)), |row| {
            row.get(0)
        })?
        .collect()
}

#[cfg(test)]
mod tests;
