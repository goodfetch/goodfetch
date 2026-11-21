//! The resolver's cache as a pure function of the answers it is given and
//! the moments it is asked at (ADR 0118; TESTING.md, Time): an answer kept
//! for its TTL, a negative one for its negative TTL, a failure never, and a
//! name forgotten whole.

use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::Duration;

use goodfetch_protocol::dns::IpFamily;
use proptest::prelude::*;
use tokio::time::Instant;

use super::{AnswerCache, Kept, Outcome};
use crate::clock::Clock;

const NAME: &str = "moving.proving.test";
const ADDRESS: IpAddr = IpAddr::V4(Ipv4Addr::LOCALHOST);

/// A moment to keep answers at; the cache only compares moments.
fn start() -> Instant {
    Clock::system().now()
}

fn family() -> impl Strategy<Value = IpFamily> {
    prop_oneof![Just(IpFamily::Ipv4), Just(IpFamily::Ipv6)]
}

/// Covers: story:3-147
#[test]
fn an_answer_expires_at_its_ttl() {
    let mut cache = AnswerCache::default();
    let now = start();
    let ttl = Duration::from_secs(60);
    cache.keep(
        NAME,
        IpFamily::Ipv4,
        Outcome::Addresses {
            addresses: vec![ADDRESS],
            ttl,
        },
        now,
    );

    let kept = [
        cache.kept(NAME, IpFamily::Ipv4, now + ttl - Duration::from_millis(1)),
        cache.kept(NAME, IpFamily::Ipv4, now + ttl),
    ];

    assert_eq!(kept, [Some(Kept::Addresses(vec![ADDRESS])), None]);
}

#[test]
fn a_name_is_kept_whatever_its_case_and_trailing_dot() {
    let mut cache = AnswerCache::default();
    let now = start();
    cache.keep(
        "Moving.Proving.Test.",
        IpFamily::Ipv4,
        Outcome::Addresses {
            addresses: vec![ADDRESS],
            ttl: Duration::from_secs(60),
        },
        now,
    );

    assert_eq!(
        cache.kept(NAME, IpFamily::Ipv4, now),
        Some(Kept::Addresses(vec![ADDRESS]))
    );
}

#[test]
fn a_negative_answer_without_an_soa_is_not_kept() {
    let mut cache = AnswerCache::default();
    let now = start();

    cache.keep(
        NAME,
        IpFamily::Ipv6,
        Outcome::Absent { negative_ttl: None },
        now,
    );

    assert_eq!(cache.kept(NAME, IpFamily::Ipv6, now), None);
}

proptest! {
    #![proptest_config(goodfetch_test_support::proptest_config())]

    /// Covers: story:3-147
    #[test]
    fn an_answer_is_kept_for_its_ttl_and_no_longer(
        ttl in 0_u64..172_800_000,
        asked_after in 0_u64..345_600_000,
        family in family(),
    ) {
        let mut cache = AnswerCache::default();
        let now = start();
        cache.keep(
            NAME,
            family,
            Outcome::Addresses { addresses: vec![ADDRESS], ttl: Duration::from_millis(ttl) },
            now,
        );

        let kept = cache.kept(NAME, family, now + Duration::from_millis(asked_after));

        prop_assert_eq!(kept.is_some(), asked_after < ttl);
    }

    /// Covers: story:3-147
    #[test]
    fn a_negative_answer_is_kept_for_its_negative_ttl_and_no_longer(
        negative_ttl in 0_u64..172_800_000,
        asked_after in 0_u64..345_600_000,
        family in family(),
    ) {
        let mut cache = AnswerCache::default();
        let now = start();
        cache.keep(
            NAME,
            family,
            Outcome::Absent { negative_ttl: Some(Duration::from_millis(negative_ttl)) },
            now,
        );

        let kept = cache.kept(NAME, family, now + Duration::from_millis(asked_after));

        prop_assert_eq!(kept, (asked_after < negative_ttl).then_some(Kept::Absent));
    }

    /// Covers: story:3-147
    #[test]
    fn a_failure_is_never_kept(
        asked_after in 0_u64..60_000,
        family in family(),
    ) {
        let mut cache = AnswerCache::default();
        let now = start();
        cache.keep(NAME, family, Outcome::Failure, now);

        let kept = cache.kept(NAME, family, now + Duration::from_millis(asked_after));

        prop_assert_eq!(kept, None);
    }

    /// Covers: story:3-147
    #[test]
    fn a_forgotten_name_loses_both_families_and_no_other_name_loses_any(
        ttl in 1_u64..172_800_000,
    ) {
        let mut cache = AnswerCache::default();
        let now = start();
        let ttl = Duration::from_millis(ttl);
        for name in [NAME, "staying.proving.test"] {
            cache.keep(name, IpFamily::Ipv4, Outcome::Addresses { addresses: vec![ADDRESS], ttl }, now);
            cache.keep(
                name,
                IpFamily::Ipv6,
                Outcome::Addresses { addresses: vec![IpAddr::V6(Ipv6Addr::LOCALHOST)], ttl },
                now,
            );
        }

        cache.forget(NAME);

        prop_assert_eq!(
            [
                cache.kept(NAME, IpFamily::Ipv4, now).is_some(),
                cache.kept(NAME, IpFamily::Ipv6, now).is_some(),
                cache.kept("staying.proving.test", IpFamily::Ipv4, now).is_some(),
                cache.kept("staying.proving.test", IpFamily::Ipv6, now).is_some(),
            ],
            [false, false, true, true]
        );
    }

    #[test]
    fn a_full_cache_keeps_the_newest_answer(
        capacity in 1_usize..16,
        names in proptest::collection::vec("[a-z]{1,4}", 1..64),
    ) {
        let mut cache = AnswerCache::with_capacity(capacity);
        let now = start();
        let ttl = Duration::from_secs(60);
        for name in &names {
            cache.keep(name, IpFamily::Ipv4, Outcome::Addresses { addresses: vec![ADDRESS], ttl }, now);
        }

        let newest = names.last().expect("one name at least");
        prop_assert_eq!(
            cache.kept(newest, IpFamily::Ipv4, now),
            Some(Kept::Addresses(vec![ADDRESS]))
        );
    }
}
