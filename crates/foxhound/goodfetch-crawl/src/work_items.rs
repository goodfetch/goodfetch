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
use std::sync::LazyLock;

use goodfetch_dataset::{Checked, Committing};
use goodfetch_engine::CallError;
use goodfetch_engine::admission::Waiting;
use goodfetch_engine::egress::Policy;
use goodfetch_engine::mask::Masks;
use goodfetch_engine::run_state::RunStateTables;
use goodfetch_frontier::{Candidate, Declared, Scope, Url, admit, payload, sitemap, trap_links};
use goodfetch_protocol::block::BlockCause;
use goodfetch_protocol::foxhound::crawl::{CrawlRequest, Seed};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpBody, HttpMethod};
use goodfetch_protocol::foxhound::outcomes::{
    AttemptError, CrawlRequestOutcome, DatasetRecords, Ending, NothingCommitted, OutcomeRequest,
    SkipReason,
};
use goodfetch_protocol::foxhound::provenance::{
    CertificateExceptionMark, EncodingProvenance, RecordProvenance, RobotsTxtMark, TierProvenance,
    TransformProvenance, UnreadField,
};
use goodfetch_protocol::run::PointInTime;
use rusqlite::{Connection, OptionalExtension as _};
use serde_json::{Map, Value};

use crate::labels::Fragments;

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
    /// 0035).
    pub(crate) trap_link_suspected: bool,
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
    pub(crate) payload: Option<&'a Value>,
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

/// Checks each of `seeds` as [`check_seed`] does, failing at the first
/// that does not hold.
pub(crate) fn check_seeds<'a>(
    payloads: &payload::PayloadRules,
    seeds: impl IntoIterator<Item = &'a Seed>,
) -> Result<(), CallError> {
    seeds
        .into_iter()
        .try_for_each(|seed| check_seed(payloads, seed))
}

/// Checks `seed` before it is admitted: a request's Payload against its
/// Label's schema and the cap `payloads` hold, and a sitemap seed's rules,
/// each pattern one the engine matches (`pattern_unsupported`) and each
/// Label one that takes no Payload, since an entry carries none.
///
/// # Errors
///
/// `payload_invalid` or `pattern_unsupported`.
pub(crate) fn check_seed(payloads: &payload::PayloadRules, seed: &Seed) -> Result<(), CallError> {
    match seed {
        Seed::Request(seed) => payloads.check(&seed.label, seed.payload.as_ref()),
        Seed::Sitemap(seed) => {
            sitemap::Rules::of(&seed.rules).map_err(CallError::from)?;
            seed.rules
                .iter()
                .try_for_each(|rule| payloads.check(&rule.label, None))
        }
    }
}

/// Reads `document`, what the sitemap fetch `id` holding its lease under
/// `lease_epoch` gave, admitting its entries and nested sitemaps within the
/// scope the run file holds and under the Run's domain policy `policy`,
/// and ends the fetch, with no outcome; answers whether its lease held.
pub(crate) fn read_sitemap(
    connection: &Connection,
    policy: &Policy,
    (id, lease_epoch): (&str, u64),
    document: &sitemap::Document,
) -> rusqlite::Result<bool> {
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    let scope = Scope::read(connection)?;
    let mut positions = Positions::of(connection)?;
    sitemap::read(connection, &scope, policy, id, document, |url, adding| {
        add(connection, &mut positions, url, from_sitemap(adding))
    })?;
    connection.execute(
        "UPDATE work_items SET status = ?1, error_code = NULL WHERE id = ?2",
        (SUCCEEDED, id),
    )?;
    Ok(true)
}

