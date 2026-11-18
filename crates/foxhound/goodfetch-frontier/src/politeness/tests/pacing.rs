//! A crawl's pacing (spec #582's Settings; story 6-247 and 6-248) through
//! the engine's rule interface: the adaptive delay, the exponentially
//! weighted mean of a domain key's response latency, never below
//! `politeness.domain.delay` nor above `politeness.backoff.max`; and the
//! per-domain rate, a token bucket (GCRA) of `politeness.domain.rate`
//! requests a minute and `politeness.domain.burst` tokens, shared by every
//! run on the key. Each is a pure function of the wall readings the tests
//! give, `proptest` driving them; two engines' runs on one Politeness file
//! are held to the rate over a minute of Tokio's paused clock.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex;
use std::time::{Duration, SystemTime};

use goodfetch_engine::clock::Clock;
use goodfetch_engine::politeness::{
    self as engine, Ending, Hold, Holder, Politeness, PolitenessError, PolitenessOptions,
    SharedRule, Step, Taken, Verdict,
};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_test_support::{CrashVfs, IoFault, TestEnv};
use proptest::prelude::*;
use rstest::rstest;
use serde_json::{Value, json};

use super::on_file::OnFile as _;
use super::{
    KEY, at, changed, file, latency_noted, milliseconds, pace, rule, send_taken, start_moved,
};
use crate::politeness::{
    DomainRule, DomainState, Pace, Parts, TABLES, Wrote, settled_at_send, started_at_send,
};

fn adaptive(delay: u64, max: u64) -> Value {
    json!({
        "politeness": {
            "domain": { "delay": delay, "adaptive_delay": true },
            "backoff": { "max": max },
        }
    })
}

fn rated(rate: u64, burst: u64) -> Value {
    json!({ "politeness": { "domain": { "rate": rate, "burst": burst } } })
}

/// With the adaptive delay on, a key's delay between starts is the longer
/// of `politeness.domain.delay` and the weighted mean of its responses'
/// latency (each new one weighing 0.3), that mean capped at
/// `politeness.backoff.max`: it lengthens as the responses slow, never
/// below the delay; off, the latency adds nothing.
///
/// Covers: story:6-247, setting:politeness.domain.adaptive_delay, setting:politeness.domain.delay
#[rstest]
#[case::no_response_yet(adaptive(1_000, 5_000), &[], 1_000)]
#[case::a_slow_response(adaptive(1_000, 5_000), &[3_000], 3_000)]
#[case::a_faster_one_after_it(adaptive(1_000, 5_000), &[3_000, 1_000], 2_400)]
#[case::slowing_down(adaptive(1_000, 5_000), &[1_500, 2_000, 4_000], 2_355)]
/// Covers: setting:politeness.backoff.max
#[case::capped_at_the_backoff_max(adaptive(1_000, 5_000), &[10_000], 5_000)]
#[case::never_below_the_delay(adaptive(1_000, 5_000), &[200, 100], 1_000)]
/// Covers: setting:politeness.backoff.max
#[case::a_delay_above_the_cap_stays(adaptive(6_000, 5_000), &[10_000], 6_000)]
#[case::off(json!({ "politeness": { "domain": { "delay": 1_000 } } }), &[3_000], 1_000)]
fn the_adaptive_delay_lengthens_with_the_keys_latency_within_its_bounds(
    #[case] run: Value,
    #[case] latencies: &[u64],
    #[case] delay: u64,
) {
    let file = file();
    for latency in latencies {
        latency_noted(&file, KEY, milliseconds(*latency)).unwrap();
    }
    let rule = rule(run);
    rule.started(&file, KEY, at(10)).unwrap();

    let verdicts = (
        rule.admit(&file, KEY, 0, at(10) + milliseconds(delay - 1))
            .unwrap(),
        rule.admit(&file, KEY, 0, at(10) + milliseconds(delay))
            .unwrap(),
    );

    assert_eq!(
        verdicts,
        (
            Verdict::Until(at(10) + milliseconds(delay), Hold::Delay),
            Verdict::Go
        )
    );
}

