//! The shared robots.txt copy (ADR 0086): its cache rule as pure functions
//! of the copy, the answer and the `Clock`'s reading, `proptest` driving it
//! over sequences of answers and waits; and the copy itself on a real
//! Politeness file in the test's folder, two engines' handles sharing it on
//! Tokio's paused clock.
#![expect(
    clippy::disallowed_methods,
    reason = "tests run on the test Clock and Tokio's paused clock (ADR 0059); its SQL statements are its own, on no Shared path (ADR 0140)"
)]
#![expect(
    clippy::disallowed_types,
    reason = "a test's fetch counts its calls in an atomic; no lock order is tested here"
)]

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, UNIX_EPOCH};

use ::url::Url;
use goodfetch_counted_work::Row;
use goodfetch_engine::clock::Clock;
use goodfetch_engine::politeness::{Politeness, PolitenessOptions};
use goodfetch_engine::robots_txt::{Answer, Copies, Fetched, MAX_SIZE, parse};
use goodfetch_engine::tasks::{BlockingPolicy, TaskOwner};
use goodfetch_test_support::TestEnv;
use proptest::prelude::*;
use rstest::rstest;

use super::{Copy, FRESH_FOR, KEPT_BYTES, SharedRobotsTxt, TABLES, applies, fresh, kept};

const DAY: i64 = 86_400_000;
const FILE: &[u8] = b"User-agent: *\nDisallow: /private/\n";

fn response(status: u16, body: &[u8]) -> Answer {
    Answer::Response {
        status,
        body: body.to_vec(),
    }
}

fn unreachable() -> Answer {
    Answer::Unreachable("connection refused".to_owned())
}

fn copy(fetched_at: i64, status: u16, body: &[u8]) -> Copy {
    Copy {
        fetched_at,
        status,
        body: body.to_vec(),
    }
}

/// Covers: story:6-241
#[rstest]
#[case::just_fetched(0, true)]
#[case::an_hour_later(3_600_000, true)]
#[case::a_moment_before_a_day(DAY - 1, true)]
#[case::a_day_later(DAY, false)]
#[case::a_week_later(7 * DAY, false)]
#[case::a_clock_set_back(-1, false)]
fn a_copy_is_used_for_less_than_a_day_after_its_fetch(#[case] since: i64, #[case] used: bool) {
    let fetched_at = 1_767_603_600_000;

    assert_eq!(
        fresh(Some(&copy(fetched_at, 200, FILE)), fetched_at + since),
        used
    );
    assert_eq!(
        i64::try_from(FRESH_FOR.as_millis()).unwrap(),
        DAY,
        "a copy lasts a day"
    );
}

/// Covers: story:6-241
#[test]
fn no_copy_is_never_fresh() {
    assert!(!fresh(None, 0));
}

/// Covers: story:6-240
#[rstest]
#[case::a_file(
    response(200, FILE),
    Some(copy(5, 200, FILE)),
    Fetched::File(parse(FILE))
)]
#[case::a_404_is_no_rules(
    response(404, b"gone"),
    Some(copy(5, 404, b"gone")),
    Fetched::NotFound(404)
)]
#[case::a_410_is_no_rules(response(410, b""), Some(copy(5, 410, b"")), Fetched::NotFound(410))]
#[case::a_429_keeps_nothing(response(429, b""), None, Fetched::Unreachable("answered 429".to_owned()))]
#[case::a_500_keeps_nothing(response(500, b""), None, Fetched::Unreachable("answered 500".to_owned()))]
#[case::a_503_keeps_nothing(response(503, b""), None, Fetched::Unreachable("answered 503".to_owned()))]
#[case::no_answer_keeps_nothing(unreachable(), None, Fetched::Unreachable("connection refused".to_owned()))]
fn with_no_copy_an_answer_is_kept_only_when_it_applies_as_it_is(
    #[case] answer: Answer,
    #[case] copy: Option<Copy>,
    #[case] applied: Fetched,
) {
    assert_eq!(
        (kept(None, &answer, 5), applies(None, &answer)),
        (copy, applied)
    );
}