/// Admits each seed through the Frontier under the Run's domain policy
/// `policy`, as a pending work item at depth 0 when added, at its position
/// from `positions`, and answers each seed's answer, in order, its key
/// keeping its URL's fragment when its Label does, as the run file records.
pub(crate) fn admit_seeds(
    connection: &Connection,
    positions: &mut Positions,
    scope: &Scope,
    policy: &Policy,
    seeds: &[Seed],
) -> rusqlite::Result<Vec<Enqueued>> {
    let fragments = Fragments::read(connection)?;
    seeds
        .iter()
        .map(|seed| {
            let seed = match seed {
                Seed::Request(seed) => seed,
                Seed::Sitemap(seed) => {
                    return sitemap::admit_seed(connection, policy, seed, |url| {
                        add(
                            connection,
                            positions,
                            url,
                            from_sitemap(sitemap::Adding::Sitemap),
                        )
                    });
                }
            };
            let declared = Declared {
                method: &seed.method,
                body: seed.body.as_ref().map(HttpBody::bytes),
                key: seed.key.as_deref(),
                keeps_fragment: fragments.keeps(&seed.label),
            };
            let candidate = Candidate::Seed {
                url: &seed.url,
                declared,
            };
            admit(connection, scope, policy, candidate, |url| {
                add(
                    connection,
                    positions,
                    url,
                    Adding {
                        method: &seed.method,
                        body: seed.body.as_ref(),
                        label: &seed.label,
                        payload: seed.payload.as_ref(),
                        priority: seed.priority,
                        forefront: seed.forefront,
                        depth: 0,
                    },
                )
            })
        })
        .collect()
}

/// The positions work items are added at, each its ID: after every work
/// item the run file holds and every position reserved for one an attempt
/// staged (ADR 0093), so a staged request keeps the ID its enqueue
/// answered once its attempt commits, and a discarded one leaves a gap.
#[derive(Debug)]
pub(crate) struct Positions {
    /// The positions an earlier staging of the admission being replayed
    /// gave each URL it added.
    kept: BTreeMap<String, u64>,
    next: u64,
    /// The positions given since the last [`Positions::taken`], by URL.
    taken: BTreeMap<String, u64>,
}

impl Positions {
    /// The positions after everything the run file `connection` holds.
    pub(crate) fn of(connection: &Connection) -> rusqlite::Result<Self> {
        let next: i64 = connection.query_row(
            "SELECT max(
                 (SELECT coalesce(max(position), 0) FROM work_items),
                 (SELECT coalesce(max(reserved), 0) FROM work_item_positions)
             ) + 1",
            [],
            |row| row.get(0),
        )?;
        Ok(Self {
            kept: BTreeMap::new(),
            next: u64::try_from(next).unwrap_or(1),
            taken: BTreeMap::new(),
        })
    }

    /// Gives each URL in `kept` the position it holds there from now on,
    /// as an admission staged before is replayed.
    pub(crate) fn keeping(&mut self, kept: BTreeMap<String, u64>) {
        self.kept = kept;
        self.taken.clear();
    }

    /// The position of the work item added at `url`.
    fn take(&mut self, url: &str) -> u64 {
        if let Some(kept) = self.kept.get(url) {
            return *kept;
        }
        let position = self.next;
        self.next += 1;
        self.taken.insert(url.to_owned(), position);
        position
    }

    /// The positions given since the last [`Positions::taken`], which are
    /// the IDs of the work items added at them.
    pub(crate) fn given(&self) -> impl Iterator<Item = u64> + '_ {
        self.taken.values().copied()
    }

    /// The positions given since the last call, by URL.
    pub(crate) fn taken(&mut self) -> BTreeMap<String, u64> {
        std::mem::take(&mut self.taken)
    }

    /// Reserves in the run file every position given so far, so that no
    /// later work item takes one, though none of it is written yet.
    pub(crate) fn reserve(&self, connection: &Connection) -> rusqlite::Result<()> {
        connection.execute(
            "INSERT INTO work_item_positions (id, reserved) VALUES (1, ?1)
             ON CONFLICT (id) DO UPDATE SET reserved = max(reserved, excluded.reserved)",
            [integer(self.next.saturating_sub(1))],
        )?;
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
    connection.query_row(
        "INSERT INTO work_items
             (id, position, url, method, body, label, depth, status, attempts, retried_from, lease_epoch, priority, forefront)
         VALUES (CAST(?1 AS TEXT), ?1, ?2, ?3, ?4, ?5, ?6, 'pending', 0, 0, 0, ?7, CASE WHEN ?8 THEN ?1 END)
         RETURNING id",
        (
            position,
            url.as_str(),
            adding.method.as_str(),
            adding.body.map(HttpBody::bytes),
            adding.label,
            adding.depth,
            adding.priority,
            adding.forefront,
        ),
        |row| row.get(0),
    )
    .and_then(|id: String| {
        payload::store(connection, &id, adding.payload)?;
        Ok(id)
    })
}