/// Every sharing run's rate draws on one bucket per key: a burst of
/// `politeness.domain.burst` starts goes at once, the next waits until a
/// token came back, one every minute divided by `politeness.domain.rate`.
///
/// Covers: story:6-248, setting:politeness.domain.rate, setting:politeness.domain.burst
#[rstest]
#[case::one_a_second(rated(60, 1), 1, at(11))]
#[case::a_burst_of_three(rated(60, 3), 3, at(11))]
#[case::thirty_a_minute(rated(30, 1), 1, at(12))]
fn the_rate_lets_a_burst_go_then_one_start_per_interval(
    #[case] run: Value,
    #[case] burst: usize,
    #[case] next: SystemTime,
) {
    let file = file();
    let (first, second) = (rule(run.clone()), rule(run));
    let mut verdicts = Vec::new();
    for at_once in 0..burst {
        let rule = if at_once % 2 == 0 { &first } else { &second };
        verdicts.push(rule.admit(&file, KEY, 0, at(10)).unwrap());
        rule.started(&file, KEY, at(10)).unwrap();
    }

    let waiting = second.admit(&file, KEY, 0, at(10)).unwrap();
    let later = first.admit(&file, KEY, 0, next).unwrap();

    assert_eq!(
        (verdicts, waiting, later),
        (
            vec![Verdict::Go; burst],
            Verdict::Until(next, Hold::Delay),
            Verdict::Go
        )
    );
}

/// A request sent later than its admission took its token counts its
/// interval from the send, so a site never sees two starts closer than
/// the rate; a send once the bucket is full again moves nothing.
///
/// Covers: story:6-248, setting:politeness.domain.rate
#[rstest]
#[case::sent_after_its_admission(at(10) + milliseconds(300), at(11) + milliseconds(300))]
#[case::sent_once_the_bucket_is_full(at(20), at(11))]
fn a_start_counts_its_interval_from_its_send(#[case] sent: SystemTime, #[case] next: SystemTime) {
    let file = file();
    let rule = rule(rated(60, 1));
    rule.started(&file, KEY, at(10)).unwrap();

    start_moved(&file, KEY, sent).unwrap();

    assert_eq!(
        rule.admit(&file, KEY, 0, at(10) + milliseconds(500))
            .unwrap(),
        if next > at(10) + milliseconds(500) {
            Verdict::Until(next, Hold::Delay)
        } else {
            Verdict::Go
        }
    );
}

/// A rated run's request sends no sooner than its rate allows after the
/// sends before it on the key, every sharing rated run's together: one
/// whose send came later than its admission, past the next request's
/// admission, would otherwise reach the site within the interval of that
/// next request, whose token the earlier admission let it take. A send
/// the bucket has no token for takes none and waits for the moment it
/// answers, then asks again, so its token is taken, and its start
/// recorded, when it goes, not when it began to wait; a burst sends at
/// once, and a run with no rate waits for none and takes none. Each send
/// is `(milliseconds after 10 s, rated)`, each answer the wait's end in
/// milliseconds after 10 s, if any.
///
/// Covers: story:6-248, setting:politeness.domain.rate, setting:politeness.domain.burst
#[rstest]
#[case::an_interval_apart(1, &[(0, true), (1_000, true)], &[None, None])]
#[case::within_the_interval(1, &[(0, true), (400, true)], &[None, Some(1_000)])]
#[case::a_wait_takes_no_token(
    1,
    &[(0, true), (100, true), (200, true), (1_000, true), (1_000, true)],
    &[None, Some(1_000), Some(1_000), None, Some(2_000)]
)]
/// Covers: setting:politeness.domain.burst
#[case::a_burst_at_once(2, &[(0, true), (0, true), (0, true)], &[None, None, Some(1_000)])]
#[case::unrated_between(
    1,
    &[(0, true), (100, false), (200, true)],
    &[None, None, Some(1_000)]
)]
fn a_rated_send_waits_for_the_interval_after_the_sends_before_it(
    #[case] burst: u64,
    #[case] sends: &[(u64, bool)],
    #[case] waits: &[Option<u64>],
) {
    let file = file();
    let pace = rule(rated(60, burst)).pace;

    let answered: Vec<Option<SystemTime>> = sends
        .iter()
        .map(|(after, rated)| {
            send_taken(
                &file,
                KEY,
                at(10) + milliseconds(*after),
                rated.then_some(pace),
            )
            .unwrap()
        })
        .collect();

    let expected: Vec<Option<SystemTime>> = waits
        .iter()
        .map(|wait| wait.map(|wait| at(10) + milliseconds(wait)))
        .collect();
    assert_eq!(answered, expected);
}

