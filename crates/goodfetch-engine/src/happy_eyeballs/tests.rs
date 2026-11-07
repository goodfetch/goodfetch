//! The race as a pure function of the `Clock` and the answers and attempt
//! outcomes it is given (ADR 0118; RFC 8305): simulated attempts on Tokio's
//! paused clock, so every moment is exact and no socket opens.
#![expect(
    clippy::doc_markdown,
    reason = "`Covers:` lines name Feature IDs, which are no prose (ADR 0072)"
)]
#![expect(
    clippy::disallowed_methods,
    reason = "the race runs on the Clock's test version (ADR 0059)"
)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::rc::Rc;
use std::time::{Duration, SystemTime};

use goodfetch_protocol::dns::{IpFamily, NameResolver};
use proptest::prelude::*;

use super::{Delays, Families, Race, RaceError, Won};
use crate::call_error::CallError;
use crate::clock::Clock;
use crate::dns::LookupError;

const NAME: &str = "dual.proving.test";
const PORT: u16 = 443;
const MILLISECOND: Duration = Duration::from_millis(1);
/// How long an attempt to an address that drops packets takes to fail, as
/// a connect timeout would.
const DROPPED: Duration = Duration::from_secs(75);

fn ip(text: &str) -> IpAddr {
    text.parse().unwrap()
}

fn socket(text: &str) -> SocketAddr {
    SocketAddr::new(ip(text), PORT)
}

/// How a simulated attempt to one address ends.
#[derive(Clone, Copy, Debug)]
enum Attempt {
    Connects(Duration),
    Fails(Duration),
}

/// An answer that arrives `after` the race starts.
#[derive(Clone, Debug)]
struct Answer {
    after: Duration,
    addresses: Result<Vec<IpAddr>, LookupError>,
}

fn answer(after_milliseconds: u64, addresses: &[&str]) -> Answer {
    Answer {
        after: Duration::from_millis(after_milliseconds),
        addresses: Ok(addresses.iter().map(|address| ip(address)).collect()),
    }
}

fn not_resolved() -> Answer {
    Answer {
        after: Duration::ZERO,
        addresses: Err(LookupError::NameNotResolved {
            name: NAME.to_owned(),
            resolver: NameResolver::Engine,
        }),
    }
}

/// What a race did: when each attempt started, from the race's start, and
/// how it ended, with when.
#[derive(Debug)]
struct Run {
    started: Vec<(Duration, SocketAddr)>,
    outcome: Result<(Duration, SocketAddr), RaceError>,
}

/// Runs a race on the paused clock with `delays` (RFC 8305's when `None`).
fn run(
    families: Families,
    ipv6: Answer,
    ipv4: Answer,
    attempts: &BTreeMap<SocketAddr, Attempt>,
    delays: Option<Delays>,
) -> Run {
    goodfetch_test_support::runtime().block_on(async {
        let clock = Clock::paused(SystemTime::UNIX_EPOCH);
        let begun = clock.now();
        let race = Race {
            clock,
            delays: delays.unwrap_or_default(),
            families,
        };
        let started = Rc::new(RefCell::new(Vec::new()));
        let answered = |answer: Answer| async move {
            clock.sleep(answer.after).await;
            answer.addresses
        };
        let record = Rc::clone(&started);
        let outcome = race
            .connect(
                NAME,
                PORT,
                answered(ipv6),
                answered(ipv4),
                move |address: SocketAddr| {
                    record
                        .borrow_mut()
                        .push((clock.now().duration_since(begun), address));
                    let attempt = attempts.get(&address).copied();
                    async move {
                        match attempt {
                            Some(Attempt::Connects(after)) => {
                                clock.sleep(after).await;
                                Ok(address)
                            }
                            Some(Attempt::Fails(after)) => {
                                clock.sleep(after).await;
                                Err(io::Error::from(io::ErrorKind::ConnectionRefused))
                            }
                            None => Err(io::Error::from(io::ErrorKind::ConnectionRefused)),
                        }
                    }
                },
            )
            .await;
        let started = started.borrow().clone();
        Run {
            started,
            outcome: outcome.map(
                |Won {
                     connection,
                     address,
                 }| {
                    assert_eq!(connection, address);
                    (clock.now().duration_since(begun), address)
                },
            ),
        }
    })
}