/// The IDs of the work items of `label` not ended, pending or leased, in
/// order: those a resume would deliver.
pub(crate) fn pending_of(connection: &Connection, label: &str) -> rusqlite::Result<Vec<String>> {
    connection
        .prepare(
            "SELECT id FROM work_items WHERE label = ?1 AND status IN ('pending', 'leased')
             ORDER BY position",
        )?
        .query_map([label], |row| row.get(0))?
        .collect()
}

/// The URL and depth of the work item `id` while its attempt under
/// `lease_epoch` holds its lease; `None` once that attempt ended.
pub(crate) fn attempt(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
) -> rusqlite::Result<Option<(String, u32)>> {
    connection
        .query_row(
            "SELECT url, depth FROM work_items WHERE id = ?1 AND status = 'leased' AND lease_epoch = ?2",
            (id, integer(lease_epoch)),
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()
}

/// The first `limit` work items not keyed yet, in order: each one's URL,
/// and how it waits once keyed, its key still empty.
pub(crate) fn unkeyed(
    connection: &Connection,
    limit: usize,
) -> rusqlite::Result<Vec<(String, Waiting)>> {
    connection
        .prepare(
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

/// Records each work item's domain key.
pub(crate) fn set_keys(connection: &Connection, keys: &[(String, String)]) -> rusqlite::Result<()> {
    let mut update = connection.prepare("UPDATE work_items SET domain_key = ?2 WHERE id = ?1")?;
    for (id, key) in keys {
        update.execute((id, key))?;
    }
    Ok(())
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
            .prepare(
                "SELECT id, domain_key, priority, position, forefront IS NOT NULL FROM work_items
                 WHERE status = 'pending' AND domain_key = ?1
                 ORDER BY forefront DESC, priority DESC, position LIMIT ?2",
            )?
            .query_map((key, bound), row)?
            .collect(),
        None => connection
            .prepare(
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
    let mut statement = connection.prepare(
        "UPDATE work_items SET status = 'leased', attempts = attempts + 1, lease_epoch = lease_epoch + 1
         WHERE id = ?1 AND status = 'pending'
         RETURNING id, url, label, attempts, lease_epoch, domain_key, priority, position,
             forefront IS NOT NULL, method, body, attempts - retried_from",
    )?;
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
            row.trap_link_suspected = trap_links::suspected(connection, &row.crawl_request.id)?;
            leased.push(row);
        }
    }
    Ok(leased)
}

/// Whether the work item `id` is still leased under `lease_epoch`.
fn still_leased(connection: &Connection, id: &str, lease_epoch: u64) -> rusqlite::Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM work_items WHERE id = ?1 AND status = 'leased' AND lease_epoch = ?2",
            (id, integer(lease_epoch)),
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// How an attempt's run fetched its document, as its Records' Provenance
/// names it: nothing where the handler was given its request alone.
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
    pub(crate) ended_at: &'a PointInTime,
}

/// Commits an attempt's Records, once, each numbered in its Dataset and
/// naming its Provenance: its request, how its document was fetched and
/// the Clean-up rules the Run holds to; marks its work item succeeded and
/// writes its outcome, ended at `ended_at`, when its lease is the one the
/// run file holds; answers whether it did.
pub(crate) fn commit(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
    committed: Committed<'_>,
) -> rusqlite::Result<bool> {
    let Committed {
        records,
        fetched,
        fields,
        masks,
        ended_at,
    } = committed;
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    let url: String =
        connection.query_row("SELECT url FROM work_items WHERE id = ?1", [id], |row| {
            row.get(0)
        })?;
    let clean_up_rules: Option<String> = connection
        .query_row("SELECT clean_up_digest FROM run", [], |row| row.get(0))
        .optional()?
        .flatten();
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
        at: ended_at,
        provenance: &provenance,
        masks,
    };
    let counted = goodfetch_dataset::commit(connection, committing, records)?;
    connection.execute(
        "UPDATE work_items SET status = ?1, error_code = NULL WHERE id = ?2",
        (SUCCEEDED, id),
    )?;
    let nothing_committed = if counted.is_empty() {
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
        },
    )?;
    Ok(true)
}

