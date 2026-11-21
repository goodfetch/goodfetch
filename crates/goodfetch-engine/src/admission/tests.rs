//! A Run's rate cap through its interface (spec #213's Seam 4): a pure
//! function of the monotonic readings the tests give it; and the room a
//! second copy of an admitted request takes, from an in-memory Politeness
//! on the paused test runtime; and a key its Run refuses until the time its
//! site asked for.
#![expect(
    clippy::disallowed_methods,
    reason = "a pure rule's readings may start from any moment (ADR 0059)"
)]
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]

use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

use proptest::prelude::*;
use rstest::rstest;
use rusqlite::Connection;
use tokio::time::Instant;

use goodfetch_counted_work::{CountedWork, Row};
use goodfetch_protocol::catalog::Product;

use super::{
    Admission, Admitted, BesideShared, Ended, Gate, Held, Journal, Keys, RateCap, Room,
    SecondCopies, Waiting,
};
use crate::clock::Clock;
use crate::politeness::{
    Ending, Hold, Holder, KeyState, MemoryTables, PermitRule, Politeness, PolitenessOptions,
    SharedRule, Step, Taken, Verdict,
};
use crate::settings::{Layers, Resolved, resolve};
use crate::tasks::{BlockingPolicy, TaskOwner};

/// Offers `cap` a request at each of `offsets` from `start`, in order, and
/// answers the offsets it admitted.
fn admitted(cap: &mut RateCap, start: Instant, offsets: &[Duration]) -> Vec<Duration> {
    offsets
        .iter()
        .filter(|offset| cap.admit(start + **offset).is_ok())
        .copied()
        .collect()
}

/// Covers: story:3-51
#[test]
fn sixty_a_minute_admit_one_request_a_second() {
    let start = Instant::now();
    let mut cap = RateCap::per_minute(60);
    let offsets: Vec<Duration> = (0..=6)
        .map(|tenth| Duration::from_millis(tenth * 500))
        .collect();

    let admitted = admitted(&mut cap, start, &offsets);

    assert_eq!(
        admitted,
        [0, 1_000, 2_000, 3_000].map(Duration::from_millis)
    );
}

/// Covers: story:3-51
#[test]
fn a_refused_request_learns_when_the_cap_admits_again() {
    let start = Instant::now();
    let mut cap = RateCap::per_minute(4);
    cap.admit(start).unwrap();

    let refused = cap.admit(start + Duration::from_secs(1));

    assert_eq!(refused, Err(Some(start + Duration::from_secs(15))));
}

/// Covers: story:3-51
#[test]
fn a_cap_of_zero_admits_nothing() {
    let start = Instant::now();
    let mut cap = RateCap::per_minute(0);

    assert_eq!((cap.admit(start), cap.ready_at(start)), (Err(None), None));
}

proptest! {
    /// However requests arrive, no minute admits more than the cap, and a
    /// request offered once the cap is ready is admitted.
    ///
    /// Covers: story:3-51
    #[test]
    fn no_minute_admits_more_than_the_cap(
        per_minute in 1_u64..=600,
        gaps in proptest::collection::vec(0_u64..=2_000, 1..400),
    ) {
        let start = Instant::now();
        let mut cap = RateCap::per_minute(per_minute);
        let mut at = start;
        let mut admitted = Vec::new();
        for gap in gaps {
            at += Duration::from_millis(gap);
            let ready = cap.ready_at(at).unwrap();
            let outcome = cap.admit(at);
            prop_assert_eq!(outcome.is_ok(), ready <= at);
            if outcome.is_ok() {
                admitted.push(at);
            }
        }
        let limit = usize::try_from(per_minute).unwrap();
        for (index, first) in admitted.iter().enumerate() {
            let in_minute = admitted[index..]
                .iter()
                .take_while(|moment| **moment < *first + Duration::from_mins(1))
                .count();
            prop_assert!(in_minute <= limit, "{in_minute} admitted in a minute under a cap of {per_minute}");
        }
    }
}

/// What a key admits in the second-copy tests: up to `cap` requests in
/// flight, each start then held for `delay` of wall time.
struct Rule {
    cap: u64,
    delay: Option<Duration>,
}

/// The state of a test rule that keeps none.
struct Stateless;