/// A crawl request's send waits for its domain's delay after the last
/// send any sharing run made there, the delay as the key stands when it
/// sends: one whose send came later than the next request's admission,
/// which counted from an earlier start, would otherwise reach the site
/// within the delay of that next request. Its own admission holds nothing,
/// a hop's or a navigation's send counts as a send without waiting, a send
/// that waits records nothing, and the adaptive delay's lengthening since
/// the admission counts. Each send is `(milliseconds after 10 s, its
/// request's own)`, each answer the wait's end in milliseconds after 10 s,
/// if any.
///
/// Covers: story:6-247, setting:politeness.domain.delay
#[rstest]
#[case::a_delay_apart(delayed(100), &[], &[(0, true), (100, true)], &[None, None])]
#[case::overtaken_within_the_delay(delayed(100), &[], &[(0, true), (28, true)], &[None, Some(100)])]
#[case::a_wait_records_nothing(
    delayed(100),
    &[],
    &[(0, true), (28, true), (100, true), (150, true)],
    &[None, Some(100), None, Some(200)]
)]
#[case::a_hop_counts_without_waiting(
    delayed(100),
    &[],
    &[(0, true), (40, false), (110, true)],
    &[None, None, Some(140)]
)]
/// Covers: setting:politeness.domain.adaptive_delay
#[case::the_adaptive_delay_lengthened(adaptive(100, 400), &[400], &[(0, true), (150, true)], &[None, Some(400)])]
#[case::a_delay_of_zero(delayed(0), &[], &[(0, true), (0, true)], &[None, None])]
fn a_send_waits_for_the_delay_after_the_sends_before_it(
    #[case] run: Value,
    #[case] latencies: &[u64],
    #[case] sends: &[(u64, bool)],
    #[case] waits: &[Option<u64>],
) {
    let file = file();
    for latency in latencies {
        latency_noted(&file, KEY, milliseconds(*latency)).unwrap();
    }
    let rule = rule(run);
    let pace = rule.pace;

    let answered: Vec<Option<SystemTime>> = sends
        .iter()
        .map(|(after, own)| {
            let now = at(10) + milliseconds(*after);
            if *own {
                rule.started(&file, KEY, now).unwrap();
            }
            send_taken(&file, KEY, now, own.then_some(pace)).unwrap()
        })
        .collect();

    let expected: Vec<Option<SystemTime>> = waits
        .iter()
        .map(|wait| wait.map(|wait| at(10) + milliseconds(wait)))
        .collect();
    assert_eq!(answered, expected);
}

/// A send inside the millisecond before its delay or its rate's interval
/// after the last send is over waits: the Politeness file keeps the last
/// send at the next whole millisecond, so a send checked at its own next
/// whole millisecond would go up to 1 ms before the delay, as #1735's
/// crawl did. The first send goes 0.4 ms into 10 s, the second half a
/// millisecond before the delay after that stored millisecond, still
/// within the delay of the first; it waits until the delay after it.
///
/// Covers: story:6-247, setting:politeness.domain.delay
#[rstest]
#[case::the_delay(delayed(100), 100)]
/// Covers: story:6-248, setting:politeness.domain.rate
#[case::the_rate(rated(60, 1), 1_000)]
fn a_send_inside_the_last_millisecond_of_the_wait_still_waits(
    #[case] run: Value,
    #[case] spacing: u64,
) {
    let file = file();
    let pace = rule(run).pace;
    let first = at(10) + Duration::from_micros(400);
    let stored = at(10) + milliseconds(1);
    let second = stored + milliseconds(spacing) - Duration::from_micros(500);

    assert_eq!(send_taken(&file, KEY, first, Some(pace)).unwrap(), None);
    let answered = send_taken(&file, KEY, second, Some(pace)).unwrap();

    assert_eq!(answered, Some(stored + milliseconds(spacing)));
}

fn delayed(delay: u64) -> Value {
    json!({ "politeness": { "domain": { "delay": delay } } })
}

/// The weighted mean of `latencies`, by floating point: the model the
/// rule's whole milliseconds are checked against.
fn mean(latencies: &[u64]) -> f64 {
    latencies
        .iter()
        .fold(None, |mean: Option<f64>, latency| {
            #[expect(clippy::cast_precision_loss, reason = "latencies below an hour")]
            let latency = *latency as f64;
            Some(mean.map_or(latency, |mean| 0.3 * latency + 0.7 * mean))
        })
        .unwrap_or(0.0)
}