/// Ends an attempt that committed nothing as `unsuccessful` says, ended at
/// `ended_at`, when its lease is the one the run file holds; answers
/// whether it did.
pub(crate) fn end(
    connection: &Connection,
    id: &str,
    lease_epoch: u64,
    unsuccessful: Unsuccessful,
    ended_at: &PointInTime,
) -> rusqlite::Result<bool> {
    if !still_leased(connection, id, lease_epoch)? {
        return Ok(false);
    }
    let (status, error_code) = match &unsuccessful {
        Unsuccessful::Retried(failure) => (PENDING, Some(failure.code.as_str())),
        Unsuccessful::Failed(failure) => (FAILED, Some(failure.code.as_str())),
        Unsuccessful::Skipped { .. } => (SKIPPED, None),
        Unsuccessful::Blocked(_) => (BLOCKED, None),
    };
    connection.execute(
        "UPDATE work_items SET status = ?1, error_code = ?2 WHERE id = ?3",
        (status, error_code, id),
    )?;
    let ending = match unsuccessful {
        Unsuccessful::Retried(failure) => {
            keep_error(connection, id, &failure)?;
            return Ok(true);
        }
        Unsuccessful::Failed(failure) => Ending::Failed {
            error: keep_error(connection, id, &failure)?,
        },
        Unsuccessful::Skipped { reason, message } => Ending::Skipped { reason, message },
        Unsuccessful::Blocked(cause) => Ending::Blocked { cause },
    };
    write_outcome(connection, id, ended_at, ending)?;
    Ok(true)
}

/// Ends the pending work items a rule the Frontier learned later merged
/// (ADR 0093): given the crawl requests it showed to be one page, each
/// still pending ends `skipped` with `duplicate` at `ended_at`, its
/// handler never run, unless none of them was leased or ended otherwise
/// than so, when the first pending one stays. Answers each one it ended,
/// with its URL and the domain key it waits under, if keyed.
pub(crate) fn end_merged(
    connection: &Connection,
    ids: &BTreeSet<String>,
    ended_at: &PointInTime,
) -> rusqlite::Result<Vec<(String, String, Option<String>)>> {
    let mut members = Vec::with_capacity(ids.len());
    for id in ids {
        let member = connection
            .query_row(
                "SELECT position, status, url, domain_key, EXISTS (
                     SELECT 1 FROM outcomes WHERE work_item = ?1 AND kind = ?2
                         AND json_extract(outcome, '$.reason') = 'duplicate')
                 FROM work_items WHERE id = ?1",
                (id, SKIPPED),
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, bool>(4)?,
                    ))
                },
            )
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
        connection.execute(
            "UPDATE work_items SET status = ?1, error_code = NULL WHERE id = ?2",
            (SKIPPED, &id),
        )?;
        write_outcome(
            connection,
            &id,
            ended_at,
            Ending::Skipped {
                reason: SkipReason::Duplicate,
                message: None,
            },
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
    let attempt: u32 = connection.query_row(
        "SELECT attempts FROM work_items WHERE id = ?1",
        [id],
        |row| row.get(0),
    )?;
    connection.execute(
        "INSERT INTO attempt_errors (work_item, attempt, code, message, details)
         VALUES (?1, ?2, ?3, ?4, ?5)",
        (
            id,
            attempt,
            &failure.code,
            &failure.message,
            (!failure.details.is_empty())
                .then(|| Value::Object(failure.details.clone()).to_string()),
        ),
    )?;
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
    ended_at: &PointInTime,
    ending: Ending,
) -> rusqlite::Result<()> {
    // A sitemap's fetch is no crawl request: its sitemap's counts, or why
    // it could not be read, are its ending (ADR 0102).
    if sitemap::is_fetch(connection, id)? {
        return sitemap::ended(connection, id, &ending);
    }
    let (url, label, depth, attempts): (String, String, u32, u32) = connection.query_row(
        "SELECT url, label, depth, attempts FROM work_items WHERE id = ?1",
        [id],
        |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
    )?;
    let errors = connection
        .prepare(
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
    let position: u64 = connection.query_row(
        "SELECT coalesce(max(position), 0) + 1 FROM outcomes",
        [],
        |row| Ok(u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default()),
    )?;
    let kind = ending.kind();
    // The work item had no counted outcome: it was leased or pending, a
    // re-admission having taken its last one out of the counts.
    count(connection, &ending, 1)?;
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
        ended_at: ended_at.clone(),
        ending,
    };
    connection.execute(
        "INSERT INTO outcomes (position, work_item, kind, label, attempts, outcome)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        (
            integer(position),
            id,
            kind.as_str(),
            label,
            attempts,
            serde_json::to_string(&outcome).expect("invariant: an outcome encodes as JSON"),
        ),
    )?;
    Ok(())
}