/// Covers: story:6-240
#[rstest]
#[case::a_429(response(429, b""))]
#[case::a_500(response(500, b"oops"))]
#[case::a_503(response(503, b""))]
#[case::no_answer(unreachable())]
fn a_failed_fetch_leaves_the_last_good_copy_applying_however_old(#[case] answer: Answer) {
    let last = copy(0, 200, FILE);

    assert_eq!(
        (
            kept(Some(last.clone()), &answer, 30 * DAY),
            applies(Some(&last), &answer)
        ),
        (Some(last), Fetched::File(parse(FILE)))
    );
}

/// Covers: story:6-240
#[test]
fn a_good_answer_replaces_an_old_copy() {
    let last = copy(0, 200, FILE);

    assert_eq!(
        (
            kept(Some(last.clone()), &response(404, b""), DAY),
            applies(Some(&last), &response(404, b""))
        ),
        (Some(copy(DAY, 404, b"")), Fetched::NotFound(404))
    );
}

/// What a fetch of the property test answers.
fn answers() -> impl Strategy<Value = Answer> {
    prop_oneof![
        Just(response(200, FILE)),
        Just(response(200, b"User-agent: *\nAllow: /\n")),
        Just(response(404, b"")),
        Just(response(403, b"")),
        Just(response(429, b"")),
        Just(response(500, b"")),
        Just(response(503, b"")),
        Just(unreachable()),
    ]
}

proptest! {
    /// Over any sequence of waits and answers, read as a crawl reads the
    /// copy: a fetch goes only when no answer that applies as it is came
    /// in the last day; a failed fetch never applies as no rules unless
    /// the last good answer was one, applying that last good answer or,
    /// with none, disallowing everything; and the same steps give the same
    /// readings.
    ///
    /// Covers: story:6-240, story:6-241
    #[test]
    fn the_cache_rule_holds_over_any_sequence_of_answers(
        steps in prop::collection::vec((0..(2 * DAY), answers()), 1..40),
    ) {
        let read = |steps: &[(i64, Answer)]| {
            let mut now = 1_767_603_600_000_i64;
            let mut copy: Option<Copy> = None;
            let mut good: Option<(i64, Answer)> = None;
            let mut readings = Vec::new();
            for (wait, answer) in steps {
                now += wait;
                let expected_fresh = good
                    .as_ref()
                    .is_some_and(|(at, _)| now - at < DAY);
                prop_assert_eq!(fresh(copy.as_ref(), now), expected_fresh);
                if expected_fresh {
                    readings.push(copy.as_ref().map(Copy::applies));
                    continue;
                }
                let applied = applies(copy.as_ref(), answer);
                let expected = if answer.usable() {
                    good = Some((now, answer.clone()));
                    answer.fetched()
                } else {
                    good.as_ref().map_or_else(
                        || answer.fetched(),
                        |(_, last)| last.fetched(),
                    )
                };
                prop_assert_eq!(&applied, &expected);
                if !answer.usable() {
                    let no_rules = matches!(applied, Fetched::NotFound(_));
                    let last_no_rules = good
                        .as_ref()
                        .is_some_and(|(_, last)| matches!(last.fetched(), Fetched::NotFound(_)));
                    prop_assert!(!no_rules || last_no_rules);
                }
                copy = kept(copy, answer, now);
                readings.push(Some(applied));
            }
            Ok(readings)
        };
        prop_assert_eq!(read(&steps)?, read(&steps)?);
    }
}

/// A Politeness file in `env`'s folder as one engine of the user opens it,
/// with the copies' table, its tasks `owner`'s.
async fn file(env: &TestEnv, engine: &str, owner: &TaskOwner) -> Politeness {
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

/// A fetch that counts itself and answers `answer`.
fn counted(
    fetches: &Arc<AtomicUsize>,
    answer: Answer,
) -> goodfetch_engine::robots_txt::Fetch<'static> {
    let fetches = Arc::clone(fetches);
    Box::pin(async move {
        fetches.fetch_add(1, Ordering::SeqCst);
        answer
    })
}

fn location() -> Url {
    Url::parse("https://shop.test/robots.txt").unwrap()
}

