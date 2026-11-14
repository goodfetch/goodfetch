//! What the crawl's Run state keeps for the run file of the tier choices it
//! counts in memory (ADR 0142), on a real run file's table.

mod model;

use goodfetch_engine::admission::Waiting;
use goodfetch_engine::run_state::{BatchRows, BatchWriter};
use goodfetch_fetch_tier::TierChoice;
use goodfetch_protocol::foxhound::fetch::FetchTier;
use rusqlite::Connection;

use super::{Learned, in_queue_order};
use crate::tier_choice::{self, AttemptPlan, Sampling, TierChoices, Weighed};

const KEY: &str = "tiers.test";

/// The moment every test is at, in milliseconds since the Unix epoch.
const NOW: u64 = 1_790_000_000_000;

/// A choice is sampled again once it served two requests.
const SAMPLING: Sampling = Sampling {
    count: 3,
    requests: 2,
    after_ms: 60_000,
};

/// The plan of the request of the Label `static` at `/products/{page}`.
fn plan(learned: &mut Learned, page: u32) -> AttemptPlan {
    let url = format!("http://tiers.test/products/{page}");
    learned.serve("static", KEY, &url, NOW)
}

/// The tier that plan is on, `None` for a sample.
fn tier(learned: &mut Learned, page: u32) -> Option<FetchTier> {
    match plan(learned, page) {
        AttemptPlan::On(TierChoice { tier, forced: None }) => Some(tier),
        AttemptPlan::Sample(_) => None,
        other => panic!("{other:?}"),
    }
}

/// Classifies as matched the sample the request at `/products/{page}` is.
fn sample(learned: &mut Learned, page: u32) {
    let AttemptPlan::Sample(sampled) = plan(learned, page) else {
        panic!("page {page} is no sample");
    };
    learned.classify(&sampled, Weighed::Matched, NOW);
}

/// What a crawl whose Label `static` matched its three samples learned.
fn learned() -> Learned {
    let mut learned = Learned::from(TierChoices::default().sampled_as(SAMPLING));
    for page in 1..=3 {
        sample(&mut learned, page);
    }
    learned
}

/// A run file with the tier choices' table.
fn run_file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in tier_choice::TABLES.statements {
        connection.execute_batch(statement).unwrap();
    }
    connection
}

/// A request a learned choice served is counted in memory and kept as a
/// row for the run file once: a crawl resumed from the rows goes on
/// counting toward the choice's refresh from where the draw left it.
///
/// Covers: story:5-114, story:3-25
#[test]
fn the_count_a_draw_moved_is_a_row_a_resumed_crawl_goes_on_counting_from() {
    let file = run_file();
    let mut learned = learned();
    let before = tier(&mut learned, 4);

    learned.counted().write(&BatchWriter::over(&file)).unwrap();
    let again = learned.counted().is_empty();
    let mut resumed = Learned::from(tier_choice::read(&file).unwrap().sampled_as(SAMPLING));
    let after = [tier(&mut resumed, 5), tier(&mut resumed, 6)];

    assert_eq!(
        (before, again, after),
        (Some(FetchTier::Http), true, [Some(FetchTier::Http), None])
    );
}

/// A sample classified after its Label's choice counted requests takes
/// those counts out of what is kept for the run file: its attempt's
/// outcome writes what the Label now learned, which an older count written
/// after it would undo.
#[test]
fn a_sample_classified_after_a_count_drops_the_count_kept_for_the_run_file() {
    let mut learned = learned();
    let served = [tier(&mut learned, 4), tier(&mut learned, 5)];

    sample(&mut learned, 6);

    assert_eq!(
        (served, learned.counted().is_empty()),
        ([Some(FetchTier::Http), Some(FetchTier::Http)], true)
    );
}

/// A work item of `key` at `order`, with `priority`, to the forefront or
/// not.
fn waiting(key: &str, order: i64, priority: i64, forefront: bool) -> Waiting {
    Waiting {
        id: format!("{key}{order}"),
        key: key.to_owned(),
        priority,
        order,
        forefront,
    }
}

/// The work items the crawl knows pending, merged with those the run file
/// holds, are bounded per key in the order the run file's read takes them
/// (ADR 0004): the latest forefront one first, then the highest priority,
/// then the earliest; read for every key, they come in order.
#[test]
fn the_items_known_pending_merge_in_the_order_the_run_file_reads() {
    let merged = || {
        vec![
            waiting("b", 8, 0, false),
            waiting("a", 1, 0, false),
            waiting("a", 4, 0, true),
            waiting("b", 6, 0, false),
            waiting("a", 2, 5, false),
            waiting("b", 5, 0, false),
            waiting("a", 3, 0, true),
            waiting("b", 7, 0, false),
        ]
    };
    let ids = |waiting: Vec<Waiting>| {
        waiting
            .into_iter()
            .map(|waiting| waiting.id)
            .collect::<Vec<String>>()
    };

    assert_eq!(
        (
            ids(in_queue_order(merged(), 3, false)),
            ids(in_queue_order(merged(), 3, true)),
        ),
        (
            ["a4", "a3", "a2", "b5", "b6", "b7"]
                .map(String::from)
                .to_vec(),
            ["a2", "a3", "a4", "b5", "b6", "b7"]
                .map(String::from)
                .to_vec(),
        )
    );
}
