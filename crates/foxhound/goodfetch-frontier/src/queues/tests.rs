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
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use goodfetch_engine::admission::{Ended, Gate, Gates, Held, Waiting};
use goodfetch_engine::entropy::Entropy;
use goodfetch_engine::settings::{Layers, Values, resolve};
use goodfetch_protocol::catalog::Product;
use goodfetch_protocol::settings::SettingsLayer;
use proptest::prelude::*;
use proptest::sample::Index;
use rstest::rstest;
use rusqlite::Connection;
use serde_json::{Value, json};
use tokio::time::Instant;

use super::{BOUND, DomainGates, DomainQueues, TABLES};
use crate::domain_key::DomainKeys;

fn queues() -> DomainQueues {
    DomainQueues::new(DomainKeys::default(), BOUND, Entropy::seeded(1))
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
    let mut queues = DomainQueues::new(DomainKeys::default(), 2, Entropy::seeded(1));
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
    queues.ended(&first.key, now, Ended::BackOff(None));

    let admitted = admit_all(&mut queues, now);
    let wait = queues.ready_at(now).map(|ready_at| ready_at - now);

    assert_eq!(admitted, vec!["a1".to_owned(), "a2".to_owned()]);
    assert!(
        wait.is_some_and(|wait| a_first_backoff(wait, Duration::from_secs(2))),
        "{wait:?}"
    );
}

