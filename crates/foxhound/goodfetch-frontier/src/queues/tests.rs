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

/// Enqueues a request of `url` named `id` at `priority`, its order the
/// count of requests enqueued before it in the test.
fn enqueue(gate: &mut dyn Gate, id: &str, url: &str, priority: i64) -> bool {
    enqueue_as(gate, id, url, priority, false)
}

/// Enqueues a request of `url` named `id` at `priority` that goes ahead of
/// its domain's pending requests.
fn enqueue_forefront(gate: &mut dyn Gate, id: &str, url: &str, priority: i64) -> bool {
    enqueue_as(gate, id, url, priority, true)
}

fn enqueue_as(gate: &mut dyn Gate, id: &str, url: &str, priority: i64, forefront: bool) -> bool {
    thread_local! {
        static ORDER: std::cell::Cell<i64> = const { std::cell::Cell::new(0) };
    }
    let key = gate.key(url);
    let order = ORDER.with(|order| order.replace(order.get() + 1));
    gate.enqueue(Waiting {
        id: id.to_owned(),
        key,
        priority,
        order,
        forefront,
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

/// Covers: story:6-215
#[test]
fn forefront_goes_ahead_of_its_domains_pending_requests_and_of_no_other_domain() {
    let now = Instant::now();
    let mut queues = queues();
    enqueue(&mut queues, "h-high", "https://heavy.test/high", 5);
    enqueue(&mut queues, "h-first", "https://heavy.test/first", 0);
    enqueue(&mut queues, "a", "https://a.test/", 0);
    enqueue_forefront(&mut queues, "h-front", "https://heavy.test/front", -1);
    enqueue_forefront(
        &mut queues,
        "h-later-front",
        "https://heavy.test/later-front",
        0,
    );
    enqueue(&mut queues, "h-higher", "https://heavy.test/higher", 9);

    let admitted = admit_all(&mut queues, now);

    assert_eq!(
        admitted,
        [
            "h-later-front",
            "a",
            "h-front",
            "h-higher",
            "h-high",
            "h-first"
        ]
    );
}

/// A full queue takes a request that goes ahead of its last one, which
/// waits outside instead; while one waits outside, a request that would go
/// after it waits outside too, though the queue has room.
///
/// Covers: story:6-215
#[test]
fn a_full_queue_puts_out_its_last_request_for_one_that_goes_ahead() {
    let now = Instant::now();
    let mut queues = DomainQueues::new(DomainKeys::default(), 2);
    let mut taken = vec![
        enqueue(&mut queues, "1", "https://heavy.test/1", 0),
        enqueue(&mut queues, "2", "https://heavy.test/2", 0),
        enqueue_forefront(&mut queues, "front", "https://heavy.test/front", 0),
    ];
    let first = queues.admit(now).unwrap();
    taken.push(enqueue(&mut queues, "3", "https://heavy.test/3", 0));
    let second = queues.admit(now).unwrap();
    taken.push(enqueue(&mut queues, "2", "https://heavy.test/2", 0));

    assert_eq!(
        (taken, first.id, second.id, second.drained),
        (
            vec![true, true, true, false, true],
            "front".to_owned(),
            "1".to_owned(),
            true
        )
    );
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

/// One step of a crawl's requests through the gate.
#[derive(Clone, Debug)]
enum Step {
    /// A request of one of three domains comes.
    Comes {
        domain: usize,
        priority: i64,
        forefront: bool,
    },
    /// The gate admits the next request ready.
    Admit,
    /// A request admitted ends, done or back to pending for a retry.
    Ends {
        attempt: proptest::sample::Index,
        retried: bool,
    },
}

fn step() -> impl Strategy<Value = Step> {
    prop_oneof![
        3 => (0_usize..3, -2_i64..=2, any::<bool>()).prop_map(|(domain, priority, forefront)| Step::Comes { domain, priority, forefront }),
        3 => Just(Step::Admit),
        2 => (any::<proptest::sample::Index>(), any::<bool>()).prop_map(|(attempt, retried)| Step::Ends { attempt, retried }),
    ]
}

/// A request in the reference's line of its domain.
#[derive(Clone, Debug)]
struct Lined {
    id: String,
    priority: i64,
    forefront: bool,
    admitted: bool,
}

/// Lines `request` up by hand: a `forefront` one in front of the line, any
/// other one behind the `forefront` ones and every one of its priority or
/// higher.
fn line_up(line: &mut Vec<Lined>, request: Lined) {
    let at = if request.forefront {
        0
    } else {
        line.iter()
            .position(|lined| !lined.forefront && lined.priority < request.priority)
            .unwrap_or(line.len())
    };
    line.insert(at, request);
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

    /// However requests come, with any priority, `forefront` or neither,
    /// are admitted, and end done or back to pending, the gate admits each
    /// domain's requests in the order a reference lines them up by hand:
    /// a `forefront` request in front of every request of its domain, any
    /// other one behind the `forefront` ones and every one of its priority
    /// or higher, a retried one where it stood; even when a queue's bound
    /// leaves requests outside, refilled from the run state when it
    /// drains. It admits nothing only when nothing is pending.
    ///
    /// Covers: story:6-215, story:3-49
    #[test]
    fn each_domain_admits_its_requests_in_the_order_they_were_lined_up(
        steps in proptest::collection::vec(step(), 1..80),
        bound in 1_usize..4,
    ) {
        let now = Instant::now();
        let mut queues = DomainQueues::new(DomainKeys::default(), bound);
        let mut lines: [Vec<Lined>; 3] = Default::default();
        let mut pending: BTreeMap<i64, (usize, Waiting)> = BTreeMap::new();
        let mut in_flight: Vec<(usize, Waiting)> = Vec::new();
        let mut order = 0;
        for step in steps {
            match step {
                Step::Comes { domain, priority, forefront } => {
                    order += 1;
                    let id = order.to_string();
                    let waiting = Waiting {
                        key: queues.key(&format!("https://d{domain}.test/{id}")),
                        id: id.clone(),
                        priority,
                        order,
                        forefront,
                    };
                    pending.insert(order, (domain, waiting.clone()));
                    queues.enqueue(waiting);
                    line_up(&mut lines[domain], Lined { id, priority, forefront, admitted: false });
                }
                Step::Admit => match queues.admit(now) {
                    Some(admitted) => {
                        let order: i64 = admitted.id.parse().unwrap();
                        let (domain, waiting) = pending.remove(&order).unwrap();
                        let next = lines[domain].iter_mut().find(|lined| !lined.admitted).unwrap();
                        prop_assert_eq!(&admitted.id, &next.id);
                        next.admitted = true;
                        in_flight.push((domain, waiting));
                        if admitted.drained {
                            let outside: Vec<Waiting> = pending
                                .values()
                                .rev()
                                .filter(|(other, _)| *other == domain)
                                .map(|(_, waiting)| waiting.clone())
                                .collect();
                            for waiting in outside {
                                queues.enqueue(waiting);
                            }
                        }
                    }
                    None => prop_assert!(pending.is_empty(), "nothing admitted while {pending:?} pend"),
                },
                Step::Ends { attempt, retried } => {
                    if in_flight.is_empty() {
                        continue;
                    }
                    let (domain, waiting) = in_flight.remove(attempt.index(in_flight.len()));
                    queues.ended(&waiting.key, now, Ended::Done);
                    let lined = lines[domain].iter().position(|lined| lined.id == waiting.id).unwrap();
                    if retried {
                        lines[domain][lined].admitted = false;
                        pending.insert(waiting.order, (domain, waiting.clone()));
                        queues.enqueue(waiting);
                    } else {
                        lines[domain].remove(lined);
                    }
                }
            }
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