proptest! {
    /// Two crawl sends of a key start at least its delay apart by the wall
    /// clock wherever in their milliseconds they fall: the second, from
    /// 2 ms before the delay after the first to 1 ms past it, goes only once
    /// that delay is over, else it waits until a moment that is, and a send
    /// then goes.
    ///
    /// Covers: story:6-247, setting:politeness.domain.delay
    #[test]
    fn a_send_never_starts_sooner_than_the_delay_after_the_last_at_any_microsecond(
        delay in 2_u64..2_000,
        first in 0_u64..1_000,
        around in 0_u64..3_000,
    ) {
        let file = file();
        let pace = rule(delayed(delay)).pace;
        let first = at(10) + Duration::from_micros(first);
        let second = first + Duration::from_micros(delay * 1_000 + around - 2_000);
        let spacing = milliseconds(delay);

        prop_assert_eq!(send_taken(&file, KEY, first, Some(pace)).unwrap(), None);
        match send_taken(&file, KEY, second, Some(pace)).unwrap() {
            None => prop_assert!(second.duration_since(first).unwrap() >= spacing),
            Some(until) => {
                prop_assert!(until > second);
                prop_assert!(until.duration_since(first).unwrap() >= spacing);
                prop_assert_eq!(send_taken(&file, KEY, until, Some(pace)).unwrap(), None);
            }
        }
    }

    /// The adaptive delay, a pure function of the wall reading, holds the
    /// key until the longer of the delay and the latency's weighted mean
    /// capped at the backoff's max, within 2 ms of the model (each step
    /// keeps whole milliseconds),
    /// and never shorter than the delay nor longer than the larger of the
    /// delay and that max.
    ///
    /// Covers: story:6-247, setting:politeness.domain.adaptive_delay
    #[test]
    fn the_adaptive_delay_follows_the_weighted_mean_within_the_delay_and_the_backoff_max(
        latencies in prop::collection::vec(0_u64..20_000, 1..12),
        delay in 0_u64..3_000,
        max in 1_u64..10_000,
    ) {
        let file = file();
        for latency in &latencies {
            latency_noted(&file, KEY, milliseconds(*latency)).unwrap();
        }
        let rule = rule(adaptive(delay, max));
        rule.started(&file, KEY, at(10)).unwrap();
        #[expect(clippy::cast_precision_loss, reason = "durations below an hour")]
        let expected = (delay as f64).max(mean(&latencies).min(max as f64));
        let held = |after: u64| rule.admit(&file, KEY, 0, at(10) + milliseconds(after)).unwrap();

        #[expect(clippy::cast_possible_truncation, clippy::cast_sign_loss, reason = "a positive mean below an hour")]
        let (below, above) = ((expected.floor() as u64).saturating_sub(2), expected.ceil() as u64 + 2);
        let shortest = held(delay.saturating_sub(1));
        prop_assert!(below == 0 || matches!(held(below), Verdict::Until(..)), "{latencies:?} {expected}");
        prop_assert_eq!(held(above), Verdict::Go, "{:?} {}", latencies, expected);
        prop_assert!(delay == 0 || matches!(shortest, Verdict::Until(..)));
        prop_assert_eq!(held(delay.max(max) + 1), Verdict::Go);
    }

    /// Admitting at arbitrary moments under a rate and a burst, each wait
    /// answered with the moment a token comes back, never lets more than
    /// the burst plus one start per interval into any span of time, and
    /// that moment always admits.
    ///
    /// Covers: story:6-248, setting:politeness.domain.rate, setting:politeness.domain.burst
    #[test]
    fn the_rate_never_lets_more_than_the_burst_and_one_start_per_interval_into_any_span(
        tries in prop::collection::vec(0_u64..3_000, 1..40),
        rate in 1_u64..600,
        burst in 1_u64..5,
    ) {
        let file = file();
        let rule = rule(rated(rate, burst));
        let interval = 60_000_u64.div_ceil(rate);
        let mut now = at(10);
        let mut starts: Vec<u64> = Vec::new();
        for wait in tries {
            now += milliseconds(wait);
            match rule.admit(&file, KEY, 0, now).unwrap() {
                Verdict::Go => {}
                Verdict::Until(until, Hold::Delay) => {
                    prop_assert!(until > now);
                    now = until;
                    prop_assert_eq!(rule.admit(&file, KEY, 0, now).unwrap(), Verdict::Go);
                }
                other => prop_assert!(false, "{other:?}"),
            }
            rule.started(&file, KEY, now).unwrap();
            starts.push(u64::try_from(now.duration_since(at(10)).unwrap().as_millis()).unwrap());
        }

        for (first, from) in starts.iter().enumerate() {
            for (last, to) in starts.iter().enumerate().skip(first) {
                let started = u64::try_from(last - first + 1).unwrap();
                prop_assert!(started <= burst + (to - from) / interval, "{starts:?}");
            }
        }
    }
}

