//! What the crawl keeps of its pending work items within the Run's memory
//! budget (#2169): the heap they hold, as the process's own allocator
//! counts it, and the budget they share with the Frontier's visits.

use std::alloc::System;

use goodfetch_engine::admission::Waiting;
use goodfetch_engine::heap;
use goodfetch_frontier::SharedBudget;
use goodfetch_protocol::foxhound::crawl::CrawlRequest;
use goodfetch_protocol::foxhound::frontier::HttpMethod;
use rstest::rstest;
use rusqlite::Connection;

use super::{Kept, Whole};
use crate::work_items::{self, Again, Leased};

// The unit tests' count of the live heap, around the system's allocator,
// which `heap::live` reads; every other test allocates as before.
#[global_allocator]
static ALLOCATOR: heap::Counting<System> = heap::Counting::new(System);

/// The test budget of `run_state.memory.budget`.
const BUDGET: u64 = 1 << 20;

/// The `n`th work item, pending on one of `domains` domains, as a seed's
/// is.
fn leased(n: u64, domains: u64) -> Leased {
    let id = format!("work-item-{n:08}");
    Leased {
        crawl_request: CrawlRequest {
            id: id.clone(),
            url: format!("https://site-{n}.test/"),
            label: "seed".to_owned(),
            payload: None,
            method: HttpMethod::get(),
            body: None,
        },
        attempt: 1,
        tries: 1,
        lease_epoch: 0,
        waiting: Waiting {
            id,
            key: format!("site-{}.test", n % domains),
            priority: 0,
            order: i64::try_from(n).unwrap(),
            forefront: false,
        },
        sitemap: false,
        trap_link_suspected: false,
    }
}

/// What a crawl keeps as it opens on an empty run file, within `budget`
/// bytes, every pending work item in memory.
fn kept(budget: u64) -> Kept {
    let connection = Connection::open_in_memory().unwrap();
    for statement in work_items::TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    let whole = Whole::read(&connection, SharedBudget::new(budget), 0).unwrap();
    assert!(
        whole.is_some(),
        "an empty run file's pending work items fit"
    );
    Kept::new(whole)
}

/// Pending work items added until they spill hold at most the budget,
/// plus 10 %, of the heap, as the process's allocator counts it, and fill
/// at least half of it: the count memory keeps is the heap they hold,
/// whether each domain's queue holds one work item or one queue holds
/// them all.
#[rstest]
#[case::one_domain(1)]
#[case::a_domain_each(u64::MAX)]
fn pending_work_items_kept_up_to_the_budget_hold_the_heap_it_names(#[case] domains: u64) {
    let mut kept = kept(BUDGET);
    let before = heap::live().unwrap();
    let mut held = 0;

    for n in 0.. {
        kept.added(leased(n, domains), 0, || 0);
        // A read that starts past the first commit sees the write: only
        // the pending work items kept whole stay.
        kept.seen(1);
        if kept.whole(|| 0).is_none() {
            break;
        }
        held = held.max(heap::live().unwrap().saturating_sub(before));
    }

    assert!(
        held * 10 <= BUDGET * 11,
        "the pending work items held {held} bytes of the heap at a budget of {BUDGET}"
    );
    assert!(
        held * 2 >= BUDGET,
        "the pending work items spilled at {held} bytes of the heap, at a budget of {BUDGET}"
    );
}

/// Adds `leased` pending, beside Frontier's visits holding `beside` bytes.
fn add(kept: &mut Kept, leased: Leased, beside: u64) {
    kept.added(leased, 0, || beside);
}

/// Settles the attempt of `leased` for a retry, beside Frontier's visits
/// holding `beside` bytes.
fn retry(kept: &mut Kept, leased: Leased, beside: u64) {
    kept.settled(leased, Again::Retried, 0, || beside);
}

/// A work item made pending while the Frontier's visits hold all of the
/// budget but a byte spills the pending work items, whether an attempt
/// added it or its own attempt was retried: together they would pass the
/// budget, though the work item alone fits.
#[rstest]
#[case::added(add)]
#[case::retried(retry)]
fn a_work_item_made_pending_past_the_budget_beside_the_frontiers_visits_spills(
    #[case] pend: fn(&mut Kept, Leased, u64),
) {
    let mut kept = kept(BUDGET);

    pend(&mut kept, leased(0, 1), BUDGET - 1);
    // Past the first commit, only the pending work items kept whole say
    // where it waits.
    kept.seen(1);

    assert!(
        kept.whole(|| 0).is_none(),
        "the pending work items stayed in memory past the budget beside the Frontier's visits"
    );
}