fn attempts<const N: usize>(entries: [(&str, Attempt); N]) -> BTreeMap<SocketAddr, Attempt> {
    entries
        .into_iter()
        .map(|(address, attempt)| (socket(address), attempt))
        .collect()
}

/// RFC 8305 section 5, the pitfall H07 ADR 0118 answers.
///
/// Covers: story:3-148, setting:resolver.attempt_delay
#[test]
fn an_ipv6_address_that_drops_packets_hands_over_to_ipv4_after_the_attempt_delay() {
    let run = run(
        Families::Both,
        answer(0, &["2001:db8::1"]),
        answer(0, &["192.0.2.1"]),
        &attempts([
            ("2001:db8::1", Attempt::Fails(DROPPED)),
            ("192.0.2.1", Attempt::Connects(MILLISECOND)),
        ]),
        None,
    );

    assert_eq!(
        run.started,
        [
            (Duration::ZERO, socket("2001:db8::1")),
            (Duration::from_millis(250), socket("192.0.2.1")),
        ]
    );
    assert_eq!(
        run.outcome.unwrap(),
        (Duration::from_millis(251), socket("192.0.2.1"))
    );
}

/// Covers: story:3-148, setting:resolver.attempt_delay
#[test]
fn the_attempt_delay_is_the_setting_given() {
    let run = run(
        Families::Both,
        answer(0, &["2001:db8::1"]),
        answer(0, &["192.0.2.1"]),
        &attempts([
            ("2001:db8::1", Attempt::Fails(DROPPED)),
            ("192.0.2.1", Attempt::Connects(MILLISECOND)),
        ]),
        Some(Delays {
            resolution: Duration::from_millis(50),
            attempt: Duration::from_millis(100),
        }),
    );

    assert_eq!(
        run.started[1],
        (Duration::from_millis(100), socket("192.0.2.1"))
    );
}

#[test]
fn a_failed_attempt_starts_the_next_at_once() {
    let run = run(
        Families::Both,
        answer(0, &["2001:db8::1"]),
        answer(0, &["192.0.2.1"]),
        &attempts([
            ("2001:db8::1", Attempt::Fails(Duration::from_millis(10))),
            ("192.0.2.1", Attempt::Connects(MILLISECOND)),
        ]),
        None,
    );

    assert_eq!(
        run.started,
        [
            (Duration::ZERO, socket("2001:db8::1")),
            (Duration::from_millis(10), socket("192.0.2.1")),
        ]
    );
}

/// RFC 8305 section 3.
///
/// Covers: story:3-148, setting:resolver.resolution_delay
#[test]
fn ipv4_answered_first_waits_the_resolution_delay_for_ipv6() {
    let run = run(
        Families::Both,
        answer(30, &["2001:db8::1"]),
        answer(0, &["192.0.2.1"]),
        &attempts([("2001:db8::1", Attempt::Connects(MILLISECOND))]),
        None,
    );

    assert_eq!(
        run.started,
        [(Duration::from_millis(30), socket("2001:db8::1"))]
    );
}

/// Covers: story:3-148, setting:resolver.resolution_delay
#[test]
fn ipv6_answered_after_the_resolution_delay_joins_the_race_in_its_turn() {
    let run = run(
        Families::Both,
        answer(80, &["2001:db8::1"]),
        answer(0, &["192.0.2.1"]),
        &attempts([
            ("192.0.2.1", Attempt::Fails(DROPPED)),
            ("2001:db8::1", Attempt::Connects(MILLISECOND)),
        ]),
        None,
    );

    assert_eq!(
        run.started,
        [
            (Duration::from_millis(50), socket("192.0.2.1")),
            (Duration::from_millis(300), socket("2001:db8::1")),
        ]
    );
}

#[test]
fn ipv6_answered_first_starts_at_once() {
    let run = run(
        Families::Both,
        answer(10, &["2001:db8::1"]),
        answer(40, &["192.0.2.1"]),
        &attempts([("2001:db8::1", Attempt::Connects(MILLISECOND))]),
        None,
    );

    assert_eq!(
        run.started,
        [(Duration::from_millis(10), socket("2001:db8::1"))]
    );
}