/// A Politeness file in `env`'s folder as one engine of the user opens it,
/// with the Frontier's tables, its tasks `owner`'s.
async fn engine_file(env: &TestEnv, engine: &str, owner: &TaskOwner) -> Politeness {
    Politeness::open(
        env.folder().join("politeness.sqlite"),
        &PolitenessOptions::default(),
        vec![TABLES],
        |_, _| false,
        engine.to_owned(),
        owner,
    )
    .await
    .unwrap()
}

/// One run taking Permits of [`KEY`] under `rule` until `end`, each
/// request it starts counted at the host, `host`, and given back once
/// handled, 3 s later, before the run asks again.
async fn run_until(
    politeness: &Politeness,
    run: &str,
    rule: SharedRule,
    clock: &Clock,
    end: SystemTime,
    host: &Mutex<Vec<(String, SystemTime)>>,
) {
    let holder = Holder {
        run_id: run.to_owned(),
        epoch: 1,
        run_folder: PathBuf::from(run),
    };
    while clock.wall() < end {
        match politeness
            .take(KEY, &holder, rule.clone(), clock.wall())
            .await
            .unwrap()
        {
            Taken::Granted(permit) => {
                host.lock().unwrap().push((run.to_owned(), clock.wall()));
                clock.sleep(Duration::from_secs(3)).await;
                politeness
                    .give_back(permit, rule.clone(), Ending::Done, clock.wall())
                    .await
                    .unwrap();
            }
            Taken::Wait(Some((until, _))) => {
                clock
                    .sleep(until.duration_since(clock.wall()).unwrap_or_default())
                    .await;
            }
            Taken::Wait(None) => clock.sleep(milliseconds(10)).await,
            Taken::Refused(until) => panic!("refused until {until:?}"),
        }
    }
}

/// Two runs, each in an engine of its own on one Politeness file and each
/// asking for 30 requests a minute on one key and handling each request
/// for 3 s, start at most 30 requests there together over a minute of the
/// paused clock, never two within 2 s, both runs among them and the rate
/// nearly used up: the rate holds the key, not each run.
///
/// Covers: story:6-248, setting:politeness.domain.rate, format:politeness-file
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs on Tokio's paused clock and the test Clock (ADR 0059)"
)]
#[tokio::test(start_paused = true)]
async fn two_runs_sharing_a_key_are_held_to_its_rate_over_a_minute() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(at(0));
    let (first, second) = (
        engine_file(&env, "first", &owner).await,
        engine_file(&env, "second", &owner).await,
    );
    let thirty = SharedRule::new(rule(rated(30, 1)));
    let host = Mutex::new(Vec::new());
    let end = at(60);

    tokio::join!(
        run_until(&first, "first", thirty.clone(), &clock, end, &host),
        run_until(&second, "second", thirty.clone(), &clock, end, &host),
    );

    let host = host.into_inner().unwrap();
    let gaps_ok = host
        .windows(2)
        .all(|pair| pair[1].1.duration_since(pair[0].1).unwrap() >= Duration::from_secs(2));
    let both = ["first", "second"]
        .iter()
        .all(|run| host.iter().any(|(started, _)| started == run));
    assert!(
        (28..=30).contains(&host.len()) && gaps_ok && both,
        "{host:?}"
    );
}