impl KeyState for Stateless {
    type Key = str;
    type Reads = ();
    type Writes = ();

    fn load(_: &Connection, _: &str, (): ()) -> rusqlite::Result<Self> {
        Ok(Self)
    }

    fn save(&self, _: &Connection, _: &str, (): &()) -> rusqlite::Result<()> {
        Ok(())
    }

    fn load_from(_: &MemoryTables, _: &str, (): ()) -> Self {
        Self
    }

    fn save_to(&self, _: &mut MemoryTables, _: &str, (): &()) {}
}

impl PermitRule for Rule {
    type State = Stateless;

    fn reads(&self, _: Step) {}

    fn concurrency(&self) -> Option<u64> {
        Some(self.cap)
    }

    fn admit(&self, _: &Stateless, in_flight: u64, now: SystemTime) -> Verdict {
        match self.delay {
            _ if in_flight >= self.cap => Verdict::Busy,
            Some(delay) => Verdict::Until(now + delay, Hold::Delay),
            None => Verdict::Go,
        }
    }

    fn started(&self, _: &mut Stateless, _: SystemTime) {}

    fn ended(
        &self,
        _: &mut Stateless,
        _: u64,
        _: Ending,
        _: SystemTime,
    ) -> (Option<SystemTime>, ()) {
        (None, ())
    }
}

/// What a second copy's take answered, by name: `taken`, `full`, or the
/// time from `now` it holds the copy until.
fn named(room: &Room, now: Instant) -> String {
    match room {
        Room::Taken(_) => "taken".to_owned(),
        Room::Full => "full".to_owned(),
        Room::Until(until) => format!("until +{:?}", until.duration_since(now)),
    }
}

/// Second copies of a Run sharing an in-memory Politeness under `rule`,
/// whose first request of `shop.test` already holds its Permit, taken
/// before the delay began, under the rate cap `rate`.
async fn beside_a_first(owner: &TaskOwner, rule: Rule, rate: Option<RateCap>) -> SecondCopies {
    let politeness = Politeness::in_memory(&PolitenessOptions::default(), owner).unwrap();
    let holder = Holder {
        run_id: "crawl".to_owned(),
        epoch: 1,
        run_folder: "crawl".into(),
        delegates: true,
    };
    let clock = Clock::paused(SystemTime::UNIX_EPOCH + Duration::from_hours(500_000));
    let before_the_delay = SharedRule::new(Rule {
        cap: rule.cap,
        delay: None,
    });
    let first = politeness
        .take("shop.test", &holder, before_the_delay, clock.wall())
        .await
        .unwrap();
    assert!(matches!(first, Taken::Granted(_)), "{first:?}");
    SecondCopies {
        shared: Some(BesideShared {
            politeness,
            holder,
            rule: SharedRule::new(rule),
            clock,
        }),
        rate: Arc::new(Mutex::new(rate)),
    }
}

fn on_the_runtime<T>(test: impl AsyncFnOnce(&TaskOwner) -> T) -> T {
    goodfetch_test_support::runtime().block_on(async {
        let owner = TaskOwner::root(BlockingPolicy::Inline);
        let answer = test(&owner).await;
        owner.cancel();
        answer
    })
}

/// A second copy is governed as a request of its own: a key that admits
/// one request at a time, which its first holds, has no room for it, and
/// one that admits two has (ADR 0086); a key whose delay has not passed
/// holds it until it has.
///
/// Covers: story:3-63
#[rstest]
#[case::one_at_a_time(Rule { cap: 1, delay: None }, "full")]
#[case::two_at_a_time(Rule { cap: 2, delay: None }, "taken")]
#[case::within_the_delay(Rule { cap: 2, delay: Some(Duration::from_millis(500)) }, "until +500ms")]
fn a_second_copy_takes_a_permit_of_its_key_beside_its_first(
    #[case] rule: Rule,
    #[case] expected: &str,
) {
    let room = on_the_runtime(async |owner| {
        let copies = beside_a_first(owner, rule, None).await;
        let now = Instant::now();
        named(&copies.take("shop.test", now).await, now)
    });

    assert_eq!(room, expected);
}