/// Two engines' runs share one copy: the second, within the day, sends no
/// fetch; a day later one goes; and a fetch then answered 500 leaves the
/// last good copy applying.
///
/// Covers: story:6-241, story:6-240, format:politeness-file
#[tokio::test(start_paused = true)]
async fn two_engines_share_one_copy_for_a_day() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let first = SharedRobotsTxt::new(file(&env, "first", &owner).await, clock);
    let second = SharedRobotsTxt::new(file(&env, "second", &owner).await, clock);
    let fetches = Arc::new(AtomicUsize::new(0));

    let read_first = first
        .copy(&location(), counted(&fetches, response(200, FILE)))
        .await;
    tokio::time::advance(Duration::from_hours(23)).await;
    let shared = second
        .copy(&location(), counted(&fetches, response(200, b"")))
        .await;
    let within_the_day = fetches.load(Ordering::SeqCst);
    tokio::time::advance(Duration::from_hours(1)).await;
    let failed = second
        .copy(&location(), counted(&fetches, response(500, b"")))
        .await;
    let after_the_day = fetches.load(Ordering::SeqCst);

    assert_eq!(
        (
            (*read_first).clone(),
            (*shared).clone(),
            within_the_day,
            (*failed).clone(),
            after_the_day,
        ),
        (
            Fetched::File(parse(FILE)),
            Fetched::File(parse(FILE)),
            1,
            Fetched::File(parse(FILE)),
            2,
        )
    );
}

/// A 404 is kept as no rules, so the next run sends no fetch; a 429 with
/// no copy disallows everything and is not kept, so the next reading
/// fetches again.
///
/// Covers: story:6-240
#[tokio::test(start_paused = true)]
async fn a_404_is_kept_and_a_429_is_not() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let copies = SharedRobotsTxt::new(file(&env, "engine", &owner).await, clock);
    let other = Url::parse("https://other.test/robots.txt").unwrap();
    let fetches = Arc::new(AtomicUsize::new(0));

    let not_found = copies
        .copy(&location(), counted(&fetches, response(404, b"")))
        .await;
    let again = copies
        .copy(&location(), counted(&fetches, response(200, FILE)))
        .await;
    let refused = copies
        .copy(&other, counted(&fetches, response(429, b"")))
        .await;
    let retried = copies
        .copy(&other, counted(&fetches, response(200, FILE)))
        .await;

    assert_eq!(
        (
            (*not_found).clone(),
            (*again).clone(),
            (*refused).clone(),
            (*retried).clone(),
            fetches.load(Ordering::SeqCst),
        ),
        (
            Fetched::NotFound(404),
            Fetched::NotFound(404),
            Fetched::Unreachable("answered 429".to_owned()),
            Fetched::File(parse(FILE)),
            3,
        )
    );
}

/// Readers of one origin in one engine wait for one fetch.
///
/// Covers: story:6-241
#[tokio::test(start_paused = true)]
async fn concurrent_readers_wait_for_one_fetch() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let copies = SharedRobotsTxt::new(file(&env, "engine", &owner).await, clock);
    let fetches = Arc::new(AtomicUsize::new(0));

    let location = location();
    let (one, two) = tokio::join!(
        copies.copy(&location, counted(&fetches, response(200, FILE))),
        copies.copy(&location, counted(&fetches, response(200, FILE))),
    );

    assert_eq!(
        (
            (*one).clone(),
            (*two).clone(),
            fetches.load(Ordering::SeqCst)
        ),
        (Fetched::File(parse(FILE)), Fetched::File(parse(FILE)), 1)
    );
}

/// Each refusal case of the Politeness file's format (ADR 0066), on a
/// file holding a copy: a version this engine reads keeps the copy in
/// use, with no fetch; one it cannot read refuses the file.
///
/// Covers: story:6-241, format:politeness-file
#[rstest]
#[tokio::test(start_paused = true)]
async fn the_copy_follows_the_politeness_files_refusal_cases(
    #[files("../../goodfetch-protocol/generated/refusals/politeness-file/*.json")]
    #[mode = str]
    case: &str,
) {
    let case: goodfetch_protocol::refusals::RefusalCase = serde_json::from_str(case).unwrap();
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let fetches = Arc::new(AtomicUsize::new(0));
    SharedRobotsTxt::new(file(&env, "writer", &owner).await, clock)
        .copy(&location(), counted(&fetches, response(200, FILE)))
        .await;
    rusqlite::Connection::open(env.folder().join("politeness.sqlite"))
        .unwrap()
        .execute("UPDATE format SET version = ?1", [case.found.to_string()])
        .unwrap();

    let reopened = Politeness::open(
        env.folder().join("politeness.sqlite"),
        &PolitenessOptions::default(),
        vec![TABLES],
        |_, _| false,
        "reader".to_owned(),
        &owner,
    )
    .await;

    match (case.compatibility, reopened) {
        (goodfetch_protocol::formats::Compatibility::Compatible, Ok(file)) => {
            let read = SharedRobotsTxt::new(file, clock)
                .copy(&location(), counted(&fetches, response(500, b"")))
                .await;
            assert_eq!(
                ((*read).clone(), fetches.load(Ordering::SeqCst)),
                (Fetched::File(parse(FILE)), 1),
                "{case:#?}"
            );
        }
        (
            compatibility,
            Err(goodfetch_engine::politeness::PolitenessError::Format {
                found,
                compatibility: refused,
            }),
        ) => assert_eq!(
            (found, refused),
            (case.found.to_string(), compatibility),
            "{case:#?}"
        ),
        (_, opened) => panic!("{case:#?} opened as {:?}", opened.map(|_| ())),
    }
}