/// A send that waited for its engine's Politeness file, which another
/// engine's or run's transaction held, counts its interval from the moment
/// the file recorded it, not from when the request asked: the request
/// leaves only once the record is written, so a start read before the wait
/// would let the next request reach the site sooner than the rate. (The
/// file's actor, a task here, runs the record only once the test yields,
/// after the paused clock moved 300 ms.)
///
/// Covers: story:6-248, setting:politeness.domain.rate, format:politeness-file
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs on Tokio's paused clock and the test Clock (ADR 0059)"
)]
#[tokio::test(start_paused = true)]
async fn a_send_that_waited_for_the_file_counts_its_interval_from_when_it_was_recorded() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(at(10));
    let politeness = engine_file(&env, "first", &owner).await;
    let one_a_second = Arc::new(rule(rated(60, 1)));
    let admitted = Arc::clone(&one_a_second);
    politeness
        .change(
            KEY,
            engine::PermitRule::reads(&*admitted, Step::Take),
            move |state: &mut DomainState| {
                ((), engine::PermitRule::started(&*admitted, state, at(10)))
            },
        )
        .await
        .unwrap();
    let asked = clock.wall();

    let (sent, ()) = tokio::join!(
        started_at_send(&politeness, KEY, clock, None),
        tokio::time::advance(milliseconds(300)),
    );
    sent.unwrap();
    let next = politeness
        .change(
            KEY,
            engine::PermitRule::reads(&*one_a_second, Step::Take),
            move |state: &mut DomainState| {
                (
                    engine::PermitRule::admit(&*one_a_second, state, 0, at(11) + milliseconds(100)),
                    Wrote::default(),
                )
            },
        )
        .await
        .unwrap();

    assert_eq!(
        (asked, next),
        (
            at(10),
            Verdict::Until(at(11) + milliseconds(300), Hold::Delay)
        )
    );
}

/// How the Politeness file fails a send's transaction.
#[derive(Clone, Copy, Debug)]
enum Failing {
    /// SQLite refuses the statement: the Frontier's table is gone.
    Sqlite,
    /// Its storage fails an I/O.
    Storage,
    /// Its actor stopped with its owner.
    Closed,
}

/// The last check before a send answers each failure of its transaction as
/// an error, the run file's, the storage's and a stopped file's alike, so
/// its caller holds the request back (issue 2321): none of them reads as a
/// send that may go.
///
/// Covers: story:6-247, format:politeness-file
#[rstest]
#[case::sqlite(Failing::Sqlite)]
#[case::storage(Failing::Storage)]
#[case::closed(Failing::Closed)]
fn a_send_whose_transaction_fails_is_answered_as_an_error(#[case] failing: Failing) {
    let sent = goodfetch_test_support::runtime().block_on(send_failing(failing));

    assert!(
        matches!(
            (failing, &sent),
            (Failing::Sqlite, Err(PolitenessError::Sqlite { .. }))
                | (Failing::Storage, Err(PolitenessError::Storage { .. }))
                | (Failing::Closed, Err(PolitenessError::Closed))
        ),
        "{sent:?}"
    );
}

/// What a send is answered on a Politeness file that fails as `failing`.
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs on Tokio's paused clock and the test Clock (ADR 0059)"
)]
async fn send_failing(failing: Failing) -> Result<Option<SystemTime>, PolitenessError> {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(at(10));
    let vfs = CrashVfs::register(&env.unique_name("send-fails"), env.seed()).unwrap();
    let path = env.folder().join("politeness.sqlite");
    let politeness = Politeness::open(
        path.clone(),
        &PolitenessOptions {
            vfs: matches!(failing, Failing::Storage).then(|| vfs.name().to_owned()),
            ..PolitenessOptions::default()
        },
        vec![TABLES],
        |_, _| false,
        "first".to_owned(),
        &owner,
    )
    .await
    .unwrap();
    match failing {
        Failing::Sqlite => rusqlite::Connection::open(&path)
            .unwrap()
            .execute_batch("DROP TABLE domain_limits")
            .unwrap(),
        Failing::Storage => vfs.fail_at(vfs.operations(), IoFault::Error),
        Failing::Closed => owner.stop().await,
    }

    started_at_send(&politeness, KEY, clock, None).await
}