/// A second copy takes its share of the Run's rate cap, as its first did:
/// at 60 a minute, a copy sent with its first waits a second for its turn,
/// and goes once the second passed.
///
/// Covers: story:3-63
#[test]
fn a_second_copy_takes_its_share_of_the_rate_cap() {
    let rooms = on_the_runtime(async |owner| {
        let start = Instant::now();
        let mut rate = RateCap::per_minute(60);
        rate.admit(start).unwrap();
        let copies = beside_a_first(
            owner,
            Rule {
                cap: 2,
                delay: None,
            },
            Some(rate),
        )
        .await;
        [
            named(&copies.take("shop.test", start).await, start),
            named(
                &copies
                    .take("shop.test", start + Duration::from_secs(1))
                    .await,
                start,
            ),
        ]
    });

    assert_eq!(rooms, ["until +1s", "taken"]);
}

/// The room a copy took goes back once it ended, so a key that admits two
/// requests has room for another copy beside the first.
///
/// Covers: story:3-63
#[test]
fn a_second_copy_given_back_leaves_room_for_the_next() {
    let rooms = on_the_runtime(async |owner| {
        let copies = beside_a_first(
            owner,
            Rule {
                cap: 2,
                delay: None,
            },
            None,
        )
        .await;
        let now = Instant::now();
        let first_copy = copies.take("shop.test", now).await;
        let while_held = named(&copies.take("shop.test", now).await, now);
        let Room::Taken(first_copy) = first_copy else {
            panic!("no room for the first copy");
        };
        copies.give_back(first_copy).await;
        [while_held, named(&copies.take("shop.test", now).await, now)]
    });

    assert_eq!(rooms, ["full", "taken"]);
}

/// A Politeness file whose every take fails: its rule's state cannot be
/// read, a storage error.
struct Unreadable;

/// A key's state whose every read raises a storage error.
struct UnreadableState;

impl KeyState for UnreadableState {
    type Key = str;
    type Reads = ();
    type Writes = ();

    fn load(_: &Connection, _: &str, (): ()) -> rusqlite::Result<Self> {
        Err(rusqlite::Error::InvalidQuery)
    }

    fn save(&self, _: &Connection, _: &str, (): &()) -> rusqlite::Result<()> {
        Ok(())
    }

    fn load_from(_: &MemoryTables, _: &str, (): ()) -> Self {
        Self
    }

    fn save_to(&self, _: &mut MemoryTables, _: &str, (): &()) {}
}

impl PermitRule for Unreadable {
    type State = UnreadableState;

    fn reads(&self, _: Step) {}

    fn admit(&self, _: &UnreadableState, _: u64, _: SystemTime) -> Verdict {
        Verdict::Go
    }

    fn started(&self, _: &mut UnreadableState, _: SystemTime) {}

    fn ended(
        &self,
        _: &mut UnreadableState,
        _: u64,
        _: Ending,
        _: SystemTime,
    ) -> (Option<SystemTime>, ()) {
        (None, ())
    }
}

/// A gate of one key, `shop.test`, ready until held, which keeps each
/// hold it is given.
#[derive(Default)]
struct OneKey {
    holds: Arc<Mutex<Vec<(Instant, Held)>>>,
}

impl Gate for OneKey {
    fn key(&self, _: &str) -> String {
        "shop.test".to_owned()
    }

    fn keys(&self) -> Keys {
        Keys::new(|_| "shop.test".to_owned())
    }

    fn bound(&self) -> usize {
        1
    }

    fn enqueue(&mut self, _: Waiting) -> bool {
        false
    }

    fn ready_at(&self, now: Instant) -> Option<Instant> {
        let holds = self.holds.lock().unwrap();
        Some(holds.last().map_or(now, |(until, _)| *until))
    }

    fn admit(&mut self, _: Instant) -> Option<Admitted> {
        None
    }

    fn next_key(&self, _: Instant) -> Option<String> {
        Some("shop.test".to_owned())
    }

    fn hold(&mut self, _: &str, until: Instant, held: Held) {
        self.holds.lock().unwrap().push((until, held));
    }

    fn wake(&mut self, _: &str) {}

    fn rule(&self, _: &Resolved) -> Option<SharedRule> {
        Some(SharedRule::new(Unreadable))
    }

    fn ended(&mut self, _: &str, _: Instant, _: Ended) {}

