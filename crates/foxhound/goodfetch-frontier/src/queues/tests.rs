//! A crawl's per-domain queues through the engine's gate interface (spec
//! #213's Seam 4: admission order), on monotonic readings the tests give,
//! and their journal through a run file in memory.
#![expect(
    clippy::disallowed_methods,
    reason = "pure rules' readings may start from any moment (ADR 0059)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::collections::BTreeMap;
use std::time::{Duration, UNIX_EPOCH};

use goodfetch_engine::admission::{Ended, Gate, Gates, Waiting};
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::SettingsLayer;
use proptest::prelude::*;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{BOUND, DomainGates, DomainQueues, TABLES, backoff};
use crate::domain_key::DomainKeys;

fn queues() -> DomainQueues {
    DomainQueues::new(DomainKeys::default(), BOUND)
}

/// Enqueues a request of `url` named `id` at `priority`.
fn enqueue(gate: &mut dyn Gate, id: &str, url: &str, priority: i64) -> bool {
    let key = gate.key(url);
    gate.enqueue(Waiting {
        id: id.to_owned(),
        key,
        priority,
    })
}

/// The IDs the gate admits at `now` until none is ready.
fn admit_all(gate: &mut dyn Gate, now: Instant) -> Vec<String> {
    std::iter::from_fn(|| gate.admit(now).map(|admitted| admitted.id)).collect()
}

/// Covers: story:3-47
#[test]
fn one_heavy_domain_and_two_small_ones_take_one_request_each_per_turn() {
    let now = Instant::now();
    let mut queues = queues();
    for page in 1..=4 {
        enqueue(
            &mut queues,
            &format!("h{page}"),
            &format!("https://www.heavy.test/{page}"),
            0,
        );
    }
    enqueue(&mut queues, "a1", "https://a.test/1", 0);
    enqueue(&mut queues, "b1", "https://b.test/1", 0);
    enqueue(&mut queues, "a2", "https://shop.a.test/2", 0);

    let admitted = admit_all(&mut queues, now);

    assert_eq!(admitted, ["h1", "a1", "b1", "h2", "a2", "h3", "h4"]);
}

/// Covers: story:3-49
#[test]
fn priority_orders_requests_within_their_domain_only() {
    let now = Instant::now();
    let mut queues = queues();
    enqueue(&mut queues, "h-low", "https://heavy.test/low", -1);
    enqueue(&mut queues, "h-first", "https://heavy.test/first", 0);
    enqueue(&mut queues, "h-high", "https://heavy.test/high", 5);
    enqueue(&mut queues, "h-second", "https://heavy.test/second", 0);
    enqueue(&mut queues, "a", "https://a.test/", 0);

    let admitted = admit_all(&mut queues, now);

    assert_eq!(admitted, ["h-high", "a", "h-first", "h-second", "h-low"]);
}

/// Covers: story:3-50
#[test]
fn a_domain_backing_off_holds_no_turn_while_the_others_go() {
    let now = Instant::now();
    let mut queues = queues();
    enqueue(&mut queues, "h1", "https://heavy.test/1", 0);
    enqueue(&mut queues, "h2", "https://heavy.test/2", 0);
    enqueue(&mut queues, "a1", "https://a.test/1", 0);
    enqueue(&mut queues, "a2", "https://a.test/2", 0);
    let first = queues.admit(now).unwrap();
    queues.ended(&first.key, now, Ended::BackOff);

    let admitted = admit_all(&mut queues, now);
    let ready_at = queues.ready_at(now);

    assert_eq!(
        (admitted, ready_at),
        (
            vec!["a1".to_owned(), "a2".to_owned()],
            Some(now + Duration::from_secs(2))
        )
    );
}

/// Covers: story:3-50
#[test]
fn a_domain_waits_out_the_moment_its_response_named() {
    let now = Instant::now();
    let mut queues = queues();
    enqueue(&mut queues, "h1", "https://heavy.test/1", 0);
    enqueue(&mut queues, "h2", "https://heavy.test/2", 0);
    let first = queues.admit(now).unwrap();
    let until = now + Duration::from_secs(120);
    queues.ended(&first.key, now, Ended::Until(until));

    let early = queues.admit(until - Duration::from_millis(1));
    let on_time = queues.admit(until).map(|admitted| admitted.id);

    assert_eq!((early, on_time), (None, Some("h2".to_owned())));
}