/// A file that fails one send's transaction records nothing of it, and
/// answers the same send asked again: the send then goes, and the next one
/// waits out the delay counted from it (issue 2321).
///
/// Covers: story:6-247, setting:politeness.domain.delay, format:politeness-file
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs on Tokio's paused clock and the test Clock (ADR 0059)"
)]
#[tokio::test(start_paused = true)]
async fn a_send_asked_again_once_the_file_answers_goes_and_its_delay_is_kept() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(at(10));
    let vfs = CrashVfs::register(&env.unique_name("send-fails-once"), env.seed()).unwrap();
    let politeness = Politeness::open(
        env.folder().join("politeness.sqlite"),
        &PolitenessOptions {
            vfs: Some(vfs.name().to_owned()),
            ..PolitenessOptions::default()
        },
        vec![TABLES],
        |_, _| false,
        "first".to_owned(),
        &owner,
    )
    .await
    .unwrap();
    let a_second = Some(pace(
        json!({ "politeness": { "domain": { "delay": 1_000 } } }),
    ));
    started_at_send(&politeness, KEY, clock, a_second)
        .await
        .unwrap();
    tokio::time::advance(milliseconds(1_500)).await;

    vfs.fail_at(vfs.operations(), IoFault::Error);
    let failed = started_at_send(&politeness, KEY, clock, a_second).await;
    let again = started_at_send(&politeness, KEY, clock, a_second).await;
    tokio::time::advance(milliseconds(100)).await;
    let next = started_at_send(&politeness, KEY, clock, a_second).await;

    assert!(
        matches!(failed, Err(PolitenessError::Storage { .. })),
        "{failed:?}"
    );
    assert_eq!(
        (again.unwrap(), next.unwrap()),
        (None, Some(at(12) + milliseconds(500)))
    );
}

/// A crawl request's own send on a Politeness held in memory (#2171), each
/// in milliseconds after 10 s.
#[derive(Clone, Copy, Debug)]
enum OwnSend {
    /// Granted at the first, its send reserved for the second.
    Reserved(u64, u64),
    /// The send reserved for the first went at the second.
    Settled(u64, u64),
}

/// A grant on a Politeness held in memory reserves its send for the moment
/// its delay after the key's last send and its rate's token allow, taken
/// as if it went then; the true send, settled after the wait, keeps the
/// later of the two, so the next reservation counts from a send that came
/// late and never from an earlier one settled after it (#2171).
///
/// Covers: story:6-247, setting:politeness.domain.delay, setting:politeness.sharing
#[rstest]
#[case::the_delay(delayed(100), &[
    OwnSend::Reserved(0, 0),
    OwnSend::Reserved(28, 100),
    OwnSend::Settled(100, 130),
    OwnSend::Reserved(150, 230),
    OwnSend::Settled(0, 5),
    OwnSend::Reserved(240, 330),
])]
/// Covers: story:6-248, setting:politeness.domain.rate
#[case::the_rate(rated(60, 1), &[
    OwnSend::Reserved(0, 0),
    OwnSend::Reserved(200, 1_000),
    OwnSend::Settled(1_000, 1_300),
    OwnSend::Reserved(1_500, 2_300),
])]
fn a_reserved_send_goes_when_its_pace_allows_and_a_late_one_holds_the_next(
    #[case] run: Value,
    #[case] sends: &[OwnSend],
) {
    let file = file();
    let pace = rule(run).pace;
    let ms = |after: u64| at(10) + milliseconds(after);

    let reserved: Vec<(u64, SystemTime)> = sends
        .iter()
        .filter_map(|send| match *send {
            OwnSend::Reserved(granted, goes) => {
                let at = send_reserved(&file, KEY, ms(granted), pace).unwrap();
                Some((goes, at))
            }
            OwnSend::Settled(reserved, sent) => {
                send_settled(&file, KEY, ms(sent), ms(reserved)).unwrap();
                None
            }
        })
        .collect();

    let expected: Vec<(u64, SystemTime)> = reserved
        .iter()
        .map(|(goes, _)| (*goes, ms(*goes)))
        .collect();
    assert_eq!(reserved, expected);
}

/// Reserves in `file` the send of a request of `key` granted at `now` to a
/// run of `pace` ([`DomainState::send_reserved`]).
fn send_reserved(
    file: &rusqlite::Connection,
    key: &str,
    now: SystemTime,
    pace: Pace,
) -> rusqlite::Result<SystemTime> {
    changed(file, key, Parts::ALL, |state: &mut DomainState| {
        state.send_reserved(now, pace)
    })
}

/// Settles in `file` the send of `key` reserved for `reserved` and sent at
/// `now` ([`DomainState::send_settled`]).
fn send_settled(
    file: &rusqlite::Connection,
    key: &str,
    now: SystemTime,
    reserved: SystemTime,
) -> rusqlite::Result<()> {
    changed(file, key, Parts::PACED, |state: &mut DomainState| {
        ((), state.send_settled(now, reserved))
    })
}