    fn journal(&mut self, _: SystemTime, _: Instant) -> Option<Journal> {
        None
    }
}

/// A key the Politeness file refuses a take for, as when the file is busy
/// past its timeout, is held for that refusal until its Run tries it
/// again, never as a key at its concurrency, so the Run's progress names
/// the file, not `politeness.domain.concurrency`.
///
/// Covers: story:3-63
#[test]
fn a_key_the_politeness_file_refuses_is_held_for_the_refusal() {
    let held = on_the_runtime(async |owner| {
        let gate = OneKey::default();
        let holds = Arc::clone(&gate.holds);
        let settings = resolve(&Layers::default(), Product::Foxhound, Product::Foxhound).unwrap();
        // A file, not a Run's own store: only a file's reads can fail.
        let folder = tempfile::tempdir().unwrap();
        let politeness = Politeness::open(
            folder.path().join("politeness.sqlite"),
            &PolitenessOptions::default(),
            Vec::new(),
            |_, _| false,
            "engine".to_owned(),
            owner,
        )
        .await
        .unwrap();
        let holder = Holder {
            run_id: "crawl".to_owned(),
            epoch: 1,
            run_folder: "crawl".into(),
            delegates: true,
        };
        let clock = Clock::paused(SystemTime::UNIX_EPOCH + Duration::from_hours(500_000));
        let mut admission = Admission::new(Box::new(gate), &settings).sharing(
            (politeness, None),
            holder,
            clock,
            &settings,
        );
        let now = Instant::now();
        let admitted = admission.admit(now).await;
        let held: Vec<(Duration, Held)> = holds
            .lock()
            .unwrap()
            .iter()
            .map(|(until, held)| (until.duration_since(now), *held))
            .collect();
        (admitted, held)
    });

    assert_eq!(
        held,
        (None, vec![(Duration::from_secs(30), Held::PolitenessFile)])
    );
}

/// What a key admits in the refusal test: any request, its ending holding
/// the key for half a second, a wait within any Run's bound.
struct HalfASecond;

impl PermitRule for HalfASecond {
    type State = Stateless;

    fn reads(&self, _: Step) {}

    fn admit(&self, _: &Stateless, _: u64, _: SystemTime) -> Verdict {
        Verdict::Go
    }

    fn started(&self, _: &mut Stateless, _: SystemTime) {}

    fn ended(
        &self,
        _: &mut Stateless,
        _: u64,
        _: Ending,
        now: SystemTime,
    ) -> (Option<SystemTime>, ()) {
        (Some(now + Duration::from_millis(500)), ())
    }
}

/// A gate of one key, `shop.test`, whose requests wait in order under
/// `rule`, ready unless held until a wake, which keeps each hold it is
/// given.
struct InOrder {
    waiting: std::collections::VecDeque<&'static str>,
    holds: Arc<Mutex<Vec<(Instant, Held)>>>,
    rule: SharedRule,
    held_until: Option<Instant>,
}

impl InOrder {
    /// The requests `waiting` of a key under `rule`, each hold kept in
    /// `holds`.
    fn new(
        waiting: &[&'static str],
        rule: SharedRule,
        holds: &Arc<Mutex<Vec<(Instant, Held)>>>,
    ) -> Self {
        Self {
            waiting: waiting.iter().copied().collect(),
            holds: Arc::clone(holds),
            rule,
            held_until: None,
        }
    }
}

impl Gate for InOrder {
    fn key(&self, _: &str) -> String {
        "shop.test".to_owned()
    }

    fn keys(&self) -> Keys {
        Keys::new(|_| "shop.test".to_owned())
    }

    fn bound(&self) -> usize {
        3
    }

    fn enqueue(&mut self, _: Waiting) -> bool {
        false
    }

    fn ready_at(&self, now: Instant) -> Option<Instant> {
        self.waiting
            .front()
            .map(|_| self.held_until.map_or(now, |until| until.max(now)))
    }

    fn admit(&mut self, _: Instant) -> Option<Admitted> {
        let id = self.waiting.pop_front()?;
        Some(Admitted {
            id: id.to_owned(),
            key: "shop.test".to_owned(),
            drained: self.waiting.is_empty(),
            refused_until: None,
        })
    }

