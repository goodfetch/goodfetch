//! The Frontier at scale (ADR 0134, TESTING.md's Memory at scale): one
//! process enqueues 100,000 synthetic URLs, then 1,000,000, with no
//! network, into a run file on disk, and its peak resident memory at the
//! second is at most 1.5 times its peak at the first. A ratio gives one
//! verdict on any machine. The duplicate check is exact: every distinct
//! URL is added and a visited one is a `duplicate`. Outside the Release
//! suite, the same check runs on a tenth of the URLs.
//!
//! No engine runs here, so no Resource governor sheds load (ADR 0040):
//! the Frontier's own memory is what grows.

use std::fmt::Write as _;

use goodfetch_engine::egress::Policy;
use goodfetch_engine::run_state::PAGE_CACHE_KIB;
use goodfetch_frontier::{
    BOUNDS_TABLES, Candidate, Declared, Narrowing, RULES_TABLES, SCOPE_TABLES, Scope, Url,
    VISITED_TABLES, admit,
};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use goodfetch_test_support::{Suite, TestEnv, peak_resident_bytes};
use rusqlite::Connection;

/// The seed every synthetic link is found on.
const SEED: &str = "https://shop.test/";

/// How many admissions one transaction of the run file holds.
const BATCH: u64 = 1_000;

/// A run file on disk holding the Frontier's tables, opened as a crawl's
/// is (WAL, `run_state.durability = fast`, the writer's bounded page
/// cache), its seed admitted.
struct RunFile {
    connection: Connection,
    scope: Scope,
    policy: Policy,
    base: Url,
}

impl RunFile {
    fn open(env: &TestEnv) -> Self {
        let connection = Connection::open(env.folder().join("run.sqlite")).unwrap();
        #[expect(
            clippy::disallowed_methods,
            reason = "the test's own new run file, which no other connection opens"
        )]
        connection
            .pragma_update_and_check(None, "journal_mode", "WAL", |_| Ok(()))
            .unwrap();
        connection
            .pragma_update(None, "synchronous", "NORMAL")
            .unwrap();
        connection
            .pragma_update(None, "cache_size", -i64::from(PAGE_CACHE_KIB))
            .unwrap();
        for statement in VISITED_TABLES
            .statements
            .iter()
            .chain(SCOPE_TABLES.statements)
            .chain(BOUNDS_TABLES.statements)
            .chain(RULES_TABLES.statements)
        {
            connection.execute(statement, []).unwrap();
        }
        let file = Self {
            connection,
            scope: Scope::default(),
            policy: Policy::new(&["*"], &[] as &[&str]).unwrap(),
            base: Url::parse(SEED).unwrap(),
        };
        let get = HttpMethod::get();
        let seed = Candidate::Seed {
            url: SEED,
            declared: Declared {
                method: &get,
                body: None,
                key: None,
                keeps_fragment: false,
            },
        };
        assert!(matches!(
            file.admit(&file.connection, seed),
            Enqueued::Added { .. }
        ));
        file
    }

    fn admit(&self, connection: &Connection, candidate: Candidate<'_>) -> Enqueued {
        admit(connection, &self.scope, &self.policy, candidate, |url| {
            Ok(url.as_str().to_owned())
        })
        .unwrap()
    }

    /// Enqueues the synthetic links `links` found on the seed, in batches,
    /// one reused buffer holding each in turn; answers how many were added.
    fn enqueue(&mut self, links: std::ops::Range<u64>) -> u64 {
        let mut href = String::new();
        let mut added = 0;
        let mut start = links.start;
        while start < links.end {
            let end = links.end.min(start + BATCH);
            let transaction = self.connection.transaction().unwrap();
            for link in start..end {
                synthetic(link, &mut href);
                if matches!(
                    admit_link(&transaction, &self.scope, &self.policy, &self.base, &href),
                    Enqueued::Added { .. }
                ) {
                    added += 1;
                }
            }
            transaction.commit().unwrap();
            start = end;
        }
        added
    }
}

/// Admits `href`, a link found on the seed's page.
fn admit_link(
    connection: &Connection,
    scope: &Scope,
    policy: &Policy,
    base: &Url,
    href: &str,
) -> Enqueued {
    let candidate = Candidate::Href {
        href,
        base,
        encoding: goodfetch_frontier::Encoding::for_label(b"utf-8").unwrap(),
        from: base,
        narrowing: Narrowing::none(),
        depth: 1,
        keeps_fragment: false,
    };
    admit(connection, scope, policy, candidate, |url| {
        Ok(url.as_str().to_owned())
    })
    .unwrap()
}

/// Writes the `link`th synthetic link into `into`: a distinct page under
/// one of 64 aisles, with no query, so no learned rule merges two of them.
fn synthetic(link: u64, into: &mut String) {
    into.clear();
    write!(into, "{SEED}aisle-{}/item-{link}", link % 64).unwrap();
}

/// A million synthetic URLs (a hundred thousand outside the Release suite)
/// are each added, the visited set never mistaking one for another, and
/// the process's peak resident memory once they are is at most 1.5 times
/// its peak after the first tenth; a visited one enqueued again is a
/// `duplicate`.
#[test]
fn a_million_urls_take_at_most_half_again_the_peak_memory_of_a_hundred_thousand() {
    let env = TestEnv::new();
    let count: u64 = match env.suite() {
        Suite::Release => 1_000_000,
        _ => 100_000,
    };
    let mut file = RunFile::open(&env);

    let first = file.enqueue(0..count / 10);
    let at_a_tenth = peak_resident_bytes().unwrap();
    let rest = file.enqueue(count / 10..count);
    let at_all = peak_resident_bytes().unwrap();
    let mut visited = String::new();
    synthetic(count / 2, &mut visited);
    let again = admit_link(
        &file.connection,
        &file.scope,
        &file.policy,
        &file.base,
        &visited,
    );

    assert_eq!(
        (first + rest, again),
        (
            count,
            Enqueued::NotAdded {
                reason: NotAddedReason::Duplicate,
                url: visited.clone(),
            }
        )
    );
    assert!(
        at_all * 2 <= at_a_tenth * 3,
        "the peak grew from {at_a_tenth} bytes at {} URLs to {at_all} at {count}",
        count / 10
    );
}