/// How many origins the copy holds a reading in flight for.
fn origins_held(copies: &SharedRobotsTxt) -> usize {
    copies.in_flight.lock().unwrap().len()
}

/// How many parses `copies` keeps, and what they weigh, once moka has
/// applied its bound.
fn parses_kept(copies: &SharedRobotsTxt) -> (u64, u64) {
    copies.parses.run_pending_tasks();
    (copies.parses.entry_count(), copies.parses.weighted_size())
}

/// The robots.txt reads and parses `copies` counted (ADR 0140's E6).
fn reads_and_parses(copies: &SharedRobotsTxt) -> (u64, u64) {
    let work = copies.politeness.work();
    (work.read(Row::RobotsReads), work.read(Row::RobotsParses))
}

/// The copy holds an origin's reading in flight only while a reader of it
/// is in flight, so readings in flight do not grow with a crawl's origins
/// (STYLE.md, Memory and hot paths).
///
/// Covers: story:6-241
#[tokio::test(start_paused = true)]
async fn an_origin_is_in_flight_only_while_it_is_read() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let copies = SharedRobotsTxt::new(file(&env, "engine", &owner).await, clock);
    let fetches = Arc::new(AtomicUsize::new(0));

    for origin in ["https://one.test", "https://two.test", "https://three.test"] {
        let location = Url::parse(&format!("{origin}/robots.txt")).unwrap();
        copies
            .copy(&location, counted(&fetches, response(200, FILE)))
            .await;
    }

    assert_eq!(
        (origins_held(&copies), fetches.load(Ordering::SeqCst)),
        (0, 3)
    );
}

/// A Run reads and parses each origin's copy once while it is fresh: its
/// later readers read neither the file nor the robots.txt again, and
/// another Run of the file reads and parses it once of its own.
///
/// Covers: story:6-241
#[tokio::test(start_paused = true)]
async fn a_run_reads_and_parses_a_fresh_copy_once() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let run = SharedRobotsTxt::new(file(&env, "engine", &owner).await, clock);
    let other_run = SharedRobotsTxt::new(file(&env, "other", &owner).await, clock);
    let fetches = Arc::new(AtomicUsize::new(0));

    let mut read = Vec::new();
    for _ in 0..3 {
        read.push(
            (*run
                .copy(&location(), counted(&fetches, response(200, FILE)))
                .await)
                .clone(),
        );
        tokio::time::advance(Duration::from_hours(1)).await;
    }
    let other = other_run
        .copy(&location(), counted(&fetches, response(200, FILE)))
        .await;

    assert_eq!(
        (
            read,
            (*other).clone(),
            fetches.load(Ordering::SeqCst),
            reads_and_parses(&run),
            reads_and_parses(&other_run),
        ),
        (
            vec![Fetched::File(parse(FILE)); 3],
            Fetched::File(parse(FILE)),
            1,
            (1, 1),
            (1, 1),
        )
    );
}

/// A reader answered from the Run's kept parse lets the work beside it run
/// before it answers, as the file read it replaces did, so a crawl whose
/// every origin is kept still lets the Connection that answers the SDK's
/// calls run page by page rather than once the crawl idles.
#[tokio::test(start_paused = true)]
async fn a_kept_parse_lets_other_tasks_run_before_it_answers() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let run = SharedRobotsTxt::new(file(&env, "engine", &owner).await, clock);
    let fetches = Arc::new(AtomicUsize::new(0));
    let location = location();
    run.copy(&location, counted(&fetches, response(200, FILE)))
        .await;
    let beside_ran = Cell::new(false);

    let ((ran_before_the_answer, answered), ()) = tokio::join!(
        async {
            let answered = run
                .copy(&location, counted(&fetches, response(200, FILE)))
                .await;
            (beside_ran.get(), answered)
        },
        async { beside_ran.set(true) },
    );

    assert_eq!(
        (
            ran_before_the_answer,
            (*answered).clone(),
            reads_and_parses(&run)
        ),
        (true, Fetched::File(parse(FILE)), (1, 1))
    );
}

