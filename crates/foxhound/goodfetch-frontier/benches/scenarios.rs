//! The Frontier's benchmark scenarios (ADR 0071), shared by its Callgrind
//! and DHAT targets: admission through the per-domain queues, from a
//! seed-free literal pattern, with no network, no sleep and no timing. One
//! heavy domain holds most of the requests and many small domains one or
//! two each, the shape fair admission exists for (ADR 0004); every request
//! is enqueued, admitted in round-robin turns and ended.

use std::sync::Arc;

use goodfetch_engine::admission::{Ended, Gate, Waiting};
use goodfetch_engine::egress::Policy;
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::run_state::PAGE_CACHE_KIB;
use goodfetch_frontier::{
    Admitting, BOUNDS_TABLES, Candidate, Declared, DomainKeys, DomainQueues, Narrowing,
    SCOPE_TABLES, Scope, Staged, Url, VISITED_TABLES, admit, admit_in, url_key,
};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod};
use rusqlite::Connection;
use tokio::time::Instant;

/// How many requests a domain's queue holds at most, as a crawl's do.
const BOUND: usize = 64;

/// The requests of the scenario: `heavy` to one domain, then one or two to
/// each of `small` other domains, every third one at a higher priority.
pub(crate) fn requests(heavy: usize, small: usize) -> Vec<(String, i64)> {
    let heavy = (0..heavy).map(|page| (format!("https://www.heavy.test/pages/{page}"), 0));
    let small = (0..small).flat_map(|domain| {
        let pages = if domain % 2 == 0 { 1 } else { 2 };
        (0..pages).map(move |page| {
            let priority = i64::from(page % 3 == 0);
            (format!("https://shop.site-{domain}.test/{page}"), priority)
        })
    });
    heavy.chain(small).collect()
}

/// Enqueues `requests` in order, each into its domain's queue while it has
/// room, then admits every one in turn and ends it; returns how many it
/// admitted and how many domains it still keeps state for, none once
/// every queue drained.
#[expect(
    clippy::disallowed_methods,
    reason = "a pure rule's readings may start from any moment (ADR 0059)"
)]
pub(crate) fn admit_all(requests: Vec<(String, i64)>) -> (usize, usize) {
    let now = Instant::now();
    let mut queues = DomainQueues::new(DomainKeys::default(), BOUND, Entropy::seeded(1));
    for (order, (url, priority)) in requests.into_iter().enumerate() {
        let key = queues.key(&url);
        queues.enqueue(Waiting {
            id: url,
            key,
            priority,
            order: i64::try_from(order).unwrap_or(i64::MAX),
            forefront: false,
        });
    }
    let mut admitted = 0;
    while let Some(request) = queues.admit(now) {
        queues.ended(&request.key, now, Ended::Done);
        admitted += 1;
    }
    (admitted, queues.domains())
}

/// `count` links as a shop's pages write them: every fourth one a spelling
/// of the one before (tracking parameters, another order, a fragment, a
/// `;jsessionid=`), the rest distinct pages with encoded characters, so the
/// key's every rule runs and identities meet.
pub(crate) fn links(count: usize) -> Vec<String> {
    (0..count)
        .map(|link| {
            let page = link / 4 * 3 + link % 4;
            match link % 4 {
                3 => format!(
                    "https://shop.test/item/{};jsessionid=0A1B?utm_source=mail&size=m&colour=%7eblue#reviews",
                    page - 1
                ),
                2 => format!("https://shop.test/item/{page}?colour=%c3%a9t%c3%a9&size=m&&gclid=x"),
                _ => format!("https://shop.test/item/{page}?size=m&colour=%7eblue"),
            }
        })
        .collect()
}

/// Each of `links`, parsed.
pub(crate) fn parsed(links: &[String]) -> Vec<Url> {
    links
        .iter()
        .map(|link| Url::parse(link).expect("invariant: a scenario's link parses"))
        .collect()
}

/// The key of each of `links`; answers their total length.
pub(crate) fn key_all(links: &[Url]) -> usize {
    links.iter().map(|link| url_key(link).len()).sum()
}

/// Admits each of `links` as a seed into a run file in memory holding the
/// Frontier's visited set, each added request numbered by the scenario;
/// answers how many were added.
pub(crate) fn admit_seeds(links: &[String]) -> usize {
    let connection =
        Connection::open_in_memory().expect("invariant: SQLite opens a database in memory");
    create_tables(&connection);
    let scope = Scope::default();
    let policy =
        Policy::new(&["*"], &[] as &[&str]).expect("invariant: the default domain policy reads");
    let get = HttpMethod::get();
    let mut added = 0_usize;
    for link in links {
        let candidate = Candidate::Seed {
            url: link,
            declared: Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            },
        };
        let answer = admit(&connection, &scope, &policy, candidate, |_| {
            Ok((added + 1).to_string())
        })
        .expect("invariant: the visited set in memory is written");
        if matches!(answer, Enqueued::Added { .. }) {
            added += 1;
        }
    }
    added
}