    fn next_key(&self, _: Instant) -> Option<String> {
        self.waiting.front().map(|_| "shop.test".to_owned())
    }

    fn hold(&mut self, _: &str, until: Instant, held: Held) {
        self.holds.lock().unwrap().push((until, held));
        self.held_until = Some(until);
    }

    fn wake(&mut self, _: &str) {
        self.held_until = None;
    }

    fn rule(&self, _: &Resolved) -> Option<SharedRule> {
        Some(self.rule.clone())
    }

    fn ended(&mut self, key: &str, _: Instant, ended: Ended) {
        if let Ended::Until(until) = ended {
            self.hold(key, until, Held::BackOff);
        }
    }

    fn journal(&mut self, _: SystemTime, _: Instant) -> Option<Journal> {
        None
    }
}

/// A key its Run refuses until a moment, its site having asked, when it
/// asked, for a time further away than the Run waits, ends each request it
/// admits before then unsent however late, naming the time the site asked,
/// though the Politeness it shares would by then only hold the key; its
/// ending holds the key for nothing, and once the moment passed its
/// requests go (ADR 0118).
///
/// Covers: story:3-160, setting:politeness.retry_after.max
#[test]
fn a_key_its_run_refuses_ends_its_requests_unsent_until_the_moment_however_late() {
    let start = SystemTime::UNIX_EPOCH + Duration::from_hours(500_000);
    let asked = start + Duration::from_secs(2);
    let (admitted, holds) = on_the_runtime(async |owner| {
        let holds = Arc::new(Mutex::new(Vec::new()));
        let gate = InOrder::new(&["1", "2", "3"], SharedRule::new(HalfASecond), &holds);
        let settings = resolve(&Layers::default(), Product::Foxhound, Product::Foxhound).unwrap();
        let politeness = Politeness::in_memory(&PolitenessOptions::default(), owner).unwrap();
        let holder = Holder {
            run_id: "crawl".to_owned(),
            epoch: 1,
            run_folder: "crawl".into(),
            delegates: true,
        };
        let clock = Clock::paused(start);
        let mut admission = Admission::new(Box::new(gate), &settings).sharing(
            (politeness, None),
            holder,
            clock,
            &settings,
        );
        let now = Instant::now();
        let first = admission.admit(now).await.unwrap();
        let mut admitted = vec![(first.id.clone(), first.refused_until)];
        admission.refuse("shop.test", now + Duration::from_secs(2), asked);
        admission
            .ended(
                &first.id,
                "shop.test",
                now,
                Ended::BackOff(Some(now + Duration::from_secs(2))),
            )
            .await;
        for at in [Duration::from_millis(1_999), Duration::from_secs(2)] {
            let next = admission.admit(now + at).await.unwrap();
            admitted.push((next.id, next.refused_until));
        }
        let holds: Vec<Held> = holds
            .lock()
            .unwrap()
            .iter()
            .map(|(_, held)| *held)
            .collect();
        (admitted, holds)
    });

    assert_eq!(
        (admitted, holds),
        (
            vec![
                ("1".to_owned(), None),
                ("2".to_owned(), Some(asked)),
                ("3".to_owned(), None),
            ],
            Vec::new()
        )
    );
}

/// A Run whose own requests in flight fill its key's concurrency asks the
/// Politeness file once for its next request, which makes it the key's
/// waiter in turn (ADR 0053), and holds the key without asking again until
/// one of them ends, whatever wakes it meanwhile: with
/// `politeness.delegation: false`, one request at a time costs its take,
/// one busy take and its give back (ADR 0140's E6, #1899).
///
/// Covers: story:3-63, setting:politeness.domain.concurrency
#[test]
fn a_run_whose_own_requests_fill_its_keys_concurrency_asks_the_file_once_meanwhile() {
    let work = CountedWork::default();
    let (admitted, holds) = on_the_runtime(async |owner| {
        let holds = Arc::new(Mutex::new(Vec::new()));
        let one_at_a_time = SharedRule::new(Rule {
            cap: 1,
            delay: None,
        });
        let gate = InOrder::new(&["1", "2"], one_at_a_time, &holds);
        let settings = resolve(&Layers::default(), Product::Foxhound, Product::Foxhound).unwrap();
        let folder = tempfile::tempdir().unwrap();
        let politeness = Politeness::open(
            folder.path().join("politeness.sqlite"),
            &PolitenessOptions {
                work: work.clone(),
                ..PolitenessOptions::default()
            },
            Vec::new(),
            |_, _| false,
            "engine".to_owned(),
            owner,
        )
        .await
        .unwrap();
        let holder = Holder {
            run_id: "crawl".to_owned(),
            epoch: 1,
            run_folder: "crawl".into(),
            delegates: false,
        };
        let clock = Clock::paused(SystemTime::UNIX_EPOCH + Duration::from_hours(500_000));
        let mut admission = Admission::new(Box::new(gate), &settings).sharing(
            (politeness, None),
            holder,
            clock,
            &settings,
        );
        let now = Instant::now();
        let first = admission.admit(now).await.unwrap();
        let while_in_flight = admission.admit(now).await.map(|admitted| admitted.id);
        admission.wake(Some("shop.test"));
        let woken_in_flight = admission.admit(now).await.map(|admitted| admitted.id);
        admission
            .ended(&first.id, "shop.test", now, Ended::Done)
            .await;
        let next = admission.admit(now).await.map(|admitted| admitted.id);
        let holds: Vec<Held> = holds
            .lock()
            .unwrap()
            .iter()
            .map(|(_, held)| *held)
            .collect();
        (
            [Some(first.id), while_in_flight, woken_in_flight, next],
            holds,
        )
    });

    assert_eq!(
        (admitted, holds),
        (
            [Some("1".to_owned()), None, None, Some("2".to_owned())],
            vec![Held::Concurrency, Held::Concurrency]
        )
    );
    assert_eq!(
        work.read(Row::PolitenessTransactions),
        4,
        "a take, one busy take, its give back and the next take"
    );
}

/// A Run whose own request in flight fills its key's concurrency keeps the
/// turn its next request began to wait for, ahead of another engine's Run
/// that waits after it: the give back of its request wakes it, and its
/// next request is admitted while the other's still waits (ADR 0053's
/// turns, #1899).
///
/// Covers: story:3-81, setting:politeness.domain.concurrency
#[test]
fn a_run_whose_own_request_fills_its_key_keeps_its_turn_ahead_of_a_later_waiter() {
    let admitted = on_the_runtime(async |owner| {
        let folder = tempfile::tempdir().unwrap();
        let one_at_a_time = SharedRule::new(Rule {
            cap: 1,
            delay: None,
        });
        let settings = resolve(&Layers::default(), Product::Foxhound, Product::Foxhound).unwrap();
        let clock = Clock::paused(SystemTime::UNIX_EPOCH + Duration::from_hours(500_000));
        let mut admissions = Vec::new();
        for (engine, waiting) in [("first", &["1", "2"][..]), ("second", &["3"][..])] {
            let politeness = Politeness::open(
                folder.path().join("politeness.sqlite"),
                &PolitenessOptions::default(),
                Vec::new(),
                |_, _| false,
                engine.to_owned(),
                owner,
            )
            .await
            .unwrap();
            let holder = Holder {
                run_id: engine.to_owned(),
                epoch: 1,
                run_folder: engine.into(),
                delegates: false,
            };
            let gate = InOrder::new(waiting, one_at_a_time.clone(), &Arc::default());
            admissions.push(Admission::new(Box::new(gate), &settings).sharing(
                (politeness, None),
                holder,
                clock,
                &settings,
            ));
        }
        let [mut first, mut second] = <[Admission; 2]>::try_from(admissions).unwrap();
        let now = Instant::now();
        let one = first.admit(now).await.unwrap();
        let first_waits = first.admit(now).await.map(|admitted| admitted.id);
        let second_waits = second.admit(now).await.map(|admitted| admitted.id);
        first.ended(&one.id, "shop.test", now, Ended::Done).await;
        let first_next = first.admit(now).await.map(|admitted| admitted.id);
        second.wake(Some("shop.test"));
        let second_next = second.admit(now).await.map(|admitted| admitted.id);
        [
            Some(one.id),
            first_waits,
            second_waits,
            first_next,
            second_next,
        ]
    });

    assert_eq!(
        admitted,
        [Some("1".to_owned()), None, None, Some("2".to_owned()), None]
    );
}