/// A Run's parse of a copy lasts as long as the copy is fresh: a day after
/// its fetch the next reader reads the file again, and a fetch then
/// answered 500 applies the last good copy without keeping it, so the
/// reader after it reads and fetches again, as a Run did with no parse kept.
///
/// Covers: story:6-240, story:6-241
#[tokio::test(start_paused = true)]
async fn a_runs_parse_lasts_as_long_as_its_copy_is_fresh() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let run = SharedRobotsTxt::new(file(&env, "engine", &owner).await, clock);
    let fetches = Arc::new(AtomicUsize::new(0));
    let newer: &[u8] = b"User-agent: *\nDisallow: /\n";

    run.copy(&location(), counted(&fetches, response(200, FILE)))
        .await;
    tokio::time::advance(Duration::from_hours(24)).await;
    let failed = run
        .copy(&location(), counted(&fetches, response(500, b"")))
        .await;
    let after_the_failure = run
        .copy(&location(), counted(&fetches, response(200, newer)))
        .await;
    let kept = run
        .copy(&location(), counted(&fetches, response(200, FILE)))
        .await;

    assert_eq!(
        (
            (*failed).clone(),
            (*after_the_failure).clone(),
            (*kept).clone(),
            fetches.load(Ordering::SeqCst),
            reads_and_parses(&run),
        ),
        (
            Fetched::File(parse(FILE)),
            Fetched::File(parse(newer)),
            Fetched::File(parse(newer)),
            3,
            (3, 3),
        )
    );
}

/// The parses a Run keeps stay within [`KEPT_BYTES`], however many origins
/// it reads, each with a file of the most bytes read.
///
/// Covers: story:6-241
#[tokio::test(start_paused = true)]
async fn the_parses_a_run_keeps_stay_within_their_bound() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let run = SharedRobotsTxt::new(file(&env, "engine", &owner).await, clock);
    let fetches = Arc::new(AtomicUsize::new(0));
    let largest = vec![b'#'; MAX_SIZE];
    let origins = KEPT_BYTES / u64::try_from(MAX_SIZE).unwrap() + 8;

    for origin in 0..origins {
        let location = Url::parse(&format!("https://{origin}.test/robots.txt")).unwrap();
        run.copy(&location, counted(&fetches, response(200, &largest)))
            .await;
    }
    let (parses, bytes) = parses_kept(&run);

    assert!(
        (1..origins).contains(&parses) && bytes <= KEPT_BYTES,
        "{parses} parses of {origins} origins kept, {bytes} bytes over {KEPT_BYTES}"
    );
}

/// A reader dropped while its fetch is in flight keeps nothing: the next
/// reader of the origin fetches it, and no state is left behind.
///
/// Covers: story:6-241
#[tokio::test(start_paused = true)]
async fn a_reader_dropped_during_its_fetch_leaves_the_next_one_to_fetch() {
    let env = TestEnv::new();
    let owner = TaskOwner::root(BlockingPolicy::Inline);
    let clock = Clock::paused(UNIX_EPOCH + Duration::from_hours(491_001));
    let copies = SharedRobotsTxt::new(file(&env, "engine", &owner).await, clock);
    let fetches = Arc::new(AtomicUsize::new(0));
    let never: goodfetch_engine::robots_txt::Fetch<'static> = Box::pin(std::future::pending());

    let dropped =
        tokio::time::timeout(Duration::from_secs(1), copies.copy(&location(), never)).await;
    let held_after_the_drop = origins_held(&copies);
    let read = copies
        .copy(&location(), counted(&fetches, response(200, FILE)))
        .await;

    assert_eq!(
        (
            dropped.is_err(),
            held_after_the_drop,
            (*read).clone(),
            fetches.load(Ordering::SeqCst),
            origins_held(&copies),
        ),
        (true, 0, Fetched::File(parse(FILE)), 1, 0)
    );
}