/// Stages `enqueues` separate enqueues of one attempt from [`SEED`]'s page,
/// as a crawl's staging does (ADR 0093): each admission runs once, inside
/// a savepoint rolled back after it, against what the attempt's earlier
/// ones did, which it then joins; answers how many were added. A cost per
/// enqueue that stays flat as `enqueues` grows is what its cases compare.
pub(crate) fn stage(enqueues: usize) -> usize {
    let connection =
        Connection::open_in_memory().expect("invariant: SQLite opens a database in memory");
    create_tables(&connection);
    let scope = Scope::default();
    let policy =
        Policy::new(&["*"], &[] as &[&str]).expect("invariant: the default domain policy reads");
    let get = HttpMethod::get();
    let declared = Declared {
        method: &get,
        body: None,
        key: None,
        keeps_fragment: false,
    };
    let base = Url::parse(SEED).expect("invariant: the seed parses");
    admit(
        &connection,
        &scope,
        &policy,
        Candidate::Seed {
            url: SEED,
            declared,
        },
        |_| Ok("1".to_owned()),
    )
    .expect("invariant: the run file in memory is written");
    let mut staged = Arc::new(Staged::default());
    let mut added = 0_usize;
    let mut link = String::new();
    for each in 0..enqueues {
        synthetic(each, &mut link);
        let candidate = Candidate::Enqueue {
            url: &link,
            base: &base,
            from: &base,
            declared,
            narrowing: Narrowing::none(),
            depth: 1,
        };
        let mut admitting = Admitting::Staging {
            earlier: Arc::clone(&staged),
            now: Staged::default(),
        };
        connection
            .execute_batch("SAVEPOINT staging")
            .expect("invariant: the run file opens a savepoint");
        let answer = admit_in(
            &connection,
            &scope,
            &policy,
            candidate,
            &mut admitting,
            |_| Ok((added + 2).to_string()),
        )
        .expect("invariant: the run file in memory is written");
        connection
            .execute_batch("ROLLBACK TO staging; RELEASE staging")
            .expect("invariant: the run file rolls the savepoint back");
        if matches!(answer, Enqueued::Added { .. }) {
            added += 1;
        }
        // Its `earlier` is dropped first, so joining copies nothing.
        if let Admitting::Staging { earlier, now } = admitting {
            drop(earlier);
            Arc::make_mut(&mut staged).absorb(now);
        }
    }
    added
}

/// Creates the Frontier's tables in the run file `connection` holds: its
/// visited set, its scope, its bounds and its learned rules.
fn create_tables(connection: &Connection) {
    for statement in VISITED_TABLES
        .statements
        .iter()
        .chain(SCOPE_TABLES.statements)
        .chain(BOUNDS_TABLES.statements)
        .chain(goodfetch_frontier::RULES_TABLES.statements)
    {
        connection
            .execute(statement, [])
            .expect("invariant: the visited set's and the scope's tables are created");
    }
}

/// The seed the synthetic links of [`enqueue`] are found on.
const SEED: &str = "https://shop.test/";

/// How many admissions one transaction of the run file holds.
const BATCH: usize = 1_000;

/// Writes the `link`th synthetic link of [`SEED`]'s site into `into`: a
/// distinct page under one of 64 aisles, with no query, so no learned rule
/// merges two of them.
fn synthetic(link: usize, into: &mut String) {
    use std::fmt::Write as _;
    into.clear();
    write!(into, "{SEED}aisle-{}/item-{link}", link % 64)
        .expect("invariant: writing to a String never fails");
}

/// Admits [`SEED`] into a run file on disk, opened as a crawl's is (WAL,
/// `run_state.durability = fast`, the writer's bounded page cache), then
/// `urls` synthetic links found on it, each written in turn into one
/// reused buffer, so the scenario holds no URL itself; answers how many
/// were added, the seed included. Its heap is the Frontier's: the page
/// cache over an exact visited set on disk (ADR 0134).
pub(crate) fn enqueue(urls: usize) -> usize {
    let folder = tempfile::tempdir().expect("invariant: a temporary folder is created");
    let mut connection = Connection::open(folder.path().join("run.sqlite"))
        .expect("invariant: SQLite opens a run file in a new folder");
    #[expect(
        clippy::disallowed_methods,
        reason = "the bench's own new run file, which no other connection opens"
    )]
    connection
        .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
        .expect("invariant: the run file takes WAL");
    connection
        .pragma_update(None, "synchronous", "NORMAL")
        .expect("invariant: the run file takes its durability");
    connection
        .pragma_update(None, "cache_size", -i64::from(PAGE_CACHE_KIB))
        .expect("invariant: the run file takes its page cache");
    create_tables(&connection);
    let scope = Scope::default();
    let policy =
        Policy::new(&["*"], &[] as &[&str]).expect("invariant: the default domain policy reads");
    let get = HttpMethod::get();
    let base = Url::parse(SEED).expect("invariant: the seed parses");
    let mut added = 0_usize;
    let mut admit_one = |connection: &Connection, candidate: Candidate<'_>| {
        let answer = admit(connection, &scope, &policy, candidate, |_| {
            Ok((added + 1).to_string())
        })
        .expect("invariant: the run file is written");
        if matches!(answer, Enqueued::Added { .. }) {
            added += 1;
        }
    };
    let seed = Candidate::Seed {
        url: SEED,
        declared: Declared {
            method: &get,
            body: None,
            key: None,
            keeps_fragment: false,
        },
    };
    admit_one(&connection, seed);
    let mut link = String::new();
    for batch in (0..urls).step_by(BATCH) {
        let transaction = connection
            .transaction()
            .expect("invariant: the run file starts a transaction");
        for each in batch..urls.min(batch + BATCH) {
            synthetic(each, &mut link);
            let href = Candidate::Href {
                href: &link,
                base: &base,
                encoding: encoding_rs::UTF_8,
                from: &base,
                narrowing: Narrowing::none(),
                depth: 1,
                keeps_fragment: false,
            };
            admit_one(&transaction, href);
        }
        transaction
            .commit()
            .expect("invariant: the run file commits a batch");
    }
    added
}