/// A Run's own Politeness, held in memory, reserves a crawl request's send
/// at its grant, and the send settled later than that holds the next
/// grant's reservation the delay after it, without the send waiting on the
/// store (#2171); a take beside a sent request, such as a hedge's second
/// copy on another key, reserves nothing.
///
/// Covers: story:6-247, setting:politeness.domain.delay, setting:politeness.sharing
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs on Tokio's paused clock and the test Clock (ADR 0059)"
)]
#[tokio::test(start_paused = true)]
async fn a_runs_own_politeness_reserves_a_send_at_its_grant_and_a_late_send_holds_the_next() {
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(at(10));
    let politeness = Politeness::in_memory(&PolitenessOptions::default(), &owner).unwrap();
    let delayed: DomainRule =
        rule(json!({ "politeness": { "domain": { "delay": 100, "concurrency": 4 } } }));
    let delayed = SharedRule::new(delayed);
    let holder = Holder {
        run_id: "own".to_owned(),
        epoch: 1,
        run_folder: PathBuf::from("own"),
    };
    let granted = |taken: Taken| match taken {
        Taken::Granted(permit) => permit,
        other => panic!("not granted: {other:?}"),
    };

    let first = granted(
        politeness
            .take(KEY, &holder, delayed.clone(), at(10))
            .await
            .unwrap(),
    );
    let beside = granted(
        politeness
            .take_beside("beside.test", &holder, delayed.clone(), at(10))
            .await
            .unwrap(),
    );
    tokio::time::advance(milliseconds(30)).await;
    settled_at_send(&politeness, KEY, clock, first.send_at().unwrap())
        .await
        .unwrap();
    let held = politeness
        .take(KEY, &holder, delayed.clone(), at(10) + milliseconds(100))
        .await
        .unwrap();
    let second = granted(
        politeness
            .take(KEY, &holder, delayed, at(10) + milliseconds(130))
            .await
            .unwrap(),
    );

    assert_eq!(
        (first.send_at(), beside.send_at(), second.send_at()),
        (Some(at(10)), None, Some(at(10) + milliseconds(130)))
    );
    assert!(
        matches!(held, Taken::Wait(Some((until, Hold::Delay))) if until == at(10) + milliseconds(130)),
        "{held:?}"
    );
}

/// What a page sends on a Run's own Politeness.
#[derive(Clone, Copy, Debug)]
enum PageSend {
    /// The crawl request's own send, at the moment its grant reserved.
    Settled,
    /// A redirect hop's, which waits for nothing.
    Hop,
}

/// A send on a Run's own Politeness held in memory goes without waiting
/// for the task that holds it, which may run its note only later, and
/// that note keeps the instant the page sent, not the one it ran: the
/// next grant on the key is held the delay after the send (#2171).
///
/// Covers: story:6-247, setting:politeness.domain.delay, setting:politeness.sharing
#[expect(
    clippy::disallowed_methods,
    reason = "the test runs on Tokio's paused clock and the test Clock (ADR 0059)"
)]
#[rstest]
#[case::the_requests_own(PageSend::Settled)]
#[case::a_hop(PageSend::Hop)]
#[tokio::test(start_paused = true)]
async fn a_send_on_a_runs_own_politeness_waits_for_no_one_and_keeps_its_own_instant(
    #[case] send: PageSend,
) {
    let clock = Clock::paused(at(10));
    let (politeness, mut own) = Politeness::own(&PolitenessOptions::default());
    let delayed: DomainRule =
        rule(json!({ "politeness": { "domain": { "delay": 100, "concurrency": 4 } } }));
    let delayed = SharedRule::new(delayed);
    let holder = Holder {
        run_id: "own".to_owned(),
        epoch: 1,
        run_folder: PathBuf::from("own"),
    };
    let Taken::Granted(first) = own.take(KEY, &holder, &delayed, at(10)) else {
        panic!("the first take is not granted");
    };

    tokio::time::advance(milliseconds(30)).await;
    let sent = async {
        match send {
            PageSend::Settled => {
                settled_at_send(&politeness, KEY, clock, first.send_at().unwrap()).await
            }
            PageSend::Hop => started_at_send(&politeness, KEY, clock, None)
                .await
                .map(drop),
        }
    };
    let sent = tokio::time::timeout(Duration::from_secs(1), sent).await;
    tokio::time::advance(milliseconds(50)).await;
    let held = own.take(KEY, &holder, &delayed, at(10) + milliseconds(100));

    assert!(matches!(sent, Ok(Ok(()))), "{sent:?}");
    assert!(
        matches!(held, Taken::Wait(Some((until, Hold::Delay))) if until == at(10) + milliseconds(130)),
        "{held:?}"
    );
}
