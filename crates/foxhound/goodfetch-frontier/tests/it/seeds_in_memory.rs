//! The Scope's seeds in memory (#2169): at the 1 MiB test budget, a crawl
//! of 100,000 seed sites keeps the Run state's heap within the budget
//! plus 10 %, as the process's own allocator counts it, while a link to a
//! seed site that left memory is still admitted and one to a site that is
//! no seed is refused as out of scope.

#![expect(
    clippy::disallowed_types,
    reason = "the test's count of acknowledged commits, which memory reads on the one thread the test runs: there is no lock order for loom to test"
)]

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use goodfetch_engine::egress::Policy;
use goodfetch_engine::heap;
use goodfetch_engine::metrics::Metrics;
use goodfetch_frontier::{
    Bounds, Candidate, Declared, Invariants, Narrowing, Patterns, Scope, ScopeStrategy, Url, admit,
};
use goodfetch_protocol::foxhound::frontier::{Enqueued, HttpMethod, NotAddedReason};
use goodfetch_test_support::TestEnv;
use rusqlite::Connection;

use crate::at_scale::run_file;

/// The test budget of `run_state.memory.budget`.
const BUDGET: u64 = 1 << 20;

/// How many seed sites the crawl starts from.
const SITES: u64 = 100_000;

/// How many admissions one batch of the run file holds.
const BATCH: u64 = 1_000;

/// The `site`th seed site's home page.
fn site(site: u64) -> String {
    format!("https://site-{site}.test/")
}

/// Admits `candidate` into the run file `connection` holds.
fn admitted(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    candidate: Candidate<'_>,
) -> Enqueued {
    admit(connection, invariants, policy, candidate, |url| {
        Ok(url.as_str().to_owned())
    })
    .unwrap()
}

/// Admits `href`, a link found on `page`.
fn link(
    connection: &Connection,
    invariants: &Invariants,
    policy: &Policy,
    page: &Url,
    href: &str,
) -> Enqueued {
    let candidate = Candidate::Href {
        href,
        base: page,
        encoding: goodfetch_frontier::Encoding::for_label(b"utf-8").unwrap(),
        from: page,
        narrowing: Narrowing::none(),
        depth: 1,
        keeps_fragment: false,
    };
    admitted(connection, invariants, policy, candidate)
}

#[test]
fn a_hundred_thousand_seed_sites_keep_the_run_states_heap_within_its_budget() {
    let env = TestEnv::new();
    let policy = Policy::new(&["*"], &[] as &[&str]).unwrap();
    let get = HttpMethod::get();
    let scope = Scope::new(
        ScopeStrategy::SameHostname,
        Patterns::default(),
        Patterns::default(),
    );
    let invariants = Invariants::new(scope, Bounds::default()).unwrap();
    let commits = Arc::new(AtomicU64::new(0));
    let before = heap::live().unwrap();
    let acknowledged = Arc::clone(&commits);
    invariants.hold_unwritten(
        BUDGET,
        move || acknowledged.load(Ordering::Relaxed),
        &Metrics::none(),
    );
    let mut connection = run_file(&env);
    invariants.fill_memory(&connection).unwrap();

    for start in (0..SITES).step_by(usize::try_from(BATCH).unwrap()) {
        let transaction = connection.transaction().unwrap();
        for seed in start..start + BATCH {
            let url = site(seed);
            let candidate = Candidate::Seed {
                url: &url,
                declared: Declared {
                    method: &get,
                    body: None,
                    key: None,
                    keeps_fragment: false,
                },
            };
            assert!(matches!(
                admitted(&transaction, &invariants, &policy, candidate),
                Enqueued::Added { .. }
            ));
        }
        transaction.commit().unwrap();
        commits.fetch_add(1, Ordering::Relaxed);
    }
    // The links' admissions take the last batch, committed, out of what
    // the file does not hold. The run file's connection, and its page
    // cache, are SQLite's: what stays is the Run state memory holds.
    drop(connection);
    let connection = Connection::open(env.folder().join("run.sqlite")).unwrap();
    let first = Url::parse(&site(0)).unwrap();
    let answers = (
        link(&connection, &invariants, &policy, &first, "/aisle-1/item-1"),
        link(
            &connection,
            &invariants,
            &policy,
            &first,
            "https://elsewhere.test/",
        ),
    );
    drop(connection);
    let held = heap::live().unwrap().saturating_sub(before);

    assert!(
        matches!(answers.0, Enqueued::Added { .. }),
        "{:?}",
        answers.0
    );
    assert_eq!(
        answers.1,
        Enqueued::NotAdded {
            reason: NotAddedReason::OutOfScope,
            url: "https://elsewhere.test/".to_owned(),
        }
    );
    assert!(
        held * 10 <= BUDGET * 11,
        "the Run state held {held} bytes of the heap at a budget of {BUDGET}"
    );
}