/// Whether `wait` is a first wait of a backoff from `initial`: drawn
/// between it and three times it (ADR 0118).
fn a_first_backoff(wait: Duration, initial: Duration) -> bool {
    (initial..=initial * 3).contains(&wait)
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
    let mut queues = DomainQueues::new(DomainKeys::default(), 2, Entropy::seeded(1));

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

/// Queues that took `settings` as their Run's, opened from an empty run
/// file at `now`.
fn queues_under(settings: &goodfetch_engine::settings::Resolved, now: Instant) -> Box<dyn Gate> {
    DomainGates
        .open(
            settings,
            &run_file(),
            SystemTime::UNIX_EPOCH + Duration::from_hours(500_000),
            now,
            Entropy::seeded(1),
        )
        .unwrap()
}

/// While the Run shares no Politeness, the queues back a domain off by
/// its own settings: the first wait drawn between
/// `politeness.backoff.initial` and three times it, and none with
/// `politeness.backoff.enabled` off (ADRs 0118 and 0138).
///
/// Covers: setting:politeness.backoff.initial, setting:politeness.backoff.enabled
#[rstest]
#[case::backoff_on(json!({ "politeness": { "backoff": { "initial": 10_000, "max": 60_000 } } }), Some(Duration::from_secs(10)))]
#[case::backoff_off(json!({ "politeness": { "backoff": { "enabled": false } } }), None)]
fn a_domain_backs_off_as_its_runs_backoff_settings_say(
    #[case] run: Value,
    #[case] initial: Option<Duration>,
) {
    let now = Instant::now();
    let mut queues = queues_under(&settings(run), now);
    enqueue(&mut *queues, "h1", "https://heavy.test/1", 0);
    enqueue(&mut *queues, "h2", "https://heavy.test/2", 0);
    let first = queues.admit(now).unwrap();
    queues.ended(&first.key, now, Ended::TooManyRequests(None));

    let wait = queues.ready_at(now).map(|ready_at| ready_at - now);

    match initial {
        Some(initial) => assert!(
            wait.is_some_and(|wait| a_first_backoff(wait, initial)),
            "{wait:?}"
        ),
        None => assert_eq!(wait, Some(Duration::ZERO)),
    }
}

/// While the Run shares no Politeness, a 429 halves the count of a
/// domain's requests in flight, from those in flight when it came: a
/// domain with four in flight then admits a request only once fewer than
/// two are (ADRs 0021 and 0086).
///
/// Covers: story:3-80
#[test]
fn a_429_halves_the_requests_a_domain_has_in_flight() {
    let now = Instant::now();
    let run = json!({ "politeness": { "backoff": { "enabled": false } } });
    let mut queues = queues_under(&settings(run), now);
    for id in ["h1", "h2", "h3", "h4", "h5"] {
        enqueue(&mut *queues, id, &format!("https://heavy.test/{id}"), 0);
    }
    let in_flight: Vec<String> = (0..4)
        .filter_map(|_| queues.admit(now).map(|admitted| admitted.id))
        .collect();
    let key = "heavy.test";
    queues.ended(key, now, Ended::TooManyRequests(None));

    let at_three = queues.admit(now).map(|admitted| admitted.id);
    queues.ended(key, now, Ended::Done);
    let at_two = queues.admit(now).map(|admitted| admitted.id);
    queues.ended(key, now, Ended::Done);
    let at_one = queues.admit(now).map(|admitted| admitted.id);

    assert_eq!(
        (in_flight.len(), at_three, at_two, at_one),
        (4, None, None, Some("h5".to_owned()))
    );
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
        .open(&settings(json!({})), &file, wall, now, Entropy::seeded(1))
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
            Entropy::seeded(1),
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
        .open(&settings(json!({})), &file, wall, now, Entropy::seeded(1))
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
            Entropy::seeded(1),
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
        let mut queues = DomainQueues::new(DomainKeys::default(), bound, Entropy::seeded(1));
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
    /// stays within the catalog's `politeness.backoff.initial` of 2 s and
    /// its `politeness.backoff.max` of a minute.
    ///
    /// Covers: story:3-50
    #[test]
    fn a_backoff_is_a_pure_function_of_now(
        offsets in proptest::collection::vec(0_u64..100_000, 1..12),
    ) {
        let start = Instant::now();
        let mut queues = queues();
        let mut at = start;
        for (index, offset) in offsets.iter().enumerate() {
            enqueue(&mut queues, &index.to_string(), "https://heavy.test/", 0);
            at = queues.ready_at(at).unwrap().max(at) + Duration::from_millis(*offset);
            let admitted = queues.admit(at).unwrap();
            queues.ended(&admitted.key, at, Ended::BackOff(None));
            enqueue(&mut queues, "next", "https://heavy.test/next", 0);
            let wait = queues.ready_at(at).unwrap() - at;
            prop_assert!(wait >= Duration::from_secs(2) && wait <= Duration::from_mins(1));
            queues.admit(queues.ready_at(at).unwrap()).unwrap();
        }
    }
}

/// A waiting request its Run ended before sending, such as one a learned
/// rule merged, leaves its domain's queue: the others are admitted in
/// their order, and a domain left with none takes no more turns.
///
/// Covers: story:6-222
#[test]
fn a_withdrawn_request_is_never_admitted() {
    let now = Instant::now();
    let mut queues = queues();
    enqueue(&mut queues, "a1", "https://a.test/1", 0);
    enqueue(&mut queues, "a2", "https://a.test/1/", 0);
    enqueue(&mut queues, "b1", "https://b.test/1", 0);
    enqueue(&mut queues, "a3", "https://a.test/3", 0);
    let a_key = queues.key("https://a.test/");
    let b_key = queues.key("https://b.test/");

    queues.withdraw(&a_key, "a2");
    queues.withdraw(&b_key, "b1");

    assert_eq!(admit_all(&mut queues, now), ["a1", "a3"]);
}

/// One change the queues' waits property test makes, each domain or
/// request named by its place among those that can take it.
#[derive(Clone, Debug)]
enum Change {
    /// A request of the domain `0`.
    Enqueue(usize),
    /// The waiting request at that place is withdrawn.
    Withdraw(Index),
    /// The next ready request is admitted.
    Admit,
    /// The domain at that place among those with waiting requests is held
    /// for so many milliseconds, for that reason.
    Hold(Index, u64, Held),
    /// That domain's hold ends.
    Wake(Index),
    /// The request in flight at that place ended, its domain to wait so
    /// many milliseconds, if any.
    Ended(Index, Option<u64>),
    /// The clock goes on so many milliseconds.
    Advance(u64),
}

fn held() -> impl Strategy<Value = Held> {
    prop_oneof![
        Just(Held::Concurrency),
        Just(Held::Delay),
        Just(Held::BackOff),
        Just(Held::PolitenessFile),
    ]
}

fn change() -> impl Strategy<Value = Change> {
    prop_oneof![
        3 => (0..5usize).prop_map(Change::Enqueue),
        1 => any::<Index>().prop_map(Change::Withdraw),
        2 => Just(Change::Admit),
        2 => (any::<Index>(), 0..5_000u64, held()).prop_map(|(at, wait, held)| Change::Hold(at, wait, held)),
        1 => any::<Index>().prop_map(Change::Wake),
        2 => (any::<Index>(), proptest::option::of(0..5_000u64)).prop_map(|(at, wait)| Change::Ended(at, wait)),
        2 => (0..3_000u64).prop_map(Change::Advance),
    ]
}

/// What the model knows of one domain: its waiting requests, its
/// requests in flight, its hold and the moment it waits until.
#[derive(Debug, Default)]
struct Modeled {
    waiting: Vec<String>,
    in_flight: usize,
    held: Option<(Instant, Held)>,
    ready_at: Option<Instant>,
}

impl Modeled {
    /// Why it waits at `now`, and until when: its hold while it lasts,
    /// else its wait while that lasts, else nothing.
    fn waits(&self, now: Instant) -> (Option<Held>, Option<Instant>) {
        match (self.held, self.ready_at) {
            (Some((until, held)), _) if until > now => (Some(held), Some(until)),
            (_, Some(ready_at)) if ready_at > now => (Some(Held::BackOff), Some(ready_at)),
            _ => (None, None),
        }
    }
}

/// Each group the queues answer at `now`, every domain in its sample: its
/// reason, its requests, and each domain with until when it waits.
type Groups = Vec<Group>;

/// One group: its reason, its requests, and each domain with until when it
/// waits.
type Group = (Option<Held>, usize, Vec<(String, Option<Instant>)>);

/// The groups a full walk of the model finds at `now`.
fn walked(model: &BTreeMap<String, Modeled>, now: Instant) -> Groups {
    let mut groups: BTreeMap<u8, Group> = BTreeMap::new();
    for (key, domain) in model
        .iter()
        .filter(|(_, domain)| !domain.waiting.is_empty())
    {
        let (held, until) = domain.waits(now);
        let rank = match held {
            None => 0,
            Some(Held::Concurrency) => 1,
            Some(Held::Delay) => 2,
            Some(Held::BackOff) => 3,
            Some(Held::PolitenessFile) => 4,
        };
        let group = groups.entry(rank).or_insert((held, 0, Vec::new()));
        group.1 += domain.waiting.len();
        let shown = until.filter(|_| held != Some(Held::Concurrency));
        group.2.push((key.clone(), shown));
    }
    groups.into_values().collect()
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// The waits the queues keep as each hold, wake, admission and end
    /// changes them, which a Run's progress shows, equal a full walk of
    /// every domain at that moment: each reason's requests and domains,
    /// with until when each waits, and the requests waiting in all.
    ///
    /// Covers: story:3-131
    #[test]
    fn the_waits_kept_equal_a_full_walk_of_the_domains(
        changes in proptest::collection::vec(change(), 1..60),
    ) {
        let start = Instant::now();
        let mut now = start;
        let mut queues = DomainQueues::new(DomainKeys::default(), 1_000, Entropy::seeded(1));
        let mut model: BTreeMap<String, Modeled> = BTreeMap::new();
        let mut in_flight: Vec<String> = Vec::new();
        let mut order = 0;
        let mut seen = Vec::new();
        for change in changes {
            match change {
                Change::Enqueue(domain) => {
                    order += 1;
                    let key = queues.key(&format!("https://d{domain}.test/"));
                    let id = format!("r{order}");
                    let enqueued = queues.enqueue(Waiting {
                        id: id.clone(),
                        key: key.clone(),
                        priority: 0,
                        order,
                        forefront: false,
                    });
                    prop_assert!(enqueued);
                    model.entry(key).or_default().waiting.push(id);
                }
                Change::Withdraw(at) => {
                    let waiting: Vec<(String, String)> = model
                        .iter()
                        .flat_map(|(key, domain)| domain.waiting.iter().map(|id| (key.clone(), id.clone())))
                        .collect();
                    if !waiting.is_empty() {
                        let (key, id) = at.get(&waiting).clone();
                        queues.withdraw(&key, &id);
                        model.get_mut(&key).unwrap().waiting.retain(|waiting| *waiting != id);
                    }
                }
                Change::Admit => {
                    if let Some(admitted) = queues.admit(now) {
                        let domain = model.get_mut(&admitted.key).unwrap();
                        domain.waiting.retain(|waiting| *waiting != admitted.id);
                        domain.in_flight += 1;
                        in_flight.push(admitted.key);
                    }
                }
                Change::Hold(at, wait, held) => {
                    let keys: Vec<String> = model
                        .iter()
                        .filter(|(_, domain)| !domain.waiting.is_empty())
                        .map(|(key, _)| key.clone())
                        .collect();
                    if !keys.is_empty() {
                        let key = at.get(&keys).clone();
                        let until = now + Duration::from_millis(wait);
                        queues.hold(&key, until, held);
                        model.get_mut(&key).unwrap().held = Some((until, held));
                    }
                }
                Change::Wake(at) => {
                    let keys: Vec<String> = model
                        .iter()
                        .filter(|(_, domain)| !domain.waiting.is_empty())
                        .map(|(key, _)| key.clone())
                        .collect();
                    if !keys.is_empty() {
                        let key = at.get(&keys).clone();
                        queues.wake(&key);
                        model.get_mut(&key).unwrap().held = None;
                    }
                }
                Change::Ended(at, wait) => {
                    if !in_flight.is_empty() {
                        let key = in_flight.remove(at.index(in_flight.len()));
                        let domain = model.get_mut(&key).unwrap();
                        domain.in_flight -= 1;
                        if let Some(wait) = wait {
                            let until = now + Duration::from_millis(wait);
                            queues.ended(&key, now, Ended::Until(until));
                            domain.ready_at = Some(domain.ready_at.map_or(until, |ready_at| ready_at.max(until)));
                        } else {
                            queues.ended(&key, now, Ended::Done);
                            if domain.waits(now).0.is_none() {
                                domain.ready_at = None;
                            }
                        }
                    }
                }
                Change::Advance(wait) => now += Duration::from_millis(wait),
            }
            // A domain the queues released holds nothing more: the model
            // forgets it too, as it holds no wait that lasts.
            model.retain(|_, domain| {
                !(domain.waiting.is_empty() && domain.in_flight == 0 && domain.waits(now).0.is_none())
            });
            let groups: Groups = queues
                .queued(now, usize::MAX)
                .into_iter()
                .map(|group| (group.held, group.requests, group.sample))
                .collect();
            seen.push((
                (groups, queues.waiting()),
                (
                    walked(&model, now),
                    model.values().map(|domain| domain.waiting.len()).sum::<usize>(),
                ),
            ));
        }
        let (kept, walked): (Vec<_>, Vec<_>) = seen.into_iter().unzip();

        prop_assert_eq!(kept, walked);
    }
}