/// RFC 8305 section 4.
#[test]
fn the_attempts_alternate_between_the_families_from_ipv6() {
    let run = run(
        Families::Both,
        answer(0, &["2001:db8::1", "2001:db8::2"]),
        answer(0, &["192.0.2.1", "192.0.2.2"]),
        &attempts([
            ("2001:db8::1", Attempt::Fails(DROPPED)),
            ("2001:db8::2", Attempt::Fails(DROPPED)),
            ("192.0.2.1", Attempt::Fails(DROPPED)),
            ("192.0.2.2", Attempt::Connects(MILLISECOND)),
        ]),
        None,
    );

    let order: Vec<_> = run.started.iter().map(|(_, address)| *address).collect();
    assert_eq!(
        order,
        [
            socket("2001:db8::1"),
            socket("192.0.2.1"),
            socket("2001:db8::2"),
            socket("192.0.2.2"),
        ]
    );
}

/// ADR 0044 through ADR 0118.
///
/// Covers: story:3-148
#[test]
fn a_session_that_holds_one_family_never_tries_the_other() {
    let run = run(
        Families::Only(IpFamily::Ipv4),
        answer(0, &["2001:db8::1"]),
        answer(0, &["192.0.2.1"]),
        &attempts([
            ("2001:db8::1", Attempt::Connects(MILLISECOND)),
            ("192.0.2.1", Attempt::Connects(MILLISECOND)),
        ]),
        None,
    );

    assert_eq!(run.started, [(Duration::ZERO, socket("192.0.2.1"))]);
}

/// Covers: story:3-148, error:address_family_unavailable
#[test]
fn a_host_with_addresses_only_in_the_family_the_session_lacks_is_unavailable() {
    let run = run(
        Families::Only(IpFamily::Ipv6),
        not_resolved(),
        answer(0, &["192.0.2.1"]),
        &attempts([("192.0.2.1", Attempt::Connects(MILLISECOND))]),
        None,
    );

    let Err(RaceError::FamilyUnavailable(unavailable)) = run.outcome else {
        panic!("the race was not refused: {run:?}");
    };
    assert_eq!(run.started, []);
    insta::assert_debug_snapshot!(CallError::from(unavailable));
}

/// RFC 8305 section 9: the caller then forgets the name's answers.
///
/// Covers: story:3-147
#[test]
fn a_race_whose_every_attempt_failed_names_every_address_tried() {
    let run = run(
        Families::Both,
        answer(0, &["2001:db8::1"]),
        answer(0, &["192.0.2.1"]),
        &BTreeMap::new(),
        None,
    );

    let Err(RaceError::Unreachable { tried }) = run.outcome else {
        panic!("the race did not fail as unreachable: {run:?}");
    };
    assert_eq!(tried, [socket("2001:db8::1"), socket("192.0.2.1")]);
}

#[rstest::rstest]
#[case::no_name_server_answered(
    LookupError::LookupFailed { name: NAME.to_owned(), resolver: NameResolver::Engine },
    LookupError::LookupFailed { name: NAME.to_owned(), resolver: NameResolver::Engine },
)]
#[case::the_name_has_no_address(
    LookupError::NameNotResolved { name: NAME.to_owned(), resolver: NameResolver::Engine },
    LookupError::NameNotResolved { name: NAME.to_owned(), resolver: NameResolver::Engine },
)]
fn a_host_with_no_address_fails_with_its_lookups_error(
    #[case] lookup: LookupError,
    #[case] expected: LookupError,
) {
    let failed = Answer {
        after: Duration::ZERO,
        addresses: Err(lookup),
    };

    let run = run(
        Families::Both,
        not_resolved(),
        failed,
        &BTreeMap::new(),
        None,
    );

    assert_eq!(
        run.outcome.map(|(_, address)| address),
        Err(RaceError::Lookup(expected))
    );
}

/// The moments a run's answers and attempts take, in milliseconds.
fn attempt() -> impl Strategy<Value = Attempt> {
    prop_oneof![
        (1_u64..400).prop_map(|after| Attempt::Connects(Duration::from_millis(after))),
        (1_u64..400).prop_map(|after| Attempt::Fails(Duration::from_millis(after))),
        Just(Attempt::Fails(DROPPED)),
    ]
}

fn families() -> impl Strategy<Value = Families> {
    prop_oneof![
        Just(Families::Both),
        Just(Families::Only(IpFamily::Ipv4)),
        Just(Families::Only(IpFamily::Ipv6)),
    ]
}