/// What `ending` is counted under in `request_counts`: its kind, and its
/// skip reason, Error code or Block cause, none for `succeeded`.
fn counted_as(ending: &Ending) -> (&'static str, String) {
    let name = match ending {
        Ending::Succeeded { .. } => String::new(),
        Ending::Failed { error } => error.code.clone(),
        Ending::Skipped { reason, .. } => match serde_json::to_value(reason) {
            Ok(Value::String(name)) => name,
            _ => String::new(),
        },
        Ending::Blocked { cause } => cause.as_str().to_owned(),
    };
    (ending.kind().as_str(), name)
}

/// Counts `ending` once more, `by` 1, or once less, `by` -1, among the
/// ended requests and, for a request that succeeded, its Records.
fn count(connection: &Connection, ending: &Ending, by: i64) -> rusqlite::Result<()> {
    let (kind, name) = counted_as(ending);
    connection.execute(
        "INSERT INTO request_counts (kind, name, count) VALUES (?1, ?2, ?3)
         ON CONFLICT (kind, name) DO UPDATE SET count = count + excluded.count",
        (kind, name, by),
    )?;
    if let Ending::Succeeded { records, .. } = ending {
        let mut counted = connection.prepare(
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
        .prepare(
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
        .prepare(
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

/// The statuses of the work items `run.retry_failed` re-admits, which
/// `run.failed_requests` lists: `failed`, and with `blocked`, `blocked`.
pub(crate) fn readmitted(blocked: bool) -> &'static [&'static str] {
    if blocked {
        &[FAILED, BLOCKED]
    } else {
        &[FAILED]
    }
}

/// How many work items [`readmit`] would re-admit with `blocked` now: none
/// in a run file no crawl wrote.
pub(crate) fn readmittable(connection: &Connection, blocked: bool) -> rusqlite::Result<u64> {
    let crawled = connection
        .query_row(
            "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'work_items'",
            [],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    if !crawled {
        return Ok(0);
    }
    let statuses = serde_json::to_string(readmitted(blocked)).expect("invariant: names encode");
    connection.query_row(
        "SELECT COUNT(*) FROM work_items WHERE status IN (SELECT value FROM json_each(?1))",
        [statuses],
        |row| Ok(u64::try_from(row.get::<_, i64>(0)?).unwrap_or_default()),
    )
}

/// Re-admits every work item [`readmitted`] names, which ends again with a
/// new outcome, and answers their IDs, in order: `run.retry_failed`'s
/// re-admission. Each keeps its ID, Label, Payload and depth, and its
/// attempts count on from where they are, with a fresh `crawl.retry.max`
/// from there (ADR 0102). Each counts as pending from then on, its latest
/// outcome taken out of the ended requests' counts until it ends again.
pub(crate) fn readmit(connection: &Connection, blocked: bool) -> rusqlite::Result<Vec<String>> {
    let statuses = serde_json::to_string(readmitted(blocked)).expect("invariant: names encode");
    let mut ids = connection
        .prepare(
            "UPDATE work_items SET status = 'pending', error_code = NULL, retried_from = attempts
             WHERE status IN (SELECT value FROM json_each(?1))
             RETURNING id, position",
        )?
        .query_map([statuses], |row| Ok((row.get::<_, i64>(1)?, row.get(0)?)))?
        .collect::<rusqlite::Result<Vec<(i64, String)>>>()?;
    ids.sort();
    let mut latest = connection.prepare(
        "SELECT outcome FROM outcomes WHERE work_item = ?1 ORDER BY position DESC LIMIT 1",
    )?;
    for (_, id) in &ids {
        let outcome: Option<String> = latest.query_row([id], |row| row.get(0)).optional()?;
        if let Some(outcome) = outcome {
            let outcome: CrawlRequestOutcome = serde_json::from_str(&outcome).map_err(|error| {
                rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(error),
                )
            })?;
            count(connection, &outcome.ending, -1)?;
        }
    }
    Ok(ids.into_iter().map(|(_, id)| id).collect())
}

/// Returns every leased work item to pending without counting its attempt:
/// the program's own stop counts nothing (ADR 0093).
pub(crate) fn release_leases(connection: &Connection) -> rusqlite::Result<()> {
    connection.execute(
        "UPDATE work_items SET status = ?1, attempts = attempts - 1 WHERE status = ?2",
        (PENDING, LEASED),
    )?;
    Ok(())
}

/// How many work items are pending, and each leased one, in flight, by its
/// ID with its attempt.
pub(crate) fn pending_and_leased(
    connection: &Connection,
) -> rusqlite::Result<(u64, BTreeSet<(String, u32)>)> {
    let pending = connection
        .query_row(
            "SELECT COUNT(*) FROM work_items WHERE status = ?1",
            [PENDING],
            |row| row.get::<_, i64>(0),
        )
        .map(|pending| u64::try_from(pending).unwrap_or(0))?;
    let mut statement =
        connection.prepare("SELECT id, attempts FROM work_items WHERE status = ?1")?;
    let leased = statement
        .query_map([LEASED], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<_>>()?;
    Ok((pending, leased))
}

/// How many work items are left to deliver: pending, or leased to an
/// attempt.
pub(crate) fn left(connection: &Connection) -> rusqlite::Result<u64> {
    connection
        .query_row(
            "SELECT COUNT(*) FROM work_items WHERE status IN (?1, ?2)",
            [PENDING, LEASED],
            |row| row.get::<_, i64>(0),
        )
        .map(|left| u64::try_from(left).unwrap_or(0))
}

/// Settles every work item still leased when its engine stopped without
/// settling it, on a resume, as [`end`] settles a live attempt that failed
/// with `failure`, ended at `ended_at`: its attempt counted and its error
/// kept, it returns to pending while it has retries left of `retry_max`
/// since its latest admission, and fails otherwise, with its outcome
/// (ADR 0102).
pub(crate) fn recover_leases(
    connection: &Connection,
    failure: &Failure,
    retry_max: u32,
    ended_at: &PointInTime,
) -> rusqlite::Result<()> {
    let lost = connection
        .prepare(
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
        end(connection, &id, lease_epoch, unsuccessful, ended_at)?;
    }
    Ok(())
}

/// Whether any work item is still pending.
pub(crate) fn any_pending(connection: &Connection) -> rusqlite::Result<bool> {
    Ok(connection
        .query_row(
            "SELECT 1 FROM work_items WHERE status = ?1 LIMIT 1",
            [PENDING],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

#[cfg(test)]
mod tests;
