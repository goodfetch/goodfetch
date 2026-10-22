//! The Frontier's benchmark scenarios (ADR 0071), shared by its Callgrind
//! and DHAT targets: admission through the per-domain queues, from a
//! seed-free literal pattern, with no network, no sleep and no timing. One
//! heavy domain holds most of the requests and many small domains one or
//! two each, the shape fair admission exists for (ADR 0004); every request
//! is enqueued, admitted in round-robin turns and ended.

use goodfetch_engine::admission::{Ended, Gate, Waiting};
use goodfetch_engine::entropy::Entropy;
use goodfetch_frontier::{
    Candidate, Declared, DomainKeys, DomainQueues, SCOPE_TABLES, Scope, Url, VISITED_TABLES, admit,
    url_key,
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
    for statement in VISITED_TABLES
        .statements
        .iter()
        .chain(SCOPE_TABLES.statements)
    {
        connection
            .execute(statement, [])
            .expect("invariant: the visited set's and the scope's tables are created");
    }
    let scope = Scope::default();
    let get = HttpMethod::get();
    let mut added = 0_usize;
    for link in links {
        let candidate = Candidate::Seed {
            url: link,
            declared: Declared {
                method: &get,
                body: None,
                key: None,
            },
        };
        let answer = admit(&connection, &scope, candidate, |_| {
            Ok((added + 1).to_string())
        })
        .expect("invariant: the visited set in memory is written");
        if matches!(answer, Enqueued::Added { .. }) {
            added += 1;
        }
    }
    added
}