/// Covers: story:3-47
#[test]
fn a_domains_queue_holds_its_bound_and_refuses_more() {
    let mut queues = DomainQueues::new(DomainKeys::default(), 2);

    let taken = [
        enqueue(&mut queues, "1", "https://heavy.test/1", 0),
        enqueue(&mut queues, "2", "https://heavy.test/2", 0),
        enqueue(&mut queues, "3", "https://heavy.test/3", 0),
        enqueue(&mut queues, "2", "https://heavy.test/2", 0),
        enqueue(&mut queues, "a", "https://a.test/", 0),
    ];

    assert_eq!(taken, [true, true, false, false, true]);
}

/// Covers: story:3-50
#[test]
fn a_drained_domain_is_released_once_its_last_request_ended() {
    let now = Instant::now();
    let mut queues = queues();
    enqueue(&mut queues, "h1", "https://heavy.test/1", 0);
    enqueue(&mut queues, "h2", "https://heavy.test/2", 0);
    let first = queues.admit(now).unwrap();
    let last = queues.admit(now).unwrap();
    let held = queues.domains();
    queues.ended(&first.key, now, Ended::Done);
    queues.ended(&last.key, now, Ended::Done);

    assert_eq!(
        ((first.drained, last.drained), held, queues.domains()),
        ((false, true), 1, 0)
    );
}

/// Covers: story:3-50
#[rstest]
#[case::the_first(1, 2)]
#[case::the_second(2, 4)]
#[case::the_fifth(5, 32)]
#[case::the_sixth_reaches_the_ceiling(6, 60)]
#[case::far_past_the_ceiling(40, 60)]
fn a_backoff_doubles_from_two_seconds_up_to_a_minute(#[case] backoffs: u32, #[case] seconds: u64) {
    assert_eq!(backoff(backoffs), Duration::from_secs(seconds));
}

/// A run file in memory with the queues' table.
fn run_file() -> Connection {
    let connection = Connection::open_in_memory().unwrap();
    for statement in TABLES.statements {
        connection.execute(statement, []).unwrap();
    }
    connection
}

fn settings(run: Value) -> goodfetch_engine::settings::Resolved {
    let Value::Object(run) = run else {
        panic!("settings are a table");
    };
    let layers = Layers {
        run: Values::read(SettingsLayer::Run, &run, Product::Foxhound).unwrap(),
        ..Layers::default()
    };
    resolve(&layers, Product::Foxhound, Product::Foxhound).unwrap()
}

/// Covers: story:3-50
#[test]
fn a_domains_wait_survives_a_resume_from_the_journal() {
    let wall = UNIX_EPOCH + Duration::from_hours(491_001);
    let now = Instant::now();
    let file = run_file();
    let mut gate = DomainGates
        .open(&settings(json!({})), &file, wall, now)
        .unwrap();
    enqueue(gate.as_mut(), "h1", "https://heavy.test/1", 0);
    let first = gate.admit(now).unwrap();
    gate.ended(&first.key, now, Ended::Until(now + Duration::from_secs(30)));
    gate.journal(wall, now).unwrap()(&file).unwrap();

    let later = now + Duration::from_secs(10);
    let mut resumed = DomainGates
        .open(
            &settings(json!({})),
            &file,
            wall + Duration::from_secs(10),
            later,
        )
        .unwrap();
    enqueue(resumed.as_mut(), "h2", "https://heavy.test/2", 0);
    enqueue(resumed.as_mut(), "a1", "https://a.test/1", 0);

    assert_eq!(
        (admit_all(resumed.as_mut(), later), resumed.ready_at(later)),
        (vec!["a1".to_owned()], Some(now + Duration::from_secs(30)))
    );
}

/// Covers: story:3-50
#[test]
fn a_released_domain_leaves_the_journal() {
    let wall = UNIX_EPOCH + Duration::from_hours(491_001);
    let now = Instant::now();
    let file = run_file();
    let mut gate = DomainGates
        .open(&settings(json!({})), &file, wall, now)
        .unwrap();
    enqueue(gate.as_mut(), "h1", "https://heavy.test/1", 0);
    let first = gate.admit(now).unwrap();
    gate.ended(&first.key, now, Ended::Until(now + Duration::from_secs(5)));
    gate.journal(wall, now).unwrap()(&file).unwrap();
    let held: i64 = file
        .query_row("SELECT count(*) FROM domains", [], |row| row.get(0))
        .unwrap();

    let later = now + Duration::from_secs(5);
    enqueue(gate.as_mut(), "h2", "https://heavy.test/2", 0);
    let second = gate.admit(later).unwrap();
    gate.ended(&second.key, later, Ended::Done);
    gate.journal(wall + Duration::from_secs(5), later).unwrap()(&file).unwrap();
    let released: i64 = file
        .query_row("SELECT count(*) FROM domains", [], |row| row.get(0))
        .unwrap();

    assert_eq!((held, released), (1, 0));
}