/// A family's answer: none, or one to three of `addresses`, after up to
/// 200 ms.
fn answer_of(addresses: [&'static str; 3]) -> impl Strategy<Value = Answer> {
    (0_u64..200, 0_usize..=3).prop_map(move |(after, count)| {
        if count == 0 {
            Answer {
                after: Duration::from_millis(after),
                ..not_resolved()
            }
        } else {
            answer(after, &addresses[..count])
        }
    })
}

const IPV6: [&str; 3] = ["2001:db8::1", "2001:db8::2", "2001:db8::3"];
const IPV4: [&str; 3] = ["192.0.2.1", "192.0.2.2", "192.0.2.3"];

fn outcomes() -> impl Strategy<Value = BTreeMap<SocketAddr, Attempt>> {
    proptest::collection::vec(attempt(), 6).prop_map(|attempts| {
        IPV6.iter()
            .chain(&IPV4)
            .map(|address| socket(address))
            .zip(attempts)
            .collect()
    })
}

fn holds(families: Families, address: SocketAddr) -> bool {
    match families {
        Families::Both => true,
        Families::Only(IpFamily::Ipv4) => address.is_ipv4(),
        Families::Only(IpFamily::Ipv6) => address.is_ipv6(),
    }
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-148
    #[test]
    fn a_race_tries_only_held_addresses_each_once_and_connects_if_one_can(
        families in families(),
        ipv6 in answer_of(IPV6),
        ipv4 in answer_of(IPV4),
        outcomes in outcomes(),
    ) {
        let offered: Vec<SocketAddr> = [&ipv6, &ipv4]
            .into_iter()
            .flat_map(|answer| answer.addresses.clone().unwrap_or_default())
            .map(|address| SocketAddr::new(address, PORT))
            .filter(|address| holds(families, *address))
            .collect();

        let run = run(families, ipv6, ipv4, &outcomes, None);

        let mut tried: Vec<_> = run.started.iter().map(|(_, address)| *address).collect();
        prop_assert!(tried.iter().all(|address| offered.contains(address)));
        tried.sort();
        tried.dedup();
        prop_assert_eq!(tried.len(), run.started.len());
        let connectable = offered
            .iter()
            .any(|address| matches!(outcomes[address], Attempt::Connects(_)));
        prop_assert_eq!(run.outcome.is_ok(), connectable);
    }

    /// Covers: story:3-148, setting:resolver.attempt_delay
    #[test]
    fn a_new_attempt_waits_the_attempt_delay_unless_one_failed_meanwhile(
        ipv6 in answer_of(IPV6),
        ipv4 in answer_of(IPV4),
        outcomes in outcomes(),
    ) {
        let run = run(Families::Both, ipv6, ipv4, &outcomes, None);

        let failed_at: Vec<Duration> = run
            .started
            .iter()
            .filter_map(|(start, address)| match outcomes[address] {
                Attempt::Fails(after) => Some(*start + after),
                Attempt::Connects(_) => None,
            })
            .collect();
        for pair in run.started.windows(2) {
            let (previous, next) = (pair[0].0, pair[1].0);
            prop_assert!(
                next.saturating_sub(previous) >= Duration::from_millis(250)
                    || failed_at.iter().any(|failed| previous <= *failed && *failed <= next),
                "{:?}",
                run.started
            );
        }
    }

    /// Covers: story:3-148, setting:resolver.resolution_delay
    #[test]
    fn the_first_attempt_waits_for_ipv6_at_most_the_resolution_delay(
        ipv6 in answer_of(IPV6),
        ipv4 in answer_of(IPV4),
        outcomes in outcomes(),
    ) {
        let answered = |answer: &Answer| answer.addresses.as_ref().is_ok_and(|found| !found.is_empty());
        let latest = [
            answered(&ipv6).then_some(ipv6.after),
            answered(&ipv4).then_some(ipv4.after + Duration::from_millis(50)),
        ]
        .into_iter()
        .flatten()
        .min();

        let run = run(Families::Both, ipv6, ipv4, &outcomes, None);

        if let (Some(latest), Some((first, _))) = (latest, run.started.first()) {
            prop_assert!(*first <= latest, "{:?}", run.started);
        }
    }
}