/// Covers: story:3-48, setting:crawl.domain_keys
#[test]
fn a_crawls_domain_keys_setting_overrides_its_queues_keys() {
    let file = run_file();
    let gate = DomainGates
        .open(
            &settings(json!({ "crawl": { "domain_keys": { "github.io": "github" } } })),
            &file,
            UNIX_EPOCH,
            Instant::now(),
        )
        .unwrap();

    assert_eq!(gate.key("https://alice.github.io/"), "github");
}

proptest! {
    /// However requests are spread over domains and priorities, between two
    /// admissions of one domain every other domain that still waits is
    /// admitted exactly once, and each domain's requests come highest
    /// priority first, then in the order they came.
    ///
    /// Covers: story:3-47, story:3-49
    #[test]
    fn admission_is_round_robin_over_domains_and_priority_within_one(
        requests in proptest::collection::vec((0_usize..5, -2_i64..=2), 1..60),
    ) {
        let now = Instant::now();
        let mut queues = queues();
        for (index, (domain, priority)) in requests.iter().enumerate() {
            enqueue(&mut queues, &index.to_string(), &format!("https://d{domain}.test/{index}"), *priority);
        }

        let admitted: Vec<(usize, i64, usize)> = admit_all(&mut queues, now)
            .iter()
            .map(|id| {
                let index: usize = id.parse().unwrap();
                (requests[index].0, requests[index].1, index)
            })
            .collect();

        prop_assert_eq!(admitted.len(), requests.len());
        for (position, (domain, _, _)) in admitted.iter().enumerate() {
            let Some(next) = admitted[position + 1..].iter().position(|(other, _, _)| other == domain) else {
                continue;
            };
            let between: Vec<usize> = admitted[position + 1..position + 1 + next].iter().map(|(other, _, _)| *other).collect();
            let still_waiting: std::collections::BTreeSet<usize> = admitted[position + 1..]
                .iter()
                .map(|(other, _, _)| *other)
                .filter(|other| other != domain)
                .collect();
            let mut seen = between.clone();
            seen.sort_unstable();
            seen.dedup();
            prop_assert_eq!(seen.len(), between.len(), "a domain went twice in one turn");
            prop_assert_eq!(seen.into_iter().collect::<std::collections::BTreeSet<_>>(), still_waiting);
        }
        let mut by_domain: BTreeMap<usize, Vec<(i64, usize)>> = BTreeMap::new();
        for (domain, priority, index) in &admitted {
            by_domain.entry(*domain).or_default().push((*priority, *index));
        }
        for order in by_domain.values() {
            let mut expected = order.clone();
            expected.sort_by_key(|(priority, index)| (std::cmp::Reverse(*priority), *index));
            prop_assert_eq!(order, &expected);
        }
    }

    /// A backoff's end is the reading it was given plus its wait, which
    /// stays within 2 s and a minute and never shrinks in a row.
    ///
    /// Covers: story:3-50
    #[test]
    fn a_backoff_is_a_pure_function_of_now(
        offsets in proptest::collection::vec(0_u64..100_000, 1..12),
    ) {
        let start = Instant::now();
        let mut queues = queues();
        let mut previous = Duration::ZERO;
        let mut at = start;
        for (index, offset) in offsets.iter().enumerate() {
            enqueue(&mut queues, &index.to_string(), "https://heavy.test/", 0);
            at = queues.ready_at(at).unwrap().max(at) + Duration::from_millis(*offset);
            let admitted = queues.admit(at).unwrap();
            queues.ended(&admitted.key, at, Ended::BackOff);
            enqueue(&mut queues, "next", "https://heavy.test/next", 0);
            let wait = queues.ready_at(at).unwrap() - at;
            prop_assert!(wait >= Duration::from_secs(2) && wait <= Duration::from_mins(1));
            prop_assert!(wait >= previous);
            previous = wait;
            queues.admit(queues.ready_at(at).unwrap()).unwrap();
        }
    }
}
